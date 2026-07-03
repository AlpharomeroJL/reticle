//! [`AnthropicModel`]: a [`ModelClient`] backed by an Anthropic-compatible endpoint.
//!
//! Given the task prompt, a snapshot of the current document, and the previous
//! iteration's verifier feedback, it asks a Claude model for the next batch of
//! [`AgentCommand`]s. The model returns them through a single tool, `emit_commands`,
//! whose input is `{ "commands": [ <AgentCommand>, ... ] }`; each element is exactly
//! the frozen serde form of [`AgentCommand`] (tagged by `op`). A JSON-array-in-text
//! fallback covers a model that answers in prose instead of a tool call.
//!
//! # Key handling
//!
//! The API key is read from `ANTHROPIC_API_KEY` (via [`ApiKey::from_env`]) and held in
//! an [`ApiKey`] that never prints, serializes, or logs the clear value. It reaches
//! the wire only as the `x-api-key` header. Any error text and any response body that
//! this module surfaces are scrubbed through [`ApiKey::scrub`] first, so a transcript
//! or log can never carry the secret. The struct's [`Debug`] omits it entirely.
//!
//! # The [`ModelClient`] seam
//!
//! [`ModelClient::propose`] carries only the prompt and a [`Context`] (iteration index
//! and the prior verification result), not the document. The loop injects the current
//! document snapshot through [`AnthropicModel::set_document_context`] before each call,
//! so the model always sees the layout it is editing without changing the frozen trait.

use std::cell::RefCell;

use reticle_agent_api::AgentCommand;
use reticle_bench::model::{Context, ModelClient};
use serde::Deserialize;

use crate::redact::ApiKey;

/// The default model id. Opus 4.8 is the current most-capable Opus-tier model and
/// the harness default; override with [`AnthropicModel::with_model`] or the CLI.
pub const DEFAULT_MODEL: &str = "claude-opus-4-8";

/// The default Anthropic API base URL (the `/v1/messages` endpoint is appended).
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// The Anthropic API version header value the Messages API requires.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// The output-token ceiling for a single proposal turn. A command batch is small
/// (JSON objects), so this is generous without risking an HTTP timeout on a blocking
/// request.
const MAX_TOKENS: u32 = 8192;

/// A [`ModelClient`] that calls an Anthropic-API-compatible endpoint to turn a task
/// prompt plus the current document into a batch of [`AgentCommand`]s.
///
/// Configurable base URL and model id (so it also drives a local Anthropic-compatible
/// proxy). Built with [`AnthropicModel::from_env`], which fails if the API key is
/// absent. The clear key is never stored anywhere printable; see the [module
/// docs](self).
pub struct AnthropicModel {
    /// The Anthropic API key, read from the environment, never printed or serialized.
    key: ApiKey,
    /// Base URL of the endpoint (`/v1/messages` is appended).
    base_url: String,
    /// The model id to request.
    model: String,
    /// The stable client id recorded in the [`ResultRecord`]; the model id, so runs
    /// against different models are distinguishable.
    ///
    /// [`ResultRecord`]: reticle_bench::ResultRecord
    id: String,
    /// A snapshot of the current document, injected by the loop before each call, so
    /// the model sees the layout it is editing. Interior mutability keeps the
    /// [`ModelClient::propose`] receiver ergonomic while the field is set out of band.
    document_context: RefCell<String>,
    /// The most recent transport or parse error, scrubbed of the key, for the loop to
    /// surface. `None` when the last call succeeded.
    last_error: Option<String>,
    /// The HTTP transport. Boxed behind a trait so tests inject a scripted transport
    /// with no network.
    transport: Box<dyn HttpTransport>,
}

impl std::fmt::Debug for AnthropicModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately omit `key` and `transport`; `key` is a secret and `transport`
        // is not meaningfully printable.
        f.debug_struct("AnthropicModel")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("id", &self.id)
            .field("last_error", &self.last_error)
            .finish_non_exhaustive()
    }
}

