# Reticle honest-limits ledger (v8.2.0)

Every fixture-backed, native-only, parked, deferred, and honest-gap surface in one place,
each row with the honest statement and a source (an ADR under docs/decisions/, a commit, or
a runnable check). Completeness is the bar: a missed caveat is the failure mode, so this is a
roll-call, not a highlight reel. Companion to the status report (STATUS.md) and the
leaderboard (docs/src/leaderboard.md). Some source cells still carry internal campaign
pointers pending a final editorial pass.

Legend for "kind": NATIVE-ONLY (works on desktop, browser path ledgered),
FIXTURE-BACKED (real code, demo/fixture data standing in for a live producer),
PARKED (deliberately not built, error-clean), DEFERRED (built machinery, remaining
work queued, often valley-run), GAP (honest partial/known limitation),
OPERATOR-OWNED (out of campaign scope), CONVENTION (a labelled non-cited value).

## Native-only surfaces

| surface | kind | honest statement | source |
|---|---|---|---|
| rhai PCell producer in the browser | NATIVE-ONLY | reticle-script (rhai, 1,630,420 raw wasm bytes, the single largest crate) is a native-only dependency of reticle-app; live sandboxed produce runs on desktop only. The browser PCell inspector shows predicted F2 provenance (a local hash of the effective parameters, no script run) plus an in-UI disclaimer that live produce runs in the desktop app. It never presents fixture-backed parameter editing as a live run. | ADR 0115; CS bundle-attribution fix-B |
| Real plan/approve/execute agent | NATIVE-ONLY | reticle-agent / reticle-bench are cfg(not wasm) deps; the real agent runs on native behind an availability gate (needs ANTHROPIC_API_KEY or Ollama). wasm ships a scripted preview with an honest banner. The live model round-trip is not CI-testable (no key/network in CI) and is never faked. | D57; CS agent-panel (204) |
| xschem probe import | NATIVE-ONLY | xschem.import_probe is exercised on native; probe-list parsing is real and capped. (The command surface is app-side; browser exercise is not claimed.) | ADR 0112; CS OPERATOR EYEBALL Gate 3 |
| Desktop go-live (share) | NATIVE-ONLY / STUB | native desktop go-live is a stub (app.rs:8903) and belongs to the Phase 4 tauri shell, not the web build. | v82-backlog (71) |

## Fixture-backed / demo-data surfaces

| surface | kind | honest statement | source |
|---|---|---|---|
| Waveform viewer operating-point rendering | FIXTURE-BACKED | operating-point rendering is covered by an in-test synthetic WaveformSet, not a committed contract fixture; a committed OP fixture is left for whichever phase needs a producer. The transient path now runs a live MNA solve (F4 swap, Gate 3). | ADR 0110; CS F4-SWAP (258) |
| Waveform banner before F4 swap | FIXTURE-BACKED (resolved at Gate 3) | until the Gate-3 fixture->live swap, waveform.run_oracle loaded the committed F4 fixture behind an honest warning banner; after the swap it solves live and the banner flips to name the pure-Rust MNA solver. | ADR 0110; CS deploy ledger (214) |
| PCell inspector (browser) | FIXTURE-BACKED | in the browser the inspector edits parameters and shows predicted provenance over demo PCellDef data; it does not run produce (see native-only rhai). | ADR 0115 |
| bench_box PCell checker geometry | FIXTURE-BACKED / GAP | the pcell_box benchmark checker ports its fixed PCell's geometry (two concentric squares) to a Rust closure instead of running reticle_script::pcell::produce, to avoid adding a checker dependency. Documented gap: if bench.box_pad's script text and the Rust formula drift, only the module doc and ADR 0113 catch it (nothing round-trips them). | ADR 0113 |
| bench-2 net-trace / PCell checkers | GAP (by design) | these checkers call the read-only reticle_extract / reticle_gen APIs directly rather than through AgentCommand (no PCell-produce or point-query command variant exists); scoring is unaffected by which path builds the document. | ADR 0113 |
| F1 open-silicon library contents | DEFERRED | the pipeline shipped the fetch-convert-verify machinery + one verified die (SkyWater sky130_fd_sc_hd inv_1, Apache-2.0) live on the Start screen. Real multi-die bulk fetch + R2 upload (ChipFoundry sky130 / TT IHP / GF180) runs from the valley queue, not yet done. | D36, D61; CS Gate 1 |

