# v8.0.0 run state (single writer: orchestrator)

## >>> OVERNIGHT AUTONOMY AUTHORIZATION (operator, 2026-07-07 night) <<<
Run the packet to completion, fully autonomous, no human input until morning. Gates are checkpoints to run+pass, not stops. Only a red gate I cannot fix-forward, or total quota exhaustion, halts progress (protocols below). Standing authorization: dispatch/gate/merge/redeploy Waves 2-9 in wave order, pipelined disjoint next-wave lanes when slots free. All prior amendments/rules/conventions in force.
PERMANENT CONVENTIONS: (a) lane done = RESULT.md or PARKED.md; exit-0-without-artifact gets ONE resume-to-finalize, then verify at gate against the brief's explicit SUCCESS BAR (not just tests-green), log outcome where RESULT.md would go (applies now to 7A, 5E, 2D-alpha). (b) placeholder ADR ids in briefs; orchestrator numbers them at merge in merge order. (c) Windows npm-shim CLIs -> .cmd when spawned from Rust. (d) app-touching lane gates include the web bin. (e) bench claims cite FULL-gate test counts, never scoped lane counts.
ADVERSARIAL REVIEW at EXACTLY two points: Wave 3 gate (GPU-vs-CPU oracle correctness, chunked compaction boundaries) + Wave 4 gate (V1-fixture migration correctness, selective-undo convergence, view-mode permission). Confirmed HIGH fix before that wave's redeploy; else tracked entries. No other reviews (quota to lanes).
RESILIENCE: rate limit -> backoff + auto resume, never thrash. Red gate -> fix forward if diagnosable+contained (Wave 1 precedent); else revert offending merge, park lane honest, continue with the rest. Near context limit -> write full resume block here FIRST then continue compacted. Every wave boundary commits RUN_STATE + pushes.
WAVE-SPECIFIC: W2 gate verify 2A peak-RSS vs bar + differential test covers committed crash fixtures before merge; dispatch converter-CLI right after 2A merges; served-archive Playwright spec runs vs LOCAL ranged server regardless of hosting. W3 GPU exclusivity ABSOLUTE (3B/3C/3D/captures/soak one at a time; 3A may pair with one GPU lane). W4B commit V1 golden fixture with pre-V2 build BEFORE any V2 schema change (non-negotiable order). W5 5B only after 5A LefDefDesign frozen at merge; container oracles use pinned E: images. W6B vision model never with GPU lanes; honest not-run if VRAM-bound. W7 leaderboard format freezes at 7A merge; 7B consumes banked bench (PARTIAL if incomplete); 7C only after bench workers fully STOPPED (edits task TOMLs). W8 captures GPU-serialized; bundle-size ledger; soak vs deployed relay if live else local+labeled.
W9 IN FULL: gauntlet; skeptical STATUS audit (every not-run/parked/PARTIAL); version bump; CHANGELOG; branch pre-v8-audit at pre-tag commit (rollback anchor); tag v8.0.0; redeploy; verify URL + README media; GitHub release w/ binaries. Claims = ONLY evidence-backed (measured, passed smokes, deployed+verified); unverified listed as such.
MORNING REPORT goes at the very top here: waves completed, shipped-live (bundle, relay version, archives), headline measured numbers, full not-run/parked ledger w/ operator commands, bench final row, review findings+dispositions, top-3 for operator.
IMMEDIATE (in order): trim bench to 3 (kill 4 leftovers) + dedupe dup task records (earliest honest per task); when 1E exits/contention clear -> redeploy relay (just conformance; wrangler deploy; record version); run deferred e2e/e2e-share/conformance no later than the Wave 2 gate.