impl AnthropicModel {
    /// Builds a model client, reading the API key from `ANTHROPIC_API_KEY`.
    ///
    /// Uses [`DEFAULT_BASE_URL`] and [`DEFAULT_MODEL`]; adjust with
    /// [`with_base_url`](Self::with_base_url) and [`with_model`](Self::with_model).
    ///
    /// # Errors
    ///
    /// Returns [`BuildError::MissingKey`] when the environment variable is unset or
    /// empty. The key is never written anywhere; it lives only in the returned value's
    /// non-printable [`ApiKey`].
    pub fn from_env() -> Result<Self, BuildError> {
        let key = ApiKey::from_env().ok_or(BuildError::MissingKey)?;
        Ok(Self::with_key(key))
    }

    /// Builds a client around an explicit [`ApiKey`] and the real HTTP transport.
    fn with_key(key: ApiKey) -> Self {
        Self {
            key,
            base_url: DEFAULT_BASE_URL.to_owned(),
            model: DEFAULT_MODEL.to_owned(),
            id: DEFAULT_MODEL.to_owned(),
            document_context: RefCell::new(String::new()),
            last_error: None,
            transport: Box::new(UreqTransport),
        }
    }

    /// Overrides the base URL (for a local or proxied Anthropic-compatible endpoint).
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

    /// The configured model id.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The most recent scrubbed transport/parse error, if the last call failed.
    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Replaces the document snapshot the next proposal is conditioned on.
    ///
    /// The loop calls this before each [`ModelClient::propose`] so the model always
    /// sees the current layout (cells, counts, a compact shape listing) it is editing.
    pub fn set_document_context(&self, context: impl Into<String>) {
        *self.document_context.borrow_mut() = context.into();
    }

    /// Builds the JSON request body for one proposal turn.
    ///
    /// The system prompt states the task, the frozen command vocabulary contract, and
    /// the current document; the tool `emit_commands` is the sole output channel. On a
    /// correcting iteration the previous violations and feedback are included so the
    /// model can fix its last batch.
    fn build_request(&self, prompt: &str, context: &Context) -> serde_json::Value {
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

        serde_json::json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "system": SYSTEM_PROMPT,
            "tools": [emit_commands_tool()],
            "tool_choice": { "type": "tool", "name": EMIT_COMMANDS },
            "messages": [ { "role": "user", "content": user } ],
        })
    }

    /// Sends one request and parses the model's chosen commands.
    ///
    /// Returns the command batch on success. On any transport, HTTP, or parse failure
    /// it records a scrubbed [`last_error`](Self::last_error) and returns an empty
    /// batch, which the loop treats as "no proposal this turn".
    fn call(&mut self, prompt: &str, context: &Context) -> Vec<AgentCommand> {
        self.last_error = None;
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let body = self.build_request(prompt, context);

        let raw = match self.transport.post_json(&url, self.key.expose(), &body) {
            Ok(raw) => raw,
            Err(e) => {
                // Scrub in case the transport echoed the header value in its error.
                self.last_error = Some(self.key.scrub(&e));
                return Vec::new();
            }
        };

        match parse_commands(&raw) {
            Ok(cmds) => cmds,
            Err(e) => {
                self.last_error = Some(self.key.scrub(&e));
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
}

impl ModelClient for AnthropicModel {
    fn id(&self) -> &str {
        &self.id
    }

    fn propose(&mut self, _task_id: &str, prompt: &str, context: &Context) -> Vec<AgentCommand> {
        self.call(prompt, context)
    }
}

/// Why [`AnthropicModel::from_env`] could not build a client.
#[derive(Debug)]
pub enum BuildError {
    /// `ANTHROPIC_API_KEY` was unset or empty. The key must come from the environment;
    /// the harness never reads it from a file or a flag.
    MissingKey,
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::MissingKey => write!(
                f,
                "ANTHROPIC_API_KEY is not set; export it in the environment (never committed)"
            ),
        }
    }
}

