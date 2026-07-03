//! [`OllamaModel`]: a [`ModelClient`] backed by an OpenAI-compatible Chat Completions
//! endpoint (Ollama at `http://localhost:11434/v1`, or any other OpenAI-shaped server).
//!
//! This mirrors [`AnthropicModel`](crate::model::AnthropicModel) but speaks the `OpenAI`
//! Chat Completions wire format instead of the Anthropic Messages format. Given the task
//! prompt, a snapshot of the current document, and the previous iteration's verifier
//! feedback, it asks the model for the next batch of [`AgentCommand`]s. The model returns
//! them through a single function tool, `emit_commands`, whose arguments are
//! `{ "commands": [ <AgentCommand>, ... ] }`; each element is exactly the frozen serde
//! form of [`AgentCommand`] (tagged by `op`), the same contract the Anthropic backend
//! uses. A JSON-array-in-text fallback covers a local model that answers in prose (or
//! ignores the forced `tool_choice`, which some local models do) instead of a tool call.
//!
//! # Configuration (environment only)
//!
//! - `RETICLE_MODEL_BASE_URL` overrides the base URL; default [`DEFAULT_OLLAMA_BASE_URL`]
//!   (`http://localhost:11434/v1`). The endpoint is `{base}/chat/completions`.
//! - `RETICLE_MODEL_NAME` sets the model id (for example `gpt-oss:16k` or
//!   `qwen2.5-coder:16k`). There is no hardcoded default model; when unset the id is a
//!   clearly labeled placeholder ([`UNSET_MODEL`]) and a call fails cleanly with
//!   [`BuildError::MissingModel`] rather than sending a bogus request.
//! - `RETICLE_MODEL_API_KEY` is optional (Ollama needs no key). When present it is held
//!   in an [`ApiKey`] and reaches the wire only as the `Authorization: Bearer <key>`
//!   header; it is never logged, serialized, or placed in [`Debug`], and it is scrubbed
//!   from every error/response string this module surfaces (see the [key handling] on
//!   [`AnthropicModel`](crate::model::AnthropicModel), reused verbatim here).
//!
//! [key handling]: crate::model::AnthropicModel
//!
//! # Response parsing
//!
//! Reads `choices[0].message.tool_calls[*].function.arguments`. In the `OpenAI` format
//! those `arguments` are a JSON **string**, not an object, so they are parsed a second
//! time into `{ "commands": [...] }`. The `emit_commands` call is preferred; if no tool
//! call is present, the fallback scans `choices[0].message.content` for a `[...]` array
//! of commands. On any transport, HTTP, or parse failure a scrubbed
//! [`last_error`](OllamaModel::last_error) is recorded and an empty batch is returned,
//! which the loop treats as "no proposal this turn", exactly like the Anthropic backend.
//!
//! # Determinism
//!
//! Local model outputs are **not** deterministic: the same prompt to `gpt-oss:16k` can
//! yield different command batches across runs, and nothing here pins a seed. This does
//! not touch the harness's determinism guarantee, which is about **transcript replay**:
//! replaying a recorded transcript re-applies the same commands and reproduces the same
//! `document_hash` regardless of which backend originally produced them. So a live local
//! run is non-reproducible at the proposal step, yet its recorded transcript still
//! replays bit-for-bit. Keep this distinction in mind when reading the benchmark chapter:
//! `backend = "ollama"` rows are not expected to match across runs; their transcripts,
//! once recorded, are.
//!
//! # Context window and summarization
//!
//! The binding constraint for a local model is a small context window (16k tokens with
//! the tool schema plus a growing transcript). `ConversationBuffer` accumulates the
//! running messages and, when the estimated token count nears the window, compacts the
//! older iterations into a single short summary message while keeping the latest
//! iteration verbatim. See `summarize_transcript` for the policy.

use std::cell::RefCell;

use reticle_agent_api::AgentCommand;
use reticle_bench::model::{Context, ModelClient};
use serde::Deserialize;

use crate::model::HttpTransport;
use crate::redact::ApiKey;

/// The default OpenAI-compatible base URL: a local Ollama server. The
/// `/chat/completions` path is appended.
pub const DEFAULT_OLLAMA_BASE_URL: &str = "http://localhost:11434/v1";

/// The model-id placeholder used when `RETICLE_MODEL_NAME` is unset. It is deliberately
/// not a real model: a call made with this id fails cleanly with
/// [`BuildError::MissingModel`] instead of sending a bogus request.
pub const UNSET_MODEL: &str = "<unset:set RETICLE_MODEL_NAME>";

/// Environment variable naming the base URL of the OpenAI-compatible endpoint.
pub const ENV_BASE_URL: &str = "RETICLE_MODEL_BASE_URL";

/// Environment variable naming the model id to request.
pub const ENV_MODEL_NAME: &str = "RETICLE_MODEL_NAME";

/// Environment variable holding an optional API key for endpoints that require one.
pub const ENV_API_KEY: &str = "RETICLE_MODEL_API_KEY";

/// The default token threshold at which `ConversationBuffer` compacts older
/// iterations. Chosen to leave headroom under a 16k-token window once the tool schema
/// and the reply are accounted for.
pub const DEFAULT_SUMMARIZE_THRESHOLD_TOKENS: usize = 12_000;

/// The output-token ceiling for a single proposal turn. A command batch is small (JSON
/// objects), so this is generous without risking a slow local generation from running
/// away.
const MAX_TOKENS: u32 = 4096;

