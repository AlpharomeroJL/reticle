# Reticle: project paper (skeleton, staged)

> Status: SKELETON, staged for the operator. Drafted 2026-07-11 during the
> v8.2.0 campaign (Phase 3, Depth). This file is a scaffold, not a finished
> paper. Per the project claim rules, the operator holds the pen on the title,
> the abstract, the framing, and every load-bearing claim; this draft fills only
> what is mechanically traceable to committed records and marks the rest.
>
> Two placeholder conventions are used throughout, both greppable so the skeleton
> is spot-checkable:
>
> - `<!-- TODO(operator): ... -->` marks a prose or framing slot the operator owns.
> - `[[MEASURE: <metric> | fill: <command> | record: <artifact>]]` marks a
>   quantitative slot. No measured number appears in this file. Every number the
>   final paper reports is filled from the named command and recorded artifact at
>   the Phase 5 close (or from a valley-queue run), never transcribed from memory.
>
> The claims-to-evidence table is a companion file: [claims-evidence.md](claims-evidence.md).
> It maps each claim to the command or artifact that checks it and restates the
> fixed claim shapes the operator must preserve verbatim.

## Working title (proposed)

Reticle: A Browser-Native Editor, a Verified Checker Core, and a Checker-Graded
Agent Surface for Very Large Hierarchical IC Layout.

<!-- TODO(operator): final title and framing are operator-owned. Alternatives to consider:
     (a) "Reticle: browsing and machine-editing billion-shape IC layouts in a browser tab";
     (b) "A verified layout checker core as an agent benchmark." Pick the framing the
     paper actually argues, then align the abstract and the contributions list to it. -->

## Abstract (stub)

<!-- TODO(operator): the abstract is operator-owned. The scaffold below names the
     load-bearing sentences the abstract must contain and, in parentheses, the committed
     record that grounds each. Write the prose; do not strengthen a claim past its record. -->

1. Problem. Physical IC layout scenes are hierarchical and very large; a cell
   placed thousands of times expands to billions of effective leaf shapes.
   (grounding: `docs/src/introduction.md`, `docs/src/positioning.md`.)
2. Approach. Reticle is one Rust codebase compiled to native and to
   `wasm32`, rendering hierarchical geometry through a retained GPU scene on
   `wgpu` (WebGPU with a WebGL2 fallback), never flattening the hierarchy for
   browsing. (grounding: `docs/src/architecture.md`, `docs/src/rendering.md`.)
3. Verified core. The design-rule checker, router, and connectivity extractor
   are each pinned to an independent reference oracle by property tests, so
   correctness is demonstrated rather than asserted. (grounding: the property
   tests listed in [claims-evidence.md](claims-evidence.md).)
4. Agent surface. The whole editing engine is exposed as a serializable command
   API and graded by those checkers in a propose-verify-correct loop; a
   benchmark suite scores it. (grounding: `docs/src/agent.md`,
   `docs/src/benchmark.md`, `docs/src/leaderboard.md`.)
5. Honest scope. Reticle is a portfolio-grade engineering project and a research
   vehicle for machine-driven layout, not a production EDA tool. (grounding:
   `docs/src/positioning.md`, the honest-limits section below.)

<!-- TODO(operator): headline result sentence. Do NOT write a number here until the
     Phase 5 results table is filled. When it is, quote a single row with its suite
     version and its own denominator (see the benchmark claim shape in claims-evidence.md). -->

## Contributions (skeleton)

<!-- TODO(operator): confirm and order the contributions to match the final framing.
     Each bullet is an intent; the grounding column in claims-evidence.md carries the check. -->

- A single-codebase, browser-native renderer for hierarchical layout that holds
  interactive frame rates on very large scenes without flattening.
- A checker core (DRC, routing, extraction, generators) whose correctness is
  pinned to independent oracles by property tests.
- An out-of-core streaming path that browses a multi-gigabyte archive over HTTP
  Range while fetching only the header, directory, and viewport tiles.
