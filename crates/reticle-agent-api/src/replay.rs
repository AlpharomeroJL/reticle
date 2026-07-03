//! Transcript replay: re-execute a recorded session and reproduce its document.
//!
//! The replay contract (see [`crate::Transcript`]) is that re-applying a
//! transcript's recorded commands in order against a fresh [`Session`] rebuilds an
//! identical document, which [`reticle_model::document_hash`] captures.
//! [`replay`] returns that hash; [`verify_replay`] checks it against the
//! transcript's recorded [`final_hash`](crate::Transcript::final_hash).
//!
//! Commands are re-applied through [`Session::apply`], so a command that failed in
//! the original run fails the same way here and leaves the document unchanged, just
//! as it did originally. Replay therefore reproduces the exact document regardless
//! of which commands succeeded.

use reticle_model::document_hash;

use crate::{AgentError, ErrorCode, Session, Transcript};

/// Re-executes a transcript's commands on a fresh [`Session`] and returns the
/// `document_hash` of the resulting document.
///
/// # Errors
///
/// Never returns an error today: individual command failures are part of the
/// recorded history and are reproduced rather than propagated. The [`Result`] is
/// kept so a future stricter replay (for example one that rejects a transcript
/// whose outcomes diverge from the recorded ones) can report the divergence
/// without a signature change.
pub fn replay(transcript: &Transcript) -> Result<u64, AgentError> {
    let mut session = Session::new();
    for record in &transcript.records {
        // Re-apply every recorded command in order. The outcome (success or the
        // same structured error) is deterministic, so the rebuilt document matches
        // the original; we do not need to inspect the per-command result here.
        let _ = session.apply(record.command.clone());
    }
    Ok(document_hash(session.document()))
}

/// Re-executes a transcript and asserts the resulting document hash equals the
/// transcript's recorded [`final_hash`](Transcript::final_hash).
///
/// # Errors
///
/// Returns an [`AgentError`] with [`ErrorCode::EngineError`] if the replayed hash
/// differs from the recorded one, which means the transcript is not reproducible
/// (a replay-contract violation).
pub fn verify_replay(transcript: &Transcript) -> Result<(), AgentError> {
    let got = replay(transcript)?;
    if got == transcript.final_hash {
        Ok(())
    } else {
        Err(AgentError::new(
            ErrorCode::EngineError,
            format!(
                "replay hash {got:#x} does not match recorded final hash {:#x}",
                transcript.final_hash
            ),
        ))
    }
}

/// Builds a [`Transcript`] from a finished [`Session`]: its records plus the
/// `document_hash` of its current document, which a correct [`replay`] reproduces.
///
/// This is the natural bridge from a live session to a verifiable transcript; a
/// caller runs commands, then snapshots with this to get a transcript whose
/// [`verify_replay`] holds.
#[must_use]
pub fn transcript_of(session: &Session) -> Transcript {
    Transcript {
        records: session.transcript().to_vec(),
        final_hash: document_hash(session.document()),
        // A session records commands, not the harness's per-iteration plan; the plan
        // log is empty here and populated by the agent harness (see `reticle-agent`).
        plan: Vec::new(),
    }
}
