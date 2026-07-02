//! Shared support for Reticle's benchmark suite.
//!
//! The Criterion `[[bench]]` targets do NOT live in this crate: they are
//! in-crate, next to the code they measure (`reticle-index/benches`,
//! `reticle-geometry/benches`, `reticle-drc/benches`, `reticle-model/benches`),
//! and all run under `cargo bench --workspace`. What this crate carries is the
//! committed baseline under `history/` and this small library of shared pieces
//! (currently the version stamp for benchmark records). `xtask perf-check`
//! compares fresh Criterion estimates against that baseline and fails on
//! regression beyond a threshold.

/// Returns the crate version, used to stamp benchmark records.
#[must_use]
pub fn bench_suite_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
