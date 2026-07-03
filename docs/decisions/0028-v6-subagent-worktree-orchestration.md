# 0028, v6.0.0 run: subagent worktree lanes, a thin integration agent

## Context

The v6.0.0 packet mandates that the run fan out: the main session is the
orchestrator and integration agent only, and every feature lane runs as a
subagent in its own git worktree, up to eight concurrent within a wave. The host
is a single Windows machine with one GPU, so eight concurrent full builds of a
wgpu-heavy workspace would thrash a shared target directory. Some steps are also
outward-facing or hard to reverse (the GitHub Pages deploy, the tagged release)
or need centralized, honestly-labeled execution (the local model benchmark runs
that must never be conflated across backends).

## Decision

Lanes run as background subagents in isolated git worktrees, each committing to a
branch named `lane/v6-<id>` and each setting `CARGO_TARGET_DIR` to a per-lane
path outside the worktree (for example `D:\dev\reticle-target-v6-1a`), so builds
do not share a target directory and a re-run keeps its own cache. A lane gate is
crate-scoped (`cargo fmt`, `cargo clippy -p <crate> -- -D warnings`,
`cargo nextest run -p <crate>`, `just check-style`); the full `just ci` runs only
on main at wave merges. The main session writes minimal feature code. It merges
each lane branch at the wave boundary, reconciles deltas, re-runs `just ci` on
main, updates `docs/TASKS.md` (single writer), and performs the outward-facing
steps itself: the Pages build-and-deploy, the live-URL verification, the local
model benchmark runs, and the release. Contract-surface amendments happen only at
a wave boundary, by the integration agent, with an ADR.

## Consequences

Lanes within a wave progress in parallel with isolated file sets, so a base-path
fix and an Ollama backend do not collide. The cost is a cold build per worktree,
mitigated by the external per-lane target directories. Centralizing the deploy and
the benchmark runs keeps the one irreversible surface (the live site) and the one
honesty-critical surface (results labeled by backend, model, and quantization) in
the hands of the single integration agent, matching the packet's rule that a wave
executed serially in the main session is a defect to correct.