## Simulation and SPICE boundaries

| surface | kind | honest statement | source |
|---|---|---|---|
| Circuit simulator scope | GAP | pure-Rust MNA solver for bounded small circuits: linear R/C/L + independent sources, DC operating point + fixed-step transient only. It is not ngspice and is labelled that way everywhere. | ADR 0109/0114 |
| Nonlinear devices | PARKED / follow-on | MOSFET/diode and any nonlinear model are out of the MVP; a future nonlinear model must route exp/log through the pinned libm crate and add a native-vs-wasm check. When added, generic device models are labelled generic wherever PDK model cards are unavailable. | ADR 0109/0114 |
| Solver size | GAP | the dense solve is O(n^3), capped at MAX_UNKNOWNS; it suits bounded small circuits, not large netlists. A sparse factorisation is the follow-on if scope grows; the honest answer for large nets is the SPICE export to an external simulator, not shipping ngspice. | ADR 0109/0114 |
| In-bundle solver fallback | (not triggered) | ADR 0109 recorded a PARK fallback if the built solver overran the ~47.6 KiB sim headroom; it was not needed (reticle_sim measured ~571 raw wasm bytes). | ADR 0109; CS bundle-attribution |
| SPICE model-name table | GAP | SpiceTech { nmos_model, pmos_model } is caller-supplied, not derived. DeviceKind recognises only Nmos/Pmos and carries no threshold-voltage flavour or body-bias variant (SKY130 does not mark Vt with a distinguishing layer). SpiceTech::sky130() uses standard-Vt names and does not claim to detect Vt flavour. | ADR 0108 |
| SPICE writer omissions | GAP | area/perimeter params (ad/pd/as/ps) are not written (Device carries no diffusion-area data; the exchange contract documents their absence). A terminal that cannot bind to a net writes the placeholder node NC, never a guessed name. | ADR 0108 |
| parse_spice | GAP | parse_spice exists only to round-trip the writer's own output and validate the committed fixture; it is explicitly not a hardened importer for arbitrary SPICE decks and has not been fuzzed the way GDS/OASIS have. It still never panics (every failure returns a SpiceParseError variant). | ADR 0108 |
| xschem export bridge | DEFERRED (temporary) | file.export_spice runs real extract_devices, then bridges the DeviceNetlist through a small local spice_netlist_from_devices with a two-entry DeviceKind->model table that covers only what the committed fixture names. A technology beyond the fixture exports with an honestly-wrong model name, not a fabricated one. The formatter itself is exact (integer long division, no float). Rewiring to reticle_extract::spice is dedup/quality, not correctness; a ledgered follow-on. | ADR 0112; CS xschem-bridge (259) |
| xschem probe-list format | GAP | xschem.import_probe reads Reticle's own minimal probe-list interchange subset (one probe per line, `<id> <node> <quantity>`), not a byte-for-byte port of any xschem schematic file; xschem's native format is out of scope. The parser caps input size and probe count and returns a structured error on malformed input, never a panic. | ADR 0112 |
| Waveform CSV export | GAP (by design) | to_csv writes raw i64 samples (time_fs + one `<probe>_n<unit>` column), not display-divided floats; a reader must divide by 1e9 (or 1e6 for time) themselves. The unit-suffixed header makes this discoverable; it is a deliberate trade of exactness over convenience. | ADR 0110 |

## Collaboration, classroom, and relay

