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
- Date of this record: 2026-07-01 (retained render path, WASM cold load, and collaboration echo measured 2026-07-02; v7.0.0 interaction latency and soak measured 2026-07-06)

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

## v5.0.0 headless pipeline scale proof (measured 2026-07-03, re-measured 2026-07-06 on this machine)

The headless CLI (`reticle`, release build) processing a deliberately large,
hierarchical layout end to end. The design is one `xtask gen-layout --shapes 2000000
--layers 8 --depth 3` output: 4 cells and 1422 bytes on disk that expand to
**4,194,304 flattened leaf shapes**. Wall time and peak working set are measured per
process by `scripts/measure-run.ps1` (peak working set polled from the OS high-water
mark, wall time by a stopwatch around the process), release binaries, same host.
The design file is passed to the harness by **absolute path**, so these figures are
always this 4,194,304-leaf (8x8x8) output and never a differently-sized
`scratch/gen.gds` left by an earlier `gen-layout` run (see the note at the end of
this section).

| Stage | Wall time | Peak memory | Note |
|---|---:|---:|---|
| Import (parse the hierarchical GDS) | 32 ms | 7.6 MB | Hierarchy stays small: 4.19M effective leaves, but the on-disk and in-memory forms are tiny because the design is cells and arrays, never flattened to import. |
| Render (flatten, then offscreen to 2560x1440 PNG) | 801 ms | 595 MB | The 4.19M leaves are flattened and drawn to a 2560x1440 image (a 2.4 MB PNG). |
| DRC (flatten, full-cell check, emit report) | 10.3 s | 1426 MB | Whole pipeline including formatting and emitting the full violation report. The synthetic design is violation-dense (4,194,304 min-width violations, one per leaf shape), so most of this time and memory is the report itself, not the check. The isolated incremental-DRC cost (the interactive path) is 5 to 37 us; see the DRC section above. |
| Extract (flatten, connectivity, emit report) | 12.7 s | 1074 MB | Finds 4,194,304 single-shape nets (the synthetic shapes are disjoint) and prints one line per net, so this figure too is dominated by emitting a multi-million-line report, not the union-find. |

The honest headline is the first two rows: a 4.19M-leaf hierarchical layout imports
in 32 ms using 7.6 MB, and flattens and renders offscreen at 2560x1440 in 801 ms
using 595 MB, on this machine. The DRC and extract rows are whole-pipeline wall times
that include emitting a per-item text report for millions of items (the CLI is
verbose by design); the core algorithm costs are isolated in the criterion sections
above (index build 227 ms per 1M shapes, DRC full pass 643 ms per 1M shapes).

## v7.0.0 interaction latency on real designs (measured 2026-07-06 on this machine)

Wave 5B profiled the *product interaction path* (open a design, see the first frame,
pan and redraw under load) on the committed real designs and a large generated one,
because product use exposes latency the synthetic `fps_bench` (flat documents at
steady state) does not. All numbers are wall clock on this host (RTX 4060 Ti, Vulkan,
release build), offscreen render plus CPU readback, so they are headless and
reproduce. The live-window wasm pan path is browser only and is labeled not measured
here below.

Method: `cargo run -p reticle-render --example interaction_latency --release --
[--iters N] FILE ...`. For each design it parses the GDS into a `Document`, then times
four phases separately: **open (CPU)** = parse + framing bbox + `Document::flatten`;
**first frame** = the CLI one-shot `WgpuRenderer::render_document_offscreen`
(rebuilds pipelines, target, palette, and the whole `SceneGeometry`, then reads the
frame back); **scene build + upload** = build the `RetainedScene`, expand, and upload
to the GPU once; **pan/redraw** = with geometry resident, shift the camera and redraw
(a camera-uniform write plus draw plus readback). Designs: the two committed real
tiles (`examples/tapeout/tt_um_reticle_tile.gds`, 89 leaves;
`corpus/tinytapeout/real_tinytapeout_min.gds`, 139 leaves) and a large generated
design (`xtask gen-layout --shapes 2000000 --layers 8 --depth 3`, 4,194,304 flattened
leaves). Figures are the typical of three runs at 1920x1080, 200 pan iterations.

| Design (leaves) | open (CPU) | first frame (one-shot) | scene build + upload | pan/redraw |
|---|---:|---:|---:|---:|
| tt_um_reticle_tile (89) | 0.15 ms | 3.7 ms | 0.85 ms | 2.7 ms/frame |
| real_tinytapeout_min (139) | 0.14 ms | 3.7 ms | 0.85 ms | 2.8 ms/frame |
| generated (4,194,304) | 31 ms | 192 ms | 125 ms | 5.3 ms/frame |

