# 0068, The DocHost edit/stream split and coarse-then-fine tile residency

## Context

Wave 2 makes silicon stream over HTTP Range requests: ADR 0062 froze the `.rtla`
container, the `TileSource` read seam, and the read-mostly scope of a streamed document.
Lane 2C wires that seam into the app. Two things had to be true. First, a streamed
document must be browse/measure/query/share only, with editing structurally impossible,
and ADR 0062 asks for that to be a compile error rather than a runtime `if is_streamed`
check that a future edit path could forget. Second, a camera move over a multi-gigabyte
die cannot block on the network: the view must keep painting while fine tiles stream in,
degrading to coarse detail rather than to a blank frame. The app already routes every
mutation through `&mut History` (the `reticle-app` editing seam), and `reticle-index`
already has a `LodPyramid` that maps a viewport to the tiles that cover it at a chosen
level, plus `reticle-render`'s `BufferPages` for in-place GPU uploads.

## Decision

Introduce `DocHost` (`reticle-app/src/dochost.rs`), the one type an open document is:
`Edited(History)` or `Streamed(StreamedScene)`. A `&mut History` is obtainable only by
matching the `Edited` arm; there is no total `fn history_mut(&mut self) -> &mut History`,
only a fallible one returning `Option`, and `StreamedScene` exposes no mutation API at
all. So editing code that does not handle the streamed case cannot name a `History` to
mutate and does not compile. The read-mostly line is therefore drawn by the type system,
not by a runtime flag.

`StreamedScene` (`reticle-app/src/streamed.rs`) holds the `.rtla` header, the tiles
resident in RAM keyed by `TileCoord`, and an LRU working-set bound; it validates the
header (magic, version, and that each level is the `2^level` square grid the mapper
assumes) and builds a shape-free `LodPyramid` purely as a viewport-to-tile mapper. On a
camera move a caller asks it which tiles are missing at the target level, fetches those
over the `TileSource` (`fetch_tile` validates and decodes each tile with `rkyv` exactly
as the mmap path does), and hands the results back through a `TileInbox` that the egui
loop drains, mirroring the existing `WebOpenInbox` async-to-sync seam. Meanwhile the
scene paints whatever `paint_level` reports: the finest resident level, no finer than the
target, whose every viewport-covering tile is resident. Coarser levels have fewer, larger
tiles and so are far likelier to be complete, which is the mechanism behind progressive
refinement. The wasm build spawns each fetch with `wasm_bindgen_futures::spawn_local`; a
thin `upload_tile_bytes` passthrough is the only place 2C touches `BufferPages`, changing
no render pipeline.

## Consequences

The residency proof (`reticle-app/tests/residency.rs`) stands up an in-memory `.rtla`
archive behind a `MemSource` that injects a per-tile fetch latency, then asserts the
whole contract: immediately after a zoom-in the scene paints from the coarse resident
level with no fine tile resident yet, and after the injected delay elapses (measured, not
assumed) the resident set has transitioned coarse to fine, the painted level is the fine
level, and the painted record set equals the fine-level query. The async fetches are
driven by a tiny park-based executor so no async-runtime dependency enters the workspace.
`MemSource` is a local test double until lane 2B's `MemTileSource` lands on the base;
both implement the same frozen trait. Two pieces are deliberately not unit-tested because
they need hardware a headless run does not have: the wasm `spawn_fetch` (a browser) and
the `BufferPages` upload (a GPU device, held by lane 1E this batch); they compile on both
targets and are exercised in the running app, the same pure-versus-glue split `webopen`
uses. `DocHost` is the routing type; making the live `App` hold one in place of its bare
`History` field is a mechanical follow-up left for the wave that adds the streamed-open
UI, so this lane stays additive and does not churn the frozen `App` surface. This reader
accepts only power-of-two square pyramids; an archive with arbitrary per-level grids is
refused with a clear error rather than mis-mapped, a documented limitation additive to
v1.
