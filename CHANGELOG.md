# Changelog

All notable changes to Reticle are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com), and the project uses
[conventional commits](https://www.conventionalcommits.org).
## [8.2.0] - 2026-07-12

### Features

- Freeze F4 waveform and F5 plugin-manifest contracts
- Freeze F1 gallery-manifest contract
- Freeze F2 produce-metadata and F3 trace-query contracts
- Freeze F6 reserved command-id ledger
- CIF classic-subset reader with capped parsing
- STL and glTF mesh export from the 3D layer stack
- DXF 2D subset reader with capped parsing
- Reticle-cli diff subcommand and a diff GitHub Action
- Conformant OASIS reader (writer-subset round-trip)
- GF180MCU layer map and DRC subset (3.3V, sourced)
- GenTech gf180 -- generators DRC-clean on a third PDK
- Design-review states on comment threads and a review-link mint
- Rerunnable fetch-convert-verify pipeline and F1 manifest generator
- Render the F1-manifest die library generically
- Wire CIF/DXF/conformant-OASIS readers into the open path
- Immutable snapshot permalinks
- Wire F1 open-silicon library live on the Start screen
- Bound PCellCache to a capacity with LRU eviction
- Implement F3 net-trace query functions (ADR 0103)
- Implement PCellDef::validate_params and PCellRegistry::infos
- Freeze agent suite determinism and add v0.6.0 coverage tasks
- Implement the sandboxed PCell producer (ADR 0107)
- Add PCell Inspector panel (F2 provenance consumer)
- Net-trace Inspector panel over the F3 trace-query contract
- Deterministic natural-language edit command bar
- M3 replay-DRC-view split, theater minimize/scrub, native real agent
- SPICE netlist writer for extracted DeviceNetlist (netlist lane)
- V0.7.0 -- Phase-3 depth tasks (net-trace, PCell params, multi-step)
- Pure-Rust dense-MNA solver in reticle-sim (ADR 0109)
- Fixture-first F4 waveform viewer panel (ADR 0110)
- Wire PCell regenerate and net-trace panel to real produce/query
- Classroom teaching mode -- roster and follow over presence (ADR 0111)
- SPICE export + xschem probe-list import (fixture-first)
- Native-only rhai (bundle back under budget) + F4 live MNA solve
- Native wasmi plugin host prototype + v0 calling convention (ADR 0116)
- Embed -- harden embed mode into a documented, testable viewer
- Tauri desktop shell -- offline bundle + native PCell produce (ADR 0119)
- Plugin-sample -- real fiducial-marker guest plugin (ADR 0116)
- Production wasm plugin host (ADR 0117)
- Image underlay -- native+browser decode, bundle-safe (ADR 0118)
- Plugin-manifest-index -- F5 static plugin index generator, ADR 0121
- Plugin-ui -- plugin manager panel, browser-browse/desktop-run split (ADR 0120)
- Gate-4 headed coverage -- underlay/embed seams + blank-doc boot + e2e spec
- Real v0.7.0 claude-code leaderboard row; rows keyed per suite
- Lead the plugin panel with its browser disclaimer; recenter onboarding card; fresh README media

### Bug fixes

- Stop the agent preview from promising a DRC fix it never performs
- Bound the vision second-oracle request so a live model cannot hang the gate
- Make DEFAULT_DBU_PER_MICRON pub so the dxf module doc link resolves
- Snapshot.rs doc uses plain backticks for private relay_ws_base (gate doc-build)
- Vision-oracle availability probes over bounded HTTP, never the ollama CLI
- Spell 'unparsable' per the typos gate in gallery fallback tests
- Drop redundant/broken doc links in the PCell scaffold (workspace doc-build)
- Produce validates and hashes the effective params (pcell-params integration)
- Close two pcell-harness findings (sandbox output cap + cache key API)
- Sandbox doc link points to PCellDef::effective_params after the move
- Wasm export status names the actual format, not hardcoded SVG

### Documentation and media

- ADR 0109 picks pure-Rust MNA sim route; reject ngspice-WASM and emscripten
- Stage project-paper skeleton under docs/paper
- Bundle-ledger Gate-4 row + ADR 0122 note at +454.5 gz
- Backfill ADR index 0101-0122 + bundle-ceiling consistency
- Promote v8.2.0 STATUS, honest-limits ledger, disposition table
- Drop uniqueness claims, reframe positioning to positive measured claims
- Fix doc-accuracy issues from the two-skeptic review
- Rewrite README to the v8.2.0 product page; two-skeptic fixes

### Testing

- Recapture ui baselines for the start-screen open-silicon library section
- Snapshot the editor and palette surfaces, not the start screen
- Adversarial harness for the PCell producer (pcell-harness)
- Point pcell-harness cache helper at effective_param_hash; mark both findings closed
- Comprehensive headed e2e for Phase-3 panels + DRC; agent-async wait
- Registry-driven exhaustive headed sweep as a permanent gate

### Build, tooling, and CI

- Replace em-dashes in the drag_press_pos doc
- Keep local-only CLAUDE.md and AGENTS.md untracked
- Scaffold reticle-sim and reticle-plugin as empty workspace members
- Reserve cif and dxf reader modules (Phase 1 pre-fan-out)
- Reconcile Cargo.lock (reticle-script for reticle-app after f2f3-wiring)
- Regenerate F5 plugin index to fold fiducial-marker sample
- Bundle amendment +456 gz (ADR 0122) + browser panel reads the real F5 index
- Finalize bundle amendment ADR 0122 at +456 gz (trim declined)
- Sync Cargo.lock with reticle-render image/jpeg deps (underlay ADR 0118)

### Other

- PCell engine interface + script-to-gen edge (ADR 0107)
- SPICE exchange contract fixture + Phase 3 doc chapter stubs
## [8.1.0] - 2026-07-09

Re-cut on a fixed head with post-tag fixes (wasm replay-hash determinism,
start-screen example loading, deep-zoom rendering, service-worker cache versioning,
command palette, canvas drawing, and the replay theater) and with non-design internal
run documents removed from history.

### Features

- Add command registry, dispatch funnel, keymap by CommandId
- Encode tokens as applied egui style and drain UI literals
- Component library and hidden gallery
- Embed subset Inter, JetBrains Mono, and Lucide fonts
- Touch mode raises hit targets to 40px, add e2e-touch gate
- Component states, functional motion, frame guard
- Registry-driven palette, shortcuts overlay, context menus, focus, esc
- Onboarding tour variants, hints, help, settings, and about
- Registry-driven menu bar and thinned icon toolbar
- Redesign Layers panel and dock the 3D + Cross-section as managed panels
- Rebuild right Inspector on segmented + collapsible panel
- Viewer chrome, presence/follow, start screen, presentation/embed
- Canvas overlay manager, navigation, and the fluidity trio
- Rfd picker, open/URL/convert/share dialogs, unified toasts
- Client-side QR in Share dialog, paste-to-open URL

### Bug fixes

- Make the wasm replay document_hash platform-independent (native/wasm parity)
- Graft the SKY130 technology so start-screen examples open with named, colored layers
- Command palette: Enter and row-click both activate the top hit
- Canvas draw: rectangle and marquee select commit from the true drag press point
- Compact the docked replay theater so the main canvas stays the hero
- Route post-boot runtime panics to a readable overlay
- Version the service-worker cache and serve stable-named assets network-first
- Guard the deep-zoom render cycle
- Pass same-origin design fetches through so ?gds= deep-link opens (e2e-touch)
- Resolve the three HIGH design-review blockers (H1/H2/H3)

### Documentation and media

- Add GitHub Sponsors button
- Add IconButton spec (ADR 0097)
- Add v8.1 catalog dispositions (100-item completeness gate)
- Add skeptical v8.1.0 section and honest-limits ledger
- Add design-system and redesign-gallery chapters
- Refresh gallery and hero media from the live redesigned bundle
- Expose additive __reticle_stats seams for headed regression detection

### Testing

- Egui_kittest 0.35 UI snapshot suite with GPU serialization
- Standing headed e2e suite for the palette, drawing, marquee, and replay theater

### Build, tooling, and CI

- Design-system foundation, contracts, and tooling
- Recapture UI baselines on merged main; fix rustdoc links and typos allowlist
## [8.0.0] - 2026-07-08

### Features

- Freeze the Wave 2 streamed-archive contract (ADR 0062)
- Reconnect backoff state and pure schedule
- Add SyncDocument::encode_full_state for reconnect resync
- Wasm reconnect with backoff and full-state resync
- Freeze the SyncMessage first-byte wire invariant
- Two-target relay conformance suite with the native half green
- Cloudflare Durable Object relay in workers-rs, DO conformance green
- Add per-shape set/remove StepEdit ops
- Mirror TransformShapes/DeleteShapes via an ElementId map
- Add native live-room run mode (reticle-agent::live)
- Pure apply_pinch touch-zoom helper
- Permalink, session, room-mint, and e2e-edit link logic
- Wire permalinks, one-click share, touch, and e2e seam
- Apply permalink and e2e-edit flag at boot
- Forward-only streaming GDS record reader
- External two-pass .rtla archive builder
- Implement the three TileSource readers over .rtla
- Archive-serving worker with R2 range, Cache API, CORS lock
- Verify-licenses redistribution gate
- DocHost edit/stream split and StreamedScene tile residency
- Convert GDSII to a streamable .rtla archive
- Wire ?archive= served-archive browse with progressive residency + HUD
- Compute-shader DRC heatmap (binning + check + overlay)
- GPU-resident hierarchy, chunked expand+cull+compact
- Track edit dirty regions for incremental live DRC
- LiveDrc incremental checker and spell-checker underline geometry
- Wire DRC-as-you-type into the frame loop with live underlines
- Scaffold reticle-metrology crate
- Per-layer area and perimeter report
- Connectivity statistics report
- Simplified per-net antenna-ratio screen
- Combined report with CSV and Markdown export
- Add web app manifest, icons, and manifest link (step 1)
- Add service worker caching the app shell (step 2)
- Register the service worker from index.html (step 3)
- Reticle-diff crate with pure LayoutDiff + oracle property tests
- Layout-diff overlay panel painting added/removed/changed rects
- Schema V2 with additive Document.comments field
- Lossless V1->V2 document migration + golden-fixture proof
- Anchored comment pins panel and canvas markers
- Multi-writer convergence + per-actor selective undo
- New crate parsing LEF/DEF into the model document
- Parse a documented subset of KLayout .lydrc DRC decks
- Second PDK (IHP SG13G2) + both-PDK cleanliness proptests
- Conformant-OASIS writer + GDS/OASIS interop harness
- PyO3 abi3 bindings for documents, generators, render
- Maturin packaging, Jupyter widget, example notebook
- Lefdef_oracle harness (pinned-container OpenROAD, honest skip)
- Recognize SKY130 MOSFETs and device-level LVS-lite
- Vision second-oracle (render+ask over Ollama, agreement test)
- Add build_rtla_to_vec, an in-memory .rtla builder for wasm
- GDS->.rtla conversion core reusable in wasm (lane v8-6c step 1)
- Convert Web Worker + Trunk wiring, writes .rtla to OPFS (step 2)
- Convert action + SW OPFS bridge to open archive via ?archive= (step 3)
- Deterministic leaderboard generator and validate-records subcommand
- Build_anchor example + scale-demo disposition (232 MiB anchor staged; GB-class ledgered)
- Expose live camera in __reticle_stats seam (wasm-only)

### Bug fixes

- Normalize out-of-range GDSII dates before gds21 parses
- Bound OASIS reader pre-allocations to input size
- Reject zero-length GDSII string records before gds21 parses
- Use SQLite-backed Durable Objects for free-plan deploy
- Em-dash in Wave 2 contract ADR heading; web passes &str to parse_e2e_edit
- Resolve the adversarial-review high/medium findings on live collab
- Reconcile the .rtla preamble so the builder and reader agree
- Private-item doc links to plain code spans, and mis-* typos in Wave 2 lane docs
- Cap DrcHeatmap::run to device limits (Wave 3 review MEDIUM)
- Renumber 5b LEF/DEF-oracle ADR 0083->0088 (collided with 5c lydrc)
- 3D-stack and cross-section windows start collapsed, tucked below the toolbar
- Offset the streaming HUD clear of the ruler so its text is not clipped
- Build_anchor uses canonical plan_levels so large anchors stream (318 MiB proven live, 0.0014% fetched)
- Move floating Convert + view-switch onto the toolbar; unblock occluded controls
- Nudge collapsed 3D + Cross-section windows clear of the panels
- Share-GIF harness forces WebGL2 and opens a chip on the sharer

### Benchmark suite

- 10 more tasks in the row extension (35/83, still PARTIAL)
- Final overnight batch, row now 81/83 (PARTIAL)
- 30M/100M GPU-resident hierarchy measurement + PERF table

### Documentation and media

- V7.0.0 shipped, final skeptical STATUS re-audit and tracker close-out
- V8 run bootstrap, disk and frozen-surface ADRs, tracker section
- Close v8 Wave 0 (gate green, hardened parsers deployed live)
- Reconnect subsection and ADR 0062
- Book chapter and ADRs for the two-relay conformance work
- ADR 0062 and agent chapter for the live-room agent
- Permalinks, touch pan-zoom, and ADR 0062
- Fully-qualify the reconcile_to intra-doc link so reticle-app docs build
- Streaming reader + .rtla builder (ADR 0063, io book, README)
- Document the .rtla TileSource readers and add ADR 0063
- CORS/Range/Cache design, license-gate policy, ADR 0068
- ADR 0068 and streaming chapter for the DocHost split and residency
- Document the convert command flatten scope and leveling
- Assign 0072 to the 2f converter ADR (placeholder -> number at merge)
- Fix rustdoc private/wasm links to code spans and drop em-dashes
- ?archive browse + streaming HUD (streaming.md, PERF numbers, ADR + README row)
- GPU DRC heatmap book page, PERF row, and ADR (placeholder number)
- Assign 0075 to the 3b GPU DRC heatmap ADR
- GPU-resident hierarchy subsection + ADR (placeholder 0074)
- DRC-as-you-type subsection, per-edit PERF numbers, ADR 0074
- Book page, ADR 0074 (placeholder), README row
- Plain code span for private device_instance_cap (doc-build gate)
- Book page, SUMMARY entry, ADR 0078, README row (step 5)
- Layout diff book page, ADR 0078 (placeholder), README rows
- Comments/annotations book page, ADR 0080, README rows
- Multi-writer collaboration book page, ADR 0081, README row
- Interop chapter, ADR 0063, and README/decisions rows
- Document the KLayout .lydrc compatibility subset
- Interop + second-PDK chapters, ADRs 0068-0070, OASIS honesty rename
- Python bindings book chapter
- Document input-gate atomicity of the conn-id allocation
- LEF/DEF import oracle (ADR 0083, book subsection, README row)
- Device recognition chapter, ADR 0063, README boundary update
- Multimodal verification chapter, ADR 0090, README row
- Add 6b vision second-oracle index row (0090); lane created the file but not the index entry
- In-browser conversion book page, ADR 0090 (placeholder), README row
- Plain code span for private MAX_TILE_RECORDS in build_rtla_to_vec (doc-build gate)
- Leaderboard chapter, submission harness, and ADR 0068
- Reconcile benchmark table to the deterministic leaderboard
- Fold in the conformant OASIS writer and Python bindings
- Add a what-is-new-in-v8 paragraph to the landing page
- Trim stale pre-v8 scaffolding and fix a broken ADR link
- Measured 3.01 GiB read-side streaming claim + live demo URL
- Real share GIF, browser-proof ADR 0068, e2e project docs
- Browser-level proof subsection in the collaboration chapter
- Correct streaming number to app-measured 188 KiB (0.006%)

### Refactoring

- Data-driven GenTech, generators read it via the tech arg

### Testing

- Commit v8 seed corpora and record the campaign
- Kill-and-reconnect resync over the real relay
- Deployed-relay smoke branch and native-tls for wss
- Convergence + skipped tests and the DRC-fix demo transcript
- In-process relay integration test for the live agent
- Prove streamed viewport query equals the R-tree query
- Coarse-then-fine residency proof against a latency-injecting MemSource
- Served-archive spec + local Range server + committed .rtla fixture
- GPU DRC heatmap flags vs CPU oracle proptest + native bench
- Adapter-gated GPU hierarchy correctness + chunking + zero-CPU
- App-level two-way live DRC and per-edit latency at 1M shapes
- E2e proving manifest + SW registration + offline shell (step 4)
- Freeze V1 golden document fixture (pre-V2 build)
- Persist comments through a V2 document, two-way
- 2 editors converge + 1 viewer sees but cannot write
- Pin the V1 golden fixture byte-exact (Wave 4 review)
- End-to-end .lydrc subset deck through DrcEngine
- KLayout verdict-comparison harness for the .lydrc subset
- LEF/DEF import oracle cross-check, proven both ways vs OpenROAD
- Add Magic container oracle for device recognition
- In-browser convert -> OPFS -> ?archive= streaming (lane v8-6c step 4)
- Golden byte-stability test and fixture record set
- Behavioral share-live proofs, phone touch project, share GIF harness

### Build, tooling, and CI

- Remove em-dashes from reconnect code and docs
- Lock native-tls deps for the conformance wss smoke
- Untrack a lane RESULT.md that was force-added into scratch (belongs out of tree)
- Add just e2e-archive recipe (served-archive browser streaming spec, reproducible)
- Lock reticle-drc dev-dep edge for reticle-render
- Strip em-dashes from lane 3c GPU-hierarchy files (voice rule)
- Lock reticle-gen dev-deps (reticle-io, toml)
- Exclude a non-default PyO3 abi3 crate from the workspace
- Allowlist IHP 'Activ' layer name and OASIS 'SSEE' notation
- Fix em-dash in ADR 0062 title to satisfy the voice gate
- V8.0.0
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
- V7.0.0
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