The real tiles are tiny, so opening and interacting is dominated by fixed GPU costs:
the first frame is about 3.7 ms (one-shot pipeline build plus draw plus readback) and
a steady pan is about 2.7 ms/frame, which is the offscreen draw-plus-CPU-readback
floor a live surface skips. There is no CPU offender on real tiles. The generated
design is where the open path costs real time, and the profile pointed at one clear
offender.

### Offender fixed: the array-flattening inner loop

Profiling the generated design showed **`Document::flatten` was the dominant open
cost: about 230 ms of a 230 ms open**, and the one-shot first frame paid it again
(the one-shot path flattens internally). `flatten_local`'s array expansion called
`transform_shape(&array.transform, shape)` inside the innermost `(col, row)` loop,
recomputing the same orientation/magnification transform `columns * rows` times per
child shape, and grew the output `Vec` from empty (tens of reallocation-and-copy
passes for millions of shapes). The fix (`crates/reticle-model/src/document.rs`)
factors the placement transform out of the per-cell loop (transform each child shape
once, then only translate per copy, which is exactly equivalent because the per-copy
step is a pure translation) and reserves the whole array's capacity up front. No
hot-path behavior changed; a new equivalence test
(`flatten_rotated_array_places_every_copy_correctly` in
`crates/reticle-model/tests/editing.rs`) pins the exact placed geometry of a rotated
array so the factoring is proven correct for non-identity orientations, and all 25
`reticle-model` tests pass.

Measured before and after with a new criterion bench
(`crates/reticle-model/benches/flatten.rs`, `cargo bench -p reticle-model --bench
flatten`, a 2-level 16x16 nested array = 1,048,576 leaves, same host):

| flatten bench (1,048,576 leaves) | Before | After | Change |
|---|---:|---:|---:|
| nested array, identity placement | 33.77 ms | 10.99 ms | -68% (about 3.1x) |
| nested array, rotated placement | 32.54 ms | 12.14 ms | -63% (about 2.7x) |

End to end on the 4.19M-leaf generated design (interaction_latency harness, same
host): **open (CPU) dropped from about 230 ms to about 31 ms** (the flatten part from
about 230 ms to about 31 ms), and the **one-shot first frame from about 330 ms to
about 192 ms** (it inherits the flatten win). `scene build + upload` and `pan/redraw`
are unchanged by this fix: `RetainedScene` expansion does not go through `flatten`,
and the pan cost is the GPU readback floor.

### Soak / stability check

`cargo run -p reticle-render --example interaction_soak --release -- [--iters N]
[--frame-ceiling-ms MS] [--drift-tolerance F] FILE` opens a design, uploads the
retained scene once, then pans and zooms and redraws it for N iterations and asserts
the run is stable. It checks, every frame, that the retained renderer's rect-instance
count, page-chunk count (draw calls), and mesh-index count never change (a pan or zoom
must not re-upload or rebuild geometry; a per-frame leak or rebuild would trip this),
and that no frame exceeds the ceiling and the last-decile mean frame time does not
exceed the first-decile mean by more than the drift tolerance (default 50%). It exits
non-zero with a printed reason on any violation, so it can gate in a script. The
heap is bounded by the retained buffer inventory (the zero-growth proxy) plus the
peak process working set when run under `scripts/measure-run.ps1`; there is no
in-process allocator hook, so true heap bytes per frame are not sampled here.

Runs on this host (each passed: no buffer growth, no frame over the ceiling, no
upward drift):

| Design | iterations | frame p50 | frame p99 | frame max | first/last decile mean | peak working set |
|---|---:|---:|---:|---:|---|---:|
| tt_um_reticle_tile (89 instances) | 5,000 | 2.79 ms | 3.08 ms | 4.59 ms | 2.77 / 2.74 ms | 160 MB |
| generated (4.19M instances) | 2,000 | 5.14 ms | 5.55 ms | 6.06 ms | 5.17 / 5.17 ms | about 305 MB |

The retained buffer inventory was constant across every frame in both runs (for the
generated design: 4,194,304 rect instances across 4 page chunks, unchanged over all
2,000 frames), and the last-decile mean equaled the first-decile mean, so there is no
per-frame growth or frame-time creep over the run. This is a bounded soak run here
(seconds, not the full 30 minutes); the harness is parameterized by `--iters` so an
operator can run it for a longer duration. A true multi-minute soak of the *live wasm
pan path in a browser* is a separate e2e/operator step and is not measured here.

