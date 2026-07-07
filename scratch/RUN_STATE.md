# v8.0.0 run state (single writer: orchestrator)

updated: 2026-07-07T14:40:00-05:00
phase: wave0/fuzz-campaign-running (parallel relaunch)

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
    0.8 gate+redeploy: pending (after gds re-confirm clean + corpus commit + STATUS fuzz update)
- wave1: briefs WRITTEN (scratch/lanes/v8-1{a-transport,b-relay,c-shareux,d-agentlive,e-proof}/brief.md); dispatch plan:
    batch 1 (4 concurrent): v8-1a-transport, v8-1b-relay, v8-1c-shareux, v8-1d-agentlive
    batch 2 (staggered, after 1c merges; gpu_lane runs alone): v8-1e-proof
    declared merge order: 1a, 1b, 1d, 1c (app-UI-heaviest last), then 1e
    file-ownership fences are in the briefs (livesync.rs=1a; share.rs/webopen.rs=1c; worker+conformance=1b; agent=1d; e2e+README media=1e)
- wave2..wave9: pending

## not-run ledger (honest)

(empty so far)

## resume (exact next action if this session dies)

Read this file top to bottom; continue at the first pending item in the current phase. Wave 0 is orchestrator-direct (no lane sessions yet); the plan file section "Wave 0" lists each step's exact commands and fallbacks.
