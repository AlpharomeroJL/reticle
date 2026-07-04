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
- Date of this record: 2026-07-01 (retained render path, WASM cold load, and collaboration echo measured 2026-07-02)

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
4060 Ti, Vulkan, 1920x1080): at 1,000,000 leaf shapes the old steady-state path (geometry
and pipelines built once, then per-frame draw plus CPU readback) runs 65.7 fps, and the
one-shot API that rebuilds the scene every frame runs 23.4 fps (the difference is the
per-frame CPU flatten and tessellate). At 10,000,000 that old steady-state path runs 6.1 fps
across two chunks, bottlenecked by the per-frame scene build and the 256 MiB single
instance buffer. Both numbers include a blocking CPU readback that a surface-presenting
loop would skip.

### Retained render path (v4.0.0, measured 2026-07-02)

The retained renderer (`RetainedRenderer`, `RetainedScene`) caches per-cell
tessellation once, expands instances/arrays into a per-instance transform buffer, and
stores geometry in fixed-size GPU pages uploaded via `queue.write_buffer`. Geometry is
uploaded once and then only *redrawn*; a camera move rewrites a single uniform, never a
buffer. Same host, same `fps_bench` harness and inputs, same 1920x1080 and blocking CPU
readback, so the numbers are directly comparable to the old path above.

| N leaf shapes | Old path (rebuild/reuse) | Retained path | Speedup | 30fps@10M target |
|---|---:|---:|---:|---|
| 1,000,000 | 65.7 fps (reuse) / 23.4 fps (one-shot) | **295 fps** (3.4 ms/frame) | ~4.5x | n/a (60fps target met) |
| 10,000,000 | 6.1 fps (reuse, 2 chunks) | **113 fps** (8.8 ms/frame, 8 page-sized draws) | ~19x | **Met** |

Why the 10M jump: the old reuse path still re-flattened and re-tessellated the whole
10M-shape scene every frame (about 463 ms of CPU work) and pushed it through a single
instance buffer capped at 8.39M rects by the device `max_buffer_size` (256 MiB). The
retained path pays that build once (334 ms on the first frame / on an edit), then each
frame is only the GPU draw plus readback across eight 64 MiB page buffers, with no
single monolithic buffer. A surface-presenting loop skips the readback, so the on-screen
windowed path runs at or above these numbers.

### GPU-driven draw list: flags vs compacted

The GPU-driven draw list replaces the flags-only cull with a cull-plus-compaction pass
that reserves survivors with a per-workgroup exclusive scan and fills a
`DrawIndexedIndirectArgs`, so the draw count comes from the GPU and only a tiny count is
read back (methodology: `bench_flags_vs_compacted` in `fps_bench`, each op blocks to GPU
completion; a viewport keeps about half the boxes).

| N cull boxes | Flags only (cull + read back N flags) | Compaction (GPU scan -> draw args, count read) | Speedup |
|---|---:|---:|---:|
| 1,000,000 | 7.19 ms/op | 3.20 ms/op | ~2.2x |
| 4,000,000 | 28.52 ms/op | 12.15 ms/op | ~2.3x |

Reading N visibility flags back to the CPU is O(N) each frame; compaction keeps the
survivor list on the GPU and returns only a count, so its per-frame CPU cost is O(1) and
the whole pass is roughly 2.2 to 2.3x faster at these sizes. The larger N is bounded at
about 4.19M by the single-dispatch cull stage's storage-binding (128 MiB) and
workgroup-count (65,535) limits; culling above that needs chunking, a follow-up. On this
adapter the compacted survivors are drawn with a native `multi_draw_indexed_indirect`
(the device enables `MULTI_DRAW_INDIRECT_COUNT`); backends without it fall back to one
`draw_indexed_indirect` per bucket, and WebGL2 (no `INDIRECT_EXECUTION`) keeps the direct
draw path.

### WASM cold load to first interactive frame (v4.0.0, measured 2026-07-02)

