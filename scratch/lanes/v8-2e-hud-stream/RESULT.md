# Lane v8-2e-hud-stream — RESULT

**Status: GREEN.** The primary bar is met: opening a served `.rtla` in the browser end to
end via `?archive=<url>` works, and the served-archive Playwright spec passes against a
local ranged server. The HUD (secondary) shipped in full. The two tertiary items (real-chip
gallery deep link, 3D ride-along orbit) were trimmed and are noted honestly below.

Branch: `lane/v8-2e-hud-stream` (parked green; not pushed). Merges at the Wave 2 completion
sub-gate.

## Commits (on top of `28ed2ae`)

| sha | what |
|---|---|
| `9e04867` | feat(app): wire `?archive=` browse with progressive residency + HUD |
| `3a720d9` | test(e2e): served-archive spec + local Range server + committed `.rtla` fixture |
| `59cb553` | docs(app): rustdoc private/wasm links → code spans; drop em-dashes |
| `bb73c9c` | docs: `?archive` browse + streaming HUD (streaming.md, PERF, ADR + README row) |

## What shipped

### 1. `?archive=<url>` wired end to end (PRIMARY)

- **Pure parser in `share.rs`:** `archive_url_from_query` / `emit_archive_link`, a distinct
  query key from `?gds=` (which imports an *editable* document). Round-trip unit-tested,
  including reserved-character (`?`, `&`, `#`) encoding.
- **Boot in `web/src/main.rs`:** a new `Boot::Archive(url)` resolved before viewer/share/view,
  constructing `App::with_archive(url)`.
- **App wiring (`archive.rs` + `app.rs`):** on the first wasm frame the app opens an
  `HttpRangeTileSource` over the URL, reads/validates the header, builds a `StreamedScene`,
  and installs it as a `DocHost::Streamed` inside a new `Option<ArchiveBrowse>` field
  (added *beside* the editing `history`, which is left untouched). Each frame the canvas
  drives one residency pass (drain fetched tiles → pick paint level for the live viewport →
  spawn fetches for covering tiles not yet resident/in-flight) and paints the resident
  records coarse-then-fine. Editing the streamed die is a **compile error** (the `Streamed`
  arm hands out no `History`); browse/measure/query work.
- **Main-thread OPFS fallback:** the frozen `HttpRangeTileSource` caches tiles via OPFS
  `createSyncAccessHandle()`, which is **Web-Worker-only** and throws synchronously on the
  main thread, aborting every fetch. Since the index crate is frozen, the browse overrides
  `navigator.storage.getDirectory` to reject (CSP-safe) so the source's `opfs_dir()` returns
  `None` and it falls back to network + in-memory LRU. This was the one real integration
  bug found and fixed; without it, 0 tiles ever became resident.

### 2. On-canvas streaming HUD (SECONDARY — shipped in full)

Bytes fetched vs archive size, tiles resident, records painted, working-set estimate, and
fps. Counter arithmetic is a pure, unit-tested `ArchiveStats`; `TileInbox` was extended to
meter raw fetched bytes; the archive total size is probed with one ranged `bytes=0-0` GET
reading `Content-Range`. The same counters are published each frame to `window.__reticle_stats`.

### 3. Served-archive Playwright spec (PRIMARY)

- `e2e/serve-archive.mjs`: a dependency-free Node HTTP server with **Range** support
  (`206`/`Content-Range`), `HEAD`, and the CORS preflight the cross-origin `Range` header
  triggers, on port 8082 (the bundle is on 8080, so tile fetches are genuinely cross-origin,
  as a hosted archive would be).
- `e2e/tests/served-archive.spec.ts` + a `served-archive` Playwright project: opens
  `?archive=http://127.0.0.1:8082/fixture.rtla` and polls `window.__reticle_stats` to assert
  tiles become resident over Range and records paint. Runs against the **local** server
  regardless of cloud hosting.
- Committed fixture `e2e/fixtures/fixture.rtla` (a 3-level power-of-two pyramid), generated
  and round-tripped by `crates/reticle-app/tests/archive_fixture.rs` (built with the frozen
  `build_rtla`, read back by `MmapTileSource` — a permanent regression guard on the framing).

