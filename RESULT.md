# Lane v8-2c-residency: RESULT

**Status: GREEN.** Async tile residency wired into the app as a read-mostly document
host, with editing a streamed document made a compile error, and the coarse-then-fine
swap proven by a MemSource latency-injection test.

## Commits (on `lane/v8-2c-residency`, not pushed)

| SHA | Summary |
|-----|---------|
| `4d95655` | feat(app): DocHost edit/stream split and StreamedScene tile residency |
| `6e00de1` | test(app): coarse-then-fine residency proof against a latency-injecting MemSource |
| `6c95230` | docs: ADR 0068 and streaming chapter for the DocHost split and residency |

## Gate (all green)

| Command | Result |
|---------|--------|
| `cargo nextest run -p reticle-app` | 594 passed, 0 skipped (2 new residency tests + 9 new streamed/dochost unit tests) |
| `cargo clippy -p reticle-app --all-targets -- -D warnings` | clean |
| `cargo clippy -p reticle-app --target wasm32-unknown-unknown -- -D warnings` | clean (the `spawn_local` fetch path compiles for wasm) |
| `cargo doc -p reticle-app --no-deps` | clean, no warnings |
| `powershell -File scripts/check-style.ps1` | OK (no em-dashes; no banned words) |

Also ran `cargo clippy --workspace --all-targets -- -D warnings` clean (the pre-commit
hook), so the change breaks nothing wider.

## What was built

- **`crates/reticle-app/src/dochost.rs`, the edit/stream split.** `DocHost` is
  `Edited(History)` | `Streamed(StreamedScene)`. A `&mut History` is obtainable **only**
  by matching the `Edited` arm; there is no total `history_mut(&mut self) -> &mut History`,
  only a fallible `Option`-returning one, and `StreamedScene` carries no mutation API.
  Editing code that does not handle the streamed case cannot name a `History` to mutate,
  so **editing a streamed document is a compile error**, not a runtime check (ADR 0062,
  0068). Browse/measure/query/share read either arm.
- **`crates/reticle-app/src/streamed.rs`, the residency logic.** `StreamedScene` holds
  the `.rtla` header, resident tiles keyed by `TileCoord`, and an LRU working-set bound.
  It validates the header (magic, version, and that each level is the `2^level` square
  grid the mapper assumes) and builds a shape-free `LodPyramid` purely as a
  viewport-to-tile mapper. `missing_tiles` drives fetches; `fetch_tile` validates and
  decodes each tile with `rkyv` exactly as the mmap path does; a `TileInbox` is the
  async-to-sync seam the egui loop drains (`spawn_fetch` hands each fetch to
  `wasm_bindgen_futures::spawn_local` on wasm). `paint_level` returns the coarsest
  resident level that fully covers the viewport, which is the coarse-then-fine mechanism.
  `upload_tile_bytes` is the thin passthrough to the existing `BufferPages::upload` API
  (no render pipeline changed).
- **`crates/reticle-app/tests/residency.rs`, the residency proof.** See below.
- **Docs.** ADR 0068 (`docs/decisions/`), a new "Streamed documents" chapter
  (`docs/src/streaming.md` + `SUMMARY.md`), decisions `README.md` row.

## The residency test (the wave's proof)

`camera_move_paints_coarse_then_swaps_to_fine_after_the_fetch_delay` stands up an
in-memory `.rtla` archive (a 1x1 / 2x2 / 4x4 power-of-two pyramid over a 1000x1000 world)
behind a `MemSource` `TileSource` that injects a real per-tile fetch latency (20 ms). It:

1. Fetches the coarse level-0 tile through the real source + inbox path and makes it
   resident, standing in for the state before a zoom-in camera move.
2. Moves the camera to a viewport whose target level is the finest (level 2, asserted via
   `target_level`), leaving all four covering fine tiles missing.
3. **Immediately** (fetches not yet complete) asserts `paint_level` returns the coarse
   level 0, the painted records are the coarse decimated block, and no fine tile is
   resident.
4. Drives the four fine-tile fetches to completion with a tiny park-based executor,
   asserting the elapsed wall-clock time was at least the injected latency (so the delay
   is measured, not assumed), then drains the inbox.
5. Asserts the resident set has **transitioned coarse to fine** (all four fine tiles
   resident), `paint_level` now returns the fine level 2, and the painted record set
   **matches the fine-level query exactly** and differs from the coarse paint.

A second test proves a fetch of a tile the archive does not carry returns an honest
`TileSourceError::OutOfRange` (never a fake success), posts nothing to the inbox, and
leaves the scene still painting its resident coarse level.

The async fetches are driven by a ~15-line park-based `block_on` in the test so no
async-runtime dependency enters the workspace; on native the app drives them on a task,
on wasm through `spawn_local`.

## Honest gaps / scope decisions

- **`DocHost` is the routing type; the live `App` still holds a bare `History` field.**
  `app.rs` has ~95 `history` references across the editor UI. Rewriting the frozen Wave 0
  `App` type to hold a `DocHost` and route every mutation through the `Edited` arm is a
  broad, mechanical change the lane's "minimal, additive" scope and allowed-files list do
  not authorize (app entry = mod wiring only). The compile-time guarantee is delivered and
  unit-tested on `DocHost` itself; making `App` hold one is a follow-up for the wave that
  adds the streamed-open UI. The existing editing path already routes all mutation through
  `&mut History`, so that invariant holds today.
- **The `BufferPages` upload (`upload_tile_bytes`) and the wasm `spawn_fetch` are compiled
  but not unit-tested.** Both need hardware a headless run does not have: a GPU device
  (held by lane 1E this batch; `gpu_lane: no`) and a browser. This is the same
  pure-versus-glue split `webopen` uses; the residency logic they wrap is fully tested on
  the CPU side. The record-to-vertex encoding those bytes carry is the renderer's existing
  job, unchanged by 2C.
- **`MemSource` is a local test double.** Lane 2B's `tile_source.rs` is still the contract
  stub on this base, so the test defines its own `MemSource` against the frozen
  `TileSource` trait (as the brief allows). When 2B's `MemTileSource` lands on the base it
  can supersede this double; both implement the same frozen trait.
- **This reader accepts only power-of-two square pyramids.** `RtlaHeader` permits arbitrary
  per-level `LevelDims`; an archive with a non-`2^level` grid is refused with a clear
  `SceneError::NonPyramidLevel` rather than mis-mapped. A documented limitation, additive
  to v1, consistent with the contract's "clear error, never a fake success" ethos.

## Files touched

- New: `crates/reticle-app/src/dochost.rs`, `crates/reticle-app/src/streamed.rs`,
  `crates/reticle-app/tests/residency.rs`, `docs/decisions/0068-dochost-edited-streamed-split.md`,
  `docs/src/streaming.md`.
- Edited (additive): `crates/reticle-app/src/lib.rs` (mod wiring + re-exports),
  `crates/reticle-app/Cargo.toml` (`rkyv` dep + dev-dep), `Cargo.lock` (one edge),
  `docs/decisions/README.md`, `docs/src/SUMMARY.md`.
- No frozen crate, no `reticle-index`/`reticle-render` source, no `crates/web`/`e2e`,
  no `docs/TASKS.md`, no `scratch/RUN_STATE.md` was touched.
