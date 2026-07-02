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

Each figure is Criterion's typical estimate, the regression slope for linear-sampled
benches, otherwise the mean, over 100 samples after warmup (`cargo bench --workspace`).
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

## v4.0.0 additions (measured on this machine)

| Benchmark | Median | Notes |
|---|---:|---|
| Per-cell bbox: uncached recompute | 6.14 µs | `reticle-model`, ~100k-effective-leaf hierarchy |
| Per-cell bbox: warm cached lookup | 20.8 ns | about 295x faster; `EditableDocument` cache, cleared on every edit |
| DRC full-cell pass, 100k / 1M rectangles | 32.8 ms / 643 ms | the cost of re-checking everything |
| DRC prepare (one-time index build), 100k / 1M | 17.5 ms / 225 ms | paid once per editing session |
| DRC incremental re-check on a prepared context, 100k / 1M | 5.12 µs / 37.5 µs | the true per-edit cost, far under the 100 ms target |

Out-of-core streaming (measured by `cargo run -p reticle-index --example stream_demo
--release`): a 30,000,000-entry tiled archive is 574 MiB on disk. Memory-mapped, a
small-viewport query returns in about 14 µs while touching only 4 of 262,144 tiles and
453 of 30,000,000 entries, and the process working set stays at 4.25 MiB (about 135x
below the file size) because only the touched pages are faulted in by the OS. The
archive builder is still bounded by RAM (ADR 0016), so a single archive above about
2 GiB is a follow-up; the read path itself is genuinely out-of-core.

Offscreen render fps (`cargo run -p reticle-render --example fps_bench --release`, RTX
4060 Ti, Vulkan, 1920x1080): at 1,000,000 leaf shapes the steady-state path (geometry
and pipelines built once, then per-frame draw plus CPU readback) runs 65.7 fps, and the
one-shot API that rebuilds the scene every frame runs 23.4 fps (the difference is the
per-frame CPU flatten and tessellate). At 10,000,000 the steady-state path runs 6.1 fps
across two chunks, bottlenecked by the per-frame scene build and the 256 MiB single
instance buffer. Both numbers include a blocking CPU readback that a surface-presenting
loop would skip.

## Targets (Section 10)

Honest status of each spec target. Two are measured with a hard number; the rest are
not yet instrumented and say so, with the reason.

| Target | Status |
|---|---|
| Bulk index build of 1M shapes under 500 ms | **Met (measured):** 227 ms. |
| Point / rubber-band picking over 1M shapes under 1 ms | **Met (measured):** 926 ns for a nearest-point query. |
| Polygon booleans fast enough for interactive DRC merges | **Measured:** 272 µs / 1.45 ms for 256 / 1024-square self-unions. |
| 1M flat shapes at a sustained 60 fps | **Met (measured, steady state):** 65.7 fps at 1920x1080 on the reuse path (scene and pipelines built once, then per-frame draw plus CPU readback), via `cargo run -p reticle-render --example fps_bench --release`. The one-shot offscreen API that rebuilds the scene every frame runs 23.4 fps; that path's bottleneck is the per-frame CPU scene build (flatten plus tessellate). A surface-presenting loop skips the readback and runs at or above the reuse number. |
| 10M flat shapes interactive at 30 fps or better | **Not met (measured): 6.1 fps** at 1920x1080. Bottlenecks: the per-frame CPU scene build (431 ms to flatten and tessellate 10M shapes) and a single instance buffer capped at 8.39M rects by the device `max_buffer_size` (256 MiB), which forces chunked draws. Caching the built geometry and batching buffers (the GPU-driven pipeline in Wave B) is the path to the target. |
| Billions of leaf shapes via cell culling and LOD | **Architecturally supported, not fps-benchmarked.** Hierarchy is never flattened for browsing; cell culling and a compute-shader cull stage are implemented and tested. |
| Incremental DRC on a local edit under 100 ms | **Met (measured):** on a prepared context, a local re-check is 5.12 µs at 100k shapes and 37.5 µs at 1M, far under 100 ms. `DrcEngine::prepare` builds the index once (17.5 ms / 225 ms); then `PreparedDrc::check_region` touches only the edit neighbourhood, and a property test pins it to the full-pass oracle. See the `incremental` bench in `reticle-drc`. |
| WASM cold load to first interactive frame under 3 s | **Not measured (needs in-browser timing).** The demo is deployed and loads (HTTP 200), but cold-load-to-interactive is not instrumented. |
| Collaboration: local edits echo within one frame; remote within 100 ms on localhost | **Not measured (needs a two-client harness).** Local edits apply immediately; the relay is a `tokio` broadcast adding no latency beyond the socket. Convergence correctness is tested; wall-clock echo is not. |

## Regression guard

`xtask perf-check` (invoked as `just perf-check`) reads the committed baseline and
Criterion's fresh `estimates.json`, prints measured-vs-baseline per benchmark, and exits
non-zero if any benchmark exceeds its baseline by more than the tolerance
(`tolerance_pct`, currently 25%). So a performance regression is caught like a test
failure. Run `cargo bench --workspace` first to produce fresh estimates.