### 4. HUD numbers → PERF.md

A "v8.0.0 served-archive streaming HUD" section with the measured functional numbers
(archive 1132 B; 588 B fetched over Range across 21 tiles; 21 resident; 16 records painted;
~588 B working set) and full methodology, framed honestly as a functional proof on a tiny
fixture, not a scale benchmark (the at-scale residency behaviour is proven by the
`residency` integration test).

### 5. Docs / ADR

- `docs/src/streaming.md`: a "`?archive=` browse" + "streaming HUD" subsection (already in
  SUMMARY, so no SUMMARY change).
- `docs/decisions/XXXX-archive-url-browse-and-streaming-hud.md` + README row (**placeholder
  number `XXXX`**, to be assigned by the orchestrator at merge).

## The served-archive spec (evidence)

```
$ npx playwright test --project=served-archive
  ok 1 [served-archive] › streams a served .rtla over HTTP Range and paints resident tiles (558ms)
  1 passed (2.4s)
```

Live `window.__reticle_stats` after settle (from a direct Playwright drive):
`{archive_file_size:1132, archive_bytes_fetched:588, archive_tiles_fetched:21,
archive_tiles_resident:21, archive_records_painted:16, archive_working_set_bytes:588}` —
no page errors. The `webgl2` boot gate still passes (no regression from the added server/project).

## Gate — all green

| gate | result |
|---|---|
| `cargo nextest run -p reticle-app` | 607 passed, 0 skipped |
| `cargo clippy -p reticle-app --all-targets -- -D warnings` | clean |
| `cargo clippy -p reticle-app --target wasm32-unknown-unknown -- -D warnings` | clean (the `?archive` wasm path compiles) |
| `RUSTDOCFLAGS=-D warnings cargo doc -p reticle-app --no-deps --document-private-items` | clean |
| served-archive Playwright spec (local) | passes |
| `scripts/check-style.ps1` + `typos` on new files | clean |

(The pre-commit hook additionally runs the full-workspace `cargo fmt --check` + `clippy
--workspace --all-targets -D warnings` on every commit; all four commits passed it.)

## What was trimmed (honest gaps)

- **Real-chip gallery deep link (tertiary):** not shipped. `emit_archive_link` (the pure
  builder a gallery entry would serialize) is implemented and tested, so the Start-screen
  entry is a small follow-up, but the Start-screen wiring itself was not done to keep the
  primary bar and the HUD solid.
- **3D ride-along orbit over the streamed die (tertiary):** not shipped.
- **Browse chrome reuses the editor chrome:** in browse mode the toolbar is still present,
  so switching to a draw tool would edit an off-screen scratch document rather than the
  streamed die (which stays read-only *by type* — the streamed scene is never mutated).
  Camera pan/zoom and measure work. A dedicated read-only browse chrome (hiding editing
  tools) is a follow-up.
- **Cross-reload OPFS persistence:** disabled on the main thread by necessity (the frozen
  source's OPFS uses a worker-only API). The in-memory LRU still spares re-fetches while
  panning; full persistence needs a worker-hosted source (lane 2D-alpha's territory) and
  would slot into the same wiring unchanged.

## Files touched (within the allowed set)

- `crates/reticle-app/src/`: new `archive.rs`; `streamed.rs` (inbox byte metering);
  `share.rs` (`?archive` parser); `app.rs` (browse field, `with_archive`, `drive_archive`,
  canvas branch, `draw_archive`/`draw_streamed_records`/`draw_archive_hud`/`publish_archive_stats`);
  `lib.rs` (module + re-exports); new `tests/archive_fixture.rs`.
- `crates/web/src/main.rs`: `Boot::Archive` boot path.
- `e2e/`: `serve-archive.mjs`, `tests/served-archive.spec.ts`, `playwright.config.ts`,
  `fixtures/fixture.rtla`.
- `docs/`: `src/streaming.md`, `PERF.md`, `decisions/XXXX-*.md`, `decisions/README.md`.

No frozen crate (`reticle-index`, `reticle-io`), `worker/**`, `docs/TASKS.md`,
`scratch/RUN_STATE.md`, or root `RESULT.md` was touched.