Note (correcting an earlier misdiagnosis): a previous revision of this section claimed
the generated GDS "parses its AREF column/row counts as 8x8 when run directly but as
7x7 (1,882,384 leaves) when launched with `UseShellExecute=false`", blamed "an
off-by-one in the AREF COLROW decode, consistent with an uninitialized read", and
warned that the scale-proof numbers above reflected that 7x7 misparse. That was wrong
on every point. The GDS parse is pure, deterministic safe Rust: `gds21`, the AREF
import in `crates/reticle-io/src/gds.rs` (which copies `aref.cols`/`aref.rows`
verbatim), and `Document::flatten` (which loops `0..columns`) contain no `unsafe` and
no uninitialized memory, so identical bytes yield identical counts on every launch.
Re-measuring on the design pinned by absolute path reproduces the 4,194,304-leaf
figures above; their DRC/extract peaks (1426 MB / 1074 MB) are about double the same
generator run at 1,882,384 leaves (695 MB / 500 MB), which confirms the numbers above
are the 8x8x8 design, not a 7x7 parse.

The real launch-context effect is a working-directory bug in the *measurement*, not a
parser bug. `scripts/measure-run.ps1` starts the child via .NET `ProcessStartInfo`
without setting `WorkingDirectory`, so a **relative** path such as `scratch/gen.gds`
resolves against `[Environment]::CurrentDirectory` (which PowerShell's `Set-Location`
does not update), whereas a direct run or `Start-Process` resolves it against the
shell's current location. When those two directories hold different `scratch/gen.gds`
files, the harness silently measures a different design than intended -- and this repo
documents two designs written to that same path: `xtask gen-layout --shapes 2000000
--layers 8 --depth 3` (the 4,194,304-leaf design measured here) and `just gen-layout
1000000 8 3 scratch/gen.gds` (README / user guide, a 1,882,384-leaf design). Each
count is the correct flatten of whichever file was read; nothing is misparsed. Passing
the design by absolute path (as done for the table above), or giving `measure-run.ps1`
an explicit `WorkingDirectory`, removes the ambiguity.

## v8.0.0 served-archive streaming HUD (measured 2026-07-07 on this machine)

The browser `?archive=<url>` browse (lane v8-2e) streams a served `.rtla` over HTTP Range
into a read-only streamed scene and paints it coarse-then-fine. The streaming HUD's
counters are read off the live `window.__reticle_stats` seam the app publishes each frame;
the numbers below are from the served-archive Playwright spec running against the local
ranged static server (`e2e/serve-archive.mjs`) in headless Chromium on the WebGL2 path.

**Methodology.** The committed fixture (`e2e/fixtures/fixture.rtla`) is a three-level
power-of-two pyramid (1x1, 2x2, 4x4) over a 10000x10000 DBU world with one record centred
in each of the sixteen finest tiles. The page opens
`?archive=http://127.0.0.1:8082/fixture.rtla`; the bundle probes the archive's total size
with one ranged `bytes=0-0` GET (reading the `Content-Range` total), then the residency
pass fetches every covering tile from the coarsest level up. Counters are read from
`window.__reticle_stats` after the scene settles.

| Quantity | Value | How |
|---|---|---|
| Archive total size | 1132 B | `Content-Range` total from the `bytes=0-0` probe (`archive_file_size`). |
| Bytes fetched over Range | 588 B (52% of the file) | Sum of the 21 tile-fetch payloads, metered on the inbox (`archive_bytes_fetched`). |
| Tiles fetched / resident | 21 / 21 | Whole pyramid (1 + 4 + 16 covering tiles); all resident, none evicted (LRU bound 256). |
| Records painted (fine level) | 16 | Finest level fully resident, one record per finest tile (`archive_records_painted`). |
| Working-set estimate | ~588 B | Resident tiles times mean fetched tile size (~28 B/tile); an estimate, since eviction is inside the scene's LRU. |

This is a *functional* proof of the streaming and HUD accounting on a deliberately tiny
fixture (so the whole pyramid fits the working-set bound and the coarse-to-fine transition
is observable end to end), not a scale benchmark: "bytes fetched vs file size" and the
working-set estimate are exercised against real ranged transport and read back through the
same seam a user's HUD shows. The at-scale residency behaviour (coarse level painting while
fine tiles stream, LRU eviction bounding RAM) is proven headlessly by the `residency`
integration test with an injected per-tile fetch latency.

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