- A serializable command API and MCP surface that turns the editor into a
  physically verified benchmark for layout agents, with a deterministically
  generated leaderboard.
- An honest-limits discipline: every public claim carries a command to check it,
  and the not-list is stated as plainly as the capability list.

## 1. Motivation

Intent: state the problem (hierarchical layout scenes are enormous, and the
tools that open them are heavyweight desktop installs), and the wedge Reticle
takes (open, inspect, check, and machine-edit real layouts in a browser tab with
no install). One to three sentences, no numbers.

Grounding: `docs/src/introduction.md`, `docs/src/positioning.md`, `README.md`.

## 2. Browser-native architecture

Intent: describe the one-codebase design (Rust compiled to native and to
`wasm32` from the same crates), the UI layer (`egui`/`eframe` with an
`egui-wgpu` paint callback for the canvas), and the render backend (`wgpu`:
WebGPU natively and in the browser, with a documented WebGL2 fallback for hosts
without compute). Note the installable-PWA app shell that loads offline.

Grounding: `docs/src/architecture.md`, `docs/src/rendering.md`,
`docs/src/deployment.md`, `docs/src/pwa.md`, ADR 0078.

## 3. Geometry and robust booleans

Intent: describe exact integer coordinates (`Dbu = i32` with wider math for
area and bbox to avoid overflow) and robust polygon booleans delegated to
`i_overlay` rather than a hand-rolled clipper, with convex decomposition by
ear-clipping. The correctness argument is an independent winding-number oracle,
not inspection.

Grounding: `docs/src/geometry.md`, `crates/reticle-geometry/tests/booleans.rs`.

## 4. Streaming very large scenes

Intent: describe how scale comes from hierarchy, not flattening: a bulk-loaded
R-tree plus a tile and level-of-detail pyramid, a compute-shader cull that flags
visible cell boxes and compacts survivors into an indirect draw, and a retained
scene that caches per-cell tessellation once. Then the out-of-core path: a
forward-only, `gds21`-free archive reader and a bounded-memory tiled-archive
builder, streamed in the browser over HTTP Range so the initial view fetches
only the header, the directory, and the tiles a viewport needs, with the fetched
bytes reported live in the streaming HUD through the stats seam.

Grounding: `docs/src/indexing.md`, `docs/src/rendering.md`,
`docs/src/streaming.md`, `docs/src/archive-hosting.md`,
`docs/src/in-browser-conversion.md`, `docs/PERF.md`, ADR 0072, ADR 0091.

<!-- TODO(operator): the streaming and render figures are measured claims. Report them
     with their measurement context (die size, fetched bytes, percentage) on the recorded
     host, per the streaming/perf claim shape. See the results placeholder (Section 8). -->

## 5. Verification: DRC, extraction, and generators

Intent: describe the declarative DRC engine (a fixed set of rule kinds evaluated
over indexed geometry, incremental on an edit by re-checking only the dirtied
neighbourhood), the cited SKY130 rule subset plus a documented KLayout `.lydrc`
subset that compiles to the same engine and is cross-checked verdict-for-verdict
against KLayout headless in a pinned container, the geometric connectivity
extractor with SKY130 MOSFET recognition and a device-count and terminal-net
LVS-lite cross-checked against Magic, and the parameterized generators that are
DRC-clean by construction over their swept parameter spaces across two PDKs.

Grounding: `docs/src/drc.md`, `docs/src/sky130.md`,
`docs/src/sky130-drc-coverage.md`, `docs/src/lydrc-compat.md`,
`docs/src/extraction.md`, `docs/src/device-extraction.md`,
`docs/src/metrology.md`, `docs/src/generators.md`, `docs/src/second-pdk.md`,
the DRC, extraction, and generator property tests in claims-evidence.md.

