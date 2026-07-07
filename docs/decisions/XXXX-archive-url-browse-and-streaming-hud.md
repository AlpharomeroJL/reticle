# XXXX, The `?archive=` browse entry point, the streaming HUD, and the served-archive e2e

> Placeholder number: the orchestrator assigns the final ADR number and renames this file
> and its README row at the Wave 2 completion merge.

## Context

ADR 0062 froze the `.rtla` container and the `TileSource` read seam; lane 2B built the
`HttpRangeTileSource` (a wasm HTTP-range reader with an in-memory LRU and an OPFS
persistent cache); lane 2C built `StreamedScene` and the `DocHost` edit/stream split with
coarse-then-fine residency (ADR 0071). What remained was to make all of that reachable by
a user: open a *served* archive in the browser from a URL, end to end, and show the stream
happening. The existing browser open path is `?gds=<url>` (`webopen.rs`), but that imports
bytes into an *editable* in-RAM document; a multi-gigabyte served die is a different thing
that is only ever browsed. The app is also built around a single `history: History` field,
not a `DocHost`, so the streamed scene had to be added alongside the editing session
without disturbing it. Finally, the Wave 2 gate requires a served-archive end-to-end test
that runs against a **local** ranged server regardless of any cloud hosting.

## Decision

**A distinct `?archive=` entry point, parsed purely in `share.rs`.** `?archive=<url>` is a
separate query key from `?gds=`, with its own pure, round-tripped parser/emitter
(`archive_url_from_query` / `emit_archive_link`). The web boot (`web/src/main.rs`) resolves
it first, into a new `Boot::Archive(url)` that constructs `App::with_archive(url)`. The two
keys stay deliberately distinct: `?gds=` opens something you can edit, `?archive=` opens a
read-only streamed die. Keeping the parser in `share.rs` (beside the viewer/permalink
parsers) keeps every URL-shape decision pure and unit-tested with no browser.

**The browse holds a `DocHost::Streamed`, added beside the editing session, not replacing
it.** `App` gains an `Option<ArchiveBrowse>` (`archive.rs`). When `Some`, the canvas paints
the streamed die and skips the document/collaboration overlays; the editing `history` field
is untouched and unused. `ArchiveBrowse` holds a `DocHost` that is *always* the `Streamed`
arm, so the read-only-by-construction guarantee of ADR 0071 carries over unchanged: there
is no `History` to edit, so an edit is a compile error, not a runtime refusal. The
per-frame residency pass (drain fetched tiles, pick the paint level for the live viewport,
spawn fetches for the covering tiles not yet resident or in flight) is driven from the
canvas where the viewport is already computed; the fetch spawning is the only wasm-only
part, and the tile-selection and counter logic are pure and unit-tested.

**On the main thread, OPFS is disabled so the frozen source falls back to the network.**
`HttpRangeTileSource` caches tiles in OPFS via `FileSystemFileHandle.createSyncAccessHandle()`,
which exists **only in a dedicated Web Worker**; on the main thread (where eframe runs) that
call throws a *synchronous* `TypeError` the source's async guards cannot catch, aborting
every tile fetch before its bytes return. Since the source is frozen (`reticle-index`), the
browse neutralizes OPFS before the first fetch by overriding `navigator.storage.getDirectory`
to return a rejected promise, so the source's `opfs_dir()` returns `None` and both its read
and write short-circuit cleanly to the network plus the in-memory LRU. The override is
CSP-safe (no `eval`/`new Function`). Losing the cross-reload persistent cache is expected:
it genuinely needs a worker (lane 2D-alpha's territory), and a worker-hosted source would
regain it without changing this wiring.

**A streaming HUD backed by a pure `ArchiveStats` and a `window.__reticle_stats` seam.** An
on-canvas HUD shows bytes fetched vs archive size, tiles resident, records painted,
a working-set estimate, and fps. The counter arithmetic is a pure `ArchiveStats` value
(unit-tested); `TileInbox` was extended to meter raw fetched bytes so the network total is
honest. The archive's total size is probed with one ranged `bytes=0-0` GET reading the
`Content-Range` total (the frozen source exposes no size accessor). The same counters are
published each frame to `window.__reticle_stats`, extending the object the share-live viewer
already writes, so the end-to-end test can assert on the stream.

**The served-archive e2e runs against a local ranged server with a committed fixture.** A
new Playwright project (`served-archive`) opens `?archive=<local-url>` and polls
`window.__reticle_stats` to assert tiles become resident over HTTP Range and records paint.
The fixture (`e2e/fixtures/fixture.rtla`, a three-level power-of-two pyramid) is committed
and generated/round-tripped by `tests/archive_fixture.rs` (via the frozen `build_rtla`,
read back by `MmapTileSource`). It is served by a dependency-free Node `serve-archive.mjs`
that answers `206`/`Content-Range`, `HEAD`, and the CORS preflight the cross-origin `Range`
request header triggers. The fixture is served on a different port than the bundle, so the
fetch is genuinely cross-origin, exactly as a hosted archive would be.

## Consequences

- A served `.rtla` opens in the browser from a URL alone and paints coarse-then-fine, with
  browse/measure/query working and editing a compile error.
- The streaming is *visible* (the HUD) and *provable* (the `__reticle_stats` seam plus the
  local ranged e2e), independent of any cloud hosting.
- The main-thread OPFS workaround is a documented, self-contained shim over a frozen crate;
  it costs only cross-reload persistence, which a future worker-hosted source restores.
- The browse currently reuses the editor chrome, so switching to a draw tool edits an
  off-screen scratch document rather than the streamed die (which stays read-only by type);
  a dedicated browse chrome is a follow-up. The real-chip gallery deep link and the 3D
  ride-along orbit were scoped as tertiary and deferred (see the lane RESULT).
