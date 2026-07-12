# Reticle v8.2.0 disposition table (every campaign item)

Walks every item across Phases 0 through 4 into one table: item | disposition | evidence.
Dispositions: shipped (merged + gated GREEN), ledgered (shipped with a recorded honest gap
or follow-on), gated (a gate/policy outcome), parked (deliberately not built), deferred
(machinery shipped, remainder queued), fixed (a gate/harness-found defect closed). No item
is silently dropped; anything uncertain is marked UNCERTAIN rather than omitted.

Evidence keys: commit hash; ADR nnnn = docs/decisions/nnnn; deploy hashes are the live
web-<hash> builds. Some rows carry internal campaign pointers (D<n>, CS) pending a final
editorial pass. This table is the scope-completeness record: it is diffed against the full
campaign packet so a missing item is a failure, not an oversight.

## Phase 0 -- close-window + scaffolding (main 93a94ff; ci GREEN, no deploy)

| item | disposition | evidence |
|---|---|---|
| Skill pack + agents + CLAUDE/AGENTS + campaign scripts | shipped | f159215 (D8) |
| reticle-sim + reticle-plugin scaffold (F4/F5 homes) | shipped | cdb452d (D9) |
| Vision second-oracle gate-hang -> 120s bounded transport | shipped | f152fde (D10) |
| FLEET-CALENDAR (TTSKY26c dates) | shipped | scratch/campaign/FLEET-CALENDAR.md (D11) |
| Phase 0 scrub + token rotation | gated (operator-owned, not performed, noted) | D12; CS 280 |
| F1 gallery-manifest contract | shipped | ADR 0101, a1fcdd1 (D13) |
| F4 waveform-record contract | shipped | ADR 0104, 957edba (D14) |
| F5 plugin manifest + ABI v0 + index contract | shipped | ADR 0105, 957edba (D15) |
| F2 produce-metadata contract + param-hash input | shipped | ADR 0102, 03ed411 (D16) |
| F3 trace-query records with revision envelope | shipped | ADR 0103, 03ed411 (D17) |
| F6 reserved command-id ledger (43 ids) | shipped | ADR 0106, 93a94ff (D18) |
| Phase 0 FINAL gate `just ci` GREEN, no deploy | gated | scratch/logs/ci-phase0-final.log; 93a94ff (D19) |

## Phase 1 -- Open Silicon + Review + Formats (main 117022a; deployed web-7309157832549c56; +348.8 KiB gz)

| item | disposition | evidence |
|---|---|---|
| CIF classic-subset reader | shipped | 7353bdd (D20) |
| DXF 2D-subset reader | shipped | 9a80ca7 + gate-fix a088cdd (D21) |
| STL + glTF/.glb mesh export | shipped | e0ef4cc (D22) |
| reticle-cli diff + diff-action composite | shipped | 80b1e1a (D23) |
| diff-action run on real GitHub Actions infra | ledgered (no workflows; YAML + reasoned only) | D24 |
| Conformant OASIS reader (round-trip, caps, zero-panic) | shipped | d4fca9c (D25) |
| OASIS CBLOCK decode / repetition-expand / short forms 0-3,5 | parked (Unsupported, no panic, cap-safe) | D26 |
| GF180MCU layer map + 3.3V DRC subset (9 layers, 11 rules) | shipped | ffe2272 (D27) |
| gf180 DBU-per-micron = 1000 | ledgered (convention, labelled, not cited) | D28 |
| gf180 physical stack z-heights | ledgered (schematic placeholder) | D29 |
| gf180 rules outside the min subset (V1.4a, CO/M/V variants, area, PL) | parked (sourced, not shipped) | D30 |
| GenTech gf180 cross-PDK (generators DRC-clean on a 3rd PDK) | shipped | 7184a25 (D31) |
| gf180 GenTech 4-slot padding + CONTACT_FALLBACK_ENCLOSURE=30 | ledgered (one value not-sourced, oracle-proven) | D32 |
| Design-review workflow (review states + link + panel) | shipped | b393f5b (D33) |
| review activation (F6/menu open) + per-actor authoring | ledgered (F6 deferred; author "you") | D34 |
| Open-silicon library pipeline + F1 manifest gen + license check | shipped | 106c3cf (D35) |
| Real multi-die library bulk fetch + R2 upload | deferred-to-valley (machinery + 1 die shipped) | D36 |
| Start-screen die gallery (F1-manifest renderer + deep links) | shipped | 7a6e33c (D37) |
| gallery live card-click open-path + layers= consumption | ledgered (deep-link shipped; die opens via ?archive at Gate 1) | D38, D61 |
| Import wiring CIF/DXF/OASIS into open path + DXF dialog | shipped | 9a9ace6 (D39) |
| file.import_{cif,dxf,oasis} menu activation | ledgered (stays RESERVED per ADR 0106) | D40 |
| Immutable snapshot permalinks (server capture + viewer) | shipped | 168c892 (D41) |
| Snapshot Cloudflare-DO relay parity | ledgered/deferred (task_d8591000) | D42 |
| F1 open-silicon library LIVE on the Start screen | shipped | d6e38f5; deployed web-7309157832549c56 (D61) |
| Vision-oracle CLI-subprocess hang (25min stall) -> bounded probe | shipped (fixed at gate) | cb3d5e7 (D62) |
| ui_snapshots editor/palette visual coverage restored | shipped (fixed at gate) | 117022a (D63) |
| C-9 tier calibration at Gate 1 | gated (KEEP policy) | D64 |

