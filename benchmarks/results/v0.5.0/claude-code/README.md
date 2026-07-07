# Claude Code agent-system run (v0.5.0 suite), partial

Backend `claude-code`, model `claude-sonnet-5`, measured on this host on 2026-07-07 with
`just bench-agent-claude-code` (`RETICLE_CLAUDE_BIN` = the resolved `claude.cmd`,
`RETICLE_MCP_BIN` = a current `reticle-mcp`).

This is a **partial** run. Of the 83 tasks, 25 ran (tiers 1 through 3) and **24 passed**
(one failed); the rest were recorded as honest not-runs (a `401`) or never reached, because
the operator's Claude subscription rate-limited the back-to-back agentic sessions and the
run was stopped before tiers 4 and 5. The per-task `ResultRecord` files here are the 25 that
ran; each carries `"backend": "claude-code"` and the model. There is no aggregate score over
83 tasks, and none is claimed: the denominator is the 25 tasks that ran, not 83, so this row
is not head-to-head comparable with the full-suite local-model rows.

The run is real proof that the agent-system backend drives the `reticle-mcp` tools end to
end: a `claude-sonnet-5` session sets the technology, creates cells, draws DRC-clean
geometry, and the applied commands replay through the checker. See the benchmark chapter
(`docs/src/benchmarks.md`) for the four backend fixes that made it work and for how to
complete the run when the rate window is clear.
