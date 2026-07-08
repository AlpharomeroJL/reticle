# 0094, v8.1 interface-packet run structure: two fan-outs, two gates, deployable-every-gate

## Context

The v8.1.0 packet redesigns the interface across roughly 15 lanes of work. The
v8 run proved the lane machinery (worktrees, briefs, UUID dispatch, RESULT.md,
placeholder ADR ids assigned at merge) but paid four serial gate barriers of
merge, recapture, and deploy time between its middle waves. The v8.1 operator
directive optimizes wall-clock time explicitly: cut serial time only, changing
neither scope, nor the catalog contract, nor the bundle budget, nor honesty
rules, nor lane-level verification.

## Decision

- Two fan-outs instead of four: Wave 1 (five lanes: theme, type/icons,
  components, visual suite, command registry) and Wave 2 (ten lanes: the
  entire IA, workflow, and polish surface, prior ids 2A-4C kept for catalog
  traceability). Two integration gates, one after each fan-out.
- Cross-lane conflict control moves from gate barriers into contracts written
  at Wave 0: an app.rs region-ownership map, shared append-only surfaces with
  marked sections, and a reserved-CommandId table (ia-inventory.md) that Gate 2
  verifies against the merged registry.
- Deploy policy becomes deployable-every-gate: every gate proves wasm-build
  plus local e2e; the full deploy ceremony (deploy-pages, gh-pages publish,
  propagation-poll of the specific new bundle hash, smoke-pages) runs after the
  Wave 2 mega-merge, at release, and as insurance before stopping whenever
  merged work sits undeployed on main.
- GPU-bound suites (ui_snapshots, frame_guard, captures) are orchestrator-only
  and serialized; lane-scoped nextest runs exclude them, keeping ten concurrent
  worktrees off the single GPU.
- Wave 5 splits into a GPU-serialized capture track (v8.0.0-worktree before
  gallery, after gallery, media refresh) beside concurrent CPU documentation
  lanes; the before-state is captured from the v8.0.0 tag because a git commit
  does not expire.

## Consequences

- Serial time drops by two full gate cycles and the docking-spike half day
  (see ADR 0096); the ten-lane fan-out runs at the v8-proven 9-11 concurrency.
- The longer no-gate distance is carried by fully self-contained briefs, a
  mid-lane checkpoint in the heavyweight canvas lane, and seam canaries at the
  gates; merge conflicts concentrate at Gate 2 and are mechanical (region map,
  append-only sections) rather than semantic (effects funnel through one
  dispatch path).
- "Ship always" now means always shippable and shipped at the two outward
  points plus insurance deploys, recorded in RUN_STATE so the claim stays
  honest.
