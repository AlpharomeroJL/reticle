# 0030, lane worktrees are created by the orchestrator before spawning

## Context

The first Wave 1 fan-out did not physically isolate the lanes. Isolation was
left to an Agent-call parameter that did not take effect, so both lane subagents
and the orchestrator operated on the same working tree at `D:\dev\reticle`. The
subagents each ran `git switch -c` on the shared repository, so the checked-out
branch changed under the orchestrator, the two lanes' edits interleaved
uncommitted in one tree, and the workspace-wide pre-commit hook (`just lint` over
every crate) made each lane's commit fail on the other lane's unfinished files. A
lane building in one target directory would also have compiled the other lane's
half-written source. The work was disjoint by file (web, justfile, and scripts for
1A; reticle-agent and reticle-bench for 1B), so it was recoverable: each lane's
file set was committed to its own branch with the hook bypassed for the
separation commit only, and main was returned clean, with no lane commit on it and
no lane edit left in its tree. Nothing was reset or discarded.

## Decision

The orchestrator creates each lane's git worktree itself, before spawning, every
time: `git worktree add ..\reticle-lanes\<lane> -b lane/<lane> main`. The
subagent's first instruction pins it to that directory: the working directory is
`D:\dev\reticle-lanes\<lane>`, cd there immediately, and never edit, build, or run
git outside it; set `CARGO_TARGET_DIR=D:\dev\reticle-target-<lane>` so the lanes do
not fight over a build lock. The main tree at `D:\dev\reticle` is reserved for the
orchestrator and integration merges only. Isolation is never delegated to an
Agent-call parameter again; a real worktree on disk is the only mechanism used.

## Consequences

Lanes are isolated on disk, so concurrent edits cannot collide and the checked-out
branch of the main tree cannot move under the orchestrator; the shared-tree
incident cannot recur. Each lane runs its full gate inside its own worktree before
handback. The cost is one worktree and one target directory per lane, cleaned up
at the wave merge. This ADR supersedes the implicit isolation assumption in
[ADR 0028](0028-v6-subagent-worktree-orchestration.md); 0028's division of labor
(thin integration agent, merges at wave boundaries, orchestrator owns deploy and
benchmark runs) stands.
