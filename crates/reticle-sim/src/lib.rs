//! Bounded circuit simulation for Reticle: extracted small-circuit netlists to
//! waveform records (transient and operating point), wall-clock and memory
//! bounded, with deterministic ordering.
//!
//! Scaffolded in the v8.2 campaign Phase 0 as the home for the F4 waveform-record
//! contract (probes, time series, bounds, analysis kind) and, later, the solver.
//! The contract types and the solver land in Phase 3; this crate compiles empty
//! until then so the workspace and `just ci` stay green.