Measured in headless Chromium (WebGPU enabled) against the release Trunk build
(`just web-build`), served over loopback so the figure is the fetch-plus-compile,
`wgpu` device init, and first-frame cost, not wide-area transfer of the 8.16 MiB
wasm. Method: an instrumented copy of the built `index.html` (a `MutationObserver`
records when the app hides the loading overlay, then a double `requestAnimationFrame`
records the first painted frame); `performance.now()` is elapsed time from navigation
start. Backend reported as WebGPU on this host.

| Phase | Cold (fresh browser process) | Warm reload |
|---|---:|---:|
| Loading overlay hidden (wasm instantiated, app entered) | 83 ms | 78 to 125 ms |
| First painted frame | **640 ms** | 126 to 139 ms |

The 3 s target is met with wide margin. The cold cost is dominated by the one-time
`wgpu` WebGPU adapter and device creation (about 560 ms of the 640 ms; the wasm itself
instantiates in about 83 ms); a warm reload, with the module compiled and the device
already live in the process, reaches first frame in about 130 ms. This is a single cold
sample (the browser process caches the compiled module and keeps the device warm across
same-process navigations, so repeat loads are warm by construction); it excludes network
transfer of the wasm, which over a real connection would add its download time on top.

### Collaboration echo latency (v4.0.0, measured 2026-07-02)

Measured by `cargo run -p reticle-server --example echo_latency --release`, which
binds the real relay on an ephemeral port, connects two `tokio-tungstenite` WebSocket
clients to one room, and each iteration has one client add a shape, encode just that
edit as a `yrs` v1 diff against the peer's state vector (about 35 bytes), and ship it;
the timer runs from the send to the moment the peer has decoded and applied it. 2000
edits, first 200 discarded as warmup.

| Path | Latency |
|---|---:|
| Local edit applied to the local document (no network) | 3.6 µs |
| Remote echo over the localhost relay (send to peer applied), median | 788 µs |
| Remote echo, mean | 844 µs |
| Remote echo, p95 | 1.67 ms |
| Remote echo, max | 2.67 ms |

All 2000 edits arrived and applied on the peer (the example asserts the final shape
count). A local edit is synchronous and lands in well under one 60 fps frame; the
remote echo on localhost is about 0.79 ms median, roughly 100x under the 100 ms target.
This is the relay-plus-CRDT round trip on one machine; a wide-area deployment adds real
network latency on top.

## v5.0.0 headless pipeline scale proof (measured 2026-07-03 on this machine)

The headless CLI (`reticle`, release build) processing a deliberately large,
hierarchical layout end to end. The design is one `xtask gen-layout --shapes 2000000
--layers 8 --depth 3` output: 4 cells and 1422 bytes on disk that expand to
**4,194,304 flattened leaf shapes**. Wall time and peak working set are measured per
process by `scripts/measure-run.ps1` (peak working set polled from the OS high-water
mark, wall time by a stopwatch around the process), release binaries, same host.

| Stage | Wall time | Peak memory | Note |
|---|---:|---:|---|
| Import (parse the hierarchical GDS) | 37 ms | 7.5 MB | Hierarchy stays small: 4.19M effective leaves, but the on-disk and in-memory forms are tiny because the design is cells and arrays, never flattened to import. |
| Render (flatten, then offscreen to 2560x1440 PNG) | 809 ms | 594 MB | The 4.19M leaves are flattened and drawn to a 2560x1440 image (a 2.4 MB PNG). |
| DRC (flatten, full-cell check, emit report) | 11.0 s | 1426 MB | Whole pipeline including formatting and emitting the full violation report. The synthetic design is violation-dense (about 2 million min-width violations), so most of this time and memory is the report itself, not the check. The isolated incremental-DRC cost (the interactive path) is 5 to 37 us; see the DRC section above. |
| Extract (flatten, connectivity, emit report) | 12.2 s | 1075 MB | Finds 4,194,304 single-shape nets (the synthetic shapes are disjoint) and prints one line per net, so this figure too is dominated by emitting a multi-million-line report, not the union-find. |

