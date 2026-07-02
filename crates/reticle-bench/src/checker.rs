//! The checker trait: given a task's final document and transcript, pass or fail.

use reticle_agent_api::Transcript;
use reticle_model::Document;

/// A specific, human-readable reason a task failed its check.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CheckFailure {
    /// What was wrong.
    pub reason: String,
}

impl CheckFailure {
    /// A failure with the given reason.
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

/// The result of a checker: a pass, or one or more concrete failures.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CheckResult {
    /// The task's requirements are met.
    Pass,
    /// The task failed, with the specific reasons.
    Fail(Vec<CheckFailure>),
}

impl CheckResult {
    /// True when the result is a pass.
    #[must_use]
    pub fn is_pass(&self) -> bool {
        matches!(self, CheckResult::Pass)
    }
}

/// A benchmark checker: decides whether a task's final `doc` (and the
/// `transcript` that produced it) satisfies the task's requirements. Each task
/// names a checker; every checker is unit-tested in both directions (it accepts a
/// known-good document and rejects a known-bad one).
pub trait Checker {
    /// Checks the final document and returns pass or a structured fail.
    fn check(&self, doc: &Document, transcript: &Transcript) -> CheckResult;
}