/// A [`ModelClient`] that calls an OpenAI-compatible Chat Completions endpoint (Ollama by
/// default) to turn a task prompt plus the current document into a batch of
/// [`AgentCommand`]s.
///
/// Built with [`OllamaModel::from_env`], which reads the base URL, model id, and optional
/// key from the environment. The optional key is never stored anywhere printable; see the
/// [module docs](self).
pub struct OllamaModel {
    /// The optional API key. `None` for a keyless server (Ollama). When present it is
    /// never printed or serialized and reaches the wire only as `Authorization: Bearer`.
    key: Option<ApiKey>,
    /// Base URL of the endpoint (`/chat/completions` is appended).
    base_url: String,
    /// The model id to request. [`UNSET_MODEL`] until set from the environment or
    /// [`with_model`](Self::with_model).
    model: String,
    /// The stable client id recorded in the [`ResultRecord`]; the model id, so runs
    /// against different local models are distinguishable.
    ///
    /// [`ResultRecord`]: reticle_bench::ResultRecord
    id: String,
    /// A snapshot of the current document, injected by the loop before each call so the
    /// model sees the layout it is editing. Interior mutability keeps the
    /// [`ModelClient::propose`] receiver ergonomic while the field is set out of band.
    document_context: RefCell<String>,
    /// The running conversation buffer, compacted when it nears the context window.
    buffer: ConversationBuffer,
    /// The most recent transport or parse error, scrubbed of the key, for the loop to
    /// surface. `None` when the last call succeeded.
    last_error: Option<String>,
    /// The HTTP transport. Boxed behind the shared [`HttpTransport`] trait so tests
    /// inject a scripted transport with no network.
    transport: Box<dyn HttpTransport>,
}

impl std::fmt::Debug for OllamaModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately omit `key` and `transport`; `key` is a secret (even its presence
        // is reported only as a bool) and `transport` is not meaningfully printable.
        f.debug_struct("OllamaModel")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("id", &self.id)
            .field("has_key", &self.key.is_some())
            .field("last_error", &self.last_error)
            .finish_non_exhaustive()
    }
}