## Phase 2 -- Agent + Full-Custom (main 88c1d16; deployed web-69b10274a73d1f02; +402.4 KiB gz)

| item | disposition | evidence |
|---|---|---|
| Phase 2 PCell engine scaffold (script->gen edge) | shipped | c1ee57a, ADR 0107 (D43) |
| PCellCache (capacity-bounded LRU) | shipped | cc2960f (D44) |
| PCellDef::validate_params + PCellRegistry::infos | shipped | 527f7ed (D45) |
| F3 net-trace query fns (net_at_point/net_extent/shorts_opens) | shipped | 18e6de0 (D46) |
| OpenRecord::pieces reports a floor of 2 | ledgered (OPEN_PIECES_FLOOR, not exact) | D47 |
| Agent benchmark frozen byte-stable + 5 tasks (v0.6.0, 83->88) | shipped | 9ff9a11 (D48) |
| Real-model benchmark leaderboard rows | deferred-to-valley (mock-labeled only) | D49 |
| Sandboxed PCell producer (fresh rhai, caps, fuzz target) | shipped | 08a166f (D50) |
| produce validates/injects/hashes the effective params | shipped (gate-fix) | 5b9ec3d (D51) |
| PCell parameter+provenance Inspector panel | shipped | b15f564 (D52) |
| F3 net-trace Inspector panel (fixture-first) | shipped | 3ca89d9 (D53) |
| Deterministic NL edit command bar (no LLM) | shipped | 3670399 (D54) |
| Agent panel (M3 replay-DRC split + scrub/minimize + real native agent) | shipped | 3d3ea6b (D55) |
| F2/F3 live-wiring (panels -> real produce / live queries) | deferred-to-followon, then shipped Phase 3 | D56; CS 277 |
| Agent live-model round-trip (browser + native key/network) | ledgered (native behind availability gate; not CI-testable) | D57 |
| Adversarial PCell sandbox harness + pinned param_hash vector | shipped (found 2 real gaps) | f524d10 (D58) |
| Sandbox output-cap security hole (instances/arrays uncounted) | fixed | 009fc2b (D59) |
| Cache-key effective-hash accessor gap | fixed | 009fc2b (D60) |
| Phase 2 GATE 2 close (seam canaries, bundle-gate, e2e, deploy) | gated/deployed | 88c1d16; web-69b10274a73d1f02 (CS 190, 215) |

## Phase 3 -- Depth (main 71797b3; deployed web-23f134b04998df27; +439.4 KiB gz). Not in dispositions.md; cite CS + ADRs + commits.

