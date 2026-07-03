# Reticle run tracker

Single source of truth for the v4.0.0 close-out and the v5.0.0 unified run.
Statuses: `not-started`, `in-progress @ <branch>`, `done-gate-green`.
Single writer: the orchestrator, on main, at every lane handoff and wave merge.
Resume protocol: read this file, `git log --oneline -15`, and `git worktree list`,
then continue from the first unfinished item. A lane in-progress has its partial
work on its worktree branch: resume it, never restart it.

History: waves 0 through 5 of the original plan shipped `v3.0.0` (see git tag and
`CHANGELOG.md`). The v4.0.0 engine work (out-of-core streaming, bbox cache,
incremental DRC, OASIS extensions) is merged on main. This tracker covers what
remains: Wave R below closes v4.0.0; waves 0 through 3 of the unified packet
produce v5.0.0.

## Run conventions

- Lanes run in git worktrees: `git worktree add ../reticle-<lane> -b lane/<name>`.
- Each lane sets `CARGO_TARGET_DIR=D:\dev\reticle-target-<lane>` (the user-level
  default `D:\dev\reticle-target` must not be shared by concurrent builds).
- Lane gate while iterating: `cargo nextest run -p <crate>` plus
  `cargo clippy -p <crate>`. Full `just ci` runs on main at merges only.
- Logs go to `scratch/logs/<lane>-<step>.log`, never into the conversation.
- After Wave 0 the frozen contract surfaces are read-only to lanes; contract
  amendments happen only at wave boundaries via the orchestrator, with an ADR.

## Wave R: finish and release v4.0.0

- [x] Setup: profiles (`dev` opt-level 1, deps 3), fail-fast `just ci` order, run tracker, `scratch/logs/`. done-gate-green
- [x] Lane R1: windowed GPU surface, persistent scene, fps readout, 10M re-measure. done-gate-green, merged to main at b98c9ab. Delivered: revision counter; RetainedScene + chunked BufferPages free-list allocator; egui-wgpu paint callback (RenderContext::from_device, format-parameterized pipelines); FrameMeter fps/frame-time readout. Re-measured on the RTX 4060 Ti: 1M 65.7 -> ~295 fps, 10M 6.1 -> ~113 fps (30fps@10M met). 36 new tests. Post-merge fixes: intra-doc link (912bb02), RUSTSEC-2026-0195 deny ignore + ADR 0017 (a89866b).
- [x] Lane R2: compaction to indirect draw, MSAA, LOD switching, flags-vs-compacted bench. done-gate-green, merged to main at 47a466d (ci green). Delivered: GPU stream compaction (exclusive scan + atomic reserve) into DrawIndexedIndirectArgs; draw_indexed_indirect with native multi-draw gated on MULTI_DRAW_INDIRECT_COUNT (wgpu-29 has no Features::MULTI_DRAW_INDIRECT), downlevel CPU fallback; 4x MSAA offscreen + tolerance golden; per-chunk LOD via lod_for_zoom. Compaction ~2.2x over flags readback (RTX 4060 Ti). 9 new tests. Follow-up flagged: pre-existing single-dispatch CellCuller caps ~4.19M (storage binding + workgroup dispatch limits).
- [x] Lane R3: canvas text labels, minimap, split viewports, rebindable keys, benches doc fix. done-gate-green, merged to main at 9dc2736 (last of the four). Delivered: egui-painter text overlay (glyphon confirmed unnecessary), minimap with click-to-recenter, multi-viewport split panes, TOML keymap + editor with conflict detection, benches doc-comment fix. 49 new tests. Merge conflicts in app.rs and lib.rs resolved (keep-both: R3 keymap/viewports beside R4 view3d/xsection).
- [x] Lane R4: `stack` tech directive, 3D layer view, cut-line cross-section. done-gate-green, merged to main at 47a466d. Delivered: stack directive; pipeline3d.rs + shaders/stack3d.wgsl extrusion with orbit camera; view3d.rs egui-wgpu panel; xsection.rs two-click cut-line cross-section; Tool::CutLine. 26 new tests (incl. a GPU golden run on the RTX 4060 Ti). Merge-integration fix bf94932: a Technology literal R1 added post-branch needed the stack field.
- [x] Close-out: WASM cold-load (~640 ms cold, WebGPU) and collab echo (~0.79 ms median) measured into PERF.md (commit 34092b6). done-gate-green
- [x] Close-out: DRC, route, collab, minimap, and 3D media captured (merged, ci green). done-gate-green. Five real engine-rendered stills (assets/{stack3d,drc,route,minimap,collab}.png) via extended `just capture-media`; deterministic regeneration verified (router shape order fixed for byte-stability).
- [x] Close-out: README refresh, skeptical STATUS update (commit 6630a14). done-gate-green
- [x] Release v4.0.0: version bumped to 4.0.0, git-cliff CHANGELOG, tag v4.0.0 pushed (commit f0fb07c), three host binaries on the GitHub release (not draft), gh-pages rebuilt with the v4.0.0 wasm demo + book (live, HTTP 200). done-gate-green. **v4.0.0 shipped.**

