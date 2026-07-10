//! Bounded circuit simulation for Reticle: extracted small-circuit netlists to
//! waveform records (transient and operating point), wall-clock and memory
//! bounded, with deterministic ordering.
//!
//! Scaffolded in the v8.2 campaign Phase 0. This crate currently ships the F4
//! waveform-record contract ([`waveform`]) that a bounded solver produces and the
//! waveform UI consumes; the solver itself lands in Phase 3. Fixture-first: the
//! UI and the interop lanes build against the committed contract fixture
//! (`tests/fixtures/contracts/f4_rc_transient.json`) before the solver exists.

pub mod waveform;

pub use waveform::{AnalysisKind, Bounds, Probe, Quantity, WaveformSet};
