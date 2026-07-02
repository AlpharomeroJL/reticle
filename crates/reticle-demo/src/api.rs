//! The demo server's HTTP API types: submit, status, and cancel.

use serde::{Deserialize, Serialize};

/// A request to run a task: a bounded natural-language prompt.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SubmitRequest {
    /// The prompt. Rejected if it exceeds the configured maximum length or uses
    /// vocabulary outside the allowed task set.
    pub prompt: String,
}

/// The response to a submit: a session id to poll and cancel, and the sync room a
/// spectator can join to watch the agent draw.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SubmitResponse {
    /// Opaque session identifier.
    pub session_id: String,
    /// The collaboration room to watch this session live.
    pub room: String,
}

/// The lifecycle state of a session.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Accepted, waiting for a free slot.
    Queued,
    /// Running the propose-verify-correct loop.
    Running,
    /// Finished successfully.
    Done,
    /// Cancelled by the client or a limit.
    Cancelled,
    /// Rejected before running (rate limit, budget, or input filter).
    Rejected,
    /// Ended in an error.
    Error,
}

/// The status of a session, returned by the status endpoint.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    /// The session this status is for.
    pub session_id: String,
    /// Its current state.
    pub state: SessionState,
    /// The current propose-verify-correct iteration.
    pub iteration: u32,
    /// The current DRC violation count.
    pub violations: u32,
    /// A short human-readable status line.
    pub message: String,
}

/// A request to cancel a running session.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct CancelRequest {
    /// The session to cancel.
    pub session_id: String,
}
