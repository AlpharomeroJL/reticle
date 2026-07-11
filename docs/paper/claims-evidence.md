# Reticle paper: claims-to-evidence (skeleton)

> Companion to [paper.md](paper.md). Drafted 2026-07-11, staged for the operator.
> This table maps each claim the paper may make to the command or artifact that
> checks it, per the project claim rules. It is a scaffold: the operator confirms
> which claims the final paper actually makes, and preserves the fixed claim
> shapes verbatim.
>
> Discipline (standing rules): measured or honestly-labeled numbers only; every
> claim carries a command someone can run to check it; no measured number appears
> in this file (numbers live in the results placeholder in paper.md and are filled
> from their commands at the Phase 5 close); no em dashes; no AI attribution;
> absence-of-evidence claims are dated and phrased as such.
>
> Commands assume the repo root on Windows with PowerShell unless noted. GPU-gated
> and browser-gated suites (`just ui-check`, `just frame-guard`, `just e2e*`) are
> orchestrator-run on the recorded host; a headless host skips them honestly.

## Fixed claim shapes (preserve verbatim; do not paraphrase into stronger forms)

The operator must keep these exact shapes wherever the topic appears. They are
reproduced here so the paper does not accidentally strengthen them.

- User-defined PCells: "DRC-checked on generate." Clean-by-construction ONLY for
  the shipped example PCells whose parameter spaces the property tests sweep. The
  harness is "a tool PCell authors run over their own parameter space."
- Natural-language editing: "command-mapped editing with checker feedback," never
  free-form synthesis.
- Benchmark rows: every row carries its suite version and its own denominator;
  never compared across denominators; an unrecorded task is "an honest not-run,"
  never a pass or a fail.