impl std::error::Error for BuildError {}

/// The HTTP seam: POST a JSON body with the API key header, return the response body.
///
/// Abstracted so tests drive the model with a scripted response and no network, and
/// so the blocking `ureq` client stays isolated behind one call.
pub trait HttpTransport {
    /// POSTs `body` as JSON to `url` with the Anthropic auth and version headers, and
    /// returns the response body text.
    ///
    /// `api_key` is the clear key; implementations must place it in the `x-api-key`
    /// header only and never log it. On any failure, return a human-readable message
    /// (the caller scrubs the key from it before surfacing).
    ///
    /// # Errors
    ///
    /// Returns an error string on connection failure, a non-2xx status, or a body that
    /// cannot be read.
    fn post_json(
        &self,
        url: &str,
        api_key: &str,
        body: &serde_json::Value,
    ) -> Result<String, String>;
}

/// The real transport: a blocking `ureq` request.
struct UreqTransport;

impl HttpTransport for UreqTransport {
    fn post_json(
        &self,
        url: &str,
        api_key: &str,
        body: &serde_json::Value,
    ) -> Result<String, String> {
        let response = ureq::post(url)
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .send_json(body);
        match response {
            Ok(mut resp) => resp
                .body_mut()
                .read_to_string()
                .map_err(|e| format!("reading response body: {e}")),
            Err(ureq::Error::StatusCode(code)) => {
                Err(format!("Anthropic API returned HTTP {code}"))
            }
            Err(e) => Err(format!("HTTP request failed: {e}")),
        }
    }
}

/// The tool name the model calls to return its command batch.
const EMIT_COMMANDS: &str = "emit_commands";

/// The system prompt: the model's role and the command-vocabulary contract.
///
/// It states that every element of the `commands` array must be the exact JSON form of
/// an [`AgentCommand`] (tagged by `op`), gives the op names and the shape of the
/// argument objects, and reminds the model that coordinates are integer database units.
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

/// Builds the JSON schema for the `emit_commands` tool.
///
/// The schema is intentionally permissive on each command's inner shape (an untyped
/// object) so the frozen [`AgentCommand`] contract, not a duplicated schema, is the
/// source of truth; validation happens when [`parse_commands`] deserializes each
/// element into an [`AgentCommand`].
fn emit_commands_tool() -> serde_json::Value {
    serde_json::json!({
        "name": EMIT_COMMANDS,
        "description": "Emit the batch of layout commands to apply now. Each element \
            must be a command object tagged by its `op` field.",
        "input_schema": {
            "type": "object",
            "properties": {
                "commands": {
                    "type": "array",
                    "description": "The ordered commands to apply, each an object with \
                        an `op` field and that op's arguments.",
                    "items": { "type": "object" }
                }
            },
            "required": ["commands"]
        }
    })
}

// ----- response parsing -----------------------------------------------------

/// A minimal view of a Messages API response: the content blocks we read.
#[derive(Deserialize)]
struct MessageResponse {
    #[serde(default)]
    content: Vec<ContentBlock>,
    /// Present on an error response (`{"type":"error","error":{...}}`).
    #[serde(default)]
    error: Option<ApiError>,
}

/// One content block. We care about `tool_use` (the `emit_commands` call) and `text`
/// (the JSON-in-prose fallback).
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock {
    /// A tool call; `input` is the tool's argument object.
    ToolUse {
        /// The tool that was called.
        name: String,
        /// The tool input object.
        input: serde_json::Value,
    },
    /// Assistant text; scanned for a JSON command array as a fallback.
    Text {
        /// The text content.
        text: String,
    },
    /// Any other block type (thinking, etc.) is ignored.
    #[serde(other)]
    Other,
}

/// The `error` object on a Messages API error response.
#[derive(Deserialize)]
struct ApiError {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    message: String,
}