<!-- TODO(operator): user-defined PCells and NL editing (v8.2 Phase 2) are governed by
     fixed claim shapes. PCells: "DRC-checked on generate," clean-by-construction ONLY for
     the shipped example PCells whose parameter spaces the property tests sweep; the harness
     is "a tool PCell authors run over their own parameter space." NL editing: "command-mapped
     editing with checker feedback," never free-form synthesis. Preserve both verbatim; see
     claims-evidence.md. -->

## 6. The agent surface and benchmark

Intent: describe the serializable `AgentCommand` API and replayable transcripts
with a document hash, the propose-verify-correct loop whose success is decided by
the DRC subset and the connectivity checker (not by the model's own claim), the
two-way-tested checkers (each must accept the intended solution and reject a
perturbed one), the MCP server that offers the same surface to any model host,
and the deterministically generated leaderboard. Distinguish a bare model
(Reticle supplies the loop and grades the result) from an agent system such as
Claude Code (brings its own loop), which is why the two are not head-to-head
comparable.

Grounding: `docs/src/agent.md`, `docs/src/agent-ux.md`, `docs/src/mcp.md`,
`docs/src/benchmark.md`, `docs/src/benchmarks.md`, `docs/src/leaderboard.md`,
`docs/src/multimodal-verification.md`, `docs/src/tapeout.md`,
`docs/src/submitting.md`, ADR 0090.

<!-- TODO(operator): benchmark reporting is governed by a fixed claim shape. Every row
     carries its suite version and its own denominator; rows are never compared across
     denominators; an unrecorded task is an honest not-run, never a pass or a fail. The
     v8.2.0 numbers come from the valley-queue runs and are finalized at Phase 5. Do not
     transcribe the current leaderboard figures into a stronger or cross-suite claim. -->

## 7. Related work and positioning

Intent: place Reticle honestly among KLayout, Magic, the commercial
place-and-route and signoff flows, and the open SKY130 flow (OpenLane and
OpenROAD), stating in one page what those tools do that Reticle does not, and the
narrow slice Reticle pushes hardest on (interactive browser-native rendering of
very large scenes, objective checkers as a first-class engine, and a
checker-graded agent surface).

Grounding: `docs/src/positioning.md` (the honest map, the not-list, and the
"what the established tools do that Reticle does not" section).

<!-- TODO(operator): the two uniqueness claims ("no other browser-native IC layout editor
     found"; "no other physically-verified layout-agent benchmark with a public leaderboard
     found") are absence-of-evidence judgments. They must be cited, dated, and RE-VERIFIED
     immediately before any citable use, per the claim shape. Do not state either without a
     fresh landscape re-scan and its date (Phase 5 has a landscape re-scan and claims re-date
     step). Leave them as dated absence-of-evidence, never as an absolute. -->

## 8. Results (placeholder)

This section is deliberately a placeholder. The v8.2.0 results are produced by
the valley-queue benchmark runs and the Phase 5 measurement pass; no number is
transcribed here in advance. Each row below names the metric, the exact command
that fills it, and the artifact that records it. The recorded host is stated with
every measured figure (results vary by host).

| # | Metric | Value | Fill command | Recorded in |
|---|--------|-------|--------------|-------------|
| R1 | Retained render frame rate at 10M leaf shapes, recorded host | `[[MEASURE: fps@10M | fill: cargo bench -p reticle-render (fps harness) | record: docs/PERF.md]]` | fps harness per `docs/src/performance.md` | `docs/PERF.md` |
| R2 | Per-edit incremental DRC re-check latency (median, p99, max) at 1M shapes | `[[MEASURE: drc_recheck@1M | fill: cargo nextest run -p reticle-app -E 'test(per_edit_recheck)' | record: docs/PERF.md]]` | `crates/reticle-app/tests/drc_live.rs`, `docs/PERF.md` | `docs/PERF.md` |
| R3 | Index build, nearest query, boolean union microbenchmarks | `[[MEASURE: engine-microbench | fill: cargo bench --workspace then just perf-check | record: benches/history/baseline.json]]` | `benches/history/baseline.json` | `benches/history/baseline.json` |
| R4 | WASM cold-load-to-interactive | `[[MEASURE: wasm-cold-load | fill: the wasm cold-load harness per docs/src/performance.md | record: docs/PERF.md]]` | `docs/src/performance.md` | `docs/PERF.md` |
| R5 | Collaboration echo, median on localhost relay | `[[MEASURE: collab-echo | fill: the relay echo harness per docs/src/performance.md | record: docs/PERF.md]]` | `docs/src/relay.md`, `docs/PERF.md` | `docs/PERF.md` |
| R6 | Streaming initial-view fetched bytes and percentage for a multi-gigabyte archive | `[[MEASURE: stream-initial-fetch | fill: open the ?archive= URL and read window.__reticle_stats / streaming HUD on the recorded host | record: README.md streaming paragraph + docs/src/streaming.md]]` | `docs/src/streaming.md`, `docs/src/archive-hosting.md` | README streaming paragraph |
| R7 | Agent benchmark pass rate per row (each with suite version and its own denominator) | `[[MEASURE: bench-rows | fill: just bench-agent-ollama / just bench-agent-claude-code, then regenerate the leaderboard | record: docs/src/leaderboard.md from benchmarks/results/]]` | `docs/src/leaderboard.md`, `benchmarks/results/` | `docs/src/leaderboard.md` |
| R8 | Bundle size (gz) and delta versus the recorded baseline | `[[MEASURE: bundle-gz | fill: just bundle-gate | record: docs/design/bundle-ledger.md]]` | `docs/design/bundle-ledger.md` | `docs/design/bundle-ledger.md` |
| R9 | Workspace test-attribute count | `[[MEASURE: test-count | fill: Select-String -Path crates/**/*.rs -Pattern '#\[(tokio::)?test' (or the STATUS grep) | record: docs/STATUS.md]]` | `docs/STATUS.md` | `docs/STATUS.md` |
| R10 | Cold full-workspace build time (fresh clone of HEAD) | `[[MEASURE: cold-build | fill: time a clean cargo build --workspace --release on the recorded host | record: docs/STATUS.md]]` | `docs/STATUS.md` | `docs/STATUS.md` |
| R11 | Conditional (Phase 3): simulation oracle agreement / waveform records | `[[MEASURE: sim-oracle | fill: the sim oracle test once the route is chosen; label pure-Rust MNA vs external, never as ngspice | record: docs/src/simulation.md, docs/src/waveforms.md]]` | `docs/src/simulation.md`, `docs/src/waveforms.md` | Phase 3 close |
| R12 | Conditional (Phase 4): plugin sandbox metrics | `[[MEASURE: plugin-sandbox | fill: only if the browser host path ships; otherwise reword to native-first per the plugin claim shape | record: Phase 4 ADR + docs]]` | Phase 4 deliverables | Phase 4 close |

<!-- TODO(operator): add or remove rows to match what the final paper actually reports.
     Keep the rule: no value cell holds a number until it is filled from its command on the
     recorded host. Rows R11 and R12 are conditional and are dropped if the phase did not
     deliver that capability. -->

## 9. Honest limits

This section is real. It consolidates the standing, carried-forward limits that
are true as of 2026-07-11 (v8.2.0 campaign, Phase 3 in progress), sourced from
`docs/STATUS.md`, the v8.0.0 honest-limits ledger, and
`docs/src/positioning.md`. The operator re-audits and finalizes this list at the
Phase 5 close (the plan carries a STATUS, honest-limits, and PERF refresh step);
until then, treat any campaign-in-progress item as provisional.

1. Not a production EDA tool. There is no logic or physical synthesis, no
   floorplanning or place-and-route flow (the router routes explicit nets on a
   grid), no static timing analysis, no parasitic (RC) extraction, and no
   tape-out signoff. (source: `docs/src/positioning.md`.)
2. Extraction is net-level, not full LVS. It is geometric net connectivity
   (union-find over touching shapes and cross-layer vias) plus SKY130 MOSFET
   recognition with a device-count and terminal-net LVS-lite, cross-checked
   against Magic's own extraction of a production inverter. It stops short of
   device-parameter and parasitic matching, so it is not a full LVS. (source:
   `docs/src/positioning.md`, `README.md`.)
3. DRC and generator scope are a cited subset, not a foundry deck. The DRC engine
   is a fixed set of rule kinds over indexed geometry, grounded in a cited SKY130
   subset (with a documented KLayout `.lydrc` subset compiled to the same engine
   and cross-checked verdict-for-verdict against KLayout headless in a pinned
   container). It is a fast first filter, explicitly not tape-out clean, and
   omits antenna, density, latch-up, most implant and well rules, and the
   exact-size and differential contact and via rules. The generators are
   DRC-clean by construction only over the parameter spaces their property tests
   sweep, across the two grounded PDKs (SKY130 and IHP SG13G2), not a full deck.
   (source: `docs/src/positioning.md`, `docs/src/sky130.md`,
   `docs/src/second-pdk.md`.)
4. OASIS is a subset, and the in-house container is not interoperable. Reticle's
   `Oasis` type is an in-house, OASIS-inspired container that no other tool
   reads. A separate conformant `oasis_std` writer emits a practical SEMI P39
   subset (export-only, uncompressed, arrays expanded to individual placements),
   validated by KLayout reading it back in a pinned container. It is not a
   general OASIS exporter; GDSII carries the full hierarchy. (source: `README.md`,
   `docs/src/interop.md`, ADR 0086.)
5. Fuzzing runs under WSL, not on the Windows/MSVC host. The libFuzzer targets do
   not link under Windows/MSVC; under WSL (nightly plus `cargo-fuzz`) the v8
   Wave 0 campaign ran the three targets, found and fixed three `reticle-io`
   defects with committed regression fixtures, and a clean-rebuilt confirmation
   produced zero surviving artifacts. Parser and boolean robustness are otherwise
   covered by proptests in the gate. (source: `docs/STATUS.md` Wave 0, the v8.0.0
   ledger.)
6. Benchmark denominators and labels carry the honesty discipline. Every row
   carries its suite version and its own denominator and is never compared across
   denominators; an unrecorded task is an honest not-run, never a pass or a fail.
   The headline agent-system row (Claude Code) is a real authenticated run
   recorded in the leaderboard over a partial set (two tasks of the full suite
   were never recorded when subscription rate limits stopped the run, so its
   denominator differs from the bare-model rows); as an agent system with its own
   loop it is not head-to-head comparable with the bare-model rows and ran a
   different task set. The bare local rows are small quantized models, a floor not
   a ceiling. Current figures live in `docs/src/leaderboard.md` (generated from
   `benchmarks/results/`); the v8.2.0 numbers are refreshed by the valley-queue
   runs and finalized at Phase 5. (source: the v8.0.0 ledger, `README.md`.)
7. The vision oracle is corroboration, not the verdict of record. A second,
   best-effort oracle renders a task's layout and asks a local vision model a
   yes/no question, reported beside the authoritative DRC and checker oracle as an
   agreement rate. On the development host it ran and agreed with the
   authoritative checker, but on a small sample (n=2 fixtures), so it is a
   small-sample agreement, not a validated statistic. A missing model or a
   host with no GPU is an honest not-run. (source: `README.md`
   multimodal-verification, the v8.0.0 ledger, ADR 0090.)
8. Several capabilities ship at the sync or model level but are not fully
   surfaced in the editor app. In-app comment save and load persistence, a
   live CRDT-backed multi-writer editor, and layout-diff `changed` classification
   with a comparison-document file loader are each tested at the crate level and
   documented as deferred in the app surface. (source: the v8.0.0 ledger rows,
   `README.md`, ADR 0079, 0080, 0081.)
9. Interface scope for the current packet. One dark theme ships this line (a
   light variant is deferred by design, ADR 0095); panels are managed and docking
   was surveyed and declined (ADR 0096); there is no internationalization
   infrastructure. (source: `docs/STATUS.md` v8.1.0.)
10. The visual-regression suite and frame guard are GPU-gated. `ui_snapshots` and
    `frame_guard` need a `wgpu` adapter and skip honestly on an adapterless host,
    so a headless run without a GPU does not exercise them; the committed
    baselines are the recorded-host capture. (source: `docs/STATUS.md` v8.1.0,
    ADR 0094.)
11. Python bindings are outside the default gate. `reticle-py` (PyO3, `abi3`)
    builds and passes its tests standalone but is workspace-excluded so the local
    gate stays Python-free. (source: `README.md`, the v8.0.0 ledger, ADR 0087.)
12. Performance numbers are host-specific and provisional until Phase 5. Every
    measured figure is recorded on the stated host and varies by host; the
    v8.2.0 results are finalized at the Phase 5 measurement pass from the
    valley-queue runs and `docs/PERF.md`. (source: `docs/PERF.md`, the campaign
    plan.)
13. History-honesty disclosure. Three deep Wave 4 commits carry a leaked
    `Co-Authored-By` trailer added by a dispatched lane; it is documented rather
    than rebased away, because stripping it would rewrite the history through the
    wave merges. (source: the v8.0.0 ledger, `scratch/sweep/attribution-manifest.md`.)

Campaign-in-progress items (Phase 3 and Phase 4), finalized at Phase 5 under
their fixed claim shapes: user-defined PCells ("DRC-checked on generate"),
natural-language editing ("command-mapped editing with checker feedback"), the
simulation oracle (a bounded oracle for extracted small circuits; if the
pure-Rust MNA route was chosen it is labeled as such everywhere and never
presented as ngspice, and generic device models are labeled wherever PDK model
cards were unavailable), and the plugin story ("sandboxed plugins that run in the
browser build" only if the browser host path shipped; otherwise reworded to
native-first before any public use). See [claims-evidence.md](claims-evidence.md)
for the verbatim shapes.

## 10. Conclusion (stub)

<!-- TODO(operator): the conclusion is operator-owned framing. Summarize the contributions
     the paper actually established, restate the honest scope (not a production EDA tool),
     and point to the ship-always posture ("deployable at every gate, deployed at phase gates
     and release, insurance deploys logged") only as stated in that fixed shape. Write no
     conclusion the results table does not yet support. -->

## References

<!-- TODO(operator): references are operator-owned. Cite the tools named in the positioning
     chapter (KLayout, Magic, OpenROAD/OpenLane, the SKY130 and IHP SG13G2 PDKs, the SEMI P39
     OASIS standard, TinyTapeout) and the dependencies named in the README tech stack. Add the
     dated landscape-scan sources behind the two uniqueness claims when (and only when) those
     claims are used. -->

## Appendix A: additional subsystems (pointers)

Beyond the sections above, these subsystems are documented and can be folded in
if the final scope calls for them. They are listed as pointers so the skeleton
stays focused, not to imply the paper must cover them.

- Collaboration and sharing: `docs/src/collaboration.md`, `docs/src/relay.md`,
  `docs/src/multi-writer.md`, `docs/src/comments.md`, `docs/src/layout-diff.md`.
- Formats and interop: `docs/src/io.md`, `docs/src/interop.md`,
  `docs/src/lef-def.md`.
- Tape-out oracle: `docs/src/tapeout.md`, `docs/src/submitting.md`,
  `examples/tapeout/precheck-results.md`, ADR 0059.
- Python and embedding: `docs/src/python.md`.

## Appendix B: claims-to-evidence

The claims-to-evidence table is the companion file
[claims-evidence.md](claims-evidence.md). It maps each claim to the command or
artifact that verifies it and restates the fixed claim shapes the operator must
preserve verbatim.