## Wave 0: contract freeze (serial)

- [x] Crate skeletons: reticle-agent-api, reticle-mcp, reticle-agent, reticle-bench, reticle-demo scaffolded, registered, compiling green (commit e94db02, ci green).
- [x] Command and response enums: AgentCommand/AgentResponse tagged serde enums + round-trip tests + structured AgentError/ErrorCode + ElementId (dda4342). Session owning EditableDocument + the element-id allocator is Wave 1 Lane A implementation against these frozen types (ADR 0018).
- [x] Transcript JSONL schema (CommandRecord/Transcript/Outcome) plus document_hash replay contract in reticle-model (277833a, d08aa4f).
- [x] Intent spec types (IntentSpec/IntentNet/Terminal/ForbiddenPair) and open/short report types (277833a).
- [x] Pin/Label model types + Edit AddLabel/RemoveLabel + structured Violation upgrade workspace-wide + document_hash (86a5be3, d08aa4f; ADR 0019).
- [x] Benchmark task/checker/manifest/results schemas in reticle-bench (81a7dcb).
- [~] MCP tools derive from the frozen AgentCommand set; JSON schema generation and model-facing descriptions are Wave 2 Lane A. Contract (command types) frozen.
- [x] Agent status channel types (AgentStatus, AGENT_ACTOR) in reticle-agent-api (277833a).
- [x] Demo server API types (submit/status/cancel) and LimitConfig in reticle-demo (81a7dcb).
- [x] tech/sky130.tech (f6573c6) and tech/sky130-drc-subset.toml with cited values, parse-guarded.
- [x] ADRs 0018 (agent-api layering), 0019 (structured Violation), 0020 (product crates in workspace). Deps: serde/serde_json pinned in workspace.dependencies. Frozen-surface manifest below.

### Frozen-surface manifest (read-only to Wave 1+ lanes; only the integration agent amends, at a wave boundary, with an ADR)

- reticle-agent-api: AgentCommand, AgentResponse, AgentError/ErrorCode, ElementId, Revision, CommandResult; args::{PointArg,RectArg,LayerArg,EndcapArg,OrientationArg,TransformArg}; CommandRecord/Transcript/Outcome; AgentStatus/AGENT_ACTOR. Re-exports the intent types from reticle-extract.
- reticle-extract: IntentSpec/IntentNet/Terminal/ForbiddenPair/IntentReport/Open/Short (ADR 0021; the checker lands in Lane 1B here).
- reticle-geometry: serde derives on Point, Rect, LayerId (ADR 0021).
- reticle-model: Label/Pin/Anchor/PinDirection; Edit::AddLabel/RemoveLabel; Cell.labels/pins; Violation structured fields (kind/layer/other_layer/measured/required) + Violation::new; document_hash; StackEntry + Technology.stack (from Wave R).
- reticle-bench: BenchTask/Tier/SuiteManifest/ResultRecord; Checker trait + CheckResult/CheckFailure.
- reticle-demo: SubmitRequest/SubmitResponse/StatusResponse/CancelRequest/SessionState; LimitConfig.
- Data: tech/sky130.tech (layers, pins, labels, stack); tech/sky130-drc-subset.toml (cited rule subset).
- Benchmark task file format: TOML deserializing to BenchTask; suite manifest is SuiteManifest.

