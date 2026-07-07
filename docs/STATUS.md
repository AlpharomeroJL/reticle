# Reticle, honest status report

This document is the result of a deliberate, skeptical audit of the whole workspace:
what is genuinely implemented and tested, what is partial, and what is claimed but
not yet built. It is written to be checked, not believed, every claim below has a
command you can run yourself (see [How to verify each claim](#how-to-verify-each-claim-yourself)).

- **Date of audit:** 2026-07-01 for the v3/v4 baseline below; the v5.0.0 agent layer
  was audited 2026-07-03 (see [v5.0.0 progress](#v500-progress-the-agent-layer-audited-2026-07-03)).
- **Machine:** NVIDIA GeForce RTX 4060 Ti 16 GB, Windows 11, Rust 1.94.1 stable
- **Gate:** `just ci`, fmt, clippy (`-D warnings`, pedantic), nextest, doctests, doc
  build (`-D warnings`), wasm build, `cargo-deny`, `typos`. Green.
- **Tests:** 291 `#[test]`/`#[tokio::test]` functions across 45 files, plus proptest
  cases; nextest reports **291 passed, 1 skipped** (the one skip is a corpus-
  regeneration utility, not a hidden failure, see below).

## Headline

Reticle is **real**. Every subsystem's core algorithm genuinely computes from its
inputs, verified by reading the code and, where it matters most (geometry booleans,
spatial index, DRC, routing, extraction, CRDT convergence), by independent
reference-oracle property tests that a faked implementation could not pass. There are
**no** `todo!`/`unimplemented!`/`unreachable!` bodies, **no** feature-gated stubs,
**no** `#[ignore]`d failing tests, and **no** `unsafe` anywhere in the workspace.

Three things are **claimed more broadly than they are built**, and were corrected in
the docs during this pass (details in [Known limitations](#known-limitations)):

1. **Out-of-core streaming**, a real zero-copy archived-index primitive exists, but
   it is not wired to a file/mmap, so "browse a bigger-than-RAM layout from disk" is
   not a shipped capability. Documented, with [ADR 0013](decisions/0013-out-of-core-streaming-scope.md).
2. **Per-cell bbox caching**, bounding boxes are computed correctly but **not**
   memoized; the README's "caching" wording was corrected to "computed".
3. **GPU-driven culling** (this v3.0.0 gap is closed in v4.0.0): the compute shader
   computes per-cell visibility, and Wave R adds the workgroup-scan compaction into a
   `DrawIndexedIndirectArgs` plus the indirect draw, so it is now a real GPU-driven
   draw list, not just a flag buffer.

## v4.0.0 progress (layered on the v3.0.0 audit above)

An in-progress v4.0.0 pass. The gate stays green (`just ci`, now with a `check-style`
step that forbids em-dashes, ADR 0014). Everything below is committed and measured on
this machine; nothing here is claimed beyond what the tests and benchmarks show. Of the
three v3.0.0 gaps above, all three are now closed (gap 3, the compacted indirect-draw
list, closed in the Wave R GPU-driven draw list below).

Delivered:

- **Voice rule + check-style gate.** Em-dashes removed tree-wide; `just check-style`
  fails the gate if any reappear (ADR 0014).
- **Per-cell bbox cache** (closes gap 2). `EditableDocument` memoizes `cell_bbox`,
  cleared on every edit. Warm reads about 295x faster (6.14 us vs 20.8 ns) on a
  100k-leaf hierarchy; tests pin it to the uncached recompute and to invalidation.
- **Out-of-core streaming via mmap** (closes gap 1). `StreamingIndex::open` memory-maps
  a tile-organized archive so a query faults in only touched tiles. Measured: a 574 MiB,
  30M-entry archive queries in about 14 us with a 4.25 MiB working set (about 135x below
  the file). One documented `unsafe` block (the workspace's only one), miri-gated tests,
  ADR 0016 (supersedes 0013). The archive builder is still RAM-bound (a single archive
  above about 2 GiB is a follow-up).
- **OASIS subset extended** to paths, instances, and arrays for read and write; the
  `Unsupported` errors are gone, a 256-case proptest round-trips every kind (ADR 0015).
- **Incremental DRC made genuinely sublinear.** `DrcEngine::prepare` builds the index
  once; `PreparedDrc::check_region` re-checks only the edit neighbourhood: 5.12 us at
  100k and 37.5 us at 1M shapes, far under the 100 ms target, pinned to the full-pass
  oracle by a property test.
- **Offscreen render fps measured, then the bottleneck fixed** (`fps_bench` example).
  The old path measured 1M at 65.7 fps and 10M at 6.1 fps (missing 30fps), bottlenecked
  by the per-frame CPU scene build and a 256 MiB single instance buffer. The retained
  renderer (below) closes that: 1M at about 295 fps and 10M at about 113 fps. See PERF.md.
- **Deep UI panels** in the egui editor: a DRC panel (run, list, click to zoom, painted
  markers), net highlighting (click a shape, extract, highlight the connected net), and
  a properties inspector (layer, bbox, width, height, area). Logic is unit-tested.

Wave R (closing v4.0.0), all merged to main with the full gate green at each merge:

- **Windowed GPU surface + retained scene (Wave A).** The interactive canvas now renders
  through `reticle-render` via an `egui-wgpu` paint callback on eframe's device. A
  `RetainedScene` caches per-cell tessellation once, expands instances into a per-instance
  transform buffer, and stores geometry in fixed-size GPU pages uploaded with
  `queue.write_buffer`; a camera move rewrites one uniform. An on-screen fps and
  frame-time readout is wired. This lifts 10M from 6.1 to about 113 fps (PERF.md).
- **GPU-driven draw list (Wave B, closes gap 3).** A workgroup-scan compaction pass turns
  the cull visibility flags into a compacted index buffer plus a `DrawIndexedIndirectArgs`,
  drawn with one indirect draw (native `multi_draw_indexed_indirect` where the adapter
  offers `MULTI_DRAW_INDIRECT_COUNT`). 4x MSAA on the offscreen target with a tolerance
  golden comparator, and zoom-driven LOD chunk selection.
- **Remaining UI (Wave C).** Canvas text labels (an egui-painter overlay; `glyphon` was
  evaluated and not needed), a minimap with click-to-recenter, a multi-viewport split,
  and rebindable keybindings loaded from a TOML keymap with an editor and conflict
  detection. Each is a unit-tested logic module.
- **3D layer-stack view (Wave E).** An extruded 3D view (orbit camera, depth buffer,
  per-layer thickness from an optional `stack` technology directive) rendered via its own
  paint callback, plus a cut-line cross-section panel.
- **Measurements (Wave F).** WASM cold-load-to-interactive (about 640 ms cold on WebGPU)
  and two-client collaboration echo (about 0.79 ms median on the localhost relay) are now
  measured with real harnesses and recorded in PERF.md.

Remaining before the v4.0.0 tag: additional overlay media (DRC, route, collaboration,
minimap, and 3D stills or GIFs beyond the existing hero and browse assets), then the
tagged release. A 30-second operator launch of the native window is the one visual check
a headless run cannot perform; correctness is otherwise covered by the golden-image and
offscreen paths plus the fps harness.

The v3.0.0 Section 16 audit below remains accurate for the v3.0.0 baseline.

## v5.0.0 progress (the agent layer, audited 2026-07-03)

v5.0.0 adds an agent layer on top of the v4.0.0 engine. This pass audited it with the
same skepticism as the v3 and v4 audits. Everything below is committed, gate-green
(`just ci` plus `just e2e`), and honest about its limits.

- **Machine and gate:** same host (RTX 4060 Ti, Windows 11, Rust 1.94.1). `just ci`
  (check-style, fmt, clippy `-D warnings`, nextest, doctests, doc build `-D warnings`,
  wasm build, `cargo-deny`, typos) is green, and `just e2e` (Playwright) is a second
  gate. Test functions across the workspace: **775** (up from 291 at v4).
- **No stubs.** Zero `todo!`/`unimplemented!` in shipped code. The two `unreachable!`
  in `reticle-mcp/src/context.rs` are defensive invariant assertions (`parse_render_args`
  always builds a `RenderPng`; `render_png` is a known tool), not unimplemented paths.
  `unsafe`: one, the documented mmap in `reticle-index` (unchanged from v4). `#[ignore]`:
  three, all legitimate (two corpus-regeneration utilities, one test gated on fetched
  SKY130 cells that are not committed); none is a hidden failure. No AI attribution and a
  single author across all history; no leaked secret in the working tree or full git
  history (`just check-keys -History`).

### What the agent layer really is

- **reticle-agent-api** DONE. A serde-serializable `AgentCommand` enum of 25 engine
  operations, a `Session` with a stable element-id allocator, and replayable transcripts
  with a `document_hash`. 32 tests including id-tracking and robustness proptests. Builds
  for wasm32 (render_png degrades to a clean error, now_ms returns 0).
- **reticle-agent** DONE. The propose-verify-correct loop verified by the SKY130 DRC
  subset plus intent; the `AnthropicModel` client (key from env only, redacted from every
  artifact and proven absent by the key scan); the `AgentCollaborator` bridge that mirrors
  edits onto the CRDT as atomic steps. Convergence and loop tests pass.
- **reticle-mcp** DONE. 25 command tools plus 3 context tools over hand-rolled stdio
  JSON-RPC, generated from the frozen command types; a subprocess test drives and asserts
  all 28.
- **reticle-bench** DONE. 63 graded tasks across five tiers, each with a two-way-tested
  checker; the deterministic mock runner; failure mining and `just bench-promote`. A
  committed test replay-verifies every task's transcript and asserts the suite is
  deterministic across runs.
- **reticle-demo and reticle-demo-server** DONE. The rate-limited service (every
  `LimitConfig` field enforced, 8 in-process abuse tests) plus a composition binary that
  runs the real harness behind it and streams each step to a relay room. Live-wiring
  integration tests decode the CRDT frames a watcher receives and prove server-side
  cancel; an abuse probe against the running binary confirmed 400, 409, and 429.
- **reticle-app** DONE (native); PARTIAL (wasm). The agent panel, live DRC overlay, and
  replay theater are implemented and tested on native. On wasm the start-view seam and
  index framing are in place but the in-page replay-theater window is native-only today
  (35 cfg gates entangle it with fs-based session persistence), so the public browser
  bundle opens to the editor, not the theater. Documented in `docs/src/deployment.md` and
  ADR 0026; un-gating it for wasm is a follow-up. The agent story on the web is the
  `agent.gif` and the demo server.

### Honest limitations (v5)

1. **The benchmark result is a mock baseline.** With no `ANTHROPIC_API_KEY` in this
   environment, the 63-task suite ran against the deterministic mock, which solves only
   the three scripted sample tasks (3/63). That validates the machinery end to end for
   all 63 tasks; it is not a measure of a language model's layout ability, and it is
   labeled as such in `docs/src/benchmarks.md`. A keyed run against the real model is a
   follow-up.
2. **The scale-proof DRC and extract numbers include the report.** The 4.19M-leaf
   pipeline import (37 ms / 7.5 MB) and render (809 ms / 594 MB) are clean; the DRC
   (11.0 s) and extract (12.2 s) rows are whole-pipeline times dominated by emitting a
   per-item text report for millions of items, not the core algorithm (isolated in the
   criterion sections). Stated plainly in PERF.md.
3. **The wasm replay theater** is native-only today, as above: a documented follow-up.
4. **Fuzzing still does not run on this Windows/MSVC host** (v4 limitation, unchanged);
   parser and boolean robustness is covered by proptests in the gate.

### How to verify (v5), from `D:\dev\reticle` (PowerShell)

```powershell
just ci                  # the whole gate, green
just e2e                 # Playwright: the webgl2 gate plus a webgpu-flagged run
just bench-agent         # the 63-task suite against the mock (3/63, labeled a baseline)
just demo-up             # the rate-limited demo server (offline scripted agent, no key)
just check-keys -History # no leaked secret in the tree or the full history
cargo nextest run -p reticle-demo-server   # live-wiring, server-side cancel, abuse tests
cargo nextest run -p reticle-bench         # replay-hash determinism over every transcript
git log --format='%an <%ae>' | Sort-Object -Unique   # one author, no AI attribution
```

## v6.0.0 progress (Wave 1, audited 2026-07-03)

v6.0.0 fixes the front door, adds a local benchmark backend, and (in later waves)
expands the editor, agent, and guided experience. Wave 1 is merged and gate-green
on this host (`just ci`, `just e2e` plus `just e2e-subpath`, and `just smoke-pages`
against the live redeployed site all pass). The full skeptical re-audit lands at
Wave 5; this section records Wave 1 and its Pages postmortem.

- **Pages base-path fix (Lane 1A) DONE and verified live.** The public bundle now
  loads under the project subpath. `just deploy-pages` builds with
  `trunk --release --public-url /reticle/` and asserts the emitted `index.html`
  carries `/reticle/`-prefixed assets and no bare-root refs; the artifact is
  published to gh-pages and `just smoke-pages` confirms the live base URL and both
  assets return 200 under `/reticle/`. The replay theater is un-gated for wasm32
  through a `SessionStore` seam (filesystem on native, a bundled transcript on
  web), so the browser bundle opens into a playing theater instead of hanging.
- **Ollama benchmark backend (Lane 1B) DONE.** An OpenAI-compatible `OllamaModel`
  in `reticle-agent` drives the propose-verify-correct loop against a local model;
  `ResultRecord` gained backend and quantization labels (ADR 0029) so mock, local,
  and frontier runs are never conflated. The full 63-task local run is an
  orchestrator step tracked in the benchmark chapter.
- **Test count:** 798 workspace test functions (up from 775 at v5), all green.

### Pages postmortem (what broke, why the gauntlet missed it, what prevents it now)

What broke: the deployed `index.html` imported its wasm loader and module at
absolute root (`/web-<hash>.js`, `/web-<hash>_bg.wasm`), but GitHub project Pages
serve this site under `/reticle/`. The browser therefore fetched the assets from
the domain root, got 404s, and the page hung forever on "Loading the replay
theater". Root cause: `just web-build` ran `trunk build --release` with no
`--public-url`, so Trunk emitted absolute-root asset paths.

Why the gauntlet missed it: the v5 QA gauntlet exercised the bundle only at the
server root (`http://127.0.0.1:8080/`), where absolute-root asset refs resolve
fine, so the subpath break never manifested. There was no deployed-URL check and
no test that served the bundle at the `/reticle/` subpath, so no gate saw the
condition GitHub Pages actually serves under.

What prevents it now: (1) `just deploy-pages` bakes `--public-url /reticle/` into
the build permanently and fails if the emitted `index.html` is not
`/reticle/`-prefixed; (2) a Playwright `ghpages-subpath` project (`just
e2e-subpath`) serves the exact bundle at `/reticle/` and asserts the app reaches
its started event, so a base-path regression fails before deploy; (3) `just
smoke-pages` fetches the live deployed URL and asserts every asset is 200 under
`/reticle/`, run at the Wave 1 merge and wired for the release; (4) the wasm
theater un-gate plus a visible-error path in `index.html` mean any load failure
surfaces a message rather than an infinite spinner. One honest limitation: the
final "started event" confirmation on the live URL was made through the
`ghpages-subpath` e2e run over the identical bundle, not a live headless browser
(the Playwright MCP browser backend was unavailable this session); `just
smoke-pages` covers the live asset resolution.

### v6.0.0 final re-audit (all waves, audited 2026-07-03)

v6.0.0 fixed the front door, added a local benchmark backend, expanded the editor
and the agent, and shipped a guided experience. Audited with the same skepticism.
`just ci` (1098 test functions, up from 775) plus `just e2e` and `just e2e-subpath`
are green; the greps below hold.

Standard greps: zero `todo!`/`unimplemented!` in shipped code; two defensive
`unreachable!` in `reticle-mcp` (invariant assertions, unchanged from v5); one
`unsafe` (the documented mmap in `reticle-index`, unchanged); three legitimate
`#[ignore]` (corpus-regeneration and a fetched-cell-gated test); a single author
across all history; no leaked secret in the tree or full history
(`just check-keys -History`); no AI-attribution string in any file or commit.

New subsystems, itemized:

- **Wave 2 editor (8 lanes) DONE.** Drawing and vertex editing (polygon/path/rect
  tools, vertex drag/insert/delete, modifier constraints); boolean and transform
  ops on the selection (union/intersection/difference/xor, offset, rotate/mirror,
  align/distribute, single-step undo via `apply_group`); productivity (copy/cut/
  paste, array, via-stack); snapping and guides (geometry snap + a `snap_world`
  seam the drawing tools route through); layer manager upgrade + technology editor
  (with a real `reticle_io::write_technology` round-trip and
  `EditableDocument::set_technology`); search/selection filter language + saved sets
  + outline tree; view/export (theme, bookmarks, SVG plus PNG); in-app agent UX.
- **Wave 3 agent (6 lanes) DONE.** Five new AgentCommands (boolean/align/distribute/
  offset/via-stack, ADR 0031) with MCP tools and two-way schema tests; region+rule
  context packs (a measured ~30x token reduction versus same-fidelity whole-document
  context); mid-session refinement folded into the loop without changing the frozen
  `Context` (a `RefinementSource` seam); a per-iteration structured plan step stored
  additively in the transcript (ADR 0032) and rendered in the panel; tool-surface
  failure-mining; the suite expanded to 75 tasks (v0.4.0).
- **Wave 4 guided experience (3 lanes) DONE.** A unit-tested first-run tour (native
  and wasm); four bundled worked use cases behind a Start screen (a SKY130 cell and
  its technology are compiled in, so they work on wasm); the positioning, benchmark,
  and sky130 credibility chapters plus a top-down README overhaul with the stale
  claims swept.

Honest limitations (v6):

1. **The benchmark is local models.** On the 75-task v0.4.0 suite: `gpt-oss:16k`
   (MXFP4) 50/75 = 67%, `qwen2.5-coder:16k` (Q4_K_M) 25/75 = 33%. These are a
   realistic floor for small quantized local models on this task, not an upper
   bound, and are labeled by backend/model/quantization. Local outputs are not
   deterministic; the transcript-replay determinism (which replays recorded
   transcripts to a fixed `document_hash`) is unaffected and is a committed test.
2. **Tool-surface failure mining has nothing to mine yet.** The clustering dimension
   is implemented and tested, but the committed local runs are `ResultRecord`-only
   (the local runner does not persist per-command transcripts), so no tool-surface
   candidate can be drafted from them. Persisting run transcripts is a follow-up.
3. **Media is the regenerated existing set.** The hero (2560x1440), the browse and
   agent GIFs, and the five engine stills are regenerated deterministically by
   `just capture-media`. The more ambitious per-new-feature GIF tour (a separate
   clip for draw, boolean, array, and via-stack) would extend the capture harness
   and is a documented partial, not shipped.
4. **The agent "fix violation" affordance is UI plus a seam.** The DRC-panel button
   assembles the violation region and rule into a real context string and launches a
   session with it; the scoped-region enforcement is the Wave 3B context-pack seam,
   so the clipping to the region is narration, not a hard constraint on the run.
5. **Fuzzing still does not run on this Windows/MSVC host** (unchanged from v4/v5);
   parser and boolean robustness stay covered by the gate's proptests.

### v6.0.1 (media truth pass, audited 2026-07-03)

A presentation pass, no engine or agent features. It corrects three of the v6.0.0
limitations above and grounds the README in re-measured numbers.

- **Media is now real UI captures** (closes limitation 3). The README hero and the five
  tour GIFs are full-window screenshots of the running application (panels plus canvas),
  captured by a scripted demo mode in `reticle-app` (`--demo-script`) that drives the editor
  through the same code paths the interactive UI uses, and assembled by `just capture-ui`.
  The old offscreen canvas-only set is no longer what the README shows. The demo scripts are
  committed under `crates/reticle-app/demo-scripts/`, so every capture is reproducible. The
  egui `ViewportCommand::Screenshot` path was verified to yield a non-blank full-window frame
  on this wgpu backend, so no Windows window-capture fallback was needed.
- **The benchmark was re-run over the true 75-task suite, with committed records**
  (updates limitations 1 and 2). Both local models ran the full v0.4.0 suite over Ollama:
  `gpt-oss:16k` (MXFP4) **52/75 = 69%**, `qwen2.5-coder:16k` (Q4_K_M) **29/75 = 39%**, with
  each task's `ResultRecord` and command transcript committed under `benchmarks/results/`.
  The v6.0.0 table showed 50/75 and 25/75, which were the 63-task pass rates projected onto a
  75 denominator; these are direct 75-task measurements instead. Because the transcripts are
  now committed alongside the records, tool-surface failure mining can draft candidates from
  them, which limitation 2 said it could not.
- **README voice plus a gate.** The README was rewritten to lead with the measured fact and
  carry a number or a link on every claim. `just check-style` gained a banned-word list for
  README.md, so a marketing adjective fails the gate.

The remaining v6.0.0 limitations (the local-model floor, the UI-plus-seam "fix violation"
affordance, and fuzzing not linking on Windows/MSVC) are unchanged.

## v7.0.0 progress (The Product Packet, audited 2026-07-06)

v7.0.0 adds the viewer wedge (open, inspect, share, generate IC layout in a browser
with no install), a parameterized generator layer, a Claude Code agent-system benchmark
backend, and a TinyTapeout tape-out oracle. Audited with the same skepticism. Same host
(RTX 4060 Ti, Windows 11). Gate green: `just ci` (check-style, fmt, clippy `-D warnings`,
nextest, doctests, doc build `-D warnings`, wasm build, `cargo-deny`, typos), plus
`just e2e` (3 passed, 1 skipped honestly headless), `just e2e-subpath` (1 passed), and
`just smoke-pages` (live). Test functions: **1333** (up from 1098 at v6). Standard greps:
zero `todo!`/`unimplemented!` in shipped code; one `unsafe` (the documented mmap in
`reticle-index/streaming.rs`, unchanged since v4); three legitimate `#[ignore]`
(corpus-regeneration and a fetched-cell-gated test); a single author across all history;
no AI-attribution string in any file or commit; no leaked secret in the tree or full
history (`just check-keys -History`). A fresh clone of HEAD cold-built the whole
workspace in 2m43s.

New subsystems, itemized with their honest limits:

- **Wave 1, the viewer wedge, DONE.** Open-anything import hardening (a 256 MiB size
  cap, safe `catch_unwind` panic containment of gds21's vectors, structured
  `ImportWarning` degradation) and the platform-neutral `reticle_app::open_document_bytes`
  seam, proven by a corpus-iteration test (opens or fails cleanly, zero panics) over a
  committed TinyTapeout corpus (ADRs 0034/0035). Browser open: drag-drop, `?gds=<url>`
  fetch, IndexedDB recents, size-banded progressive load, a measured 256 MiB ceiling
  (ADRs 0036/0037). Read-only shareable sessions: a viewer joins `?mode=view` and
  receives the sharer's frames but never publishes, enforced server-side (the relay
  drops viewer frames) and app-side, with follow-mode and rate-limited/TTL share rooms
  (ADRs 0038/0039). A Start screen with an example-chip gallery, one app-level
  error/notification surface, and a tour covering open and share (ADRs 0040/0041).
  *Honest gap:* the share LINK is generated and the read-only guarantee is proven by
  tests, but the LIVE client transport (a wasm `web_sys::WebSocket` collaboration path,
  sharer-publish and viewer-subscribe) is NOT built, so a second browser does not yet see
  a session live; the drop-and-open path is proven by an app-level integration test, not
  a full browser e2e. Both are documented, not hidden.
- **Wave 2, the generator layer, DONE.** Six generators (guard ring, via farm, pad ring,
  seal ring, density-aware fill, probe-able test structures) behind a typed `Generator`
  trait plus a type-erased registry, each DRC-clean by construction against the SKY130
  subset, proven by 400-case cleanliness proptests over the real `DrcEngine` (ADRs
  0042-0047). A Generate panel with a schema-driven form and live preview; each generator
  exposed as an agent and MCP tool via an additive `RunGenerator` command (ADRs
  0048-0050); the benchmark suite grown to 83 tasks (v0.5.0) with a two-way-tested
  generator checker. *Honest:* the generators cover the cited SKY130 subset, not the full
  foundry deck, and their numbers are baked, not read from an arbitrary technology.
- **Wave 3, the Claude Code agent-system backend, DONE (built), NOT RUN (here).**
  Server-side transcript capture in `reticle-mcp` (every applied command streamed to a
  JSONL, replay-verified; ADR 0051) so a client the harness does not control leaves a
  mineable transcript. A `claude-code` backend that drives `claude -p` non-interactively
  as an agent system (it brings its own loop, so not a `ModelClient`), replays the
  captured transcript, and checks it into a `ResultRecord` labeled `claude-code`, with a
  distinct `NotRunRecord` for a missing/unauthenticated CLI that can never be counted as
  pass or fail (ADR 0052). *Honest, and the whole point of the wave:* the `claude` CLI is
  present here but UNAUTHENTICATED, so the Claude Code row is a not-run, no score is
  published, and the benchmark chapter explains the agent-system versus bare-model
  distinction and the re-run (`claude` `/login`, then `just bench-agent-claude-code`; on
  Windows also set `RETICLE_CLAUDE_BIN`). The two local rows shown are still the 75-task
  v0.4.0 numbers; re-running local models on v0.5.0 is a follow-up; transcript mining has
  no new server-side transcripts to mine yet.
- **Wave 4, the tape-out oracle, DONE (with the live run deferred).** A TinyTapeout
  technology-plus-template bundle for a GDS-mode tile, transcribed from TinyTapeout's own
  DEF/init files and validated zero-tolerance against them and cross-checked against the
  published `tt_um_analog_mux` submission (ADR 0053). `just tt-precheck <gds>` wraps
  TinyTapeout's own Magic+KLayout precheck via a pinned Docker image, with a
  structured-failure parser and an agent-loop feedback seam, two-way tested over honestly
  labeled synthesized fixtures (ADR 0054). A committed worked tile
  (`examples/tapeout/tt_um_reticle_tile.gds`) built through the frozen agent surface with
  a replayable transcript, DRC-subset-clean (ADR 0055). *Honest:* the tile is
  generator-built (NOT agent-authored, the CLI is unauthenticated) and DRC-clean against
  the SKY130 SUBSET, NOT run through the real precheck (that needs a multi-GB image and is
  the operator's step); no tile in the repo is claimed to pass the precheck.
- **Wave 5, presentation, DONE.** The README rebuilt as a product page (voice rules and
  banned-word gate kept) with a newly captured generator GIF driving the real Generate
  panel and an honest three-row benchmark table (two bare-local-model rows plus the
  Claude Code not-run). A real interaction-latency fix: `Document::flatten_local` no
  longer recomputes the array-placement transform per copy, a measured 68% drop on the
  flatten bench (open CPU on a 4.19M-leaf design ~230 to ~31 ms), correctness pinned by an
  equivalence test, with a zero-buffer-growth soak (before/after in `docs/PERF.md`).
  *Honest:* wasm live-pan latency is browser-only and not measured here. (An earlier
  revision filed a "GDS AREF-decode off-by-one found in passing"; that was a misdiagnosis.
  The AREF import copies the COLROW counts verbatim and flatten loops `0..columns`, so the
  parse is launch-independent; the real launch-context effect was a working-directory bug in
  `scripts/measure-run.ps1`. Retracted and closed at the v7 finish, the decode pinned correct
  by a round-trip leaf-count test. See ADR 0057 and `docs/PERF.md`.)

Honest limitations (v7), consolidated: (1) the Claude Code benchmark row is a **not-run**
(CLI unauthenticated in this environment); (2) the TinyTapeout **precheck** live run and
the **share-live** browser transport are operator/follow-up steps, not shipped here;
(3) the local benchmark rows are the 75-task v0.4.0 numbers, not the 83-task v0.5.0
suite; (4) the generators and DRC are the SKY130 subset, not the full deck; (5) fuzzing
still does not link on this Windows/MSVC host (unchanged from v4). None of these is
hidden behind a passing test; each is documented in its ADR, the book, and above.

## Section 16 (definition of done), item by item

| # | Item | Status | Evidence |
|---|------|--------|----------|
| 1 | Every crate builds; `just ci` green | **DONE** | `just ci` → `ci: GREEN`; `cargo build --workspace --release` exits 0 |
| 2 | Native app + browser demo run; live-demo link works | **DONE** | `cargo run -p reticle-app`; live demo returns HTTP 200 at the Pages URL; browser-verified in the prior build |
| 3a | Editing + hierarchy function w/ tests | **DONE** | `reticle-model` undo/redo records real inverses (`editable.rs`), `flatten` expands transforms (`document.rs:260`); `prop_editing.rs` undo/redo identity property |
| 3b | DRC functions w/ tests | **DONE** | 8 rule kinds in `reticle-drc/src/lib.rs`; `tests/property.rs` matches a naive O(n²) oracle over 400 cases/rule |
| 3c | Routing functions w/ tests | **DONE** | A* via `pathfinding` + rip-up/reroute (`route/src/lib.rs:274`); `routing.rs` Manhattan-optimality oracle + impossible-enclosure failure test |
| 3d | Extraction functions w/ tests | **DONE** | R-tree union-find + cross-layer via bridging (`connectivity.rs`); `extract/tests/property.rs` independent O(n²) union-find oracle |
| 3e | IO functions w/ tests | **DONE (subset)** | GDSII via `gds21`; in-house OASIS subset read+write round-trips (`oasis_roundtrip.rs`). OASIS covers rect+polygon; paths/instances/arrays return `Unsupported` (honest error, not silent drop) |
| 3f | Scripting functions w/ tests | **DONE** | `reticle-script` rhai API (`scripting.rs`, `plugins.rs`) |
| 3g | Collaboration functions w/ tests | **DONE** | `reticle-sync` yrs CRDT; `convergence.rs` asserts order-independent identical state across independently-mutated docs |
| 4 | Performance measured + recorded | **DONE (v4.0.0)** | Index build 227 ms, nearest query 926 ns, union 271 µs; retained render 1M ~295 fps and 10M ~113 fps; WASM cold load ~640 ms; collab echo ~0.79 ms median. All on this machine, all in PERF.md |
| 5 | Property/golden/CRDT tests pass; fuzz targets exist | **PARTIAL** | Property, golden-image (`render/tests/golden.rs`), and CRDT convergence tests all pass. Fuzz **targets exist** but the libFuzzer engine does not link on Windows/MSVC (no compiler-rt), documented in `fuzz/README.md` |
| 6 | Book + rustdoc build + deployed | **DONE** | `mdbook build docs` exits 0 (16 chapters, no placeholders); `RUSTDOCFLAGS=-D warnings cargo doc` green; book returns HTTP 200 |
| 7 | Hero + browse GIF in README | **DONE**; overlay/3D media in progress | `assets/hero.png` (2560×1440, non-blank) and `assets/browse.gif` (48 real frames) generated by `xtask capture-media`. Additional DRC, route, collaboration, minimap, and 3D-view media are being captured through the overlay and offscreen passes the Wave R merges added |
| 8 | Tagged v3.0.0 release w/ binaries + notes | **DONE** | `gh release view v3.0.0`: not draft, 3 uploaded `.exe` assets with sha256, real notes |
| 9 | Requirements-mapping table complete + honest | **DONE** | `docs/requirements.md` |
| 10 | No AI attribution; no backdated/forged history | **DONE** | 26 commits, all author+committer `Josef Long <Josefdean@protonmail.com>`; 0 AI-attribution strings in any message; all dates 2026-07-01 |

## Per-crate status

| Crate | Status | What's real | Caveats |
|-------|--------|-------------|---------|
| `reticle-geometry` | **DONE** | Exact-integer primitives; `i_overlay` booleans/offset; winding; convex decomposition (ear-clipping). Booleans checked vs an independent winding-number oracle | None |
| `reticle-proto` | **DONE** | prost types, schema version + migration hook; round-trip tests | None |
| `reticle-index` | **DONE** (queries); **PARTIAL** (streaming) | R-tree (`rstar` STR bulk-load), uniform grid, tile/LOD pyramid; point/rect/nearest/k-NN vs `LinearIndex` oracle | `streaming.rs` is a zero-copy **in-memory** primitive; not wired to disk/mmap. See ADR 0013 |
| `reticle-io` | **DONE** (subset) | GDSII read/write (`gds21`); OASIS subset read/write; tech-file parser; robustness proptests | OASIS: rect+polygon only; paths/instances/arrays error as `Unsupported` |
| `reticle-model` | **DONE** | Cells/instances/arrays, nested transforms, transactional undo/redo, `flatten` | No per-cell bbox **cache**, `cell_bbox` recomputes (correct, uncached) |
| `reticle-render` | **DONE** | Offscreen and on-screen render (golden tests assert exact pixels); instanced + retained pipelines; GPU cull with workgroup-scan compaction to an indirect draw; 4x MSAA; 3D layer-stack pipeline | v4.0.0 closes the old gaps: the cull now compacts survivors into a `DrawIndexedIndirectArgs`, and the interactive canvas presents through an egui-wgpu paint callback |
| `reticle-drc` | **DONE** | 8 rule kinds, spatial-index accelerated, incremental `check_region`; oracle property test | Operates on bounding boxes (exact for rects; conservative for polygons/paths, documented) |
| `reticle-route` | **DONE** | Grid + maze A* (`pathfinding`), rip-up/reroute, cross-layer vias; oracle test | Synthetic CLI demo nets can be long on very large flattened designs |
| `reticle-extract` | **DONE** | Union-find connectivity over touching geometry + via bridging; netlist compare (opens/shorts); oracle test | Path shapes use bbox adjacency (conservative; rect/rect exact) |
| `reticle-sync` | **DONE** | yrs CRDT mirror, encode/decode, presence + comments, offline reconcile; order-independent convergence tests | None |
| `reticle-server` | **DONE** | axum + tokio relay, rooms, broadcast; relay tests | None |
| `reticle-script` | **DONE** | rhai API (create/query/transform/DRC/export), plugin folder, examples | None |
| `reticle-app` | **DONE** | egui editor: tools, palette, layers, measure, selection, session, undo panel; ~80 tests; native + wasm | Live 60-fps interaction is not captured by an automated fps benchmark |
| `reticle-cli` | **DONE** | Headless import/DRC/route/extract/export/render; now **flattens** the top cell so hierarchical designs are checked as real geometry; 14 tests | None |
| `web` | **DONE** | Trunk harness, WebGPU + WebGL2 fallback; compiles to `wasm32` in the gate | None |
| `xtask` | **DONE** | Deterministic layout generator; offscreen media capture; **real** `perf-check` (parses Criterion vs the committed baseline, fails on regression) | `perf-check` was a stub before this pass; now implemented |
| `benches` (workspace crate) | **DONE** | The actual Criterion benches live **in-crate** (`reticle-index/benches`, `reticle-geometry/benches`) and run under `cargo bench --workspace`; a committed baseline lives in `benches/history/` | The crate's `lib.rs` doc was corrected in v4.0.0 to describe its real role (history baseline + version stamp), not to claim it hosts the bench targets |

## Step 1 inventory, what the greps found

Run over the whole workspace:

- `todo!(` / `unimplemented!(` / `unreachable!(` : **zero** occurrences.
- `panic!(` outside `#[cfg(test)]` : only defensive `# Panics` bounds-checks in
  `reticle-index` (`grid.rs`, `lod.rs`) and `union_find.rs`, documented invariants,
  not stubs.
- `#[ignore]` : **one**, `reticle-io/tests/gds_corpus.rs:38`
  `regenerate_corpus`, an explicit `#[ignore = "run explicitly to regenerate
  tests/corpus/basic.gds"]` fixture-regeneration utility. Not a hidden failure.
- `#[cfg(feature …)]` gate : **zero** feature-gated code paths in the crates.
- `unsafe` : **zero** in the entire workspace (so `miri` has no unsafe to check).
- AI attribution (`Co-Authored-By`, `Claude`, `Generated with`, …) in files or commit
  messages : **zero**.
- "stub"/"for now"/"in a real"/"placeholder" comments : all are either legitimate
  domain terms (routing "via stub" = a physical via-stub `Path`) or honest
  scope disclosures (render cull "compaction is left as a follow-up"). The one true
  code stub found, `xtask perf-check` ("Wave 5 stub"), was **implemented** during
  this pass.

### "Functions returning a hardcoded value while pretending to compute", searched hard, found none

The DRC, router, and extractor were audited specifically for this. All three genuinely
compute, and each is pinned by an **independent reference oracle**:

- DRC engine equals a naive O(n²) width/spacing/area checker over 400 random layouts
  (`drc/tests/property.rs`).
- Router length equals an independently-derived Manhattan optimum, and an impossible
  enclosure reports `routed = 0` (not fake success) (`route/tests/routing.rs`).
- Extractor components equal a separate, private union-find's partition over 400 random
  layouts (`extract/tests/property.rs`).

## Known limitations

These were the honest gaps at v3.0.0. Items 1, 2, and 3 are closed in v4.0.0 (see the
v4.0.0 progress section above); they are kept here, marked closed, so the history is
legible. None is hidden behind a passing test.

1. **Out-of-core streaming (closed in v4.0.0).** `StreamingIndex::open` now memory-maps
   a tile-organized archive so a query faults in only touched tiles: a 574 MiB, 30M-entry
   archive queries in about 14 us with a 4.25 MiB working set. One documented `unsafe`
   block, miri-gated tests, [ADR 0016](decisions/0016-memmap2-out-of-core-streaming.md)
   (supersedes 0013). The archive builder is still RAM-bound (a single archive above about
   2 GiB is a follow-up).
2. **Per-cell bbox cache (closed in v4.0.0).** `EditableDocument` memoizes `cell_bbox`,
   cleared on every edit; warm reads about 295x faster, pinned to the uncached recompute
   and to invalidation by tests.
3. **GPU-driven draw list (closed in v4.0.0).** The cull's visibility flags are compacted
   by a workgroup scan into a `DrawIndexedIndirectArgs` and drawn with an indirect draw;
   4x MSAA and LOD switching are in. The CPU-side R-tree cull remains as the downlevel
   (WebGL2, no compute) fallback path.
4. **Fuzzing does not run on this host.** Targets are authored and committed, but
   libFuzzer needs LLVM compiler-rt (sancov/asan) which does not link under
   Windows/MSVC. Parser/boolean robustness is instead covered by proptests (2048 cases)
   in the gate. Run the fuzzers on Linux.
5. **OASIS is a subset.** Rectangles and polygons round-trip; paths, instances, and
   arrays are rejected with an explicit `Unsupported` error rather than silently
   dropped. GDSII (via `gds21`) preserves the full hierarchy.
6. **Performance targets measured (updated in v4.0.0).** The index/geometry targets, the
   retained render fps (1M ~295, 10M ~113), the WASM cold-load (~640 ms), and the
   collaboration echo (~0.79 ms median) are all measured on this machine now. See PERF.md.
7. **Additional overlay and 3D media in progress.** Hero image and browse GIF are real;
   DRC, route, collaboration, minimap, and 3D-view stills or GIFs are being captured
   through the overlay and offscreen passes the Wave R merges added.
8. **The `benches` crate doc is corrected (v4.0.0).** Its `lib.rs` doc now states that
   the real Criterion benches live in-crate (`reticle-index`, `reticle-geometry`,
   `reticle-drc`, `reticle-model`) and that this crate holds the committed history
   baseline and the suite version stamp.
9. **Rendering extras built (v4.0.0).** Window/surface presentation is now a real
   `egui-wgpu` paint callback on the retained scene, not a stub, and text labels (an
   egui-painter overlay; `glyphon` evaluated and not needed), a minimap, live DRC and net
   overlays, 4x MSAA, and a 3D layer-stack view with a cut-line cross-section are all
   implemented and tested. The one remaining visual check a headless run cannot do is a
   30-second operator launch of the native window.
10. **App features implemented (v4.0.0).** Rebindable keys (a TOML keymap with an editor
    and conflict detection) and multi-viewport split panes are now built and unit-tested,
    alongside the existing command palette, layer manager, selection filters, measurement,
    session save/restore, and undo history.

## How to verify each claim yourself

From `D:\dev\reticle` (PowerShell):

```powershell
# The whole gate (fmt, clippy -D warnings, tests, doctests, docs, wasm, deny, typos).
just ci

# Release build of every crate.
cargo build --workspace --release

# Full test suite with counts (no hidden #[ignore]).
cargo nextest run --workspace

# Benchmarks on this machine, then the regression gate against the committed baseline.
cargo bench --workspace
just perf-check      # reads Criterion's fresh estimates; PASS/REGRESSED per bench

# Headless pipeline on a generated design (drc/route/extract flatten the hierarchy).
just gen-layout 400 4 1 scratch/flat.gds
cargo run -p reticle-cli --release -- import  scratch/flat.gds
cargo run -p reticle-cli --release -- drc     scratch/flat.gds   # ~400 real violations
cargo run -p reticle-cli --release -- route   scratch/flat.gds   # 2 nets routed, real length
cargo run -p reticle-cli --release -- extract scratch/flat.gds   # 400 nets
cargo run -p reticle-cli --release -- render  scratch/flat.gds --out scratch/flat.png

# Round-trip through the OASIS subset (flat geometry).
cargo run -p reticle-cli --release -- export crates/reticle-io/tests/corpus/basic.gds --out scratch/basic.oasis
cargo run -p reticle-cli --release -- import scratch/basic.oasis

# Book and API docs.
mdbook build docs
$env:RUSTDOCFLAGS='-D warnings'; cargo doc --workspace --no-deps

# Release chain (read-only).
gh release view v3.0.0 --json assets
gh api repos/AlpharomeroJL/reticle/pages

# History honesty: expect a single author and zero AI attribution.
git log --format='%an <%ae>' | Sort-Object -Unique
git log --format='%B' | Select-String -Pattern 'Co-Authored-By|Claude|Anthropic'   # no matches
```

## Interview-defense notes (architecture → code)

- **Contract-first workspace.** Traits and types (`reticle-geometry`, `reticle-model`)
  were frozen in Wave 0 so consumer crates compiled against a stable surface while
  implementations filled in. See `docs/decisions/` and `docs/PLAN.md`.
- **Exact-integer geometry, robust booleans.** `Dbu = i32` with `i64`/`i128` math for
  area/bbox to avoid overflow; polygon booleans delegate to `i_overlay` rather than a
  hand-rolled clipper, and are checked against an independent winding-number oracle
  (`geometry/tests/booleans.rs`). Convex decomposition is ear-clipping with an
  area-conservation + convexity oracle.
- **Scale through hierarchy, not flattening.** Cells/instances/arrays keep the document
  tiny on disk; rendering culls whole instances. The GPU compute cull
  (`render/shaders/cull.wgsl`) is the first stage of a GPU-driven draw list.
- **Spatial acceleration everywhere.** One `rstar` R-tree underpins index queries, DRC
  candidate pruning (`drc` expands each shape's bbox by the rule threshold), and
  extraction adjacency, turning O(n²) sweeps into ~O(n log n).
- **Incremental DRC.** `DrcEngine::check_region` re-checks only geometry touching an
  edited rectangle, bounded by one index query (`drc/src/lib.rs:104`).
- **Negotiated-congestion routing.** `MazeRouter` rips up conflicting nets and reorders
  failed nets to the front for the next pass (`route/src/lib.rs:247`).
- **Order-independent collaboration.** The model maps onto a yrs CRDT with unique
  `actor:counter` keys and deterministic materialization, so concurrent edits converge
  regardless of delivery order (`sync/src/mapping.rs`, `sync/tests/convergence.rs`).
- **One gate, no hidden CI.** `just ci` is the sole gate; `xtask perf-check` guards
  performance regressions against a committed baseline.

## Changes made during this audit pass

- Implemented `xtask perf-check` for real (was a stub): parses the committed baseline
  and Criterion's `estimates.json`, prints measured-vs-baseline per bench, fails on a
  regression beyond a configurable tolerance.
- Restructured `benches/history/baseline.json` to be self-describing (per-bench
  Criterion path, unit, tolerance) with an accurate methodology note.
- Made the CLI `drc`/`route`/`extract` **flatten the top cell** (`flatten_top_cell`),
  so hierarchical designs (including every `gen-layout` output) are checked as real
  geometry instead of an empty pure-array top cell. Added a test.
- Implemented convex decomposition in `reticle-geometry` with an oracle property test.
- Corrected README/streaming docs that overstated out-of-core streaming and bbox
  caching, and refreshed the stale README status blurb.
- Added [ADR 0013](decisions/0013-out-of-core-streaming-scope.md).