/// The `emit_commands` tool input.
#[derive(Deserialize)]
struct EmitInput {
    commands: Vec<AgentCommand>,
}

/// Parses the model's response body into the proposed command batch.
///
/// Prefers the `emit_commands` tool call; if none is present, falls back to a JSON
/// array of commands embedded in a text block. Returns an error string (for a scrubbed
/// `last_error`) when the body is an API error, is unparseable, or contains no commands
/// in either form.
fn parse_commands(raw: &str) -> Result<Vec<AgentCommand>, String> {
    let response: MessageResponse =
        serde_json::from_str(raw).map_err(|e| format!("parsing model response: {e}"))?;

    if let Some(err) = response.error {
        return Err(format!("model API error ({}): {}", err.kind, err.message));
    }

    // Preferred path: the emit_commands tool call.
    for block in &response.content {
        if let ContentBlock::ToolUse { name, input } = block
            && name == EMIT_COMMANDS
        {
            let parsed: EmitInput = serde_json::from_value(input.clone())
                .map_err(|e| format!("parsing emit_commands input: {e}"))?;
            return Ok(parsed.commands);
        }
    }

    // Fallback: a JSON array of commands inside a text block.
    for block in &response.content {
        if let ContentBlock::Text { text } = block
            && let Some(cmds) = commands_from_text(text)
        {
            return Ok(cmds);
        }
    }

    Err("model response contained no emit_commands call or command array".to_owned())
}