The honest headline is the first two rows: a 4.19M-leaf hierarchical layout imports
in 37 ms using 7.5 MB, and flattens and renders offscreen at 2560x1440 in 809 ms
using 594 MB, on this machine. The DRC and extract rows are whole-pipeline wall times
that include emitting a per-item text report for millions of items (the CLI is
verbose by design); the core algorithm costs are isolated in the criterion sections
above (index build 227 ms per 1M shapes, DRC full pass 643 ms per 1M shapes).

## Targets (Section 10)

Honest status of each spec target. Two are measured with a hard number; the rest are
not yet instrumented and say so, with the reason.

| Target | Status |
|---|---|
| Bulk index build of 1M shapes under 500 ms | **Met (measured):** 227 ms. |
| Point / rubber-band picking over 1M shapes under 1 ms | **Met (measured):** 926 ns for a nearest-point query. |
| Polygon booleans fast enough for interactive DRC merges | **Measured:** 272 µs / 1.45 ms for 256 / 1024-square self-unions. |
| 1M flat shapes at a sustained 60 fps | **Met (measured):** the retained path runs **295 fps** at 1920x1080 (geometry cached and uploaded once, then per-frame draw plus CPU readback), via `cargo run -p reticle-render --example fps_bench --release`. The old reuse path was 65.7 fps; the one-shot API that rebuilt the scene every frame was 23.4 fps. A surface-presenting loop skips the readback and runs at or above the retained number. |
| 10M flat shapes interactive at 30 fps or better | **Met (measured):** the retained path runs **113 fps** at 1920x1080 (8.8 ms/frame across eight 64 MiB page buffers), up from 6.1 fps on the old reuse path (a ~19x gain). The retained renderer caches tessellation, expands instances into a per-instance transform buffer, uploads once via `queue.write_buffer`, and never builds a single 256 MiB buffer; a camera move is a uniform write only. The one-time scene build (334 ms) is paid on an edit, not per frame. |
| Billions of leaf shapes via cell culling and LOD | **Architecturally supported, not fps-benchmarked.** Hierarchy is never flattened for browsing; cell culling and a compute-shader cull stage are implemented and tested. |
| Incremental DRC on a local edit under 100 ms | **Met (measured):** on a prepared context, a local re-check is 5.12 µs at 100k shapes and 37.5 µs at 1M, far under 100 ms. `DrcEngine::prepare` builds the index once (17.5 ms / 225 ms); then `PreparedDrc::check_region` touches only the edit neighbourhood, and a property test pins it to the full-pass oracle. See the `incremental` bench in `reticle-drc`. |
| WASM cold load to first interactive frame under 3 s | **Met (measured):** about 640 ms cold to first painted frame on the WebGPU path in headless Chromium (wasm instantiates in about 83 ms; the rest is one-time `wgpu` device init), warm reload about 130 ms. Measured over loopback, so it excludes wide-area transfer of the 8.16 MiB wasm. See the WASM cold load section above. |
| Collaboration: local edits echo within one frame; remote within 100 ms on localhost | **Met (measured):** a local edit applies in 3.6 µs (far inside one frame); a remote edit echoes through the localhost relay to the peer in about 0.79 ms median (p95 1.67 ms), roughly 100x under the 100 ms target. Measured with the `echo_latency` example (two real WebSocket clients, real `yrs` diffs); see the collaboration echo section above. |

## Regression guard

`xtask perf-check` (invoked as `just perf-check`) reads the committed baseline and
Criterion's fresh `estimates.json`, prints measured-vs-baseline per benchmark, and exits
non-zero if any benchmark exceeds its baseline by more than the tolerance
(`tolerance_pct`, currently 25%). So a performance regression is caught like a test
failure. Run `cargo bench --workspace` first to produce fresh estimates.
