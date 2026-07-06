# 0052, Claude Code as an agent-system backend, driven non-interactively

## Context

The existing benchmark backends (`anthropic`, `ollama`) are
[`ModelClient`](../../crates/reticle-bench/src/model.rs)s: the `reticle-agent` harness
owns the propose-verify-correct loop and asks the model for a batch of commands each
iteration. Claude Code is different in kind. It is an agent system: it brings its own
reasoning loop, its own tool-calling scaffold, and its own iteration control. Wiring it in
as a `ModelClient` would fight that grain (the harness would try to own a loop the agent
already owns), and it would also mean the harness, not the agent, decides when to call
tools.

Wave 3A's goal is to run the same 83-task suite through Claude Code and record honest
results. Two things make that possible without a `ModelClient`. First, ADR 0051 gave
`reticle-mcp` a server-side transcript sink (`RETICLE_MCP_TRANSCRIPT`), so a client the
harness does not control still leaves a replay-verifiable transcript. Second, the frozen
`ResultRecord` already carries `backend`/`model` provenance (ADR 0029), so a Claude-Code
row needs no schema change. The open questions are how to drive the CLI deterministically
enough to test, and how to keep an absent or unauthenticated CLI from fabricating a
pass or a fail.

## Decision

Add a `claude-code` backend to `reticle-agent` (a new `--backend` value and a
[`claude_code`](../../crates/reticle-agent/src/claude_code.rs) driver module) that treats
Claude Code as an external agent system rather than a `ModelClient`. Per task it:

1. Writes an MCP config JSON (`<id>.mcp.json`) of the exact `{ "mcpServers": { "reticle":
   { "command", "args", "env" } } }` shape Claude Code's `--mcp-config` accepts, launching
   the `reticle-mcp` binary with `RETICLE_MCP_TRANSCRIPT=<id>.transcript.jsonl` (capture on)
   and `RETICLE_MCP_BUDGET=<n>` (the `--command-budget`). The stale transcript is removed
   first, since the sink appends.
2. Runs one `claude` session non-interactively:
   `claude -p "<prompt>" --mcp-config <cfg> --strict-mcp-config --model <model>
   --output-format stream-json --verbose --permission-mode bypassPermissions
   --allowed-tools <namespaced reticle-mcp tool names>`. The flags are the verified
   `claude` v2.1 surface. `--strict-mcp-config` means only the reticle server is reachable,
   and the allowlist names are `mcp__reticle__<tool>` derived from `reticle_mcp::tool_names`
   (a new public function, so the allowlist cannot drift from what the server serves). The
   prompt carries a preamble that asks the agent to `set_technology` with the task's
   technology, because the harness does not own the session and cannot pre-install it.
3. After the session, reconstructs the document by replaying the captured transcript (each
   JSONL line's `command`, the same replay contract as `reticle_agent_api::replay`), runs
   the task's checker from the same `CheckerRegistry` the other backends use, and records a
   `ResultRecord` with `backend = "claude-code"` and `model = <the id the stream reports>`.
4. If the CLI cannot be launched (absent, or a spawn error), or the session did not
   authenticate or was out of quota (detected from the stdout/stderr stream), records an
   honest not-run: a distinct `NotRunRecord` written to `<id>.notrun.json`, never a
   `ResultRecord`, so it can never be counted as a pass or a fail.

The subprocess-spawn step is a `ClaudeRunner` seam and the `claude`/`reticle-mcp` paths are
overridable (`RETICLE_CLAUDE_BIN`, `RETICLE_MCP_BIN`), so a deterministic test injects a
fake runner that itself reads the generated `--mcp-config`, spawns the `reticle-mcp` command
it names with that config's env, and drives a couple of scripted commands over stdio. That
test proves the config is correct and the captured transcript replays into a checked
`ResultRecord` labeled `claude-code`, all without the real, non-deterministic CLI. Another
fake proves the CLI-absent path is an honest not-run. The real `claude -p` smoke and the
full 83-task suite are the orchestrator's step, not a unit test, because they consume quota
and are non-deterministic.

## Consequences

Claude Code runs the suite as itself, driving the reticle tools through its own loop, and
every run leaves the same replay-verifiable, checker-evaluated artifacts the other backends
do, plus its MCP config and the server-captured transcript. Because the driver is not a
`ModelClient`, the harness never fights the agent for control of the loop, and `run_single`
/ `run_suite` route the backend to its own driver before any `ModelClient` is built. The
one field the transcript cannot express is a per-proposal boundary (Claude Code owns its
iterations), so a Claude-Code record reports the applied-command count for `iterations` and
the final DRC count for both violation fields rather than inventing intermediate numbers;
the honest end state is recorded, not a fabricated trajectory. Honesty is enforced
structurally: a not-run is a different artifact type from a result, so a broken or
unauthenticated environment produces an auditable not-run list and is never folded into a
pass rate. Adding `reticle_mcp::tool_names` keeps the allowlist single-sourced. The suite
driver writes ran records to `suite-claude-code.json` and, when any task did not run, a
separate `suite-claude-code-notrun.json` plus a printed tally, so a partial run is never
mistaken for a clean sweep.