impl OllamaModel {
    /// Builds a model client from the environment.
    ///
    /// Reads [`ENV_BASE_URL`] (default [`DEFAULT_OLLAMA_BASE_URL`]), [`ENV_MODEL_NAME`]
    /// (the model id; left as [`UNSET_MODEL`] when absent, which makes a later call fail
    /// cleanly), and [`ENV_API_KEY`] (an optional key). This never fails: a missing model
    /// id is surfaced when a call is actually attempted, not at construction, so a caller
    /// can build the client and inspect it without the environment being complete.
    #[must_use]
    pub fn from_env() -> Self {
        let base_url = std::env::var(ENV_BASE_URL)
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_OLLAMA_BASE_URL.to_owned());
        let model = std::env::var(ENV_MODEL_NAME)
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| UNSET_MODEL.to_owned());
        let key = ApiKey::from_env_named(ENV_API_KEY);
        Self::build(key, base_url, model)
    }

    /// Builds a client from explicit parts and the real HTTP transport.
    fn build(key: Option<ApiKey>, base_url: String, model: String) -> Self {
        Self {
            key,
            base_url,
            id: model.clone(),
            model,
            document_context: RefCell::new(String::new()),
            buffer: ConversationBuffer::new(DEFAULT_SUMMARIZE_THRESHOLD_TOKENS),
            last_error: None,
            transport: Box::new(super::model::openai_ureq_transport()),
        }
    }

    /// Builds a client around explicit parts, for tests. Test-only.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn for_test(key: Option<ApiKey>, base_url: &str, model: &str) -> Self {
        Self::build(key, base_url.to_owned(), model.to_owned())
    }

    /// Overrides the base URL (for a non-default OpenAI-compatible endpoint).
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Overrides the model id (also the recorded client id).
    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        let model = model.into();
        self.id.clone_from(&model);
        self.model = model;
        self
    }

    /// Overrides the summarization threshold (tokens) of the conversation buffer.
    #[must_use]
    pub fn with_summarize_threshold(mut self, tokens: usize) -> Self {
        self.buffer = ConversationBuffer::new(tokens);
        self
    }

    /// The configured model id.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Whether a model id has been configured (that is, it is not [`UNSET_MODEL`]).
    #[must_use]
    pub fn has_model(&self) -> bool {
        self.model != UNSET_MODEL
    }

    /// The most recent scrubbed transport/parse error, if the last call failed.
    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Replaces the document snapshot the next proposal is conditioned on.
    ///
    /// The loop calls this before each [`ModelClient::propose`] so the model always sees
    /// the current layout it is editing, exactly as the Anthropic backend does.
    pub fn set_document_context(&self, context: impl Into<String>) {
        *self.document_context.borrow_mut() = context.into();
    }

    /// Scrubs `text` of the key when one is set, or returns it unchanged.
    fn scrub(&self, text: &str) -> String {
        match &self.key {
            Some(k) => k.scrub(text),
            None => text.to_owned(),
        }
    }

    /// Builds the user-message text for one proposal turn (task, document, feedback).
    ///
    /// Identical in spirit to the Anthropic backend's user message: it states the task,
    /// includes the current document, and on a correcting iteration lists the previous
    /// violations and feedback so the model can fix its last batch.
    fn build_user_message(&self, prompt: &str, context: &Context) -> String {
        use std::fmt::Write as _;
        let doc = self.document_context.borrow();
        let mut user = format!("Task:\n{prompt}\n\nCurrent document:\n{doc}\n");
        if context.iteration > 0 {
            // Writing into a String is infallible; the Result is discarded deliberately.
            let _ = write!(
                user,
                "\nYour previous attempt left {} verification violation(s). \
                 Correct them. Feedback:\n",
                context.prev_violations
            );
            if context.feedback.is_empty() {
                user.push_str("(no detail provided)\n");
            } else {
                for f in &context.feedback {
                    let _ = writeln!(user, "- {f}");
                }
            }
        }
        user.push_str(
            "\nRespond by calling emit_commands with the batch of commands to apply now.",
        );
        user
    }

    /// Builds the JSON request body for one proposal turn.
    ///
    /// The system prompt states the task and the frozen command vocabulary; the single
    /// function tool `emit_commands` is the intended output channel, forced via
    /// `tool_choice`. `stream` is false so the whole reply arrives in one blocking body.
    /// The `messages` array is drawn from the conversation buffer (already compacted if it
    /// neared the window) with the current user message appended.
    fn build_request(&self, messages: &[Message]) -> serde_json::Value {
        serde_json::json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "stream": false,
            "messages": messages,
            "tools": [emit_commands_tool()],
            "tool_choice": {
                "type": "function",
                "function": { "name": EMIT_COMMANDS }
            },
        })
    }

    /// Sends one request and parses the model's chosen commands.
    ///
    /// Returns the command batch on success. On a missing model id, or any transport,
    /// HTTP, or parse failure, it records a scrubbed [`last_error`](Self::last_error) and
    /// returns an empty batch, which the loop treats as "no proposal this turn".
    fn call(&mut self, prompt: &str, context: &Context) -> Vec<AgentCommand> {
        self.last_error = None;

        if !self.has_model() {
            self.last_error = Some(BuildError::MissingModel.to_string());
            return Vec::new();
        }

        // Record this turn's user message into the buffer, compacting older turns first
        // if the running conversation nears the context window.
        let user = self.build_user_message(prompt, context);
        self.buffer.push_user(user);
        let messages = self.buffer.messages(SYSTEM_PROMPT);

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let body = self.build_request(&messages);

        // The optional key reaches the transport as the bearer value; an empty string
        // means "no Authorization header", which the transport honors.
        let bearer = self.key.as_ref().map_or("", ApiKey::expose);

        let raw = match self.transport.post_json(&url, bearer, &body) {
            Ok(raw) => raw,
            Err(e) => {
                // Scrub in case the transport echoed the header value in its error.
                self.last_error = Some(self.scrub(&e));
                return Vec::new();
            }
        };

        match parse_commands(&raw) {
            Ok(parsed) => {
                // Keep the assistant's raw reply in the buffer so the next turn has the
                // full running context (which summarization will later compact).
                self.buffer.push_assistant(parsed.assistant_text);
                parsed.commands
            }
            Err(e) => {
                self.last_error = Some(self.scrub(&e));
                Vec::new()
            }
        }
    }

    /// Swaps in a scripted transport. Test-only.
    #[cfg(test)]
    pub(crate) fn with_transport(mut self, transport: Box<dyn HttpTransport>) -> Self {
        self.transport = transport;
        self
    }

    /// The current number of buffered messages (excluding the system prompt). Test-only.
    #[cfg(test)]
    pub(crate) fn buffered_len(&self) -> usize {
        self.buffer.turns.len()
    }
}

impl ModelClient for OllamaModel {
    fn id(&self) -> &str {
        &self.id
    }

    fn propose(&mut self, _task_id: &str, prompt: &str, context: &Context) -> Vec<AgentCommand> {
        self.call(prompt, context)
    }
}

/// Why an [`OllamaModel`] call could not proceed.
#[derive(Debug)]
pub enum BuildError {
    /// `RETICLE_MODEL_NAME` was unset, so there is no model to request. The client is
    /// still constructible; the error surfaces on the first call.
    MissingModel,
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::MissingModel => write!(
                f,
                "{ENV_MODEL_NAME} is not set; export the model id \
                 (for example {ENV_MODEL_NAME}=gpt-oss:16k)"
            ),
        }
    }
}

impl std::error::Error for BuildError {}

// ----- conversation buffer and summarization --------------------------------

/// One chat message in the `OpenAI` format: a role and its text content.
///
/// Only the `role`/`content` shape is sent; tool-call plumbing is not round-tripped back
/// into the request, because the harness re-derives the next user message from the
/// document and feedback rather than continuing a native tool-call thread. Keeping the
/// assistant's text lets a summarizer compress the history without losing what happened.
#[derive(Clone, Debug, serde::Serialize)]
struct Message {
    /// `"system"`, `"user"`, or `"assistant"`.
    role: &'static str,
    /// The message text.
    content: String,
}

/// A running buffer of user/assistant turns that compacts itself when it nears the model
/// context window.
///
/// The system prompt is not stored here (it is prepended fresh by
/// [`messages`](Self::messages)); only the alternating user/assistant turns accumulate.
/// Before each new user turn is pushed, [`compact_if_needed`](Self::compact_if_needed)
/// checks the estimated token count and, if it is at or over the threshold, replaces all
/// but the most recent turns with a single summary message (see [`summarize_transcript`]).
#[derive(Debug)]
struct ConversationBuffer {
    /// The accumulated turns, oldest first.
    turns: Vec<Message>,
    /// The token count (chars/4 estimate) at or above which older turns are compacted.
    threshold_tokens: usize,
}

