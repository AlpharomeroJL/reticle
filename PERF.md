# Performance

Every number here is measured on real hardware with `criterion`, not estimated.
Where a target is not yet backed by a formal benchmark, that is stated plainly
rather than papered over. See `docs/src/performance.md` for the methodology.

## Host

- GPU: NVIDIA GeForce RTX 4060 Ti 16 GB
- OS: Windows 11
- Toolchain: Rust 1.94.1 (stable), release/bench profile (`opt-level = 3`, thin LTO)
- Inputs: the deterministic layout generator (`xtask gen-layout`), so runs reproduce.
- Date of this record: 2026-07-01

## Measured

Medians from `cargo bench --workspace` (criterion, 100 samples after warmup).

| Benchmark | Median | Notes |
|---|---:|---|
| R-tree bulk load, 1,000,000 shapes | 232 ms | `reticle-index`, `rstar` bulk load |
| R-tree nearest-point query | 888 ns | over the 1,000,000-shape tree |
| Polygon self-union, 256 overlapping squares | 277 µs | `reticle-geometry`, `i_overlay` |
| Polygon self-union, 1024 overlapping squares | 1.49 ms | scales roughly linearly |

## Targets

| Target | Result |
|---|---|
| Bulk index build of 1M shapes under 500 ms | Met: 232 ms measured. |
| Point / rubber-band picking over 1M shapes under 1 ms | Met: 888 ns measured for a nearest-point query. |
| 1M flat shapes at a sustained 60 fps | Demonstrated at scale: the offscreen renderer draws the generated ~1.88M-leaf-shape design at 2560x1440 (see `assets/hero.png`); an interactive per-frame fps benchmark on the surface path is a follow-up. |
| 10M flat shapes interactive at 30 fps or better | Rests on GPU-driven culling and LOD (both implemented); a 10M formal benchmark is a follow-up. |
| Billions of leaf shapes via cell culling and LOD | The hierarchy is never flattened for browsing; cell culling and a compute-shader cull stage are implemented and tested against a CPU reference. |
| Incremental DRC on a local edit under 100 ms | `reticle-drc` provides `check_region` bounded by an index query; a formal incremental-edit benchmark is a follow-up. |
| WASM cold load to first interactive frame under 3 s | The Trunk release bundle plus `wasm-opt` shrinks the demo; the browser-measured cold-load time is recorded once the demo is deployed. |
| Collaboration: local edits echo within one frame; remote within 100 ms on localhost | Local edits are immediate; the relay is a `tokio` broadcast with no added latency beyond the socket. |

## Regression guard

Benchmark medians are recorded here and in `benches/history/`. `xtask perf-check`
compares a fresh run against the committed baseline and flags regressions beyond a
threshold, so a performance regression is caught like a test failure.
