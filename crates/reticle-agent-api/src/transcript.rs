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

/// One iteration's planning narration: what the agent said it would do before it
/// proposed commands.
///
/// A plan step is *narration for the viewer and material for failure mining*, not a
/// binding contract: nothing enforces that the iteration's applied commands match
/// [`intended_tools`](Self::intended_tools), nor that the checker named in
/// [`expected_checks`](Self::expected_checks) actually passes. It records what the
/// harness derived from the task and the model's proposal at the top of an iteration
/// so the agent panel can render it and a later analysis can compare stated intent
/// against recorded outcome.
///
/// Plan steps live alongside the [`CommandRecord`]s in [`Transcript::plan`] as a
/// parallel log; they carry no document state and do not affect replay.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct PlanStep {
    /// The iteration's goal, in one human-readable line (typically derived from the
    /// task prompt).
    pub goal: String,
    /// The `op` names of the commands the model proposed this iteration (for example
    /// `create_cell`, `add_rect`), in proposal order and with duplicates preserved.
    /// Empty when the model proposed nothing.
    pub intended_tools: Vec<String>,
    /// The checks the harness will run to verify the iteration (the task's checker
    /// name, plus `drc` for the always-on DRC oracle).
    pub expected_checks: Vec<String>,
}

/// A whole session transcript plus the document hash a correct replay reproduces.
///
/// The [`plan`](Self::plan) field is an additive, replay-neutral parallel log of the
/// per-iteration [`PlanStep`]s (agent planning transparency). It carries
/// `#[serde(default)]`, so transcript JSON written before the field existed still
/// deserializes (as an empty plan) and the replay contract is unchanged:
/// [`replay`](crate::replay()) and [`verify_replay`](crate::verify_replay) read only
/// [`records`](Self::records) and [`final_hash`](Self::final_hash).
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Transcript {
    /// The command records, in order.
    pub records: Vec<CommandRecord>,
    /// The `document_hash` of the final document; a replay must match it.
    pub final_hash: u64,
    /// The per-iteration planning narration, in iteration order. Additive and
    /// replay-neutral; see the type note. Defaults to empty for transcripts written
    /// before the plan log existed.
    #[serde(default)]
    pub plan: Vec<PlanStep>,
}
