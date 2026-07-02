//! The structured agent-command error.
//!
//! Every command returns `Result<_, AgentError>`; the API never panics on bad
//! input or a failed engine operation. The [`ErrorCode`] is machine-readable so a
//! harness can branch on it, and the message is for humans and transcripts.

use serde::{Deserialize, Serialize};

/// A machine-readable classification of an agent-command failure.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCode {
    /// A referenced cell does not exist.
    NoSuchCell,
    /// A referenced [`crate::ElementId`] does not exist in this session.
    NoSuchElement,
    /// A command argument was invalid: out of range, malformed geometry, or a
    /// self-inconsistent request.
    InvalidArgument,
    /// A referenced layer is not in the active technology.
    NoSuchLayer,
    /// An underlying engine operation failed (IO, DRC, routing, extraction).
    EngineError,
    /// The session command or token budget was exhausted.
    BudgetExhausted,
}

/// A structured, serializable error returned by an agent command.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AgentError {
    /// The machine-readable error code.
    pub code: ErrorCode,
    /// A human-readable description of what went wrong.
    pub message: String,
}

impl AgentError {
    /// Builds an error with `code` and a human-readable `message`.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// A `NoSuchCell` error naming the missing cell.
    pub fn no_such_cell(name: &str) -> Self {
        Self::new(ErrorCode::NoSuchCell, format!("no such cell `{name}`"))
    }

    /// An `InvalidArgument` error with a reason.
    pub fn invalid(reason: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgument, reason)
    }
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for AgentError {}
