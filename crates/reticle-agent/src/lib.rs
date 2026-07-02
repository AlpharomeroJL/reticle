//! Propose-verify-correct agent harness for Reticle.
//!
//! Drives any Anthropic-API-compatible model against the `reticle-agent-api`
//! command surface in a loop: the model proposes edits, the harness applies them,
//! and DRC plus intent verification (where a task carries an intent spec) is the
//! oracle that decides pass or correct-and-retry. Every run writes a transcript,
//! a final GDS, a rendered PNG, and a result record; failures are recorded as
//! failures. The API key is read from the environment only and redacted from
//! transcripts. Frozen Wave 0 skeleton; the loop lands in a later wave.