## RESUME BLOCK (if this session dies, start here)
Position (2026-07-07 night): WAVE 2 FULLY SHIPPED (2A-2F merged incl. converter + browser ?archive streaming). main pushed 79d0cbf+. Site live web-ca5048627c96929e (gh-pages 53f02ea, smoke live). Relay 33fe97b0. Served-archive spec passes (just e2e-archive). Wave 2 CI GREEN.
WAVE 3 progress: 3A GREEN-parked (DRC-as-you-type, ADR 0074, reticle-app; two-way test + us-scale per-edit latency). 3B GREEN-parked (GPU DRC heatmap, ADR placeholder->0075 at merge, reticle-render; GPU-vs-CPU proptest RAN+passed on this host). 3C GREEN-parked (GPU-resident hierarchy, ADR 0074-placeholder->0076 at merge, reticle-render; measured 100M flat-equiv @111fps culled/10fps all, 30M @114fps, chunked 11/36 chunks, zero-CPU-per-frame asserted; CellCompactor API unchanged). IN FLIGHT: v8-3d-metrology (density GPU overlay + area/perimeter/antenna/connectivity reports; GPU lane; likely a NEW crate reticle-metrology + a density_overlay.rs in reticle-render; UUID 380d9a66-85e4-4188-a205-4c79f70c87d6). Then Wave 3 gate: merge 3A(app-last)/3B/3C/3D, resolve render lib.rs mod-line conflicts + assign ADR numbers 0074(3a taken)/0075(3b)/0076(3c)/0077(3d) in merge order, ADVERSARIAL REVIEW (GPU-vs-CPU oracle, chunked compaction boundaries), fix confirmed HIGH before redeploy, ci+redeploy+smoke. (Dispatch lesson: instant-exit-1-no-output = stuck session UUID; retry with a FRESH uuid. Do NOT combine `just lane`+uuid-gen+dispatch in one background cmd - the uuid gets lost; recover from the task output if so.) Done-parked awaiting their WAVE gates (merge at Wave 5/6/7): 5A/5C/5D/5E (Wave 5), 6A (Wave 6), 7A (Wave 7), 1E (Wave 1 tail - can merge anytime, low priority).
NEXT: (1) when 3A+3B green -> Wave 3 batch 2: dispatch 3C (GPU hierarchy, reticle-render) THEN 3D (metrology, GPU overlays) ONE AT A TIME (GPU exclusivity absolute; 3C and 3B/3D never concurrent). 3C/3D touch reticle-render (serialize vs 3B) - so 3C dispatches only after 3B merges. 3A(app) can pair with one GPU lane. (2) Wave 3 gate: ADVERSARIAL REVIEW (GPU-vs-CPU oracle correctness, chunked compaction boundaries) - confirmed HIGH fix before redeploy. Merge order app-last. Redeploy + smoke. (3) Wave 4 (post-Wave-3): 4A diff, 4B comments (COMMIT V1 GOLDEN FIXTURE with pre-V2 build BEFORE any V2 schema change - non-negotiable), 4C write-multiplayer, 4D PWA. Adversarial review at Wave 4 gate too. (4) Wave 5: merge 5A/5C/5D/5E + dispatch 5B (after 5A LefDefDesign frozen at merge). (5) Wave 6: merge 6A + dispatch 6B(vision, never w/GPU)/6C. (6) Wave 7: merge 7A + freeze leaderboard format + dispatch 7B/7C(7C only after bench STOPPED - it is stopped). (7) Wave 8: product page + soak. (8) Wave 9: gauntlet + audit + tag v8.0.0 + release. Bench DONE 81/83 (72 pass).
CONVENTIONS: lane RESULT.md is at scratch/lanes/<id>/RESULT.md in the lane WORKTREE (D:\dev\reticle-lanes\<id>\...) - some lanes wrongly wrote /RESULT.md at root (now gitignored + git-rm'd at merge). Verify exit-0-no-artifact lanes at their gate vs the brief success bar. ADR ids in briefs are PLACEHOLDERS; assign at merge in merge order (Wave 2 used 0068 2a-onwire, 0069 2b-framing, 0070 2d-alpha-worker, 0071 2c-dochost; next free 0072). Tooling: dispatch-lane.ps1/resume-lane.ps1/bench-extension.ps1 under scratch/. Deploy: just deploy-pages -> gh-pages worktree publish -> just smoke-pages. Relay: cd worker; npx wrangler deploy.
KNOWN GAP: served-archive Playwright spec + browser ?archive wiring was 2E scope (not in 2A-2D); lane 2E now building it. Streaming proven natively (rtla_writer_reader_agree cross-test, 2B streamed==in-RAM proptest, 2C residency test, 2A builder 126.9 MiB @30M).

updated: 2026-07-07 night (overnight autonomous)
phase: wave2/shipped -> wave2-completion(2e,2f) + wave3/dispatch-next
head: Wave 2 merged + shipped live

## SHIPPED LIVE (running tally):
- Site bundle: web-ca5048627c96929e (Wave 2 COMPLETE: streaming reader/builder, TileSource, DocHost+residency, archive worker, license xtask, gds->rtla converter, browser ?archive streaming + HUD, served-archive e2e), gh-pages 53f02ea, live. URL https://alpharomerojl.github.io/reticle/.
- DO relay: reticle-relay.josefdean.workers.dev, version 33fe97b0-8b2f-4bc0-8591-e3724b365ae9 (hardened). Conformance GREEN both relays.
- Bench claude-code row FINAL: 81/83 committed, 72 passed (89% of run tasks); PARTIAL (2 tier-5 tasks produced no record - persistent auth/quota on those). All bench workers STOPPED (unblocks 7C at the Wave 7 gate). vs v7's 25/83. Row stays labeled PARTIAL; 7B consumes this banked set.
- Wave 2 HEADLINE NUMBERS (measured): external builder 30M-entry .rtla at 126.9 MiB peak RSS (budget ~2 GiB); 120M records -> 2.42 GB archive in 45.2 s; builder/reader framing agreement fixed (ce04438) + cross-test guards it; streamed query == in-RAM R-tree query (2B proptest).
- Immediate overnight actions DONE: bench trimmed to 3 (0 dup records), relay redeployed, deferred e2e/e2e-share/conformance ran GREEN.

## AMENDMENT (operator, 2026-07-07): pipelined dispatch authorized
When concurrency slots are free AND the next wave has lanes whose file surfaces are provably disjoint from every in-flight lane, run that wave's contract step and dispatch those lanes early. They STILL merge only at their own wave gate, in declared order. Wave 1 lanes merge first at the Wave 1 gate; early-dispatched lanes park green and merge at their own gates.
Immediate application (spend the unspent ~half of the 5-hour window, resets ~40 min): (1) Wave 2 contract step on main NOW (.rtla v1 types + TileSource trait + gds_stream event API as doc-contracted stubs in crates/reticle-index/src/archive.rs + crates/reticle-io/src/gds_stream.rs + ADR; additive files, no Wave 1 lane touches these crates); scoped ci, commit, push. (2) Dispatch 2A + 2B off post-contract main, E: targets, briefs first, UUIDs recorded before dispatch. Extra brief requirement for both: every count/size field from a stream or header is UNTRUSTED, never reserve capacity beyond remaining input (OASIS OOM lesson, commit 1b1b56b); 2A's fuzz target seeds from the committed GDS crash fixtures so the streaming reader cannot reintroduce the date-panic class. (3) If a slot remains, dispatch 5A (new crate crates/reticle-lefdef; LefDefDesign freeze stays a Wave 5 merge event). DO NOT dispatch Waves 3/4/6/7/8; DO NOT touch Wave 1 lanes/worktrees/merge order. On rate limit: standard backoff; session UUIDs are resume handles; expect the window reset and continue.
Disjointness verified: contract-step + 2A/2B code files are in reticle-index/reticle-io (no Wave 1 lane touches these); 2A owns archive_build.rs+gds_stream.rs impl+fuzz, 2B owns tile_source.rs impl (pre-declared as separate modules in the contract step so the two never edit the same lib.rs region); 5A is a brand-new crate. Only shared append points are docs/decisions/README.md (ADR index) and root Cargo.toml members (5A vs 1B) - trivial sequential appends, orchestrator-merged.

## WAVE 1 ADVERSARIAL REVIEW (workflow wb6z581wt, 32 agents): 8 confirmed (3 high), 5 refuted. MUST FIX before Wave 1 gate redeploy.
HIGH:
  1. reconnect-resync app.rs:1570 - drive_sharer_publish rebuilds a FRESH SyncDocument::from_document(VIEWER_ACTOR,..) per publish; yrs clock resets to 0, so a viewer that applied snapshot-1 drops snapshot-2 as duplicate/stale -> shapes added after the first publish (incl. agent edits, the cross-lane target) NEVER reach viewers. Fix: ONE long-lived sharer SyncDocument, clocks advance monotonically.
  2. do-relay-wire worker/src/lib.rs:147 - DO never logs PRESENCE frames (0x12), only updates; native relay logs+replays EVERY binary frame, so a late joiner gets no last-known presence on the DO. Fix: log presence too (match native), + add a conformance vector: presence-before-late-join.
  3. abuse-security worker/src/lib.rs:129 - DO accepts binary frames of any size and appends every non-presence frame to DO storage with NO size/count cap -> one editor can OOM/exhaust room storage. Fix: cap frame size + cap log length.
MEDIUM: 4 (reconnect test share_live.rs:843 reuses one doc, masks #1 - fix WITH #1 by replaying app path), 5 (worker presence/update FIFO order not preserved), 6 (worker no per-room conn cap / room-creation rate limit).
LOW: 7 (livesync.rs:479 reconnect loop: on_open resets attempt=0 with no min-uptime guard -> unthrottled redial), 8 (collab.rs:408 mirror draws AddRect/Path/Polygon even when session rejected them -> degenerate CRDT shape).
REFUTED (5, informational, no action): worker log-unbounded dup, coalesced-presence-from-departed, mirror_transform drift, shared-alarm-slot, origin-not-validated (this last one, worker/src/lib.rs:63 open cross-origin ws, is REFUTED but worth a look during the fix).
FIXED (commit 5f8dbd0): findings 1,2,3 (HIGH) + 4,5,6 (MEDIUM). reticle-sync reconcile_to + test (viewer converges across publishes), app.rs persistent sharer doc, worker presence-replay + frame/log caps + FIFO flush + per-room conn cap. Verified: reticle-sync test green, reticle-app native+wasm clippy clean, worker wasm check clean.
DEFERRED (LOW, honest follow-ups, do NOT block redeploy): 7 (livesync.rs:479 reconnect resets attempt=0 on open; already floored ~500ms so bounded not infinite; a proper min-uptime guard needs uptime tracking - follow-up), 8 (collab.rs:408 mirror draws AddRect/Path/Polygon even when the session rejected them -> a degenerate CRDT shape absent from the session; needs the per-command session-acceptance handshake threaded into mirror_command - follow-up, low: cosmetic CRDT inconsistency, no crash/dataloss). Both are candidate work for a focused follow-up lane.
Fixing on main now (orchestrator-direct); lanes 2C (reticle-app) and 2D-alpha (worker/archive) rebase at their gates - my fixes touch app.rs/livesync.rs/worker-src/collab.rs/share_live.rs, 2C touches a new dochost module + session/tool, 2D-alpha touches worker/archive - low conflict.

## WAVE 1 GATE + REDEPLOY DONE (2026-07-07, post-reconciliation)
  just ci GREEN (Wave 1 review fixes validated on the full workspace). deploy-pages built web-e6f6b8398ffb8c08; published to gh-pages (763660a); just smoke-pages PASS against the live URL. The flagship reconnect fix + all Wave 1 app features are LIVE at https://alpharomerojl.github.io/reticle/.
  PENDING (not blocking): (a) worker DO redeploy - my worker fixes (presence-replay, frame/log caps, FIFO, conn cap; commit 5f8dbd0) are on main but the DEPLOYED relay (reticle-relay.josefdean.workers.dev) still runs 1B's original code; needs `just conformance` (validate my vectors) then `wrangler deploy` from worker/ when load is lower. (b) e2e/e2e-share/conformance as browser validation - deferred to avoid port/GPU contention with the running 1E lane; ci (incl wasm-build) already validated compilation.
  Session-ended-without-RESULT lanes (turn-budget hit the final RESULT.md step; committed real work; do NOT resume-loop - verify their gate directly at their wave gate): 7a-leaderboard (4 commits), 5e-python (5 commits), 2d-alpha-worker (4 commits). 7a was resumed once and added commits but still ran out before RESULT.

## LANE STATE RECONCILIATION (housekeeping directive, 2026-07-07)
Ground truth via Win32_Process + RESULT.md/PARKED.md + stream-log write-age. NO true zombies: every completed lane's wrapper OS process had already exited (the "still Running" was harness bookkeeping, not runaway compute; nothing to kill).
  COMPLETE, RESULT present, parked green for their wave gates (process gone, log 45-59 min stale): v8-2a-reader-builder, v8-2b-tilesource, v8-5a-lefdef, v8-5c-lydrc, v8-6a-devextract.
  RUNNING, real work (log age 0, live wrapper): v8-1e-proof, v8-2c-residency, v8-2d-alpha-worker, v8-5d-interop-pdk. (v8-5e-python dispatch just completed exit 0 - verify RESULT next pass.)
  RESUMING (finished session exit 0 without RESULT): v8-7a-leaderboard (session 37c3aa0a..., resume b9pis648w) - report state, write RESULT or park.
  Corrected running count: ~5 lane sessions (1e,2c,2d-alpha,5d + 7a-resume) vs cap 11. Well under. 3 bench workers grinding untouched (separate).
  Wave 2 gate precondition: when 2a,2b,2c,2d-alpha all green -> run Wave 2 contract-completion + gate, incl. dispatching the deferred converter-CLI scope (held out of 2d-alpha pending 2a; 2a is now green, so the converter can dispatch once 2c/2d-alpha land).

## AMENDMENT 3 dispatch record (session ids = resume handles; `claude -r <id> -p "Continue per scratch/lanes/<id>/brief.md"` in the worktree with CARGO_TARGET_DIR=E:\dev\reticle-target-<id>)
  v8-1e-proof         75881dd0-fe1d-4b7c-8935-8a0b4d0e2f38  (gpu lane; e2e/assets/README/justfile)
  v8-2c-residency     bed46f3a-4db4-41f8-a7bd-e69824ea45be  (reticle-app DocHost/residency)
  v8-2d-alpha-worker  819572bb-9513-4aea-acce-a9a70d6f6b67  (worker/archive + xtask license; converter OUT of scope)
  v8-5d-interop-pdk   bb9528e1-5f5d-4d39-8b3d-9b35946110d9  (oasis_std + IHP + GenTech reticle-gen refactor)
  v8-5e-python        d03b47c2-440d-4e49-85cb-ddee149b1c5d  (new crate reticle-py, non-default)
  v8-7a-leaderboard   37c3aa0a-d154-490a-bbd0-627ccbdb46a6  (reticle-bench leaderboard subcommand)
  Also running from amendment 2: v8-2a-reader-builder (no RESULT yet). Bench: 3 stride workers (skip-build). Review: workflow wb6z581wt.
  Still to do after the burst: full Wave 1 gate (just ci + e2e + e2e-subpath + e2e-share + just conformance + redeploy + smoke), then merge the parked Wave-2/5/6/7 lanes at their own gates.

## WAVE 1 GATE DEFERRED to post-burst (contention). Main state e0b3bd9..HEAD:
  - Wave 1 merged (1a/1b/1d/1c) + wave1-gate fix + Wave 2 contract (0a745d7) + bench checkpoint (b938f53) + review fixes (5f8dbd0) + doc-link fix (this commit pending).
  - Review HIGH+MEDIUM fixed and VERIFIED (reticle-sync test green, app native+wasm clippy clean, worker wasm check clean; pre-commit hook ran clippy --workspace --all-targets for 5f8dbd0 = full compile incl tests, passed). Main IS sound.
  - `just ci` attempted but the test-compile was KILLED (exit -1) = resource contention with 9 lane builds + 3 bench workers + resumed 2C all compiling. NOT a code error.
  - DECISION: do NOT run full ci/e2e/redeploy while the burst is spending the expiring-week quota (contention steals CPU from the lanes + makes the gate flaky). Run the gate + redeploy AFTER the burst subsides (lanes complete / quota window closes / machine frees). The committed fixes are safe on main until then; the live site keeps serving the prior (v7-parser-hardened) bundle, and the flagship live-collab fix ships at the deferred redeploy.
  - RESUME the gate: cd D:\dev\reticle; just ci; just e2e; just e2e-subpath; just e2e-share; just conformance; just deploy-pages; publish scratch/pages to gh-pages; just smoke-pages. Then process lane completions + merge parked lanes at their gates.
  - Commit the pending doc-link fix (livesync.rs SHARER_ACTOR intra-doc link -> reticle_sync::SyncDocument::reconcile_to) with the gate run.

## AMENDMENT 3 (operator, 2026-07-07): max dispatch, cap 11, expiring-week burst
Minutes left on the weekly quota; sink it. Cap 11. Wave 1 MERGED on main (e0b3bd9; ADRs renumbered 0062-0067; a wave1-gate fix commit landed). Burst lanes from amendment 2: 2b/5a/5c/6a DONE+green (parked, merge at their gates), 2a still running.
Dispatch order (briefs first, UUIDs recorded, standard rules):
  0. Resume claude-code bench extension, full priority, up to 3 concurrent task workers (stride partition). Checkpoint per task.
  1. 1E browser proof pack (GPU lane, runs captures; only GPU consumer this batch). 1C stats seam is on main.
  2. 2C async residency + DocHost (Wave 1 merged + Wave 2 contract frozen + 2B parked make app-touching safe; brief has coarse-then-fine MemSource latency test).
  3. 2D-alpha SCOPED: archive-serving Worker only (worker/archive/, R2 binding, ranged reads, Cache API, CORS to Pages origin) + license-manifest xtask over staged content dir. Converter CLI OUT until 2A merges. Range headers untrusted.
  4. 5D round-trip interop + IHP SG13G2 + GenTech refactor + timeboxed OASIS writer (disjoint from 2A's gds_stream; oasis.rs/tech/reticle-gen untouched in flight).
  5. 5E reticle-py + notebook widget (new non-default workspace crate, fully disjoint).
  6. 7A leaderboard generator (reads committed benchmark records only; deterministic same-records-same-bytes).
Adversarial multi-agent review of the merged Wave 1 diff (b938f53..e0b3bd9) NOW, added target: does 1A's reconnect full-state snapshot capture shapes created via 1D's agent APIs mid-session.
DO NOT dispatch: 2E (collides 2C in reticle-app), 3A (collides 5C in reticle-drc), 3B/3C/3D/6B (GPU exclusivity while 1E holds captures), 4A-4D (post-Wave-3), 6C (needs 2A merged), 7C (bench workers read task TOMLs). Throttling expected; backoff + UUID resume handles standard.

## AMENDMENT 2 (operator, 2026-07-07): burst mode, cap 9
Weekly quota resets in ~1h; capacity spent before then is free. Maximize safe dispatch now.
- Concurrency cap RAISED to 9 for this burst (original 4 was D: target-dir pressure; targets now on E: per ADR 0060, worktrees thin on D:). CPU contention accepted, throughput over latency. GPU-lane exclusivity still holds (none of tonight's lanes are GPU).
- Dispatch two MORE disjoint lanes now (on top of 2A/2B/5A from amendment 1): 5C (KLayout .lydrc deck compat, new module in reticle-drc; no in-flight lane touches drc; verdict compare vs KLayout headless in the pinned container on E:; untrusted-input rule for the deck parser) and 6A (device-recognition extraction, new module in reticle-extract; MOSFET recognition; magic/netgen oracle in the pinned container; reticle-extract intent types stay frozen per ADR 0061, 6A is an additive sibling module and must not touch them). Both park green, merge at their own wave gates (5C at Wave 5, 6A at Wave 6). UUIDs recorded before dispatch.
- Tonight's full burst = 4 Wave-1 + 2A + 2B + 5A + 5C + 6A = 9 concurrent (exactly the cap). None are GPU lanes.
- Claude-code benchmark row extension (58 unrun tasks) STARTED as a low-priority background process, checkpointed per task (result records to benchmarks/results/v0.5.0/claude-code/), stops cleanly on any rate-limit signal. Deferred to 7B only for quota; free this hour. Row stays labeled PARTIAL until complete; whatever finishes shrinks 7B.
- Unchanged: nothing from Waves 3, 4, 6B, 7A, 7C, 8 dispatches early. Wave 1 lanes/worktrees/merge order untouched. Expect heavy rate limiting through the reset; standard backoff; session UUIDs are the resume handles.

## burst lanes dispatched (amendments 1+2, off main 0a745d7 = Wave 2 contract)
Resume any killed lane: claude -r <session> -p "Continue per scratch/lanes/<id>/brief.md" in D:\dev\reticle-lanes\<id> with CARGO_TARGET_DIR=E:\dev\reticle-target-<id>.
  v8-2a-reader-builder  fdaf5fe4-369f-45d6-83ee-912f673255f1   (Wave 2; gds_stream reader + .rtla builder; merges Wave 2 gate)
  v8-2b-tilesource      18b78d8f-7bbd-44ee-8bda-d54d23821b89   (Wave 2; TileSource impls; merges Wave 2 gate)
  v8-5a-lefdef          e46ecfd2-8412-48ff-a586-6817bc62b704   (Wave 5; new crate reticle-lefdef; freezes LefDefDesign at Wave 5 merge, gates 5B)
  v8-5c-lydrc           0716ecb0-0e17-4a76-baab-cab675b16eb6   (Wave 5; .lydrc deck compat in reticle-drc; merges Wave 5 gate)
  v8-6a-devextract      01837f41-603d-4bf1-9477-01ec95c0ec13   (Wave 6; device recognition in reticle-extract; merges Wave 6 gate)
Concurrency now: 3 Wave-1 (1a DONE exit0, 1b/1c/1d running) + 5 burst = 8 sessions + bench = under cap 9. All disjoint (reticle-index/io=2a/2b, reticle-lefdef=5a new crate, reticle-drc=5c, reticle-extract=6a; no in-flight lane shares a crate). None are GPU lanes.
Wave-1 lanes COMPLETED + verified green (NOT merged; merge at Wave 1 gate in order 1a,1b,1d,1c then 1e):
  - 1a-transport DONE, gate green (613 tests): reconnect backoff + full-state resync; ViewerTransport still no send path (grep-proven); 6 commits.
  - 1c-shareux DONE, gate green (580 tests): permalinks + one-click share + touch pinch; 5 commits.
  - 1d-agentlive DONE, gate green (139 tests): TransformShapes/DeleteShapes mirroring via ElementId map (closes ADR 0022 gap), native live-room mode, committed deterministic DRC-fix transcript (final_hash 3163482734529708122, honestly labeled non-LLM); 6 commits.
  Still running: 1b-relay (the long pole: DO relay + conformance suite).

WAVE 1 GATE reconciliation checklist (execute when 1b done, before/during merge in order 1a,1b,1d,1c then 1e):
  1. ADR 0062 collision x4: Wave 2 contract (main, KEEP 0062) + 1a + 1c + 1d each authored a 0062. Renumber the THREE lane ADRs to next-free (0063,0064,0065 or whatever's free post-burst-lane merges) and fix references: commit-msg mentions, "ADR 0062" strings in livesync.rs/share.rs/collab.rs/live.rs + docs, and docs/decisions/README.md rows. Merge order decides which lane gets which new number.
  2. reticle-sync/src/lib.rs OVERLAP: 1a added encode_full_state; 1d added set_shape/remove_shape (documented deviation - the mapping.rs helpers were pub(crate)). Both additive; ensure clean coexistence when 1d rebases onto main-with-1a.
  3. Rebase all four lanes onto post-contract main (0a745d7) before merging (they branched off 635ff13, which lacks the Wave 2 contract; contract files are in reticle-index/io, disjoint from Wave 1 lane files, so rebase should be clean apart from item 1's docs/decisions/README.md ADR-row appends).
  4. share_live.rs: 1a edited it (kill-and-reconnect test); 1d added NEW file agent_live.rs (per brief) - no conflict.
  5. After merge: run the full Wave 1 gate (just ci + e2e + e2e-subpath + e2e-share + just conformance from 1b), then dispatch 1e (needs 1c's seam, now on main), then redeploy + smoke.

ADR-NUMBER COLLISION to fix at the Wave 1 merge: the Wave 2 contract took ADR 0062 on main (0a745d7). Lanes 1a AND 1c ALSO each authored an "ADR 0062" (branched off 635ff13 before 0062 existed). At merge, RENUMBER 1a's and 1c's ADRs to the next free numbers (0063, 0064, ...) and fix their references (commit msgs + any "ADR 0062" strings in livesync.rs/share.rs/docs) + docs/decisions/README.md rows. The Wave 2 contract keeps 0062 (first on main). Expect the burst lanes (off 0a745d7) to collide on 0063+ similarly - renumber sequentially at each wave gate. This is standard parallel-ADR reconciliation; ordering by merge is the tie-break.

1c stats seam (for lane 1e when dispatched post-1c-merge): window.__reticle_stats = { applied_frames:number (per applied CRDT frame that changed geometry), applied_shapes:number (SET to top-cell shape count after each frame) }, wasm-only, absent until first frame (treat undefined as 0). ?e2e-edit=1 with ?share=1 makes the publisher place ONE rect layer 68/20 from (0,0)-(1000,1000) DBU after go-live. Permalink URL: ?gds=<url>&cell=<name>&view=<x>,<y>,<zoom>&layers=<layer/datatype,...> (RFC-3986 component-encoded; empty layers= hides all).

bench extension: FIXED (backend needed RETICLE_CLAUDE_BIN=C:\Users\jo312\AppData\Roaming\npm\claude.cmd - the .ps1 shim is not exec-able by Rust Command; RETICLE_MCP_BIN=D:\dev\reticle-target\release\reticle-mcp.exe). Relaunched low-priority; stops cleanly on the first rate/auth/quota not-run. Results to benchmarks/results/v0.5.0/claude-code/; 25/83 committed so far, row stays PARTIAL.

## dispatch mechanism VALIDATED (2026-07-07)
- Headless dispatch works: `<prompt-on-stdin> | claude -p --permission-mode bypassPermissions --strict-mcp-config --session-id <uuid> --output-format text` in a pinned cwd ran a sub-session autonomously (no prompts), used its Write tool to create a file in the correct cwd, and returned clean (exit 0). This validates the Wave 1+ lane fan-out model.
- Resume works: `claude -r <uuid> -p <prompt> ...` recalled the exact content the original session wrote, confirming context persistence across resume. This is the quota-backoff recovery lever; it is real.
- For real lane dispatch use --output-format stream-json (machine-parseable) + 1>/2> capture to scratch/logs/<lane>.{stream.jsonl,err.log}.
head: bbf853defd9bb8638709b4c4794cfad519cafde8
plan: C:\Users\jo312\.claude\plans\reticle-v8-0-0-the-expressive-iverson.md
disk: C: 37.9 GB free, D: 36.1 GB free, E: 780.2 GB free (measured 2026-07-07T12:45)
concurrency: 4 (6-lane mode DISABLED at start: D: below 60 GB; re-evaluate after Wave 0 disk work)
quota: consecutive_failures=0; last_event=none; backoff_next=15m
cloudflare: authed=yes (wrangler 4.82.2 OAuth, account 86e3e2cfeb39c385931af8bb9e1934e6); bucket=reticle-archives CREATED (Standard class; binding: r2_buckets [{bucket_name reticle-archives, binding reticle_archives}]); r2 scope concern resolved by the successful create

## preflight evidence (Wave 0.1)

- v7.0.0 anchor VERIFIED: tag v7.0.0 present; gh release v7.0.0 not-draft with 4 assets (reticle-app.exe, reticle-demo-server.exe, reticle-server.exe, reticle.exe); `just smoke-pages` PASS against https://alpharomerojl.github.io/reticle/ (base + 2 assets 200 under /reticle/). Packet Wave 0 item 1 (release the held v7) was already complete before this run; recorded as verify-only.
- Benchmark rows: already v0.5.0/83-task on main (gpt-oss:16k 49/83, qwen2.5-coder:16k 29/83, claude-code 24/25-ran PARTIAL tiers 1-3). Packet Wave 0 item 4 verify-only; claude-code row extension deferred to Wave 7B (same subscription quota the lanes need).
- Tapeout oracle: official precheck already PASSED in v7 (examples/tapeout/precheck-results.md). Re-run scheduled as the Docker-relocation acceptance test (Wave 0.5).
- Tools: claude 2.1.202; gh 2.86.0; node v24.14.0; Python 3.14.3; uv 0.11.6; git 2.53.0.windows.2; just 1.55.1; rustc/cargo 1.94.1; Docker 29.5.3 (Desktop, WSL2 backend); WSL default Ubuntu v2; wrangler 4.82.2 (4.107.0 available).
- claude CLI dispatch flags re-verified from --help: -p/--print, --permission-mode, --session-id <uuid>, --strict-mcp-config, --output-format stream-json, -r/--resume, --fallback-model. v7 lessons honored: stdin prompt, NO --allowed-tools, absolute transcript paths.
- Ollama models present: gpt-oss:16k (13 GB), qwen2.5-coder:16k (9.0 GB), gpt-oss:20b, qwen2.5-coder:14b. NO vision model (Wave 6B pulls one).
- Cloudflare agent-setup guidance file at repo root: ABSENT. Substitution: installed cloudflare:* skills + wrangler --help probing; orchestrator inlines verified command shapes into lane briefs (1B, 2D).
- Appendix B recovered: "Windows and PowerShell rules" appendix of gitignored reticle-autonomous-build-plan.md; digest carried into every lane brief.
- fuzz/corpus/ absent despite .gitignore comment; Wave 0.7 creates + commits minimized seeds.

## waves

- wave0: in-progress
    0.1 preflight+RUN_STATE: done (this file)
    0.2 disk policy: done (ADR 0060; stale v7 lane target deleted, D: 51.5 GB free; E: probe 1.23x; lane targets on E:\dev\reticle-target-<id> this run)
    0.3 tracker+frozen-surface ADR: done (ADR 0061; TASKS.md v8 section)
    0.4 cloudflare bootstrap: done (bucket reticle-archives created)
    0.5 docker-to-E + precheck acceptance: done VERIFY-ONLY (already on E:; precheck reproduced committed verdict exactly, wall 28.6 s, exit 1 = the four documented out-of-scope artifact checks)
    0.6 bench verify-only: done (README rows match committed benchmarks/results/v0.5.0 records: 49/83, 29/83, 24/25-partial; labels honest)
    0.7 fuzz campaign: THREE REAL BUGS FOUND AND FIXED (all reticle-io parsers); gds final re-confirm running.
        Big-picture honesty win: prior STATUS (v4-v7) claimed "fuzzing does not run on this Windows/MSVC host (libFuzzer will not link)". FALSE for the WSL path: the campaign ran fine under WSL Ubuntu (nightly + cargo-fuzz 0.13.2, fork=4). STATUS must be corrected at Wave 0 close.
        Bugs (all would ABORT a wasm tab; catch_unwind only saves native, and the fuzz build aborts on panic exactly like wasm, which is why it surfaced them):
        - gds_import date panic: malformed BGNLIB/BGNSTR dates -> gds21 feeds chrono -> panic. Fixed pre-parse (commit 8d4457a). 4 crash fixtures.
        - gds_import zero-length-string panic (gds21 read.rs:170 read_str indexes data[-1]): was KNOWN and relied on catch_unwind (native-only); fuzz proved catch_unwind insufficient for wasm. Fixed: guard rejects zero-length string records pre-parse (commit e8752f7). 3 crash fixtures. This was the 440-crash "second class" the date fix did not cover.
        - oasis_import OOM: unbounded Vec::with_capacity(count) (53,321 oom artifacts from ~28-byte inputs). Fixed: Reader::prealloc caps at remaining/min_elem_bytes (commit 1b1b56b). 4 oom fixtures.
        Confirmed clean (fresh clean build, 30 min each, 0 artifacts): oasis_import, geometry_boolean.
        PROCESS LESSON (critical): the /mnt/d 9p mount defeats cargo incremental rebuild (a 4s "build" reused a pre-fix binary and reported 1063 already-fixed crashes as if new). ALWAYS use a FRESH CARGO_TARGET_DIR for fuzz builds after a source change; verified staleness by running 40/40 crash artifacts through the fixed NATIVE importer (all clean).
        gds re-confirm (commit e8752f7, fresh target dir, seeded with all 440 crash inputs, 15 min): RUNNING (task b733dhx5r).
    0.8 gate+redeploy: DONE. gds re-confirm CLEAN (0/15min). Corpus committed (e18cc43, 150 seed files). STATUS+fuzz README corrected. Gate: just ci GREEN, just e2e 3-pass-1-skip, just e2e-subpath 1-pass, just e2e-share 1-pass. Redeployed: gh-pages 4bb08b1 (web-db328fe1, hardened parsers live), just smoke-pages PASS against live URL. main + gh-pages pushed, origin synced.
  WAVE 0 COMPLETE.
- wave1: DISPATCHED batch 1 (4 concurrent) 2026-07-07. Worktrees off 635ff13. Resume a killed lane with: claude -r <session> -p "Continue per scratch/lanes/<id>/brief.md" in its worktree with CARGO_TARGET_DIR=E:\dev\reticle-target-<id>.
    session ids:
      v8-1a-transport  42c18e70-3b4a-4d17-a0ab-628bada525c6
      v8-1b-relay      a8d80065-faa6-40bc-a1f6-5cb498b1de97
      v8-1c-shareux    7211a729-11df-47c3-a1a6-e504c2f3eec3
      v8-1d-agentlive  c8b3c29c-f991-4acc-af03-fad814f5a638
    batch 2 (staggered, after 1c merges; gpu_lane runs alone): v8-1e-proof (worktree not yet created)
    declared merge order: 1a, 1b, 1d, 1c (app-UI-heaviest last), then 1e
    file-ownership fences (in briefs): livesync.rs=1a; share.rs/webopen.rs=1c; worker+conformance+proto-tag=1b; agent=1d; e2e+README media=1e. Shared-file merge risks (additive, sequential merge handles): justfile (1b conformance + 1e capture-share), collaboration.md (1a reconnect + 1c permalink/touch).
    lane RESULT.md lands in the lane worktree: D:\dev\reticle-lanes\<id>\scratch\lanes\<id>\RESULT.md. Orchestrator logs: scratch/logs/<id>.stream.jsonl + .err.log.
- wave2..wave9: pending

## not-run ledger (honest)

(empty so far)

## resume (exact next action if this session dies)

Read this file top to bottom; continue at the first pending item in the current phase. Wave 0 is orchestrator-direct (no lane sessions yet); the plan file section "Wave 0" lists each step's exact commands and fallbacks.
