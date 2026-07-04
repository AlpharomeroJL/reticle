# 0036, The browser open path: drop, `?gds=` URL, and an IndexedDB recent list

## Context

The desktop app opens files from the filesystem, but a public web visitor has no
filesystem. The document-open seam (ADR 0035) already gives a platform-neutral
`open_document_bytes(bytes, format)`, but nothing in the wasm build actually obtains
bytes for it: there was no way to drop a `.gds`/`.oas` file onto the page, no way to
share a link that opens a remote file, and nothing remembered what a visitor had
opened across a reload. These three, plus a Start screen (owned by a sibling lane)
that wants to show a recent list, are the no-install open story for the browser.

The browser halves of this are inherently DOM- and network-bound: `web_sys` fetch for
a remote URL, and `IdbFactory`/`IndexedDB` for persistence. Those cannot be exercised
in a headless unit test, and egui's `update` loop is synchronous while fetch and
IndexedDB are async.

## Decision

Add a `reticle_app::webopen` module that is the browser open path over the seam, with
a hard line between pure logic and DOM glue:

- **Drag-and-drop:** the App reads egui's per-frame `ctx.input(|i| i.raw.dropped_files)`
  (each `DroppedFile` carries `.bytes` on web and `.name`), classifies the format from
  the name via `DocFormat::from_extension`, and opens through the seam. A "drop a .gds
  or .oas file" affordance is drawn while `i.raw.hovered_files` is non-empty. A drop of
  a non-layout file, or a failed import, sets a clear status rather than doing nothing.
- **`?gds=<url>`:** `gds_url_from_query` parses the parameter out of
  `window.location.search` (pure, unit-tested, percent-decoding included); the wasm
  glue fetches the URL with `web_sys` and posts the bytes back. A CORS or network
  failure becomes a human-readable message shown in a dismissible card, never a
  console-only error and never a hang.
- **Recent files in IndexedDB:** a pure `RecentFiles` model (dedupe by reopen-URL-else-
  name, most-recent-first, capped) with a hand-rolled JSON round-trip; the wasm glue
  reads/writes it under one key via `IdbFactory`. The App exposes `recent_files() ->
  &[RecentFile]` so a Start screen (Lane 1D) can render reopen rows; this lane owns
  recording and persistence, 1D only reads.

The async/sync bridge is a single-slot `WebOpenInbox`: spawned tasks post
`WebOpenEvent`s (progress, opened-bytes, failure, recents-loaded) into it, and the
App drains it each frame and applies each event on the main thread through the same
methods a native open uses. All decisions (classify, size-band, dedupe) live in the
pure half; the glue is thin. Fetch/IndexedDB glue and the wasm inbox plumbing are
`#[cfg(target_arch = "wasm32")]`; the native build keeps an empty inbox and never
spawns anything, so the model type is identical on both targets.

## Consequences

The no-install open path works three ways in the browser with no server round trip for
a drop, and every decision it makes is proven in plain unit tests (format
classification, URL parsing, the recent-list model and its JSON, the progress state
machine). What genuinely needs a browser, the fetch and the IndexedDB read/write, is
isolated behind the inbox and is verified by the orchestrator's Wave 1 end-to-end pass
(drop a corpus file, it renders; reload, the recent list is there), not faked in a
unit test. The App's `webopen` edits stay surgical: reading dropped/hovered files and a
small open-dispatch, plus a `recent_files` field and accessor, with all fetch/IndexedDB
logic in `webopen` behind a wasm gate, so the sibling lanes editing other regions of
`app.rs` do not collide. Recent entries store only a label and size (and a reopen URL
for remote files), never the bytes, so the list is a set of reopen targets, not a
cache; a dropped local file cannot be reopened without re-dropping it, which the model
records honestly by leaving its URL `None`.
