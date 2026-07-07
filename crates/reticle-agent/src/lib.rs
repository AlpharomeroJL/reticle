//! Propose-verify-correct agent harness for Reticle.
//!
//! Drives any Anthropic-API-compatible model against the `reticle-agent-api`
//! command surface in a loop: the model proposes edits, the harness applies them,
//! and DRC plus intent verification (where a task carries an intent spec) is the
//! oracle that decides pass or correct-and-retry. Every run writes a transcript,
//! a final GDS, a rendered PNG, and a result record; failures are recorded as
//! failures, never retro-edited to passes.
//!
//! # Layout of the crate
//!
//! - [`model`]: [`AnthropicModel`], a [`reticle_bench::ModelClient`] that calls an
//!   Anthropic-compatible endpoint (base URL and model id configurable) and parses
//!   the model's tool-use / JSON output into [`AgentCommand`](reticle_agent_api::AgentCommand)s.
//!   The API key is read from the environment only and never printed, serialized, or
//!   written to any artifact (see [`redact`]).
//! - [`ollama`]: [`OllamaModel`], the same [`ModelClient`](reticle_bench::ModelClient)
//!   contract against an OpenAI-compatible Chat Completions endpoint (Ollama by default),
//!   with the same `emit_commands` tool contract and a context-window summarization
//!   policy for small local models. Its optional key uses the same [`redact`] discipline.
//! - [`redact`]: the [`ApiKey`] wrapper that hides the secret from every
//!   [`Debug`](std::fmt::Debug) / [`Display`](std::fmt::Display) / serialize path, plus
//!   a text scrubber.
//! - [`run`]: the propose-verify-correct loop and the four-artifact writer.
//! - [`claude_code`]: the `claude-code` backend, which drives Claude Code as an external
//!   *agent system* (not a [`ModelClient`](reticle_bench::model::ModelClient)): per task it
//!   generates an MCP config that launches `reticle-mcp` with server-side transcript
//!   capture and a budget, runs `claude -p` non-interactively over that server, then
//!   replays the captured transcript and runs the task's checker into a
//!   [`ResultRecord`](reticle_bench::ResultRecord) labeled `backend = "claude-code"`. A
//!   missing or unauthenticated CLI is recorded as an honest not-run, never a fabricated
//!   result.
//! - [`collab`]: the [`AgentCollaborator`] bridge that mirrors the agent's edits onto
//!   the `reticle-sync` CRDT under [`AGENT_ACTOR`](reticle_agent_api::AGENT_ACTOR) as
//!   atomic per-step transactions, and publishes cursor/selection presence plus an
//!   [`AgentStatus`](reticle_agent_api::AgentStatus) over the awareness layer. Its
//!   id-addressed edits (`TransformShapes`, `DeleteShapes`) are mirrored by learning the
//!   `ElementId -> CRDT id` association at create time (ADR 0022's closed gap).
//! - [`live`] (native only): drives an [`AgentCollaborator`] over a real WebSocket into a
//!   `reticle-server` relay room, so the agent edits beside browser humans; publishes
//!   each step's CRDT delta and presence as binary frames and applies inbound peer
//!   frames back into its document.
//!
//! # Reuse of `reticle-bench`
//!
//! The harness builds on `reticle-bench`'s frozen seams rather than re-deriving them:
//! its [`ModelClient`](reticle_bench::ModelClient) trait and [`Context`](reticle_bench::model::Context),
//! the [`Checker`](reticle_bench::Checker) / [`CheckResult`](reticle_bench::CheckResult)
//! contract and [`CheckerRegistry`](reticle_bench::CheckerRegistry), and the
//! [`ResultRecord`](reticle_bench::ResultRecord) plus its JSON writer. The one amendment
//! to `reticle-bench` is the `backend`/`quantization` provenance added to
//! [`ResultRecord`](reticle_bench::ResultRecord) so mock, local, and frontier runs are
//! never conflated (authorized by ADR 0029).

pub mod claude_code;
pub mod collab;
pub mod context_pack;
// The live-room run mode drives the collaborator over a native WebSocket to a real relay
// room. Its async transport (`tokio` / `tokio-tungstenite`) does not build for wasm, so
// the module (and its deps) are native-only.
#[cfg(not(target_arch = "wasm32"))]
pub mod live;
pub mod model;
pub mod ollama;
pub mod redact;
pub mod run;

pub use claude_code::{
    ClaudeCodeConfig, ClaudeRunner, ClaudeTaskOutcome, NotRunRecord, SystemClaudeRunner,
    run_claude_code_task,
};
pub use collab::{AgentCollaborator, Pacing, StepReport};
pub use context_pack::{ContextPack, DEFAULT_SHAPE_CAP, token_estimate, whole_document_context};
#[cfg(not(target_arch = "wasm32"))]
pub use live::{LiveConfig, LiveError, run_in_room};
pub use model::{AnthropicModel, BuildError, DEFAULT_BASE_URL, DEFAULT_MODEL, HttpTransport};
pub use ollama::{
    BuildError as OllamaBuildError, DEFAULT_OLLAMA_BASE_URL, DEFAULT_SUMMARIZE_THRESHOLD_TOKENS,
    OllamaModel,
};
pub use redact::{ApiKey, REDACTED};
pub use run::{
    Artifacts, LoopOptions, NoRefinements, Provenance, RefinementFn, RefinementSource, RunOutcome,
    run_agent_task, run_agent_task_refined,
};
