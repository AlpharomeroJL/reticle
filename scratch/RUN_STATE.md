# v8.0.0 run state (single writer: orchestrator)

updated: 2026-07-07T15:40:00-05:00
phase: wave1/dispatch
head: (Wave 0 closed; main pushed) e18cc43 + wave0-close commit

## AMENDMENT (operator, 2026-07-07): pipelined dispatch authorized
When concurrency slots are free AND the next wave has lanes whose file surfaces are provably disjoint from every in-flight lane, run that wave's contract step and dispatch those lanes early. They STILL merge only at their own wave gate, in declared order. Wave 1 lanes merge first at the Wave 1 gate; early-dispatched lanes park green and merge at their own gates.
Immediate application (spend the unspent ~half of the 5-hour window, resets ~40 min): (1) Wave 2 contract step on main NOW (.rtla v1 types + TileSource trait + gds_stream event API as doc-contracted stubs in crates/reticle-index/src/archive.rs + crates/reticle-io/src/gds_stream.rs + ADR; additive files, no Wave 1 lane touches these crates); scoped ci, commit, push. (2) Dispatch 2A + 2B off post-contract main, E: targets, briefs first, UUIDs recorded before dispatch. Extra brief requirement for both: every count/size field from a stream or header is UNTRUSTED, never reserve capacity beyond remaining input (OASIS OOM lesson, commit 1b1b56b); 2A's fuzz target seeds from the committed GDS crash fixtures so the streaming reader cannot reintroduce the date-panic class. (3) If a slot remains, dispatch 5A (new crate crates/reticle-lefdef; LefDefDesign freeze stays a Wave 5 merge event). DO NOT dispatch Waves 3/4/6/7/8; DO NOT touch Wave 1 lanes/worktrees/merge order. On rate limit: standard backoff; session UUIDs are resume handles; expect the window reset and continue.
Disjointness verified: contract-step + 2A/2B code files are in reticle-index/reticle-io (no Wave 1 lane touches these); 2A owns archive_build.rs+gds_stream.rs impl+fuzz, 2B owns tile_source.rs impl (pre-declared as separate modules in the contract step so the two never edit the same lib.rs region); 5A is a brand-new crate. Only shared append points are docs/decisions/README.md (ADR index) and root Cargo.toml members (5A vs 1B) - trivial sequential appends, orchestrator-merged.

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
