//! The session transcript: a record per command for audit and replay.
//!
//! A transcript is a sequence of [`CommandRecord`]s, written one JSON object per
//! line (JSONL). The replay contract is that re-executing the recorded commands
//! in order against a fresh session reproduces the same document, which a caller
//! verifies with `reticle_model::document_hash`; [`Transcript::final_hash`]
//! records the expected value.

use serde::{Deserialize, Serialize};

use crate::{AgentCommand, AgentError, AgentResponse, Revision};

/// The outcome of one command: a response or a structured error.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The command succeeded.
    Ok(AgentResponse),
    /// The command failed.
    Err(AgentError),
}

/// One transcript record: a command, its outcome, and surrounding metadata.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct CommandRecord {
    /// Zero-based position of this command in the session.
    pub seq: u64,
    /// The command that was applied.
    pub command: AgentCommand,
    /// The document revision before the command.
    pub revision_before: Revision,
    /// The document revision after the command.
    pub revision_after: Revision,
    /// The command outcome.
    pub outcome: Outcome,
    /// Wall-clock start, milliseconds since the session began.
    pub ts_start_ms: u64,
    /// Wall-clock end, milliseconds since the session began.
    pub ts_end_ms: u64,
    /// Model input tokens attributed to this command, if driven by a model.
    #[serde(default)]
    pub tokens_in: Option<u64>,
    /// Model output tokens attributed to this command, if driven by a model.
    #[serde(default)]
    pub tokens_out: Option<u64>,
}

/// A whole session transcript plus the document hash a correct replay reproduces.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Transcript {
    /// The command records, in order.
    pub records: Vec<CommandRecord>,
    /// The `document_hash` of the final document; a replay must match it.
    pub final_hash: u64,
}