## Wave 1: foundations (batch 1 then batch 2)

- [x] Lane A: reticle-agent-api implementation, property tests, transcript logger/replayer. done-gate-green, merged. Session + stable element-id allocator, all ~25 commands, transcript replay with document_hash, render_png via GPU; 27 tests incl. robustness + id-tracking proptests. CheckIntent wired to Lane B's check_intent (commit e1dee30). Batch 1 complete, ci green.
- [x] Lane B: intent checker in reticle-extract, oracle tests both directions. done-gate-green, merged. check_intent(doc,cell,spec)->IntentReport + sky130_connection_rules; 10 tests incl. two-direction perturbation proptests (break->open, bridge->short). TODO: wire agent-api CheckIntent (Lane A stub) to this.
- [x] Lane C: pins and labels through model and io, GDS TEXT round-trip. done-gate-green, merged. GDSII TEXT<->Label import/export + OASIS label subset (v3); 8 tests; anchors collapse to Center through GDS (gds21 presentation fields private), preserved in OASIS.
- [x] Lane D: SKY130 DRC subset over reticle-drc, coverage table, per-rule fixtures. done-gate-green, merged. sky130_drc_rules() loads 26 cited rules from the toml; 24 tests (9 both-direction); coverage page in the book. toml pinned =1.1.2.
- [x] Lane E: sky130_fd_sc_hd cell import. done-gate-green, merged. Fetched 5 real cells (network worked), committed 3 minimal to corpus/sky130/ with Apache-2.0 NOTICE; round-trip stable (no importer gaps); DRC subset run with honest findings (fill_1 clean; tap_1/inv_1 flag li.5/li.3/poly.8 from bbox-conservative engine + deck approximations, documented, not tape-out-clean).
- [x] Lane F: benchmark infrastructure. done-gate-green, merged. reticle-bench: loader, ModelClient + deterministic MockModel, runner (monotonic clock), CheckerRegistry (rect_present/drc_clean/intent, two-way tested), results writer + markdown summary, bin + `just bench-agent`; 3 tier-1 sample tasks; 26 tests.
- [x] Lane G: demo server. done-gate-green, merged. axum submit/status/cancel enforcing every LimitConfig field (429 rate, 409 per-IP, 503 global, 400 prompt/vocab, budget->cancel); Harness trait + MockHarness + CancelToken; 24 tests incl. 8 abuse tests.
- [x] Lane H: 3D stack SKY130 thicknesses. done-gate-green, merged. 3D layer stack renders on real SKY130 physical heights (nm-to-DBU scaling verified, met5 thickest at top); GPU golden ran on the RTX 4060 Ti; cross-section test lands cuts in the correct met1/met2 z bands. **Wave 1 complete, ci green.**

## Wave 2: composition (three batches)