| surface | kind | honest statement | source |
|---|---|---|---|
| Deployed share relay (H1) | OPERATOR-OWNED / OPEN | DEFAULT_SERVER is 127.0.0.1:3030 (share.rs:25, crates/web/src/main.rs:356). On the HTTPS Pages site, Go-live dials ws://127.0.0.1:3030 (mixed-content, no relay); the deployed reticle-relay worker URL is unconfirmed in the repo. If the relay was never deployed, share-on-web has not worked. Operator-owned; the top v8.2 collaboration item. | v82-backlog H1; premises.md (22); CS ledger-triage (46) |
| Classroom instructor roster | GAP (honest empty state) | the student "follow instructor" half is real today (rides the instructor viewport via the ADR-0038 viewer). The instructor's live roster is empty until a future write-capable "join and publish my own presence" path lands (a read-only viewer, by design, never will). bring_everyone/unlock_student are fully implemented and tested against that future roster shape; the panel renders an honest empty state, not a fabricated row. | ADR 0111 |
| Classroom depends on relay | GAP | classroom runs over whatever relay is configured; the deployed public relay is operator-owned (H1). Solo, the roster is honestly empty. | ADR 0111; CS OPERATOR EYEBALL Gate 3 (219) |
| Snapshot DO relay parity | DEFERRED | snapshot permalinks are native-server-side; the Cloudflare Durable-Object relay did not get parallel snapshot routes (task_d8591000, "two relays one protocol" architecture gap). | D42 |
| Design-review activation | DEFERRED | the review panel exposes open()/toggle(); F6 activation was deferred (commands.rs untouched that wave); the reviewer author is "you", like the comment panel. A later refinement. | D34 |

## GF180 PDK provenance (sourced vs labelled)

