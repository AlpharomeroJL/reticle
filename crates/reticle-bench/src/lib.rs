//! Benchmark infrastructure for the Reticle agent suite.
//!
//! Defines the per-task schema (id, tier, prompt, technology reference, checker
//! spec), the checker trait (a final document plus a transcript yields pass or a
//! structured fail), the versioned suite manifest, and the results record. It
//! runs the suite against any model client, including a deterministic mock that
//! exercises the propose-verify-correct loop with no live-model dependency.
//! Frozen Wave 0 skeleton; the loader, runner, and mock land in a later wave.
