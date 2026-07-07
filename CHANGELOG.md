# Changelog

All notable changes to Reticle are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com), and the project uses
[conventional commits](https://www.conventionalcommits.org).
## [7.0.0] - 2026-07-07

### Bug fixes

- Measure-run.ps1 resolves relative paths against a stale working directory

### Performance

- Factor placement transform out of the array-flatten inner loop

### Editor and app

- Document-open seam with structured warnings
- Read-only viewer sync and viewer share link
- App-level integration test for the Wave 1 drop path
- Add a Generate panel driving the parameterized generators
- Headless UI test for the Generate section and its one-undo placement
- Silence two wasm-only clippy lints in cfg stubs

### Formats and I/O

- Guard GDS record parsing and add import warnings
- Reword corpus-generator comment to clear the typos gate
- Pin deterministic GDS/OASIS cell output order with a direct test
- Make GDSII export byte-reproducible with a fixed date stamp
- Pin the GDSII AREF decode with an exact round-trip leaf-count test

### Collaboration and sync

- Presence carries the viewport for follow-mode

### Agent, MCP, and tools

- Expose generators as RunGenerator command and MCP tools
- Server-side transcript capture for uncontrolled clients
- Claude-code backend driving Claude Code as an agent system
- Fully-qualify the CommandRecord intra-doc link in claude_code
- Launch the claude npm shim on Windows (powershell/cmd wrapper)
- Make the claude-code backend actually drive the tools on Windows

### Generators

- Generator trait, schema, and registry
- Guard ring and via farm generators
- Drop redundant explicit ErasedGenerator intra-doc link
- Fill and probe-able test-structure generators

### Benchmark suite

- Add 8 generator tasks and a generator checker; suite 75 to 83
- Document the Claude Code agent-system row and its honest not-run status
- Interaction-latency harness, soak, and flatten bench
- Local two-model v0.5.0 (83-task) results, gpt-oss 49/83 and qwen 29/83
- Publish the honest three-row v0.5.0 table with the real Claude Code row

### Demo, web, and server

- Read-only room join enforced server-side
- Rate-limit and expire share rooms
- Drop redundant explicit RateLimiter intra-doc link
- Open dropped and remote GDS/OASIS in the browser

### Tape-out

- Honest TTSKY26c GDS-mode submission plan, grounded in live specs
- New TinyTapeout tile template bundle and Start-screen entry
- ADR 0053 and the tapeout book chapter for the tile bundle
- Precheck oracle (Lane 4B): just tt-precheck, structured-failure parser, agent-loop seam
- Correct the precheck live-run status to the observed facts
- Worked example tile, generator-built through the command path
- Run the real TinyTapeout precheck to a verdict; fix the prBoundary bug it caught

### Documentation and media

- Mark v7 Lane 1A done-gate-green in the run tracker
- Mark v7 lanes 1C and 1D done-gate-green in the run tracker
- ADRs 0036/0037 for the browser open path and big-file bands
- Mark v7 Lane 1B done and the Wave 1 merge-gate status
- Wave 1 drop path proven at app level; browser/share e2e still open
- Mark v7 Lane 2A done-gate-green; Wave 2 Batch 2 next
- Sharpen the Wave 1 share-live gap scope (no live relay client exists yet)
- ADRs 0046 and 0047 for fill and test-structure generators
- Mark v7 lanes 2B and 2C done-gate-green; Batch 3 (2D) next
- ADRs 0048-0050 and a Layout generators book chapter
- Fix rustdoc intra-doc link warnings in generator surfaces
- Mark v7 Lane 2D done; Wave 2 (generator layer) complete
- Record the weekly-limit incident; orchestrator does the Wave 3 serial step
- Wave 3 serial done; verified claude CLI flags for 3A; 3A/3B run blocked on weekly quota
- Wave 4 tape-out plan done and grounded facts recorded; 4A/4B/example remain
- Weekly limit reset (plan upgrade); Wave 3 lane-based flow resumes
- Mark v7 lanes 3A and 4A done, 3B honest not-run; tracker current
- Lane 4C worked example done; Wave 4 (tape-out oracle) complete
- Add a Generate-panel demo-script and capture the generator tour GIF
- Restructure the README and book landing as a product page
- Mark v7 Wave 5 (presentation) complete; entering the Wave 6 gauntlet
- Skeptical v7.0.0 STATUS re-audit, every subsystem itemized with evidence
- Wave 6 gauntlet + STATUS audit done; v7.0.0 release held by operator
- Retract the AREF off-by-one misfiling and re-measure the scale proof
- Track the v7 finish progress (worktrees, AREF, precheck done; share-live and benchmarks in flight)
- Resolve share-live intra-doc links under the workspace doc gate

### Build, tooling, and CI

- V7.0.0 kickoff housekeeping and run tracker
- Remove internal v6 packet spec from the tree and gitignore it
- Move standalone tool configs to .config/ and PERF.md to docs/
- Bump crossbeam-epoch to 0.9.20 for RUSTSEC-2026-0204

### Other

- TinyTapeout GDS corpus and malformed samples
- @
- @
- @
- @
- @
- @
- @
- @
- @
- @
- @
## [6.0.1] - 2026-07-04

### Editor and app

- Prove egui viewport screenshot capture path (demo-capture spike)
- Scripted demo-capture mode (--demo-script) for real UI media
- Demo editing/query/3D actions and six real UI captures

### Benchmark suite

- 75-task v0.4.0 re-run of gpt-oss:16k and qwen2.5-coder:16k

### Documentation and media

- V6.0.0 shipped (Wave 5 gauntlet, re-audit, tag, release, gh-pages redeploy)
- Implementation plan for v6.0.1 README and media truth pass
- README and book in an engineer's voice; banned-word gate

### Build, tooling, and CI

- Capture-ui assembles README media from demo scripts

### Other

- V6.0.1 (README and media truth pass)
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

### Bug fixes

- Demote private and wasm-cfg intra-doc links to code spans
- Wait for the demo-server slot release instead of asserting on the instant
- Misread and PNG images in Wave 2 chapters; TASKS Wave 2 complete
- Give each mining test a fresh temp dir (clear stale before create)
- Fresh temp dir for loader and plugin tests (clear stale before create)

### Benchmark suite

- Surface backend/quantization provenance in the results summary
- Gpt-oss:16k local run (42/63, 67%); TASKS benchmark + Batch-1 incident
- Qwen2.5-coder:16k local run (28/63, 44%); Wave 2 Batch-1 done
- Cluster failures by Wave 3 tool surface
- Expand suite to 75 tasks with 12 Wave-3 tasks (v0.4.0)
- 75-task v0.4.0 two-model results and tables (gpt-oss 50/75=67%, qwen 25/75=33%)

### Documentation and media

- V5.0.0 shipped (STATUS re-audit and release complete)
- V6.0.0 run scaffolding and shared-tree incident record
- Pages postmortem in STATUS; Wave 1 done-gate-green in TASKS
- 0031 authorize Wave 3 AgentCommand expansion for the Wave 2 tools
- Wave 3 batching plan and Batch-1 dispatch (agent capability expansion)
- Wave 3 Batch 1 done-gate-green; record 3A command surface and 3D plan log (ADR 0032)
- Wave 3 complete; Wave 4 dispatched; 75-task local re-run in progress
- V6.0.0 credibility chapters + README overhaul (Lane 4C)
- Regenerate hero (2560x1440) and browse GIF for v6.0.0 (deterministic capture-media)
- Wave 4 done; entering Wave 5 (QA gauntlet + v6.0.0 release)

### Testing

- Regression-test the live qwen content-embedded tool call

### Build, tooling, and CI

- Add lane/lane-done worktree recipes; record mandatory lane procedure
- Un-gate the replay theater on wasm; green the lane gate

### Other

- V6.0.0 (version bump, CHANGELOG, STATUS re-audit)
- Fix a typo carried into the CHANGELOG from an old commit subject
## [5.0.0] - 2026-07-03

### Bug fixes

- Fix remaining typos flagged by the gate (unparsable, reword mis-checking)

### Performance

- V5.0.0 headless-pipeline scale proof (4.19M-leaf layout)

### Editor and app

- Cross-section test on the real SKY130 met1/met2 bands
- Agent panel with scripted propose-verify-correct narration
- DRC overlay tracks agent verify steps live
- Replay theater plays transcripts back through a live session
- Share-this-session room link in the side panel
- Unlink private items from public rustdoc

### Rendering engine

- Prove the 3D stack view on real SKY130 heights

### Formats and I/O

- Import GDSII TEXT elements as first-class labels
- Export Cell.labels as GDSII TEXT elements
- Prove GDSII labels round-trip alongside shapes
- Carry text labels in the Reticle-OASIS subset (format v3)
- Add a script that fetches real sky130_fd_sc_hd cells
- Commit three minimal sky130_fd_sc_hd cells as corpus fixtures
- Import, round-trip, and DRC-check the real SKY130 cells

### Verification: DRC, extraction, routing

- Load the committed SKY130 rule subset via sky130_drc_rules()
- Per-rule fixtures for the SKY130 subset, both directions
- Intent_check module with terminal-to-component mapping, opens, shorts
- Two-direction oracle tests for check_intent
- Drop private intra-doc link so doc-build passes with -D warnings

### Collaboration and sync

- Public grouped-edit step() and an awareness status slot

### Agent, MCP, and tools

- Session, stable-id allocator, and the command apply loop
- Transcript replay and verification
- Focused, replay, and property tests
- Fix rustdoc intra-doc links for the doc-build gate
- Wire CheckIntent to the reticle-extract intent checker
- Stdio JSON-RPC server wrapping the agent command surface
- Stdio integration test driving every tool as a subprocess
- Fix typos flagged by the gate (unparsable, base64 test vector)
- AnthropicModel, propose-verify-correct loop, and artifact writers
- CLI to run one prompt/task against a model
- Full-loop mock tests and API-key-redaction proof
- Replace em dashes to satisfy the check-style gate
- An AgentCollaborator bridge onto the reticle-sync CRDT
- Convergence tests for the agent/human collaboration
- Satisfy the doc-build and typos gates in the collab module
- Make reticle-agent-api build for wasm32

### Benchmark suite

- Agent benchmark library (loader, mock model, checkers, runner, results)
- Reticle-bench runner binary, sample suite, and just bench-agent
- Parameterized geometric checkers with two-way tests
- 50 tier 1-4 layout tasks; manifest v0.2.0
- Suite integration test for load + checker dispatch
- Qualify the BenchTask doc link in params so doc-build is clean
- Check the contact/via size tasks by placement, not DRC
- 10 tier-5 real-SKY130 tasks; manifest v0.3.0
- Two-way solvability test for the tier-5 tasks
- Mine failure clusters from run records and transcripts
- Draft mined candidates with provenance and two-way vectors
- Promote subcommand gates candidates on their two-way vectors
- Synthetic-corpus tests for the mining and promotion pipeline
- Replay-hash determinism test over every benchmark transcript

### Demo, web, and server

- Rate-limited submit/status/cancel server enforcing every LimitConfig field
- Abuse tests driving the real router for every limit
- Playwright browser suite with a WebGL2 gate and a WebGPU-flagged run
- Reticle-demo-server binary with a streaming reticle-agent harness
- Open the public web bundle to the replay theater
- Unlink wasm-only reticle_app symbols from native rustdoc
- Live-wiring integration tests (real harness streams to a watcher; server-side cancel)

### Documentation and media

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
- Flagship agent replay capture (propose-verify-correct with live DRC)
- V5.0.0 README positioning refresh and agent/MCP book chapters
- Wave 3 items 1-5 done; QA gauntlet in progress
- Wave 3 QA gauntlet complete (ci, e2e, replay determinism, abuse, fresh-clone, greps)
- Skeptical v5.0.0 STATUS re-audit (agent layer, honest limitations)

### Build, tooling, and CI

- Allow CDLA-Permissive-2.0 (webpki-roots CA bundle via ureq)
- Scripts/check-keys.ps1 secret scan, wired as just check-keys
- Ignore git hashes in typos; skip minified assets in the history key scan

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
- V5.0.0 (version bump, changelog, requirements table)
## [4.0.0] - 2026-07-02

### Performance

- Measure WASM cold load and collaboration echo; record real numbers

### Editor and app

- DRC panel, net highlighting, and a properties inspector
- Status-bar fps and frame-time readout
- Fix intra-doc link to eframe::egui_wgpu::Callback
- Per-chunk LOD selection reusing lod_for_zoom thresholds
- 3D stack window with orbit input via egui-wgpu callback
- Cut-line cross-section panel with a two-click cut tool
- Add stack field to Technology literal exposed by the R4 merge
- Canvas text-label overlay for cell names and live dimensions
- Minimap overview panel with click-to-recenter navigation
- Multi-viewport split with per-pane cameras over the shared document
- Rebindable keyboard shortcuts with a TOML keymap and editor window
- Unlink two private consts from public rustdoc

### Rendering engine

- Add a headless fps benchmark; record 1M/10M offscreen fps
- Retained per-cell scene cache with instance expansion
- Chunked GPU buffer pages with a free-list allocator
- Windowed surface via egui-wgpu paint callback
- Multi-page retained rects, bench + PERF.md re-measure
- 4x MSAA offscreen path with resolve and tolerance golden
- GPU stream compaction with exclusive scan and indirect args
- Indirect draw from compacted buffer with multi-draw and downlevel gates
- Extruded 3D layer-stack pipeline with orbit camera

### Geometry, model, and indexing

- Add convex decomposition by ear clipping
- Memoize cell_bbox with an edit-invalidated cache
- Memory-mapped out-of-core streaming with one unsafe block
- Add monotonic document revision counter

### Formats and I/O

- Extend the OASIS subset to paths, instances, and arrays
- Add optional physical stack directive to the technology format

### Verification: DRC, extraction, routing

- Add an incremental re-check latency benchmark
- Make incremental re-check genuinely sublinear via a prepared context

### Benchmark suite

- Flags-vs-compacted cull comparison in fps_bench; record numbers in PERF.md
- Correct the crate doc to match where the bench targets live

### Demo, web, and server

- Check the whole design by flattening the top cell

### Documentation and media

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
- Render the 3D layer stack to assets/stack3d.png
- Render DRC violation markers to assets/drc.png
- Render the minimap overview still to assets/minimap.png
- Render maze-routed nets to assets/route.png
- Render two-user presence to assets/collab.png
- Sort routed shapes so route.png is byte-stable
- Add a v4.0.0 gallery of the captured engine media
- Mark Wave R media captured and merged

### Build, tooling, and CI

- Implement perf-check as a real regression gate
- Forbid em-dashes (voice rule), sweep the tree, add check-style gate
- Lock criterion dev-deps for the new model and drc benches
- Dev profile tuning, fail-fast ci order, v4/v5 run tracker
- Ignore RUSTSEC-2026-0195 (quick-xml), unreachable upstream-pinned transitive advisory
- Ignore quick-xml companion advisory RUSTSEC-2026-0194, allowlist iy identifier

### Other

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

### Documentation and media

- Add the mdbook book, changelog config, and gate exclusions
- Hero media in the README, requirements table, and changelog
- Record measured performance results
- Document targets, corpora, and the Windows sanitizer caveat

### Build, tooling, and CI

- Scaffold workspace, cross-crate contracts, and local CI gate
- V3.0.0
- Skip pre-commit lint when no justfile; add live-demo link
