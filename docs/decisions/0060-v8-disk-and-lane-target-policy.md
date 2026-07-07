# 0060, v8 run disk policy: lane target dirs move to E:, D: keeps the shared main target

## Context

The v8 packet dispatches up to four concurrent lane worktrees, each with its own
`CARGO_TARGET_DIR` (ADR 0030), and its own thresholds say D: below 40 GB free forces
two-lane concurrency. At run start D: had 36.1 GB free: the stale
`D:\dev\reticle-target-v7-share-live` (16.0 GB, a leftover from the v7 lane whose
worktree was already removed) plus the shared `D:\dev\reticle-target` (114.8 GB)
consumed the drive, while E: had 780 GB free. A cold `cargo build -p reticle-geometry`
probe measured E: at 16.2 s vs D: at 13.2 s (1.23x), well inside the 2x acceptability
bar set in the run plan.

## Decision

Delete the stale v7 lane target (done; D: back to 51.5 GB free). For the v8 run,
per-lane target dirs live at `E:\dev\reticle-target-<lane>` and are deleted at
`lane-done`. The shared `D:\dev\reticle-target` stays on D: for main-checkout builds
(the warm cache keeps `just ci` fast at merge gates). Disk is remeasured before every
wave; below 25 GB free on D: the recovery lever is `cargo clean` of the shared target,
and lane concurrency drops per the packet thresholds. Six-lane mode stays disabled for
this run.

## Consequences

Four concurrent lanes cost D: nothing beyond their worktree checkouts; the 1.23x
build-time penalty applies only inside lanes, not to the merge-gate `just ci`. The
`just lane` recipe's documented D: convention is overridden for this run by the
orchestrator exporting `CARGO_TARGET_DIR` at dispatch, so no recipe change is needed;
a future run on a roomier D: reverts by simply not overriding. Docker image storage is
handled separately (ADR 0062).
