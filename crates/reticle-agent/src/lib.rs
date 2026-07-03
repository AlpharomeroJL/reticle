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
//! - [`redact`]: the [`ApiKey`] wrapper that hides the secret from every
//!   [`Debug`](std::fmt::Debug) / [`Display`](std::fmt::Display) / serialize path, plus
//!   a text scrubber.
//! - [`run`]: the propose-verify-correct loop and the four-artifact writer.
//! - [`collab`]: the [`AgentCollaborator`] bridge that mirrors the agent's edits onto
//!   the `reticle-sync` CRDT under [`AGENT_ACTOR`](reticle_agent_api::AGENT_ACTOR) as
//!   atomic per-step transactions, and publishes cursor/selection presence plus an
//!   [`AgentStatus`](reticle_agent_api::AgentStatus) over the awareness layer.
//!
//! # Reuse of `reticle-bench`
//!
//! The harness builds on `reticle-bench`'s frozen seams rather than re-deriving them:
//! its [`ModelClient`](reticle_bench::ModelClient) trait and [`Context`](reticle_bench::model::Context),
//! the [`Checker`](reticle_bench::Checker) / [`CheckResult`](reticle_bench::CheckResult)
//! contract and [`CheckerRegistry`](reticle_bench::CheckerRegistry), and the
//! [`ResultRecord`](reticle_bench::ResultRecord) plus its JSON writer. `reticle-bench`
//! itself is left unmodified.

pub mod collab;
pub mod model;
pub mod redact;
pub mod run;

pub use collab::{AgentCollaborator, Pacing, StepReport};
pub use model::{AnthropicModel, BuildError, DEFAULT_BASE_URL, DEFAULT_MODEL, HttpTransport};
pub use redact::{ApiKey, REDACTED};
pub use run::{Artifacts, LoopOptions, Provenance, RunOutcome, run_agent_task};