/// Extracts a JSON array of commands from free text, if one is present.
///
/// Scans for the first `[` and the last `]` and tries to parse the span between them as
/// `Vec<AgentCommand>`. Returns `None` when there is no bracketed span or it does not
/// parse as commands, so a text block without a command array falls through cleanly.
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
    use super::{AnthropicModel, HttpTransport, ModelClient, commands_from_text, parse_commands};
    use crate::redact::ApiKey;
    use reticle_agent_api::AgentCommand;
    use reticle_bench::model::Context;
    use std::sync::Mutex;

    /// A scripted transport that returns a canned response body and ignores the request.
    struct FakeTransport {
        body: String,
    }

    impl FakeTransport {
        fn new(body: impl Into<String>) -> Self {
            Self { body: body.into() }
        }
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

    /// A transport that captures the request body into a shared slot (for inspecting
    /// what the model sent) and returns a canned response.
    struct Recording(std::sync::Arc<Mutex<Option<serde_json::Value>>>, String);

    impl HttpTransport for Recording {
        fn post_json(
            &self,
            _url: &str,
            _api_key: &str,
            body: &serde_json::Value,
        ) -> Result<String, String> {
            *self.0.lock().unwrap() = Some(body.clone());
            Ok(self.1.clone())
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
            Err(format!("connection refused with x-api-key: {api_key}"))
        }
    }

    fn model_with(body: &str) -> AnthropicModel {
        AnthropicModel::with_key(ApiKey::from_raw("sk-ant-test-secret"))
            .with_transport(Box::new(FakeTransport::new(body)))
    }

    #[test]
    fn parses_emit_commands_tool_call() {
        let body = serde_json::json!({
            "content": [
                { "type": "text", "text": "Here you go." },
                { "type": "tool_use", "name": "emit_commands", "input": {
                    "commands": [
                        { "op": "create_cell", "name": "top" },
                        { "op": "add_rect", "cell": "top",
                          "layer": { "layer": 68, "datatype": 20 },
                          "rect": { "min": { "x": 0, "y": 0 }, "max": { "x": 500, "y": 500 } } }
                    ]
                }}
            ]
        })
        .to_string();
        let cmds = parse_commands(&body).expect("parse");
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0], AgentCommand::CreateCell { name: "top".into() });
    }

    #[test]
    fn parses_json_array_from_text_fallback() {
        let body = serde_json::json!({
            "content": [
                { "type": "text", "text":
                    "I'll do: [ {\"op\":\"create_cell\",\"name\":\"a\"} ] now." }
            ]
        })
        .to_string();
        let cmds = parse_commands(&body).expect("parse");
        assert_eq!(cmds, vec![AgentCommand::CreateCell { name: "a".into() }]);
    }

    #[test]
    fn surfaces_api_error_body() {
        let body = serde_json::json!({
            "type": "error",
            "error": { "type": "authentication_error", "message": "invalid x-api-key" }
        })
        .to_string();
        let err = parse_commands(&body).expect_err("error body");
        assert!(err.contains("authentication_error"));
    }

    #[test]
    fn empty_content_is_an_error() {
        let body = serde_json::json!({ "content": [] }).to_string();
        assert!(parse_commands(&body).is_err());
    }

    #[test]
    fn commands_from_text_ignores_non_command_brackets() {
        assert!(commands_from_text("an array [1, 2, 3] of numbers").is_none());
        assert!(commands_from_text("no brackets here").is_none());
    }

    #[test]
    fn propose_calls_endpoint_and_returns_commands() {
        let body = serde_json::json!({
            "content": [ { "type": "tool_use", "name": "emit_commands", "input": {
                "commands": [ { "op": "list_layers" } ]
            }}]
        })
        .to_string();
        let mut model = model_with(&body);
        model.set_document_context("(empty document)");
        let cmds = model.propose("t1", "List the layers.", &Context::default());
        assert_eq!(cmds, vec![AgentCommand::ListLayers]);
        assert!(model.last_error().is_none());
    }

    #[test]
    fn request_body_carries_prompt_document_and_tool() {
        let body = serde_json::json!({
            "content": [ { "type": "tool_use", "name": "emit_commands",
                           "input": { "commands": [] } }]
        })
        .to_string();
        let seen_probe = std::sync::Arc::new(Mutex::new(None::<serde_json::Value>));
        let mut model = AnthropicModel::with_key(ApiKey::from_raw("sk-ant-x"))
            .with_transport(Box::new(Recording(seen_probe.clone(), body)));
        model.set_document_context("cell top: 1 shape");
        let ctx = Context {
            iteration: 1,
            prev_violations: 2,
            feedback: vec!["m1.1: too narrow".into()],
        };
        let _ = model.propose("t1", "Draw a wide met1 rect.", &ctx);
        let req = seen_probe
            .lock()
            .unwrap()
            .clone()
            .expect("request captured");
        let text = req.to_string();
        assert!(text.contains("Draw a wide met1 rect."));
        assert!(text.contains("cell top: 1 shape"));
        assert!(text.contains("emit_commands"));
        assert!(text.contains("too narrow"));
        // The model id defaults to Opus 4.8.
        assert_eq!(req["model"], "claude-opus-4-8");
    }

    #[test]
    fn transport_error_is_scrubbed_and_empty() {
        let mut model = AnthropicModel::with_key(ApiKey::from_raw("sk-ant-leak-me"))
            .with_transport(Box::new(LeakyErrorTransport));
        let cmds = model.propose("t1", "p", &Context::default());
        assert!(cmds.is_empty());
        let err = model.last_error().expect("error recorded");
        assert!(
            !err.contains("sk-ant-leak-me"),
            "key must be scrubbed: {err}"
        );
        assert!(err.contains("[REDACTED]"));
    }

    #[test]
    fn debug_does_not_leak_key() {
        let model = AnthropicModel::with_key(ApiKey::from_raw("sk-ant-hidden"))
            .with_transport(Box::new(LeakyErrorTransport));
        assert!(!format!("{model:?}").contains("sk-ant-hidden"));
    }

    #[test]
    fn with_model_sets_id_and_model() {
        let model = AnthropicModel::with_key(ApiKey::from_raw("k")).with_model("claude-fable-5");
        assert_eq!(model.model(), "claude-fable-5");
        assert_eq!(model.id(), "claude-fable-5");
    }
}