impl ConversationBuffer {
    /// A fresh buffer with the given summarization threshold in tokens.
    fn new(threshold_tokens: usize) -> Self {
        Self {
            turns: Vec::new(),
            threshold_tokens,
        }
    }

    /// Pushes a user turn, compacting older turns first if the buffer nears the window.
    fn push_user(&mut self, content: String) {
        self.compact_if_needed();
        self.turns.push(Message {
            role: "user",
            content,
        });
    }

    /// Pushes an assistant turn verbatim (no compaction: it is the newest turn).
    fn push_assistant(&mut self, content: String) {
        self.turns.push(Message {
            role: "assistant",
            content,
        });
    }

    /// The full message list to send: the system prompt followed by every buffered turn.
    fn messages(&self, system_prompt: &'static str) -> Vec<Message> {
        let mut out = Vec::with_capacity(self.turns.len() + 1);
        out.push(Message {
            role: "system",
            content: system_prompt.to_owned(),
        });
        out.extend(self.turns.iter().cloned());
        out
    }

    /// Compacts the buffer when its estimated token count reaches the threshold.
    ///
    /// Keeps the most recent turn verbatim (the one the model most needs to see in full)
    /// and folds everything before it into a single summary message, so the running
    /// conversation stays under the window as iterations accumulate.
    fn compact_if_needed(&mut self) {
        let tokens = estimate_tokens(self.turns.iter().map(|m| m.content.as_str()));
        if tokens < self.threshold_tokens || self.turns.len() <= 1 {
            return;
        }
        let keep_from = self.turns.len() - 1;
        let (older, latest) = self.turns.split_at(keep_from);
        let summary = summarize_transcript(older);
        let mut compacted = Vec::with_capacity(2);
        compacted.push(Message {
            role: "user",
            content: summary,
        });
        compacted.extend(latest.iter().cloned());
        self.turns = compacted;
    }
}

/// Estimates the token count of a set of message bodies with a chars/4 heuristic.
///
/// Deliberately crude: a real tokenizer is model-specific and not worth a dependency for
/// a window-management guardrail. Four characters per token is the widely used rule of
/// thumb for English-plus-JSON text and errs slightly high (so it compacts a touch early),
/// which is the safe direction for staying under a hard context limit.
fn estimate_tokens<'a>(contents: impl IntoIterator<Item = &'a str>) -> usize {
    let chars: usize = contents.into_iter().map(str::len).sum();
    chars / 4
}

/// Compacts older conversation turns into a single short summary message.
///
/// # Policy (cited in the benchmark chapter)
///
/// The binding constraint for a local model is a 16k-token context window shared between
/// the tool schema and a transcript that grows one propose/verify/correct iteration at a
/// time. Rather than let the window overflow (which a local server truncates opaquely,
/// usually dropping the *system* prompt and the tool schema first, i.e. the worst thing
/// to lose), the buffer keeps the **latest iteration verbatim** and replaces all older
/// iterations with the fixed-size summary this function produces. The summary states how
/// many earlier iterations were folded and preserves the last user turn among them (the
/// most recent document/feedback the model saw before the kept turn), truncated to a
/// bounded length. This keeps the request size bounded and monotonic in the number of
/// summarizations, not in the number of iterations, so a long correction run still fits.
///
/// The summary is intentionally lossy: it is a window-management device, not a record.
/// The authoritative, lossless record of a run is the on-disk transcript (which replays
/// to the same `document_hash`); nothing here affects that. See the [determinism note] in
/// the module docs.
///
/// [determinism note]: self
fn summarize_transcript(older: &[Message]) -> String {
    /// The longest tail of the last older turn to keep in the summary, in characters.
    const TAIL_CHARS: usize = 800;

    let folded = older.len();
    let last_context = older
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| tail(&m.content, TAIL_CHARS))
        .unwrap_or_default();

    let mut summary = format!(
        "[Summary of {folded} earlier iteration(s), compacted to fit the context window. \
         The authoritative transcript is recorded separately.]"
    );
    if !last_context.is_empty() {
        summary.push_str("\nMost recent prior context before the kept turn:\n");
        summary.push_str(&last_context);
    }
    summary
}

/// Returns the last `max_chars` characters of `text` (on a char boundary), prefixed with
/// an ellipsis marker when the text was longer.
fn tail(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_owned();
    }
    let start = count - max_chars;
    let tail: String = text.chars().skip(start).collect();
    format!("...{tail}")
}

// ----- request tool schema --------------------------------------------------

/// The function-tool name the model calls to return its command batch.
const EMIT_COMMANDS: &str = "emit_commands";

/// The system prompt: the model's role and the command-vocabulary contract.
///
/// Kept byte-for-byte identical to the Anthropic backend's system prompt so the two
/// backends present the same command contract to the model; only the wire framing around
/// it differs.
const SYSTEM_PROMPT: &str = "\
You are a layout-automation agent for an integrated-circuit design tool. You edit a \
layout by proposing a batch of commands, which a deterministic engine applies. After \
each batch a design-rule and connectivity checker verifies the result and may hand you \
its violations to correct.