| surface | kind | honest statement | source |
|---|---|---|---|
| gf180 DBU-per-micron | CONVENTION | 1000 is a documented convention, not a pinned .lyt citation (the open GF180MCU rule-deck ships no KLayout .lyt with a <dbu>). Area values resolve to exact integers at a 1 nm grid; matches sky130/sg13g2. Labelled honestly in the .tech header, not presented as cited. | D28 |
| gf180 physical stack z-heights | CONVENTION / placeholder | z-heights are a schematic display placeholder (3D-viewer only); no public GF180MCU BEOL cross-section/thickness table was found. Out of the HARD-RULE scope (layer#/datatype/DRC-value). | D29 |
| gf180 rules outside the min subset | PARKED | V1.4a, array/wide-conditional variants (CO.2b/M1.2b/M2.2b/V1.2b), CO.3/CO.4 enclosures, area rules (DF.9/M1.3/M2.3), PL.3a/PL.4 are all sourced but not shipped: array/width-conditional kinds are not expressible by the reticle-drc engine; the rest are outside the brief's minimum coverage. Candidates for a later subset extension. | D30 |
| gf180 GenTech contact fallback | GAP (one labelled value) | GenTech is a fixed 4-slot shape; the gf180 subset carries Metal1/Metal2 only, padded by repeating Metal2. Padded cut slots use Contact (no enclosure rule), not Via1 (whose V1.3a would make a bare via violate). CONTACT_FALLBACK_ENCLOSURE=30 is the one value labelled not-sourced; Via1 enclosure kept at the sourced 0; all oracle-proven safe. | D32 |
| gf180 GenTech derive-provenance | GAP | derive-provenance is honestly documented as stack-order-guard-rejected, not faked. | D31 |

## Format-reader boundaries (parked, error-clean)

| surface | kind | honest statement | source |
|---|---|---|---|
| OASIS CBLOCK decode | PARKED | CBLOCK (DEFLATE) decode is not implemented (it needs a DEFLATE dep in a Cargo.toml outside the lane's owned paths). Rejected cleanly (Unsupported error, no panic, cap-safe). | D26 |
| OASIS repetition-expand + short point-list forms 0-3,5 | PARKED | never emitted by Reticle's own writer, so a decoder is non-circularly unvalidatable; rejected cleanly (Unsupported error, no panic, cap-safe). | D26 |
| CIF / DXF / SPICE readers | GAP (subset) | CIF is a classic-subset reader; DXF is a 2D-subset reader; parse_spice is round-trip-only. All error on malformed input, never panic. Only GDS and OASIS have been through a fuzz campaign. | premises.md; ADR 0108 |
| import menu-command ids | RESERVED | file.import_{cif,dxf,oasis} stay RESERVED per ADR 0106 (F6 deferral); the OS picker filter list and dialogs.rs format list were noted stale (outside owned paths), a later polish item. | D40 |

## App / UI gaps

| surface | kind | honest statement | source |
|---|---|---|---|
| F2/F3 live-wiring history | (resolved Phase 3) | at Gate 2 the PCell inspector and net-trace panel rendered over demo/fixture data; the live wiring was a ledgered follow-on, then done in Phase 3 (f2f3-wiring). Note F2 produce is native-only (ADR 0115). | D56; CS closed-phases Phase 3 (277) |
| gallery card live click-through | DEFERRED (largely closed) | the deep-link builder shipped + tested at Phase 1; the verified die opens in-session via the existing ?archive seam at Gate 1. Full card-click open-path + layers= consumption wiring was the original honest gap. | D38, D61 |
| nl-edit scope | GAP | numeric-only layers (no tech-name lookup); the input-bar widget is not yet canvas-bound (field + apply glue are ready). Deterministic, no LLM. | CS nl-edit (206) |
| pcell-params validation | GAP | no cross-field validation (the schema carries none); extra undeclared param keys are tolerated by design. | CS pcell-params (209) |
| OpenRecord::pieces | GAP | reports a documented floor of 2 (OPEN_PIECES_FLOOR), not an exact split count; intent::Open records only that a net is split, not into how many pieces. | D47 |
| Deep-zoom far-from-origin | PARKED | far-from-origin f32 catastrophic-cancellation is intentionally deferred (ADR 0100); near-origin clamps shipped. Revisit only if a real far-origin use case appears. | v82-backlog (69) |
| v8.2 UI polish debt | GAP / triaged | M1 Open-Recent-no-reopen, M2 SVG-web-no-download, M4 streamed-tile-error-swallow were triaged into Phase-1 lanes (import-wiring / export3d / gallery-pipeline); L1 cut-line-tooltip, L2 no-drag-move, L3 clipboard-chords, and Layers-panel UX were left for a polish sweep. UNCERTAIN: the sources do not explicitly tick M1/M2/M4 as closed by those lanes; verify at Phase 5. | v82-backlog; CS ledger-triage (46) |

## Plugins, image underlay, embed, desktop (Phase 4, Gate 4)

| surface | kind | honest statement | source |
|---|---|---|---|
| Plugin execution in the browser | NATIVE-ONLY | the embedded wasm plugin runtime (wasmi) is a native-only dependency; wasmi is absent from the wasm target (zero browser bytes). The browser plugin manager lists and previews the committed F5 plugin index behind an in-UI disclaimer ("Plugins run in the desktop app; this browser build lists and previews the index only, it never runs a plugin"); the desktop app runs them for real through Host::run in one undo group. The plugin moat is worded to the NATIVE sandboxed-plugin capability, never "plugins that run in the browser build" (the browser host path did not ship; native-first per ADR 0115/0116). | ADR 0116; ADR 0120 |
| Plugin edit-decoder fuzz soak | DEFERRED | the edit-decoder cargo-fuzz target ran 24.7M execs / 61 s zero-crash; the long soak is deferred to the valley queue. | ADR 0117; dispositions plugin-host row |
| ABI v0 stability | GAP | the plugin manifest + ABI v0 stays unstable until the v8.2.0 tag (see standing boundaries). | premises.md (37); ADR 0105 |
| Image underlay decode path | GAP (browser vs native split) | on wasm the underlay decodes through the browser's own codec (createImageBitmap + canvas readback), NOT the Rust `image` crate, whose unconditional moxcms colour management measured a +493 KiB bundle breach; native uses the `image` crate. Both paths cap allocation before decoding. The feature works in the browser; only the decoder implementation differs by target. | ADR 0118 |
| Embed mode | (shipped, no material gap) | embed.toggle flips minimal chrome live; the embed mode itself already existed. Moved into REGISTRY with no chord (keymap test untouched); embeddable-iframe docs chapter added. Bound by the standing seam-preservation invariant, no feature gap. | dispositions embed row; docs/src/embedding.md |
| Tauri human GUI menu-click | GAP (un-automated, ledgered) | the desktop native-produce path is proven by a scripted smoke (`cargo test` runs run_native_produce, the exact code the "Regenerate demo PCell (native, offline)" menu triggers) plus an offline-open window; the human menu-click -> alert itself was NOT automatable in this headless env (the dev exe is not allowlistable for computer-use, the session is non-interactive, the native Tauri menu is not DOM-drivable). One manual desktop click is the sole un-automated step, ledgered honestly, never claimed as automated. | ADR 0119; CS Gate-4 Tauri verify |

## Bundle and testing-coverage honesty

| surface | kind | honest statement | source |
|---|---|---|---|
| Bundle headroom + v8.2 ceiling amendment | GAP (tight) / accepted amendment | Through Gate 3 the bundle sat at +439.4 KiB gz over the v8.0 baseline under a +450 ceiling. At Gate 4 the plugin runtime (wasmi) and the rhai producer stayed native-only, adding zero browser bytes; but two WANTED browser features breach +450 by +3.9 combined: image-underlay browser decode (ADR 0118, +6.6) and plugin-manager F5 browse (ADR 0120, +6.7). ADR 0122 raised the ceiling to +456 KiB gz, bounded to the +453.8 KiB measurement; the deployed Gate-4 bundle is +454.5 KiB gz (the +0.7 is the Gate-4 additive test seams). The +450 trim (replace a typed `from_str::<Index>` parse with a fragile serde_json::Value hand-parse, when serde_json is already unconditionally in wasm via 8 non-plugin panels) was evaluated against the code and DECLINED as a contortion for gz-noise. +456 is the accepted final ceiling; `just bundle-gate` asserts 456. | ADR 0122; docs/design/bundle-ledger.md; CS STEP-0 bundle decision |
| pcell-produce fuzz soak | DEFERRED | the pcell_produce cargo-fuzz target ran ~15k execs zero-crash under WSL; the full soak is deferred to the valley queue. | D50; CS pcell-produce (205) |
| Real-model benchmark rows | DEFERRED | zero real-model leaderboard rows exist; the MockModel is deterministic for the freeze; real Anthropic/Ollama/claude-code rows run from the valley queue, labelled per tier. An unrecorded task is an honest not-run. | D49 |
| Start-screen browser render coverage | GAP (coverage hole, guarded going forward) | a browser egui-wgpu start-screen path was for a time guarded by neither native offscreen ui_snapshots (a different render path) nor the e2e examples-guard (boots into one example). The Gate-3 color-histogram guard passed legitimately (not waived) and the flagged egui-wgpu panic was diagnosed as an in-app Browser-pane environment artifact, not a live regression (the operator loaded the live site fine). The comprehensive-interactive gate policy closes the coverage hole going forward. | CS white-example section (263-270) |
| e2e grow-per-gate follow-ons | DEFERRED (grown at Gate 4) | egui text input is not DOM-drivable, so nl_edit stays unit-only (~50 unit tests cover it), not e2e. The DRC exact bad-vs-clean two-way is unit-covered (reticle-drc/tests/sky130.rs); the headed e2e covers DRC detection, not the exact two-way. Gate 4 added a blank-doc draw case + a tt03 draw to the headed suite (demo-phase4); per-example tool-driving BEYOND sky130 + tt03 + blank remains a grow-per-gate follow-on (not every start-screen example is tool-driven at e2e yet). | CS demo-phase3 (265); CS Gate-4 headed coverage (b70c3d7) |
| diff-action on real CI infra | GAP | the diff-action composite GitHub Action was not run on real GitHub Actions infra (no .github/workflows in the repo); it is valid YAML + reasoned-through only, flagged in the README honest-limits. | D24 |

## Standing scope boundaries (campaign-wide)

| surface | kind | honest statement | source |
|---|---|---|---|
| Phase 0 scrub + token rotation | OPERATOR-OWNED | out of campaign scope; not performed, noted. Runbook at scratch/regression/scrub-completion.md. | D12; CS Phase 0 close (280) |
| reticle-py | (excluded) | PyO3 abi3, workspace-excluded. | premises.md (17) |
| ABI v0 stability | GAP | plugin manifest + ABI v0 is unstable until the v8.2.0 tag. | premises.md (37) |
| Out-of-scope by charter | (excluded) | no accounts/servers/multi-agent-product/photonics; no GPL linking (file or subprocess boundary only). | premises.md (37) |
| Uniqueness claims | OPERATOR-OWNED / one REFUTED | Re-verified 2026-07-12. "No other browser-native IC layout editor found" is REFUTED: Layout Studio (layoutstudio.org) is a live browser-native GDS/OASIS editor that draws shapes and runs DRC in the browser; the claim must not be asserted in v8.2.0. "No other physically-verified layout-agent benchmark with a public leaderboard found" is CHALLENGED: physically-verified layout-agent benchmarks now exist (for example PDAgent-Bench, arXiv 2606.17253, which grades DRC/LVS cleanliness) but without a public leaderboard, so the claim survives only on the narrow public-leaderboard qualifier and needs the operator to narrow, date, and re-verify or drop it. Neither is asserted anywhere in this release. | reticle-claims; layoutstudio.org; arXiv 2606.17253 |

## Phase 4 folded (2026-07-11, Phase-5 prep) -- completeness check
The Phase-4 honest-limits surfaces are now in the tables above, not pending. Every surface
on the operator's Phase-4 completeness list is accounted for (silent omission is the failure
mode, so this is an explicit roll-call):
- browser PCell param-edit is FIXTURE-BACKED, live produce desktop-only -> "Fixture-backed"
  (PCell inspector browser) + "Native-only" (rhai producer).
- agent real-execute, PCell produce, and plugins are NATIVE-ONLY -> "Native-only surfaces"
  (agent, rhai producer) + "Plugins ... (Phase 4)" (plugin execution).
- sim oracle is a pure-Rust MNA solver, labelled as such, never "ngspice" -> "Simulation
  and SPICE boundaries" (circuit simulator scope).
- classroom ships an empty-roster honest state -> "Collaboration, classroom, and relay".
- nl_edit is unit-tested not e2e (egui text not DOM-drivable) -> "Bundle and testing-coverage"
  (e2e grow-per-gate).
- DRC two-way is unit-covered, e2e covers detection -> same e2e grow-per-gate row.
- per-example tool-driving beyond sky130 is a grow-per-gate follow-on -> same row.
- Tauri manual GUI menu-click was un-automated (scripted native smoke covers the exact code)
  -> "Plugins ... (Phase 4)" (Tauri human GUI menu-click).
- the +456 KiB bundle is an accepted amendment (ADR 0122) -> "Bundle headroom + v8.2 ceiling
  amendment".
- diff-action is not run on real GH infra -> "diff-action on real CI infra".
- image underlay browser decode path (createImageBitmap, not the image crate) + plugin
  browse-only + embed + plugin fuzz soak -> "Plugins, image underlay, embed, desktop (Phase 4)".
- STILL UNCERTAIN (carried, not dropped): whether v82-backlog M1/M2/M4 were closed by their
  triaged Phase-1 lanes -> "App / UI gaps" (v8.2 UI polish debt); resolve at the release gate.
