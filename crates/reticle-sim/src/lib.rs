//! Bounded circuit simulation for Reticle: extracted small-circuit netlists to
//! waveform records (transient and operating point), wall-clock and memory
//! bounded, with deterministic ordering.
//!
//! Scaffolded in the v8.2 campaign Phase 0 with the F4 waveform-record contract
//! ([`waveform`]) that the solver produces and the waveform UI consumes. Phase 3
//! adds the solver itself: a pure-Rust dense modified-nodal-analysis (MNA) engine
//! ([`circuit`], [`mna`], [`transient`]) that stamps linear R/C/L and independent
//! sources, solves by hand-rolled Gaussian elimination over `f64` (only `+ - * /`,
//! no external solver, deterministic across native and `wasm32`), and emits the F4
//! [`WaveformSet`] directly. It reproduces the committed contract fixture
//! (`tests/fixtures/contracts/f4_rc_transient.json`), the first-order RC step it was
//! generated from, to the exact nano-unit. The route is recorded in ADR 0109 and the
//! numerical sub-decisions (trapezoidal integration, partial pivoting) in ADR 0114.

pub mod waveform;

pub use waveform::{AnalysisKind, Bounds, Probe, Quantity, WaveformSet};

pub mod circuit;
pub mod mna;
pub mod transient;

pub use circuit::{Circuit, Element, GROUND, NodeId};
pub use mna::{MnaBuilder, SimError, Solution};
pub use transient::{ProbeSpec, TransientOptions, solve_operating_point, solve_transient};