| item | disposition | evidence |
|---|---|---|
| oracle-feasibility sim-route decision (Route 3 pure-Rust MNA) | shipped (decision + docs) | e47c1d6, ADR 0109 (CS 246) |
| netlist SPICE writer (DeviceNetlist -> SPICE, W/L only) | shipped | b8f2f22 / merged 021c1f0, ADR 0108 (CS 249, 256) |
| SPICE model table caller-supplied (no Vt-flavour detect) | ledgered | ADR 0108 |
| sim-engine pure-Rust dense-MNA solver (linear R/C/L, DC+transient) | shipped (byte-exact F4, 0 nV) | 9978d97 / merged 09b2e52, ADR 0114 (CS 250) |
| Nonlinear device models (MOSFET, diode) | parked (labelled-generic follow-on) | ADR 0109/0114 |
| waveform-ui panel (F4 render, CSV, honest banner) | shipped | 1eb6258 / merged a571af2, ADR 0110 (CS 251, 257) |
| waveform OP rendering | ledgered (synthetic set, no committed OP fixture) | ADR 0110 |
| F4 fixture -> live sim swap (waveform.run_oracle solves live) | shipped | CS F4-SWAP (258), deploy (214) |
| bench-2 Phase-3 tasks (v0.7.0, +7, 88->95) | shipped | 99d7f04 / merged d2a5818, ADR 0113 (CS 252) |
| bench-2 pcell_box geometry ported to Rust (not a real produce) | ledgered (documented drift gap) | ADR 0113 |
| SPICE/netlist benchmark task | ledgered (no writer to grade at the time; not built) | ADR 0113 |
| classroom teaching mode (roster+follow over presence) | shipped | cf49205 / merged 92fc7ee, ADR 0111 (CS 256, 257) |
| classroom instructor live roster | ledgered (honest empty until write-capable presence) | ADR 0111 |
| f2f3-wiring: PCell inspector -> real produce; trace -> live queries | shipped | 50381b5 / merged e880616 (CS 257, 277) |
| xschem file.export_spice + xschem.import_probe | shipped | fab0723 / merged 43e729a, ADR 0112 (CS 256, 257) |
| xschem export bridge rewire to reticle_extract::spice | ledgered follow-on (dedup/quality, not correctness) | ADR 0112; CS 259 |
| paper-skeleton (docs/paper/paper.md + claims-evidence.md) | shipped (staged, every numeric slot a placeholder) | c828a9f / merged c595524 (CS 248) |
| wasm export status text names the actual format | shipped (fix) | bc9f024 (CS 254, 257) |
| native-only rhai (browser-frugal, bundle back under +450) | shipped | ADR 0115 (CS 274) |
| browser PCell predicted-provenance + desktop-for-live disclaimer | shipped | ADR 0115 |
| pcell-produce full fuzz soak | deferred-to-valley (~15k execs zero-crash so far) | D50; CS 205 |
| white-example carry-item | gated/resolved (not reproducing; color guard green; browser-pane artifact) | CS 263-270 |
| Gate 3 close (seam canaries, e2e-headed 22, bundle-gate, deploy, smoke) | gated/deployed | 71797b3; web-23f134b04998df27 (CS 214, 277) |

## Cross-phase valley queue / carry items

| item | disposition | evidence |
|---|---|---|
| Real multi-die bulk fetch + R2 content-hash upload | deferred-to-valley | D36 |
| Real-model benchmark leaderboard rows (Anthropic/Ollama/claude-code) | deferred-to-valley | D49 |
| pcell-produce full fuzz soak | deferred-to-valley | D50; CS 205 |
| H1 deployed share relay (share-on-web default localhost) | operator-owned, OPEN | v82-backlog H1; CS 46 |
| Deep-zoom far-from-origin f32 cancellation | parked (ADR 0100) | v82-backlog (69) |
| v8.2 UI polish (L1 tooltip, L2 drag-move, L3 clipboard chords, Layers UX) | deferred (polish sweep) | v82-backlog |
| M1 Open-Recent / M2 SVG-web-download / M4 streamed-tile-error | triaged to Phase-1 lanes; UNCERTAIN if closed | v82-backlog; CS 46 |
| bundle-ledger.md missing Phase 1/2/3 rows | GAP (append at Phase 5) | see perf-refresh.md |

## Phase 4 -- Reach (main c41fce2; deployed web-466ed5fe877ba93c; +454.5 KiB gz). All 9 lanes merged + Gate 4 closed. Not in dispositions.md before this session; now backfilled (D66-D75 + Gate-4-close rows).

