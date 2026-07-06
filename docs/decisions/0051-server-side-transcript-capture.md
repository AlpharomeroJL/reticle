# 0051, server-side transcript capture in reticle-mcp

## Context

Every Reticle `Session` already builds a command transcript internally (one
`CommandRecord` per applied command, with the outcome and the surrounding
revisions), and the `reticle-agent` harness persists that transcript so a run can be
replay-verified and failure-mined. But a client the harness does not control, a raw
MCP client such as Claude Code driving the `reticle-mcp` server directly (Wave 3A),
never asks for the transcript to be written out, so its session's transcript is built
and then lost when the process ends. That leaves those runs unreplayable and
unmineable, and it is the same gap that local runs have.

## Decision

Give the `reticle-mcp` server an optional transcript sink. `Server::new` captures
nothing (the default, existing behavior and tests unchanged); `Server::with_transcript`
takes a `Box<dyn Write>` and, after every request, streams any command records the
session added since the last flush to that writer as JSONL, one `CommandRecord` per
line, flushed each time so a crash leaves a valid partial transcript. Capture happens
at the single dispatch chokepoint (`handle_line`), so it records commands from every
path (a command tool, a generator tool, and a session-applying context tool) and,
because `Session::apply` records failures too, it captures failed commands with a
failure `Outcome`. The `reticle-mcp` binary turns it on when `RETICLE_MCP_TRANSCRIPT`
names a file (created, then appended).

## Consequences

Any MCP client now leaves a replay-verifiable, mineable transcript without changing
the client, which is what Wave 3A's Claude-Code-driven runs need, and it closes the
local-run mining gap. The sink streams the session's own records rather than
rebuilding them, so the captured JSONL replays to exactly the document a direct
application produces (a test asserts this, including the two failure cases). The
capture is opt-in and off by default, so nothing changes for callers that do not set
the env var. A future change that wanted a whole-`Transcript` object (with
`final_hash`) rather than a raw record stream would add a session-end flush; the
per-line stream was chosen for crash-safety and because the final hash is derivable by
replay.