- [x] Lane A: reticle-mcp server, every tool integration-tested over stdio. done-gate-green, merged. 28 tools (25 command + 3 context) over hand-rolled stdio JSON-RPC; subprocess test drives+asserts all 28; command budget. Post-merge: typos fix.
- [x] Lane B: reticle-agent propose-verify-correct harness, mock-model loop tests. done-gate-green, merged. AnthropicModel (ureq, tool-use), API key env-only + redacted + proven absent from artifacts; loop with DRC+intent verify; 4 artifacts (transcript/GDS/PNG/result); 28 tests. Post-merge: em-dash + typos + CDLA license fixes.
- [x] Lane E: benchmark tiers 1-4, 50 tasks, two-way checker tests. done-gate-green, merged. 7 parameterized geom checkers (contact_stack/via_chain/comb/guard_ring/compound_cell/shape_count/layer_area), two-way tested; 50 tasks (manifest v0.2.0); params in the checker string (schema unchanged).
- [x] Lane F: benchmark tier 5 SKY130, 10 tasks, two-way checker tests. done-gate-green, merged at dab5ea0. 10 tier-5 real-SKY130 tasks; two-way solvability test; suite manifest v0.3.0 (63 tasks total).
- [x] Lane C: agent as CRDT collaborator, atomic transactions, presence, convergence tests. done-gate-green, merged. AgentCollaborator bridge; SyncDocument::step() grouped atomic edits (no half-shape, tested); presence cursor+selection + status channel; 5 convergence tests. ADR 0022.
- [x] Lane D: agent panel, live DRC overlay, replay theater, share link, WASM build. done-gate-green, merged at 4181f58. Agent panel with scripted propose-verify-correct narration; live DRC overlay tracking verify steps; replay theater plays transcripts back through a live session; share-this-session room link. reticle-agent-api wired as a reticle-app dependency and made wasm32-buildable (render_png degrades to a clean EngineError on wasm, now_ms returns 0; commit 5a7b5ff).
- [x] Lane G: failure mining, candidates with provenance, `just bench-promote`. done-gate-green, merged at 9a288b9. mining.rs mines failure clusters from run records and transcripts, drafts candidates with provenance and two-way vectors; promote subcommand gates candidates on their two-way vectors and bumps the manifest minor; `just bench-promote <id>`; synthetic-corpus tests.
- [x] Lane H: `just demo-up`, Dockerfile and VPS docs, Pages replay bundle, key-pattern release check. done-gate-green, merged at d4eb9fa (+ rustdoc integration fix c8a8300). reticle-demo-server composition binary: the rate-limited reticle-demo service + an in-process reticle-server relay + a harness that runs the real reticle-agent propose-verify-correct loop (AnthropicModel when ANTHROPIC_API_KEY is set, else a deterministic offline scripted loop), streaming each atomic step to the watch room as a real yrs CRDT frame and enforcing every LimitConfig field plus cancel. Multi-stage non-root Dockerfile (rustls, key never baked); docs/src/deployment.md chapter. scripts/check-keys.ps1 secret scan wired as `just check-keys` (independently verified: clean on the tree, exits 1 on a planted sk-ant key). Web `?view=` opens a public visitor to the replay theater (ADR 0026). ADRs 0024-0026. Known gap (honest): the in-page replay-theater WINDOW is native-only; on wasm the start-view selection and index framing are in place but the window itself is TODO(wave3) (un-gate the model-free theater modules for wasm). Carried into Wave 3.
- [x] Lane I: Playwright e2e suite, `just e2e`, WebGPU and WebGL2 runs. done-gate-green, committed at e933a33. e2e/ Playwright suite driven by `just e2e` (its own gate): the webgl2 project is the hard gate (WebGPU hidden so wgpu takes its WebGL2 fallback; the app must boot and render, proven by web/src/main.rs hiding #overlay only after WebRunner::start resolves), and the webgpu project launches with the WebGPU-enabling flags and asserts the WebGPU path where a real adapter exists, skipping those checks honestly where it does not. probe-capability.mjs established that Playwright's headless Chromium ships without WebGPU on this host, so the webgpu backend check skips and WebGL2 is the verified path; measured 3 passed, 1 skipped, exit 0. ADR 0027.

**Wave 2 complete (all lanes A through I done, ci green).** Batch 3 (H, I) closed at d4eb9fa/c8a8300 and e933a33; `just ci`, `just e2e`, and `just check-keys` all green on main.

## Wave 3: whole-system validation and release (serial)

- [ ] Live wiring test: server drives harness, browser watches via sync, cancel works. not-started
- [ ] Full benchmark run (live model if key present, else mock, honestly labeled). not-started
- [ ] Scale proof through the headless pipeline into PERF.md. not-started
- [ ] Flagship media via capture-media. not-started
- [ ] mdbook chapters and README positioning refresh. not-started
- [ ] QA gauntlet: ci, e2e both modes, replay determinism, abuse tests, fresh-clone smoke, leak/attribution greps. not-started
- [ ] Final skeptical STATUS re-audit. not-started
- [ ] Release v5.0.0. not-started

## Frozen-surface manifest (recorded at Wave 0 merge)

Not yet frozen.