You return commands ONLY by calling the emit_commands tool. Its input is an object \
{\"commands\": [ ... ]} whose elements are commands in the exact JSON form below. Each \
command is an object tagged by an \"op\" field (snake_case). Coordinates are integer \
database units; layer/datatype are GDSII integers.

Common commands (op : other fields):
- create_cell : name
- delete_cell : name
- add_rect : cell, layer{layer,datatype}, rect{min{x,y},max{x,y}}
- add_polygon : cell, layer{layer,datatype}, points[{x,y}, ...]  (>= 3 points)
- add_path : cell, layer{layer,datatype}, width, points[{x,y}, ...], endcap(optional: flat|square|round)
- place_instance : cell, child, transform{orientation,mag_num,mag_den,dx,dy}
- place_array : cell, child, transform{...}, columns, rows, column_pitch, row_pitch
- transform_shapes : ids[<int>, ...], transform{...}
- delete_shapes : ids[<int>, ...]
- query_shapes : cell, layer(optional), region(optional)
- get_cell_info : cell
- list_layers
- set_technology : source
- run_drc : cell, region(optional)
- route_net : cell, net, layer{...}, terminals[{x,y}, ...]
- run_extract : cell
- check_intent : cell, intent
- export_gds / export_oasis / render_png / save_session

Return the smallest batch that makes progress. To create geometry you must first \
create_cell the target cell (unless it already exists in the current document), then \
add shapes to it. Prefer generous, DRC-clean dimensions over minimal ones.";

/// Builds the `OpenAI` function-tool object for `emit_commands`.
///
/// Permissive on each command's inner shape (an untyped object) so the frozen
/// [`AgentCommand`] contract, not a duplicated schema, is the source of truth; validation
/// happens when [`parse_commands`] deserializes each element into an [`AgentCommand`].
fn emit_commands_tool() -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": EMIT_COMMANDS,
            "description": "Emit the batch of layout commands to apply now. Each element \
                must be a command object tagged by its `op` field.",
            "parameters": {
                "type": "object",
                "properties": {
                    "commands": {
                        "type": "array",
                        "description": "The ordered commands to apply, each an object \
                            with an `op` field and that op's arguments.",
                        "items": { "type": "object" }
                    }
                },
                "required": ["commands"]
            }
        }
    })
}

// ----- response parsing -----------------------------------------------------

/// The parsed outcome of a successful response: the command batch plus the assistant's
/// text (for keeping the running conversation buffer coherent).
#[derive(Debug)]
struct Parsed {
    /// The proposed commands.
    commands: Vec<AgentCommand>,
    /// The assistant message text, if any (empty for a pure tool-call reply).
    assistant_text: String,
}

/// A minimal view of a Chat Completions response.
#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<Choice>,
    /// Present on an error response (`{"error":{...}}`), which OpenAI-compatible servers
    /// (including Ollama) return with a non-2xx status and this body shape.
    #[serde(default)]
    error: Option<ApiError>,
}

/// One choice: we read its message.
#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

/// The assistant message on a choice: optional text content and optional tool calls.
#[derive(Deserialize)]
struct ChoiceMessage {
    /// Assistant text; scanned for a JSON command array as a fallback. Absent on a pure
    /// tool-call reply.
    #[serde(default)]
    content: Option<String>,
    /// The tool calls, if the model made any.
    #[serde(default)]
    tool_calls: Vec<ToolCall>,
}

/// One tool call: its function name and stringified arguments.
#[derive(Deserialize)]
struct ToolCall {
    #[serde(default)]
    function: ToolCallFunction,
}

/// The function part of a tool call.
#[derive(Deserialize, Default)]
struct ToolCallFunction {
    #[serde(default)]
    name: String,
    /// The arguments as a JSON **string** (`OpenAI` encodes tool arguments this way),
    /// which must be parsed again into `{ "commands": [...] }`.
    #[serde(default)]
    arguments: String,
}

/// The `error` object on an OpenAI-compatible error response.
#[derive(Deserialize)]
struct ApiError {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    message: String,
}

/// The `emit_commands` arguments once decoded from the tool-call string.
#[derive(Deserialize)]
struct EmitArgs {
    commands: Vec<AgentCommand>,
}

/// Parses a Chat Completions response body into the proposed command batch.
///
/// Prefers the `emit_commands` tool call in `choices[0].message.tool_calls` (whose
/// `arguments` are a JSON string parsed a second time); if none is present, falls back to
/// a JSON array of commands embedded in `choices[0].message.content`. Returns an error
/// string (for a scrubbed `last_error`) when the body is an API error, is unparsable, or
/// contains no commands in either form.
fn parse_commands(raw: &str) -> Result<Parsed, String> {
    let response: ChatResponse =
        serde_json::from_str(raw).map_err(|e| format!("parsing model response: {e}"))?;

    if let Some(err) = response.error {
        return Err(format!("model API error ({}): {}", err.kind, err.message));
    }

    let Some(choice) = response.choices.into_iter().next() else {
        return Err("model response contained no choices".to_owned());
    };
    let message = choice.message;
    let assistant_text = message.content.clone().unwrap_or_default();

    // Preferred path: the emit_commands tool call, arguments parsed from their string.
    for call in &message.tool_calls {
        if call.function.name == EMIT_COMMANDS {
            let args: EmitArgs = serde_json::from_str(&call.function.arguments)
                .map_err(|e| format!("parsing emit_commands arguments: {e}"))?;
            return Ok(Parsed {
                commands: args.commands,
                assistant_text,
            });
        }
    }

    // Some local models ignore forced tool_choice and answer in prose. Fall back to a
    // JSON array of commands inside the assistant text.
    if let Some(text) = &message.content
        && let Some(commands) = commands_from_text(text)
    {
        return Ok(Parsed {
            commands,
            assistant_text,
        });
    }

    Err("model response contained no emit_commands call or command array".to_owned())
}

