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
- [ ] Lane R1: windowed GPU surface, persistent scene, fps readout, 10M re-measure. in-progress @ lane/r1-windowed-surface
- [ ] Lane R2: compaction to indirect draw, MSAA, LOD switching, flags-vs-compacted bench. not-started
- [ ] Lane R3: canvas text labels, minimap, split viewports, rebindable keys, benches doc fix. not-started
- [ ] Lane R4: `stack` tech directive, 3D layer view, cut-line cross-section. in-progress @ lane/r4-3d-view
- [ ] Close-out: WASM cold-load and collab echo measured into PERF.md. not-started
- [ ] Close-out: DRC/route/collab GIFs plus minimap and 3D media. not-started
- [ ] Close-out: README refresh, skeptical STATUS update. not-started
- [ ] Release v4.0.0: git-cliff notes, binaries, Pages rebuild, tag. not-started

## Wave 0: contract freeze (serial)

- [ ] Crate skeletons: reticle-agent-api, reticle-mcp, reticle-agent, reticle-bench, reticle-demo. not-started
- [ ] Command and response enums, Session with revision, structured AgentError. not-started
- [ ] Transcript JSONL schema plus document_hash replay contract. not-started
- [ ] Intent spec types and open/short report types. not-started
- [ ] Pin/Label model types plus Edit variants; structured Violation upgrade workspace-wide. not-started
- [ ] Benchmark task/checker/manifest/results schemas (reticle-bench). not-started
- [ ] MCP tool JSON schemas and descriptions. not-started
- [ ] Agent status channel types over sync awareness. not-started
- [ ] Demo server API types and limit config. not-started
- [ ] tech/sky130.tech with citations; tech/sky130-drc-subset.toml. not-started
- [ ] Dependency pins; ADRs 0017+; frozen-surface manifest recorded here. not-started

## Wave 1: foundations (batch 1 then batch 2)

- [ ] Lane A: reticle-agent-api implementation, property tests, transcript logger/replayer. not-started
- [ ] Lane B: intent checker in reticle-extract, oracle tests both directions. not-started
- [ ] Lane C: pins and labels through model and io, GDS TEXT round-trip. not-started
- [ ] Lane D: SKY130 DRC subset over reticle-drc, coverage table, per-rule fixtures. not-started
- [ ] Lane E: sky130_fd_sc_hd cell import, corpus samples, DRC-clean gate. not-started
- [ ] Lane F: benchmark infrastructure, mock model, `just bench-agent`. not-started
- [ ] Lane G: demo server with enforced limits, abuse tests. not-started
- [ ] Lane H: 3D stack with true SKY130 thicknesses. not-started

## Wave 2: composition (three batches)

- [ ] Lane A: reticle-mcp server, every tool integration-tested over stdio. not-started
- [ ] Lane B: reticle-agent propose-verify-correct harness, mock-model loop tests. not-started
- [ ] Lane E: benchmark tiers 1-4, 50 tasks, two-way checker tests. not-started
- [ ] Lane F: benchmark tier 5 SKY130, 10 tasks, two-way checker tests. not-started
- [ ] Lane C: agent as CRDT collaborator, atomic transactions, presence, convergence tests. not-started
- [ ] Lane D: agent panel, live DRC overlay, replay theater, share link, WASM build. not-started
- [ ] Lane G: failure mining, candidates with provenance, `just bench-promote`. not-started
- [ ] Lane H: `just demo-up`, Dockerfile and VPS docs, Pages replay bundle, key-pattern release check. not-started
- [ ] Lane I: Playwright e2e suite, `just e2e`, WebGPU and WebGL2 runs. not-started

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
