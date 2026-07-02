//! The agent status channel: live narration carried over the collaboration layer.
//!
//! While an agent runs, it publishes [`AgentStatus`] updates so a watching UI can
//! narrate the propose-verify-correct loop (the current step, the iteration, and
//! how many DRC violations remain). These ride the existing sync presence layer
//! under the [`AGENT_ACTOR`] identity.

use serde::{Deserialize, Serialize};

/// A live status update from a running agent.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AgentStatus {
    /// The current iteration of the propose-verify-correct loop (zero-based).
    pub iteration: u32,
    /// A short human-readable description of what the agent is doing now.
    pub step: String,
    /// The current DRC violation count; verification drives this toward zero.
    pub violations: u32,
    /// Whether the agent is still running (false when it has stopped).
    pub running: bool,
}

/// The actor identity an agent uses on the collaboration layer, so a human client
/// can distinguish agent edits and cursors from its own.
pub const AGENT_ACTOR: &str = "reticle-agent";
