# Performance

Every number here is measured on real hardware with `criterion`, not estimated.
Where a target is not yet backed by a formal benchmark, that is stated plainly rather
than papered over. See `docs/src/performance.md` for the methodology and
`docs/STATUS.md` for the honest per-target status.

## Host

- GPU: NVIDIA GeForce RTX 4060 Ti 16 GB
- OS: Windows 11
- Toolchain: Rust 1.94.1 (stable), release/bench profile (`opt-level = 3`, thin LTO)
- Inputs: the deterministic layout generator (`xtask gen-layout`), so runs reproduce.
- Date of this record: 2026-07-01

## Measured

Each figure is Criterion's typical estimate — the regression slope for linear-sampled
benches, otherwise the mean — over 100 samples after warmup (`cargo bench --workspace`).
The **committed** column is the source-controlled baseline in
`benches/history/baseline.json`; the **re-run** column is a fresh run on 2026-07-01 that
`xtask perf-check` confirmed is within the 25% regression tolerance (PASS on all four).

| Benchmark | Committed | Re-run (2026-07-01) | Notes |
|---|---:|---:|---|
| R-tree bulk load, 1,000,000 shapes | 232 ms | 227 ms | `reticle-index`, `rstar` bulk load |
| R-tree nearest-point query | 888 ns | 926 ns | over the 1,000,000-shape tree |
| Polygon self-union, 256 overlapping squares | 277 µs | 272 µs | `reticle-geometry`, `i_overlay` |
| Polygon self-union, 1024 overlapping squares | 1.49 ms | 1.45 ms | scales roughly linearly |

The benches live in-crate (`crates/reticle-index/benches`, `crates/reticle-geometry/benches`)
and run under `cargo bench --workspace`.

## Targets (Section 10)

Honest status of each spec target. Two are measured with a hard number; the rest are
not yet instrumented and say so, with the reason.

| Target | Status |
|---|---|
| Bulk index build of 1M shapes under 500 ms | **Met (measured):** 227 ms. |
| Point / rubber-band picking over 1M shapes under 1 ms | **Met (measured):** 926 ns for a nearest-point query. |
| Polygon booleans fast enough for interactive DRC merges | **Measured:** 272 µs / 1.45 ms for 256 / 1024-square self-unions. |
| 1M flat shapes at a sustained 60 fps | **Not measured (no fps harness).** The offscreen renderer draws the generated ~1.88M-leaf-shape design at 2560×1440 (`assets/hero.png`, confirmed non-blank); a per-frame fps benchmark on the surface-present path is a follow-up (surface presentation is not yet wired — see `docs/STATUS.md`). |
| 10M flat shapes interactive at 30 fps or better | **Not measured.** Rests on GPU-driven culling (implemented; compute shader validated against a CPU oracle) and an LOD pyramid; a 10M formal benchmark is a follow-up. |
| Billions of leaf shapes via cell culling and LOD | **Architecturally supported, not fps-benchmarked.** Hierarchy is never flattened for browsing; cell culling and a compute-shader cull stage are implemented and tested. |
| Incremental DRC on a local edit under 100 ms | **Not measured (no latency benchmark).** `reticle-drc::check_region` re-checks only geometry touching the edit, bounded by one index query, and is correctness-tested against a full-cell pass; a formal incremental-edit latency benchmark is a follow-up. |
| WASM cold load to first interactive frame under 3 s | **Not measured (needs in-browser timing).** The demo is deployed and loads (HTTP 200), but cold-load-to-interactive is not instrumented. |
| Collaboration: local edits echo within one frame; remote within 100 ms on localhost | **Not measured (needs a two-client harness).** Local edits apply immediately; the relay is a `tokio` broadcast adding no latency beyond the socket. Convergence correctness is tested; wall-clock echo is not. |

## Regression guard

`xtask perf-check` (invoked as `just perf-check`) reads the committed baseline and
Criterion's fresh `estimates.json`, prints measured-vs-baseline per benchmark, and exits
non-zero if any benchmark exceeds its baseline by more than the tolerance
(`tolerance_pct`, currently 25%). So a performance regression is caught like a test
failure. Run `cargo bench --workspace` first to produce fresh estimates.
