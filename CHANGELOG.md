# Changelog

All notable changes to Reticle are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com), and the project uses
[conventional commits](https://www.conventionalcommits.org).
## [6.0.1] - 2026-07-04

### Documentation

- V6.0.0 shipped (Wave 5 gauntlet, re-audit, tag, release, gh-pages redeploy)
- Implementation plan for v6.0.1 README and media truth pass
- README and book in an engineer's voice; banned-word gate

### Other

- Prove egui viewport screenshot capture path (demo-capture spike)
- Scripted demo-capture mode (--demo-script) for real UI media
- Capture-ui assembles README media from demo scripts
- Demo editing/query/3D actions and six real UI captures
- 75-task v0.4.0 re-run of gpt-oss:16k and qwen2.5-coder:16k
## [6.0.0] - 2026-07-03

### Features

- Drawing tools and vertex editing
- Boolean and transform operations on the selection
- Productivity editing (clipboard, array, move-delta, via stacks)
- Geometry snapping, ruler guides, and snap settings
- In-app agent UX (conversation, history browser, ask-to-fix)
- View and export panel (theme, bookmarks, SVG/PNG, monochrome)
- Filter query language, saved sets, select-similar, outline tree
- Layer manager upgrade and technology editor (lane v6-2e)
- Expose Wave 2 editor ops as agent commands and MCP tools
- Region-scoped context packs for scoped sessions
- Mid-session refinement protocol in the propose-verify-correct loop
- Per-iteration plan-step transparency
- Embedded first-run tour with a pure state machine (lane 4A)
- Bundled worked use cases + Start-screen chooser (Lane 4B)

### Bug Fixes

- Demote private and wasm-cfg intra-doc links to code spans
- Wait for the demo-server slot release instead of asserting on the instant
- Misread and PNG images in Wave 2 chapters; TASKS Wave 2 complete
- Give each mining test a fresh temp dir (clear stale before create)
- Fresh temp dir for loader and plugin tests (clear stale before create)

### Documentation

- V5.0.0 shipped (STATUS re-audit and release complete)
- V6.0.0 run scaffolding and shared-tree incident record
- Pages postmortem in STATUS; Wave 1 done-gate-green in TASKS
- 0031 authorize Wave 3 AgentCommand expansion for the Wave 2 tools
- Wave 3 batching plan and Batch-1 dispatch (agent capability expansion)
- Wave 3 Batch 1 done-gate-green; record 3A command surface and 3D plan log (ADR 0032)
- Wave 3 complete; Wave 4 dispatched; 75-task local re-run in progress
- V6.0.0 credibility chapters + README overhaul (Lane 4C)
- Wave 4 done; entering Wave 5 (QA gauntlet + v6.0.0 release)

### Testing

- Regression-test the live qwen content-embedded tool call

### Build and tooling

- Add lane/lane-done worktree recipes; record mandatory lane procedure

### Other

- Ollama backend, recovered from shared-tree incident
- Ollama.rs OpenAI-compatible backend and agent wiring
- Surface backend/quantization provenance in the results summary
- Ollama OpenAI-compatible benchmark backend
- Pages fix, recovered from shared-tree incident
- Session store abstraction, bundled theater transcript, e2e subpath project
- Un-gate the replay theater on wasm; green the lane gate
- Pages base-path fix, wasm theater un-gate, deploy/smoke recipes
- Gpt-oss:16k local run (42/63, 67%); TASKS benchmark + Batch-1 incident
- Drawing and vertex editing tools
- Boolean and transform operations
- Productivity editing (clipboard, array, via-stack)
- Snapping and guides
- Qwen2.5-coder:16k local run (28/63, 44%); Wave 2 Batch-1 done
- View and export polish (theme, bookmarks, PNG/SVG)
- Search and selection depth (filter language, saved sets, outline)
- Layer manager upgrade and technology editor (round-trip)
- Richer agent tool surface (boolean/align/distribute/offset/via-stack)
- Scoped sessions and minimal context packs
- Iterative refinement protocol
- Agent planning transparency (plan step per iteration)
- Cluster failures by Wave 3 tool surface
- Failure-mining tool-surface clustering
- Expand suite to 75 tasks with 12 Wave-3 tasks (v0.4.0)
- Benchmark expansion to 75 tasks (v0.4.0, boolean/array/via-stack/refinement)
- Embedded first-run tour
- Worked use cases and start screen
- Credibility chapters (positioning/benchmark/sky130) and README overhaul
- 75-task v0.4.0 two-model results and tables (gpt-oss 50/75=67%, qwen 25/75=33%)
- Regenerate hero (2560x1440) and browse GIF for v6.0.0 (deterministic capture-media)
- V6.0.0 (version bump, CHANGELOG, STATUS re-audit)
- Fix a typo carried into the CHANGELOG from an old commit subject
## [5.0.0] - 2026-07-03

### Bug Fixes

- Fix remaining typos flagged by the gate (unparsable, reword mis-checking)

### Performance

- V5.0.0 headless-pipeline scale proof (4.19M-leaf layout)

### Documentation

- V4.0.0 shipped; Wave R complete
- Record Wave 0 progress (skeletons, command surface, SKY130 tech)
- Reword ADR 0018 to avoid a typos false positive
- Wave 0 complete; Wave 1 Batch 1 lanes dispatched
- SKY130 rule coverage table for the DRC subset
- Lanes 1B and 1D merged
- Lane 1C merged
- Batch 1 complete; Batch 2 lanes dispatched
- Batch 2 lanes interrupted by session limit; resume state recorded
- Lanes 1E, 1F, 1G merged (Batch 2)
- Wave 1 complete; Wave 2 Batch 1 dispatched
- Defer benchmark tier-5 lane to Batch 2 (depends on Lane E checkers)
- Wave 2 Batch 1 merged; Batch 2 dispatched
- Lane 2C merged; 2D/2F/2G resuming after session-limit interruption
- Wave 2 Batch 2 merged (lanes 2D, 2F, 2G); Batch 3 (H, I) remains
- ADR 0023 (resume orientation, authoritative plan); index ADR 0022
- Dockerfile and deployment chapter for the demo server
- Wave 2 complete (Batch 3 lanes H and I merged, ci + e2e green)
- Agent benchmark chapter with the honest mock machinery baseline
- V5.0.0 README positioning refresh and agent/MCP book chapters
- Wave 3 items 1-5 done; QA gauntlet in progress
- Wave 3 QA gauntlet complete (ci, e2e, replay determinism, abuse, fresh-clone, greps)
- Skeptical v5.0.0 STATUS re-audit (agent layer, honest limitations)

### Build and tooling

- Ignore git hashes in typos; skip minified assets in the history key scan

### Chores

- Scripts/check-keys.ps1 secret scan, wired as just check-keys

### Other

- Scaffold agent-api, mcp, agent, bench, and demo crate skeletons
- Freeze the agent-api command and response surface with round-trip tests
- Commit the cited SKY130 technology file with layers, pins, and physical stack
- Enrich Violation with kind, layer, and measured-vs-required fields
- Freeze the pin and label model, Edit label variants, and document_hash
- Freeze transcript, intent spec, and agent status channel types
- Freeze benchmark schemas and demo server API and limit types
- SKY130 DRC subset table, ADRs 0018-0020, and the frozen-surface manifest
- Move intent types to reticle-extract, serde on geometry value types (ADR 0021)
- Load the committed SKY130 rule subset via sky130_drc_rules()
- Per-rule fixtures for the SKY130 subset, both directions
- Lane 1D SKY130 DRC rule subset with coverage table
- Intent_check module with terminal-to-component mapping, opens, shorts
- Two-direction oracle tests for check_intent
- Drop private intra-doc link so doc-build passes with -D warnings
- Lane 1B connectivity intent checker (LVS-lite)
- Import GDSII TEXT elements as first-class labels
- Export Cell.labels as GDSII TEXT elements
- Prove GDSII labels round-trip alongside shapes
- Carry text labels in the Reticle-OASIS subset (format v3)
- Lane 1C GDSII and OASIS label round-trip
- Session, stable-id allocator, and the command apply loop
- Transcript replay and verification
- Focused, replay, and property tests
- Fix rustdoc intra-doc links for the doc-build gate
- Lane 1A agent command API implementation (session, apply, replay)
- Wire CheckIntent to the reticle-extract intent checker
- Add a script that fetches real sky130_fd_sc_hd cells
- Commit three minimal sky130_fd_sc_hd cells as corpus fixtures
- Import, round-trip, and DRC-check the real SKY130 cells
- Lane 1E real sky130_fd_sc_hd cell import and DRC-subset run
- Rate-limited submit/status/cancel server enforcing every LimitConfig field
- Abuse tests driving the real router for every limit
- Lane 1G rate-limited demo server with abuse tests
- Agent benchmark library (loader, mock model, checkers, runner, results)
- Reticle-bench runner binary, sample suite, and just bench-agent
- Lane 1F benchmark infrastructure, mock model, just bench-agent
- Prove the 3D stack view on real SKY130 heights
- Cross-section test on the real SKY130 met1/met2 bands
- Lane 1H 3D layer stack on real SKY130 thicknesses
- Stdio JSON-RPC server wrapping the agent command surface
- Stdio integration test driving every tool as a subprocess
- Lane 2A MCP server wrapping the agent command surface
- Fix typos flagged by the gate (unparsable, base64 test vector)
- AnthropicModel, propose-verify-correct loop, and artifact writers
- CLI to run one prompt/task against a model
- Full-loop mock tests and API-key-redaction proof
- Lane 2B propose-verify-correct agent harness
- Replace em dashes to satisfy the check-style gate
- Allow CDLA-Permissive-2.0 (webpki-roots CA bundle via ureq)
- Parameterized geometric checkers with two-way tests
- 50 tier 1-4 layout tasks; manifest v0.2.0
- Suite integration test for load + checker dispatch
- Qualify the BenchTask doc link in params so doc-build is clean
- Check the contact/via size tasks by placement, not DRC
- Lane 2E benchmark checkers and 50 tier 1-4 tasks
- Public grouped-edit step() and an awareness status slot
- An AgentCollaborator bridge onto the reticle-sync CRDT
- Convergence tests for the agent/human collaboration
- Satisfy the doc-build and typos gates in the collab module
- Lane 2C agent as a live CRDT collaborator (atomic steps, presence, status)
- 10 tier-5 real-SKY130 tasks; manifest v0.3.0
- Two-way solvability test for the tier-5 tasks
- Lane 2F 10 tier-5 SKY130 benchmark tasks (suite v0.3.0, 63 tasks)
- Make reticle-agent-api build for wasm32
- Agent panel with scripted propose-verify-correct narration
- DRC overlay tracks agent verify steps live
- Replay theater plays transcripts back through a live session
- Share-this-session room link in the side panel
- Unlink private items from public rustdoc
- Lane 2D frontend (agent panel, replay theater, live DRC overlay, share link)
- Mine failure clusters from run records and transcripts
- Draft mined candidates with provenance and two-way vectors
- Promote subcommand gates candidates on their two-way vectors
- Synthetic-corpus tests for the mining and promotion pipeline
- Lane 2G failure mining (mined candidates with provenance, two-way vectors, just bench-promote)
- Playwright browser suite with a WebGL2 gate and a WebGPU-flagged run
- Reticle-demo-server binary with a streaming reticle-agent harness
- Open the public web bundle to the replay theater
- Lane 2H demo-up (demo-server binary, streaming agent harness, Dockerfile, VPS docs, key scan, replay-theater default)
- Unlink wasm-only reticle_app symbols from native rustdoc
- Live-wiring integration tests (real harness streams to a watcher; server-side cancel)
- Flagship agent replay capture (propose-verify-correct with live DRC)
- Replay-hash determinism test over every benchmark transcript
- V5.0.0 (version bump, changelog, requirements table)
## [4.0.0] - 2026-07-02

### Performance

- Measure WASM cold load and collaboration echo; record real numbers

### Documentation

- Final task record and Section 16 self-audit
- Honest status audit; correct overstated claims
- Correct the same overstatements in the book chapters
- OASIS/streaming ADRs and the v4.0.0 measured numbers
- Record honest v4.0.0 progress in STATUS.md
- Mark lanes R1 and R4 in progress
- Record R1 and R4 checkpoint resume state
- Mark lane R4 done, parked for merge slot
- Mark lane R1 merged and done
- Mark lane R3 done, parked to merge last
- Mark lanes R2 and R4 merged
- Mark lane R3 merged, all four Wave R lanes integrated
- Refresh README and STATUS for the v4.0.0 rendering, UI, 3D, and measurement work
- Close-out measurements and doc refresh done, media in progress
- Add a v4.0.0 gallery of the captured engine media
- Mark Wave R media captured and merged

### Build and tooling

- Dev profile tuning, fail-fast ci order, v4/v5 run tracker
- Ignore quick-xml companion advisory RUSTSEC-2026-0194, allowlist iy identifier

### Chores

- Lock criterion dev-deps for the new model and drc benches

### Other

- Implement perf-check as a real regression gate
- Check the whole design by flattening the top cell
- Add convex decomposition by ear clipping
- Forbid em-dashes (voice rule), sweep the tree, add check-style gate
- Memoize cell_bbox with an edit-invalidated cache
- Extend the OASIS subset to paths, instances, and arrays
- Add an incremental re-check latency benchmark
- Make incremental re-check genuinely sublinear via a prepared context
- Memory-mapped out-of-core streaming with one unsafe block
- Add a headless fps benchmark; record 1M/10M offscreen fps
- DRC panel, net highlighting, and a properties inspector
- Ignore RUSTSEC-2026-0195 (quick-xml), unreachable upstream-pinned transitive advisory
- Add monotonic document revision counter
- Retained per-cell scene cache with instance expansion
- Chunked GPU buffer pages with a free-list allocator
- Windowed surface via egui-wgpu paint callback
- Status-bar fps and frame-time readout
- Multi-page retained rects, bench + PERF.md re-measure
- Fix intra-doc link to eframe::egui_wgpu::Callback
- Lane R1 windowed GPU surface and retained scene (10M at ~113 fps)
- 4x MSAA offscreen path with resolve and tolerance golden
- GPU stream compaction with exclusive scan and indirect args
- Indirect draw from compacted buffer with multi-draw and downlevel gates
- Per-chunk LOD selection reusing lod_for_zoom thresholds
- Flags-vs-compacted cull comparison in fps_bench; record numbers in PERF.md
- Lane R2 GPU-driven draw list (compaction, indirect, MSAA, LOD)
- Add optional physical stack directive to the technology format
- Extruded 3D layer-stack pipeline with orbit camera
- 3D stack window with orbit input via egui-wgpu callback
- Cut-line cross-section panel with a two-click cut tool
- Lane R4 3D layer-stack view and cut-line cross-section
- Add stack field to Technology literal exposed by the R4 merge
- Canvas text-label overlay for cell names and live dimensions
- Minimap overview panel with click-to-recenter navigation
- Multi-viewport split with per-pane cameras over the shared document
- Rebindable keyboard shortcuts with a TOML keymap and editor window
- Correct the crate doc to match where the bench targets live
- Unlink two private consts from public rustdoc
- Lane R3 UI (text labels, minimap, split viewports, keybindings)
- Render the 3D layer stack to assets/stack3d.png
- Render DRC violation markers to assets/drc.png
- Render the minimap overview still to assets/minimap.png
- Render maze-routed nets to assets/route.png
- Render two-user presence to assets/collab.png
- Sort routed shapes so route.png is byte-stable
- Capture DRC, route, collab, minimap, and 3D media
- Bump workspace to 4.0.0 and refresh CHANGELOG
## [3.0.0] - 2026-07-01

### Features

- Robust polygon booleans and offsetting via i_overlay
- Generate Rust types from the schema with prost
- R-tree, uniform grid, LOD pyramid, and rkyv streaming
- GDSII and OASIS import/export and technology parsing
- Transactional editing, flattening, and recursive bboxes
- Offscreen wgpu renderer with GPU-driven culling
- Declarative, incremental design-rule checker
- Grid and maze router with rip-up and reroute
- Connectivity extraction and netlist compare
- Deterministic layout generator; add fuzz harness
- Yrs CRDT collaboration with presence and comments
- Axum WebSocket collaboration relay
- Rhai scripting API over the model
- Headless import, DRC, route, extract, export, render pipeline
- Trunk harness with WebGPU capability check and WebGL2 fallback
- Interactive egui editor, native and WASM
- Offscreen media capture for the hero image and browse GIF
- Mount the egui app in the browser via eframe

### Documentation

- Add the mdbook book, changelog config, and gate exclusions
- Hero media in the README, requirements table, and changelog
- Record measured performance results
- Document targets, corpora, and the Windows sanitizer caveat

### Chores

- Scaffold workspace, cross-crate contracts, and local CI gate
- V3.0.0
- Skip pre-commit lint when no justfile; add live-demo link