- The two uniqueness claims ("no other browser-native IC layout editor found";
  "no other physically-verified layout-agent benchmark with a public leaderboard
  found"): absence-of-evidence judgments, cited, dated, and re-verified before any
  citable use.
- Plugin moat: "sandboxed plugins that run in the browser build" ONLY if the
  browser host path shipped; if the spike fell back to native-first, the claim is
  reworded BEFORE any public use.
- Simulation oracle: a bounded oracle for extracted small circuits; if the
  pure-Rust MNA route was chosen, it is labeled as such everywhere and never
  presented as ngspice; generic device models are labeled wherever PDK model cards
  were unavailable.
- Streaming and performance numbers: app-measured via the stats seam, quoted with
  measurement context (die size, fetched bytes, percentage), on the recorded
  hardware.
- Ship-always: "deployable at every gate, deployed at phase gates and release,
  insurance deploys logged."

## Verified checker core (independent-oracle property tests)

| # | Claim (honest shape) | Grounding artifact | Verify command | Evidence kind |
|---|----------------------|--------------------|----------------|---------------|
| C1 | Polygon booleans are robust, matched to an independent winding-number oracle | `crates/reticle-geometry/tests/booleans.rs` | `cargo nextest run -p reticle-geometry` | oracle |
| C2 | Spatial index queries (point, rect, nearest, k-NN) match a linear oracle | `reticle-index` oracle tests | `cargo nextest run -p reticle-index` | oracle |
| C3 | The DRC engine equals a naive reference checker over random layouts | `crates/reticle-drc/tests/property.rs` | `cargo nextest run -p reticle-drc` | oracle |
| C4 | Extraction components equal a separate union-find partition | `crates/reticle-extract/tests/property.rs` | `cargo nextest run -p reticle-extract` | oracle |
| C5 | Router length equals an independent Manhattan optimum; an impossible net reports zero routed, not fake success | `crates/reticle-route/tests/routing.rs` | `cargo nextest run -p reticle-route` | oracle |
| C6 | CRDT edits converge regardless of delivery order | `crates/reticle-sync/tests/convergence.rs` | `cargo nextest run -p reticle-sync` | oracle |
| C7 | Metrology area and perimeter match a coordinate-compression oracle | `docs/src/metrology.md`, the metrology property test | `cargo nextest run -p reticle-drc` (metrology tests) then confirm the exact filter | oracle |

## Generators and PDK grounding

| # | Claim (honest shape) | Grounding artifact | Verify command | Evidence kind |
|---|----------------------|--------------------|----------------|---------------|
| C8 | The six generators are DRC-clean by construction over their swept parameter spaces | `crates/reticle-gen/tests/` (cleanliness proptests) | `cargo nextest run -p reticle-gen` | oracle |
| C9 | The generators run against two grounded PDKs (SKY130 and IHP SG13G2) from technology data, not baked constants | `crates/reticle-gen/tests/second_pdk.rs`, `docs/src/second-pdk.md` | `cargo nextest run -p reticle-gen` | oracle |
| C10 | The SKY130 DRC subset is a cited fast first filter, not tape-out clean | `docs/src/sky130.md`, `docs/src/sky130-drc-coverage.md` | read the coverage chapter (which rules are and are not checked) | structural |
| C11 | A documented KLayout `.lydrc` subset compiles to the same engine, verdict-for-verdict against KLayout headless | `docs/src/lydrc-compat.md`, the two-way `.lydrc` tests | `cargo nextest run -p reticle-drc` then the in-container KLayout cross-check per the chapter | cross-tool |

## Scale, streaming, and rendering (measured; values live in the results placeholder)

| # | Claim (honest shape) | Grounding artifact | Verify command | Evidence kind |
|---|----------------------|--------------------|----------------|---------------|
| C12 | Hierarchy is never flattened to browse; the retained GPU path holds interactive frame rates on very large scenes on the recorded host | `docs/src/rendering.md`, `docs/PERF.md` (results row R1) | `cargo bench -p reticle-render` (fps harness); record in `docs/PERF.md` | measured |
| C13 | Incremental DRC re-checks only the dirtied neighbourhood, under the interactive budget at 1M shapes | `crates/reticle-app/tests/drc_live.rs`, `docs/PERF.md` (R2) | `cargo nextest run -p reticle-app -E 'test(per_edit_recheck)'` | measured |
| C14 | A multi-gigabyte archive streams in the browser over HTTP Range, the initial view fetching only the header, directory, and viewport tiles, reported live | `docs/src/streaming.md`, `docs/src/archive-hosting.md`, the `?archive=` demo URL (R6) | open the `?archive=` URL, read `window.__reticle_stats` / the streaming HUD on the recorded host | measured |
| C15 | The bounded-memory archive builder turns a large layout into an archive at a small peak RSS | `docs/src/streaming.md`, ADR 0072 | run the converter on a generated layout and record peak RSS on the recorded host | measured |
| C16 | The browser converts a GDS to a streamable `.rtla` in a Web Worker into OPFS, no server, proven headless | `docs/src/in-browser-conversion.md`, ADR 0091, the `browser-convert` e2e | `just e2e` (browser-convert project); orchestrator-run | integration |
| C17 | Microbenchmarks (index build, nearest query, boolean union) are guarded against regression versus a committed baseline | `benches/history/baseline.json`, R3 | `cargo bench --workspace` then `just perf-check` | measured |
| C18 | The merged bundle stays within the recorded gz budget | `docs/design/bundle-ledger.md`, R8 | `just bundle-gate` | measured |

## Agent surface and benchmark

| # | Claim (honest shape) | Grounding artifact | Verify command | Evidence kind |
|---|----------------------|--------------------|----------------|---------------|
| C19 | Every edit is a serializable, replayable command with a document hash; transcripts replay deterministically | `docs/src/agent.md`, the transcript-replay determinism test | `cargo nextest run -p reticle-bench` | integration |
| C20 | Each benchmark checker is two-way tested (accepts the intended solution, rejects a perturbed one) | `docs/src/benchmark.md`, the checker tests | `cargo nextest run -p reticle-bench` | integration |
| C21 | The leaderboard is generated deterministically from committed result records; each row carries its suite version and its own denominator | `docs/src/leaderboard.md`, `benchmarks/results/`, R7 | regenerate the leaderboard from `benchmarks/results/`; run `just bench-agent-ollama` / `just bench-agent-claude-code` to fill | measured |
| C22 | An unrecorded task is an honest not-run, never a pass or a fail; the agent-system row is not head-to-head comparable with the bare-model rows | `README.md` benchmark section, the v8.0.0 ledger | read the leaderboard and the row labels (backend, model, quantization, suite) | structural |
| C23 | The vision oracle is best-effort corroboration on a small sample (n=2), not the verdict of record; a missing model is an honest not-run | `docs/src/multimodal-verification.md`, ADR 0090 | `cargo nextest run -p reticle-agent` (vision oracle tests) | integration |
| C24 | The MCP server offers the same surface (including one tool per generator) to any model host | `docs/src/mcp.md`, the MCP subprocess test | `cargo nextest run -p reticle-mcp` | integration |

## Interop and external-tool cross-checks

| # | Claim (honest shape) | Grounding artifact | Verify command | Evidence kind |
|---|----------------------|--------------------|----------------|---------------|
| C25 | The conformant `oasis_std` writer emits a SEMI P39 subset that KLayout reads back correctly (export only, uncompressed, arrays expanded) | `docs/src/interop.md`, ADR 0086 | the in-container KLayout read-back cross-check per the interop chapter; orchestrator-run | cross-tool |
| C26 | LEF/DEF import matches OpenROAD on macro, component, and pin counts and die area for a faithful import, and diverges on a corrupted one | `docs/src/lef-def.md`, ADR 0088 | the in-container OpenROAD cross-check per the chapter; orchestrator-run | cross-tool |
| C27 | The worked TinyTapeout tile passes every Magic and KLayout DRC and geometry check against their own decks; the remaining failures are submission artifacts a geometry generator does not produce | `examples/tapeout/precheck-results.md`, ADR 0059 | `just tt-precheck examples/tapeout/tt_um_reticle_tile.gds`; orchestrator-run | cross-tool |
| C28 | Every parser caps count-driven allocation against remaining bytes; malformed input errors, never panics; committed fuzz regressions stay panic-free | `crates/reticle-io/tests/fuzz-regressions/`, `docs/STATUS.md` Wave 0 | `cargo nextest run -p reticle-io`; full fuzz campaign under WSL per `docs/STATUS.md` | oracle |

## Robustness, honesty, and ship-always

| # | Claim (honest shape) | Grounding artifact | Verify command | Evidence kind |
|---|----------------------|--------------------|----------------|---------------|
| C29 | The whole gate is green (style, format, clippy `-D warnings`, tests, doctests, doc build, wasm build, deny, typos) | `justfile`, `docs/STATUS.md` | `just ci` | integration |
| C30 | No `todo!`/`unimplemented!` in shipped code; the workspace `unsafe` count is the one documented mmap | `docs/STATUS.md` grep inventory | the grep set in `docs/STATUS.md` (`Select-String` over `crates/`) | structural |
| C31 | No AI attribution and a single author across all history | `docs/STATUS.md` history checks | `git log --format='%an <%ae>' \| Sort-Object -Unique`; `git log --format='%B' \| Select-String 'Co-Authored-By\|Claude\|Anthropic'` | history |
| C32 | UI color and size contrast is proven by an in-crate WCAG test, not eyeballed | `crates/reticle-app/src/theme/contrast.rs`, `docs/src/design-system.md` | `cargo nextest run -p reticle-app -E 'test(contrast) or test(theme)'` | integration |
| C33 | The visual-regression suite and frame guard exist and are GPU-gated (skip honestly without an adapter) | `crates/reticle-app/tests/ui_snapshots.rs`, `tests/frame_guard.rs`, ADR 0094 | `just ui-check`; `just frame-guard` (orchestrator-run on the recorded host) | integration |
| C34 | The installable PWA app shell loads offline; the manifest, registration, and offline reload are proven | `docs/src/pwa.md`, ADR 0078, the `pwa` e2e | `just e2e` (pwa project); orchestrator-run | integration |
| C35 | Deployable at every gate, deployed at phase gates and release, insurance deploys logged | the campaign plan deploy ceremony, `docs/STATUS.md` deploy records | `just deploy-pages` then propagation-confirmed `just smoke-pages` | deploy |

## Positioning and uniqueness (operator-owned, dated)

| # | Claim (honest shape) | Grounding artifact | Verify command | Evidence kind |
|---|----------------------|--------------------|----------------|---------------|
| C36 | Reticle is a browser-native viewer and editor with a verified checker core and a checker-graded agent layer, not a production EDA tool; the not-list (no synthesis, no timing, no device-LVS, no tape-out signoff) is stated plainly | `docs/src/positioning.md` | read the positioning chapter (the map, the not-list, the "what the established tools do that Reticle does not" section) | structural |
| C37 | "No other browser-native IC layout editor found" | a dated landscape scan (operator) | operator landscape re-scan; date it; re-verify immediately before any citable use (Phase 5 has a landscape re-scan and claims re-date step) | absence |
| C38 | "No other physically-verified layout-agent benchmark with a public leaderboard found" | a dated landscape scan (operator) | operator landscape re-scan; date it; re-verify immediately before any citable use | absence |

## Campaign-in-progress (Phase 3 and Phase 4; confirm at the phase close)

These rows depend on work that is in progress during this campaign. The command
column names where the check will live; the operator confirms the exact test path
and the disposition at the phase close, and applies the fixed claim shape above.

| # | Claim (honest shape) | Grounding artifact | Verify command | Evidence kind |
|---|----------------------|--------------------|----------------|---------------|
| C39 | User-defined PCells are "DRC-checked on generate"; clean-by-construction only for shipped example PCells whose parameter spaces the property tests sweep; the harness is a tool PCell authors run over their own parameter space | the PCell chapter and harness (Phase 2), `crates/reticle-gen` | `cargo nextest run -p reticle-gen` then confirm the PCell sweep filter at the phase close | conditional |
| C40 | Natural-language editing is "command-mapped editing with checker feedback," never free-form synthesis | the NL-edit surface (Phase 2), `docs/src/agent.md` | confirm the NL-edit tests at the phase close | conditional |
| C41 | The simulation oracle is a bounded oracle for extracted small circuits; if pure-Rust MNA was chosen it is labeled as such and never presented as ngspice; generic device models are labeled where PDK model cards were unavailable | `docs/src/simulation.md`, `docs/src/spice-export.md`, `docs/src/waveforms.md`, `docs/src/xschem-interop.md` | confirm the sim oracle route and tests at the Phase 3 close; fill R11 | conditional |
| C42 | Plugins: "sandboxed plugins that run in the browser build" only if the browser host path shipped; otherwise reworded to native-first before any public use | Phase 4 plugin deliverables and ADR | confirm the plugin host route at the Phase 4 close; fill R12 or reword | conditional |

## Notes for the operator

- No value cell in this file or in the paper's results table holds a number. Fill
  each from its command on the recorded host at the Phase 5 close (or from a
  valley-queue run), and state the host with every measured figure.
- Rows C37 and C38 are the only absence-of-evidence claims. They are not stated
  in the paper without a fresh, dated landscape re-scan. Leave them dated and
  phrased as absence, never as an absolute.
- The conditional rows (C39 to C42) are dropped or reworded to match what the
  phase actually delivered. Do not carry a claim past its record.