/// Extracts a JSON array of commands from free text, if one is present.
///
/// Scans for the first `[` and the last `]` and tries to parse the span between them as
/// `Vec<AgentCommand>`. Returns `None` when there is no bracketed span or it does not
/// parse as commands, so a text block without a command array falls through cleanly. This
/// mirrors the Anthropic backend's `commands_from_text` exactly.
fn commands_from_text(text: &str) -> Option<Vec<AgentCommand>> {
    let start = text.find('[')?;
    let end = text.rfind(']')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<Vec<AgentCommand>>(&text[start..=end]).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        ConversationBuffer, DEFAULT_SUMMARIZE_THRESHOLD_TOKENS, Message, OllamaModel, UNSET_MODEL,
        commands_from_text, estimate_tokens, parse_commands,
    };
    use crate::model::HttpTransport;
    use crate::redact::ApiKey;
    use reticle_agent_api::AgentCommand;
    use reticle_bench::model::{Context, ModelClient};
    use std::sync::{Arc, Mutex};

    /// A scripted transport that returns a canned response body and ignores the request.
    struct FakeTransport {
        body: String,
    }

    impl HttpTransport for FakeTransport {
        fn post_json(
            &self,
            _url: &str,
            _api_key: &str,
            _body: &serde_json::Value,
        ) -> Result<String, String> {
            Ok(self.body.clone())
        }
    }

    /// A transport that captures the request body and bearer value into shared slots and
    /// returns a canned response, so a test can inspect what was sent.
    struct Recording {
        seen_body: Arc<Mutex<Option<serde_json::Value>>>,
        seen_bearer: Arc<Mutex<Option<String>>>,
        body: String,
    }

    impl HttpTransport for Recording {
        fn post_json(
            &self,
            _url: &str,
            api_key: &str,
            body: &serde_json::Value,
        ) -> Result<String, String> {
            *self.seen_body.lock().unwrap() = Some(body.clone());
            *self.seen_bearer.lock().unwrap() = Some(api_key.to_owned());
            Ok(self.body.clone())
        }
    }

    /// A transport that always fails, echoing the key into its error to prove scrubbing.
    struct LeakyErrorTransport;

    impl HttpTransport for LeakyErrorTransport {
        fn post_json(
            &self,
            _url: &str,
            api_key: &str,
            _body: &serde_json::Value,
        ) -> Result<String, String> {
            Err(format!(
                "connection refused with Authorization: Bearer {api_key}"
            ))
        }
    }

    /// A model with a canned response body, a key, and a set model id.
    fn model_with(body: &str) -> OllamaModel {
        OllamaModel::for_test(
            Some(ApiKey::from_raw("sk-local-secret")),
            "http://localhost:11434/v1",
            "test-model:16k",
        )
        .with_transport(Box::new(FakeTransport {
            body: body.to_owned(),
        }))
    }

    /// A recorded-response body: an `emit_commands` tool call with two commands.
    fn tool_call_body() -> String {
        serde_json::json!({
            "choices": [
                { "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        { "type": "function", "function": {
                            "name": "emit_commands",
                            "arguments": "{\"commands\":[\
                                {\"op\":\"create_cell\",\"name\":\"top\"},\
                                {\"op\":\"add_rect\",\"cell\":\"top\",\
                                 \"layer\":{\"layer\":68,\"datatype\":20},\
                                 \"rect\":{\"min\":{\"x\":0,\"y\":0},\
                                           \"max\":{\"x\":500,\"y\":500}}}]}"
                        }}
                    ]
                }}
            ]
        })
        .to_string()
    }

    #[test]
    fn parses_emit_commands_tool_call() {
        let cmds = parse_commands(&tool_call_body()).expect("parse").commands;
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0], AgentCommand::CreateCell { name: "top".into() });
    }

    #[test]
    fn parses_json_array_from_content_fallback() {
        // The model ignored tool_choice and answered in prose with a command array.
        let body = serde_json::json!({
            "choices": [
                { "message": {
                    "role": "assistant",
                    "content": "Sure: [ {\"op\":\"list_layers\"} ] should do it."
                }}
            ]
        })
        .to_string();
        let cmds = parse_commands(&body).expect("parse").commands;
        assert_eq!(cmds, vec![AgentCommand::ListLayers]);
    }

    #[test]
    fn surfaces_api_error_body() {
        let body = serde_json::json!({
            "error": { "type": "invalid_request_error", "message": "model not found" }
        })
        .to_string();
        let err = parse_commands(&body).expect_err("error body");
        assert!(err.contains("invalid_request_error"));
        assert!(err.contains("model not found"));
    }

    #[test]
    fn empty_choices_is_an_error() {
        let body = serde_json::json!({ "choices": [] }).to_string();
        assert!(parse_commands(&body).is_err());
    }

    #[test]
    fn malformed_tool_arguments_is_an_error() {
        // arguments is a string but not valid JSON.
        let body = serde_json::json!({
            "choices": [ { "message": {
                "role": "assistant",
                "tool_calls": [ { "type": "function", "function": {
                    "name": "emit_commands", "arguments": "{not json"
                }}]
            }}]
        })
        .to_string();
        let err = parse_commands(&body).expect_err("bad arguments");
        assert!(err.contains("emit_commands arguments"));
    }

    #[test]
    fn commands_from_text_ignores_non_command_brackets() {
        assert!(commands_from_text("an array [1, 2, 3] of numbers").is_none());
        assert!(commands_from_text("no brackets here").is_none());
    }

    #[test]
    fn parses_live_qwen_content_embedded_tool_call() {
        // A verbatim capture of what a real local qwen2.5-coder:16k returned to a forced
        // tool_choice probe on this host: it did NOT populate the native `tool_calls`
        // array; instead it serialized the whole tool call as a JSON object into
        // `message.content`. The text fallback must still recover the command array. This
        // is exactly the "some local models ignore forced tool_choice" case, exercised
        // against a genuine wire response so the fallback is not just theoretical.
        let content = "{\n  \"name\": \"emit_commands\",\n  \"arguments\": {\n    \
             \"commands\": [\n      {\n        \"op\": \"list_layers\"\n      }\n    ]\n  }\n}";
        let body = serde_json::json!({
            "id": "chatcmpl-658",
            "object": "chat.completion",
            "model": "qwen2.5-coder:16k",
            "choices": [ { "index": 0, "message": {
                "role": "assistant",
                "content": content
            }, "finish_reason": "stop" }]
        })
        .to_string();
        let cmds = parse_commands(&body).expect("parse live content").commands;
        assert_eq!(cmds, vec![AgentCommand::ListLayers]);
    }

    #[test]
    fn propose_calls_endpoint_and_returns_commands() {
        let mut model = model_with(&tool_call_body());
        model.set_document_context("(empty document)");
        let cmds = model.propose("t1", "Draw a met1 rect.", &Context::default());
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0], AgentCommand::CreateCell { name: "top".into() });
        assert!(model.last_error().is_none());
    }

    #[test]
    fn request_body_carries_openai_shape_and_bearer() {
        let seen_body = Arc::new(Mutex::new(None::<serde_json::Value>));
        let seen_bearer = Arc::new(Mutex::new(None::<String>));
        let mut model = OllamaModel::for_test(
            Some(ApiKey::from_raw("sk-local-xyz")),
            "http://localhost:11434/v1",
            "gpt-oss:16k",
        )
        .with_transport(Box::new(Recording {
            seen_body: seen_body.clone(),
            seen_bearer: seen_bearer.clone(),
            body: tool_call_body(),
        }));
        model.set_document_context("cell top: 1 shape");
        let ctx = Context {
            iteration: 1,
            prev_violations: 2,
            feedback: vec!["m1.1: too narrow".into()],
        };
        let _ = model.propose("t1", "Draw a wide met1 rect.", &ctx);

        let req = seen_body.lock().unwrap().clone().expect("request captured");
        // OpenAI-compatible framing: model, non-streaming, a function tool, forced choice.
        assert_eq!(req["model"], "gpt-oss:16k");
        assert_eq!(req["stream"], false);
        assert_eq!(req["tools"][0]["type"], "function");
        assert_eq!(req["tools"][0]["function"]["name"], "emit_commands");
        assert_eq!(req["tool_choice"]["function"]["name"], "emit_commands");
        // The prompt, document, and feedback all made it into a message.
        let text = req.to_string();
        assert!(text.contains("Draw a wide met1 rect."));
        assert!(text.contains("cell top: 1 shape"));
        assert!(text.contains("too narrow"));
        // The bearer value is exactly the key (the transport places it in the header).
        assert_eq!(seen_bearer.lock().unwrap().clone().unwrap(), "sk-local-xyz");
    }

    #[test]
    fn transport_error_is_scrubbed_and_empty() {
        let mut model = OllamaModel::for_test(
            Some(ApiKey::from_raw("sk-local-leak-me")),
            "http://localhost:11434/v1",
            "test-model:16k",
        )
        .with_transport(Box::new(LeakyErrorTransport));
        let cmds = model.propose("t1", "p", &Context::default());
        assert!(cmds.is_empty());
        let err = model.last_error().expect("error recorded");
        assert!(
            !err.contains("sk-local-leak-me"),
            "key must be scrubbed: {err}"
        );
        assert!(err.contains("[REDACTED]"));
    }

    #[test]
    fn keyless_model_sends_no_bearer_and_does_not_panic() {
        // Ollama needs no key: a keyless client works and passes an empty bearer.
        let seen_bearer = Arc::new(Mutex::new(None::<String>));
        let mut model =
            OllamaModel::for_test(None, "http://localhost:11434/v1", "qwen2.5-coder:16k")
                .with_transport(Box::new(Recording {
                    seen_body: Arc::new(Mutex::new(None)),
                    seen_bearer: seen_bearer.clone(),
                    body: tool_call_body(),
                }));
        let cmds = model.propose("t1", "p", &Context::default());
        assert_eq!(cmds.len(), 2);
        assert_eq!(seen_bearer.lock().unwrap().clone().unwrap(), "");
    }

    #[test]
    fn debug_does_not_leak_key() {
        let model = OllamaModel::for_test(
            Some(ApiKey::from_raw("sk-local-hidden")),
            "http://localhost:11434/v1",
            "m:16k",
        );
        let dbg = format!("{model:?}");
        assert!(!dbg.contains("sk-local-hidden"));
        // Presence is reported only as a bool.
        assert!(dbg.contains("has_key: true"));
    }

    #[test]
    fn missing_model_errors_cleanly_without_calling() {
        // A client with no model id must not send a request; it records a clean error.
        let mut model = OllamaModel::for_test(None, "http://localhost:11434/v1", UNSET_MODEL)
            .with_transport(Box::new(LeakyErrorTransport));
        assert!(!model.has_model());
        let cmds = model.propose("t1", "p", &Context::default());
        assert!(cmds.is_empty());
        let err = model.last_error().expect("error recorded");
        assert!(err.contains("RETICLE_MODEL_NAME"));
    }

    #[test]
    fn with_model_sets_id_and_model() {
        let model = OllamaModel::for_test(None, "http://x", UNSET_MODEL).with_model("gpt-oss:16k");
        assert_eq!(model.model(), "gpt-oss:16k");
        assert_eq!(model.id(), "gpt-oss:16k");
        assert!(model.has_model());
    }

    #[test]
    fn estimate_tokens_is_chars_over_four() {
        // 40 characters -> 10 tokens.
        assert_eq!(estimate_tokens(["0123456789".repeat(4).as_str()]), 10);
        assert_eq!(estimate_tokens([""; 0]), 0);
    }

    #[test]
    fn long_transcript_triggers_summarization_and_stays_under_threshold() {
        // A tiny threshold makes a handful of turns overflow, so the buffer must compact.
        const THRESHOLD: usize = 500; // tokens (~2000 chars)
        let mut buffer = ConversationBuffer::new(THRESHOLD);
        // Each user turn is ~1600 chars (~400 tokens); the assistant echoes ~400 chars.
        let big_user = "U".repeat(1600);
        let big_assistant = "A".repeat(400);

        // Track the request size seen at each turn boundary (post-compaction, i.e. what
        // would actually be sent for that iteration's user message).
        let mut max_request_tokens_at_send = 0_usize;
        for i in 0..12 {
            buffer.push_user(format!("{big_user} (iteration {i})"));
            // `push_user` compacted first if needed, so this is the send-time size.
            let at_send = estimate_tokens(buffer.turns.iter().map(|m| m.content.as_str()));
            max_request_tokens_at_send = max_request_tokens_at_send.max(at_send);
            buffer.push_assistant(format!("{big_assistant} (reply {i})"));
        }

        // Compaction happened: the buffer is bounded to a summary plus a small constant
        // of recent turns, far fewer than the 24 raw messages, and the summary marker is
        // present.
        let msgs = buffer.messages(super::SYSTEM_PROMPT);
        let non_system: Vec<&Message> = msgs.iter().filter(|m| m.role != "system").collect();
        assert!(
            non_system.len() <= 4,
            "buffer should have compacted to a summary plus the latest turn(s), got {}",
            non_system.len()
        );
        let joined: String = non_system.iter().map(|m| m.content.as_str()).collect();
        assert!(
            joined.contains("Summary of"),
            "a summary message must be present after compaction"
        );

        // The send-time request size never ran away: a single latest verbatim turn plus
        // the bounded summary keeps every request comfortably within a small multiple of
        // the threshold, rather than growing without bound across the 12 iterations. (A
        // request always includes the newest turn verbatim, so it can momentarily exceed
        // the threshold by roughly one turn; what matters is that it stays bounded.)
        let one_turn_tokens = estimate_tokens([big_user.as_str()]);
        assert!(
            max_request_tokens_at_send < THRESHOLD + 2 * one_turn_tokens,
            "send-time request size ({max_request_tokens_at_send} tokens) must stay bounded \
             near the {THRESHOLD}-token threshold, not grow with the iteration count"
        );
    }

    #[test]
    fn summarization_keeps_latest_turn_verbatim() {
        const THRESHOLD: usize = 300;
        let mut buffer = ConversationBuffer::new(THRESHOLD);
        for i in 0..8 {
            buffer.push_user(format!("{} iteration {i}", "X".repeat(1200)));
        }
        // The final pushed user turn (its distinctive tail) must survive verbatim.
        let last = buffer.turns.last().expect("a latest turn");
        assert!(
            last.content.contains("iteration 7"),
            "the latest turn must be kept verbatim, not summarized"
        );
    }

    #[test]
    fn buffer_accumulates_across_iterations() {
        // Without hitting the threshold, turns accumulate (proof the buffer is stateful).
        let mut model = model_with(&tool_call_body());
        model.set_document_context("doc");
        let _ = model.propose("t1", "p", &Context::default());
        let _ = model.propose(
            "t1",
            "p",
            &Context {
                iteration: 1,
                prev_violations: 1,
                feedback: vec!["fix".into()],
            },
        );
        // Two user turns plus two assistant turns were buffered.
        assert_eq!(model.buffered_len(), 4);
        // The default threshold is generous, so nothing compacted for two small turns.
        assert!(model.buffered_len() <= (DEFAULT_SUMMARIZE_THRESHOLD_TOKENS / 4).max(4));
    }
}
