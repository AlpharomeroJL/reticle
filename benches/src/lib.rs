//! Cross-crate Criterion benchmarks for Reticle.
//!
//! Wave 5 adds `[[bench]]` targets here (index build and query, geometry booleans,
//! DRC, routing, render throughput) plus a committed baseline under `history/`.
//! `xtask perf-check` compares fresh runs against that baseline and fails on
//! regression beyond a threshold. This library holds shared benchmark fixtures.

/// Returns the crate version, used to stamp benchmark records.
#[must_use]
pub fn bench_suite_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