| item | disposition | evidence |
|---|---|---|
| Native wasm plugin runtime (wasmi, native-only) + v0 calling convention | shipped | ADR 0116, 2827ef8 -> 67bfbfa; 17 tests; wasmi absent from wasm target (zero bundle delta) |
| Native plugin host production (real EditableDocument query surface + all 8 Edit opcodes + edit-decoder fuzz) | shipped | ADR 0117, e4490f1 -> 4743a13; 30/30; fuzz 24.7M execs/61s zero-crash (long soak -> valley) |
| Sample plugin (fiducial-marker, real wasm32 guest via v0 ABI) | shipped | 4cd0583 -> e3b3c62; 333B deterministic wasm (sha256 ead8bd02); 6 host-run tests; workspace-excluded |
| F5 static plugin index generator + committed index | shipped | ADR 0121, 6991ab6; index regenerated at gate 367e5b9 to fold fiducial-marker |
| Plugin manager UI (F5 browser browse/preview + desktop-run split) | shipped | ADR 0120, 20099d8 -> 4511b76; browser lists index + disclaimer (never runs); native runs via Host::run one undo-group; plugin.* reserved -> REGISTRY (RESERVED now empty) |
| Plugin browser is browse/preview-only; runs are desktop-only | ledgered (native-only) | ADR 0120; in-UI disclaimer "Plugins run in the desktop app; this browser build lists and previews the index only" |
| Image underlay (PNG/JPEG, browser-native decode avoiding image crate in wasm) | shipped | ADR 0118, 99f05be -> e006ae3; measured PNG-via-image-crate +493.4 BREACH -> pivoted createImageBitmap decode +446.0 PASS; 1113 tests; cap-before-allocate |
| Embed mode hardened (embed.toggle live + minimal ChromeLayout + embeddable-iframe docs) | shipped | 006fc37 -> bf789e4; embed.toggle reserved -> REGISTRY no chord; 1029/1029; docs/src/embedding.md |
| Tauri desktop shell (reticle-desktop, offline bundle, native rhai produce in-window) | shipped | ADR 0119, 1fefe21 -> 973cfc0; desktop/ workspace-excluded; depends reticle-script/agent directly (no reticle-app edit); docs/src/desktop.md |
| Tauri human GUI menu-click (offline open + live produce) | ledgered (un-automated; scripted native smoke covers the exact code) | Gate 4; run_native_produce cargo-test 2/2 + offline-open window; dev exe not allowlistable for computer-use, non-interactive session, native menu not DOM-drivable |
| Bundle budget amendment +450 -> +456 KiB gz (v8.2 new major) | gated/FINAL (trim declined) | ADR 0122, d9a9420 then 5ea6fec; +453.8 gz reconfirmed; +450 trim evaluated vs code and declined as a contortion; bundle-ledger backfilled |
| Gate-4 headed coverage: additive underlay_loaded/embed seams + App::blank_editor() + ?e2e-example=blank + demo-phase4 e2e spec | shipped | b70c3d7; headed Playwright 22/22 both backends; ui-check no-shift; seam canaries additive-only; bundle +454.5 gz |
| Gate 4 close (ci, headed Playwright 22/22, bundle-gate, ui-check, seam canaries, Tauri offline verify, deploy, smoke) | gated/deployed | c41fce2; web-466ed5fe877ba93c (smoke-pass); gh-pages 7b9802f |

## Phase 5 prep -- this session (docs + benchmark data; no app code; no deploy)

| item | disposition | evidence |
|---|---|---|
| ADR README index backfill (0101-0122; was stale at 0100/0121) | shipped | docs/decisions/README.md; 122 files == 122 rows, both-ways verified |
| dispositions.md Phase-4 + Gate-4-close rows | shipped | scratch/campaign/dispositions.md (this session) |
| Leaderboard regenerated + byte-stable verified x2 + committed == generated | verified (deterministic, no model run) | `cargo run -p reticle-bench -- leaderboard --out -` x2 identical; == docs/src/leaderboard.md |
| v0.7.0 real-model benchmark rows | pending / honest not-run | no ANTHROPIC_API_KEY; claude-code backend shares this session's subscription quota (throttles); RETICLE_MCP_BIN unbuilt; only old ollama runs at superseded suites (adhoc/0.4.0) exist -> valley-queue, label pending |
| STATUS + honest-limits + perf-refresh drafts folded through Phase 4 | shipped (scratch drafts) | scratch/campaign/phase5-prep/ (this session) |
| bundle-ledger.md Phase-4 row | already current | docs/design/bundle-ledger.md backfilled through Gate 4 b70c3d7 (+454.5 gz) during Gate-4 close |

## Sweep notes for Phase 5
- Phases 0-4 are all CLOSED and DEPLOYED (CS deploy ledger; Phase 4 = web-466ed5fe877ba93c,
  smoke-pass). Phase 5 prep (this session) is docs + benchmark data only, no app code.
- STILL UNCERTAIN, resolve at the release gate: whether v82-backlog M1/M2/M4 were actually
  closed by their triaged Phase-1 lanes (the dispositions rows for those lanes do not
  explicitly tick them). The wasm export-status fix (bc9f024) addresses the M2-adjacent
  status-text bug but not necessarily the SVG-web file-download itself; verify. Carried into
  honest-limits as a GAP/UNCERTAIN row, not silently dropped.
- LEDGER SHAPE: dispositions.md (the append-log) carries P0, P1, Gate 1, P2, and now P4 +
  Gate-4-close rows, but NOT the Phase-3 rows (netlist, sim-engine, waveform-ui, classroom,
  xschem, bench-2, f2f3-wiring, paper-skeleton, native-only-rhai), which are sourced to CS
  gate-evidence + ADRs 0108-0115. THIS table is the comprehensive P0-4 matrix. If a single
  consolidated ledger is wanted at the release gate, backfill dispositions.md P3 from this
  table; it is a consolidation, not new evidence (a release-gate option, not a prep blocker).
- v0.7.0 real-model leaderboard rows are an honest not-run / pending (no model access this
  session; see the Phase-5-prep table). The committed leaderboard's 3 real rows are at
  EARLIER suites (adhoc, 0.4.0) and are never compared across the v0.7.0 denominator.
