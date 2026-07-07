# v8.0.0 run state (single writer: orchestrator)

updated: 2026-07-07T13:55:00-05:00
phase: wave0/fuzz-campaign-running
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
    0.7 fuzz campaign: RUNNING in background (WSL Ubuntu; fork=4, 3600 s per target; ~3.3 h; script scratch/fuzz-campaign.sh; logs $HOME/reticle-fuzz/<target>.log in WSL)
    0.8 gate+redeploy: pending (after 0.7)
- wave1..wave9: pending

## not-run ledger (honest)

(empty so far)

## resume (exact next action if this session dies)

Read this file top to bottom; continue at the first pending item in the current phase. Wave 0 is orchestrator-direct (no lane sessions yet); the plan file section "Wave 0" lists each step's exact commands and fallbacks.
