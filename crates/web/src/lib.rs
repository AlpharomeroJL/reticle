//! Library surface of the Reticle web crate.
//!
//! The crate ships two wasm binaries built by Trunk -- the egui app (`main.rs`) and the
//! GDS->`.rtla` convert Web Worker (`bin/convert_worker.rs`, lane v8-6c) -- plus a no-op
//! native `main`. The conversion core lives here so both the worker and the native
//! round-trip test can reach it, and so it is compiled (and unit-tested) on every target
//! rather than only under wasm.

pub mod convert;
