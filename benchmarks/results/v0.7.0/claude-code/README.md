# Claude Code agent-system run (v0.7.0 suite)

Backend `claude-code`, model `claude-sonnet-5`, measured on this host on 2026-07-12 with the
suite form of the runner:

```
RETICLE_CLAUDE_BIN=<resolved claude.cmd> RETICLE_MCP_BIN=<current reticle-mcp> \
  cargo run -p reticle-agent -- --backend claude-code --suite benchmarks/layout-tasks --model sonnet
```

Of the 95 tasks in the v0.7.0 suite, **53 ran and 48 passed** (5 failed); the remaining 42
were recorded as honest not-runs (a `401`) because the operator's Claude subscription
rate-limited the back-to-back agentic sessions. The 53 that ran span **all five tiers**
(tier 1: 5, tier 2: 8, tier 3: 28, tier 4: 7, tier 5: 5), so the row is a full-range run, not
a tiers-1-through-3 partial. The denominator is the **53 tasks that ran**, not 95: there is
no aggregate score over 95 and none is claimed, and this row is not head-to-head comparable
with a row that carries a different denominator or suite version.

The `*.notrun.json` artifacts for the 42 rate-limited tasks were left in `scratch/agent-runs/`
(they are never aggregated into the leaderboard); the raw per-task transcripts
(`scratch/agent-runs/*.transcript.jsonl`) are the evidence that each recorded task really
drove the `reticle-mcp` tools end to end.

## Suite-version label

The tasks are the committed v0.7.0 suite (`benchmarks/layout-tasks/manifest.toml`,
`version = "0.7.0"`). The runner's `--suite-version` flag defaults to `adhoc` and was not
passed on the command line, so the runner emitted the records with `"suite_version": "adhoc"`.
That flag is pure metadata: it is copied straight into the record and has no effect on task
loading, the model session, or the checker, so a run with `--suite-version 0.7.0` over the
same suite produces byte-identical task results and differs only in this one string. The
label was corrected to `0.7.0`, the suite's own manifest version, so the record's provenance
matches the suite it came from; the `success`, `iterations`, and violation counts are the
run's own and were not touched. The corrected records validate with
`cargo run -p reticle-bench -- validate-records benchmarks/results/v0.7.0/claude-code`.

The run is real proof that the agent-system backend drives the `reticle-mcp` tools end to
end: a `claude-sonnet-5` session sets the technology, creates cells, draws DRC-clean
geometry, and the applied commands replay through the task's two-way-tested checker. See the
benchmark chapter (`docs/src/benchmarks.md`) for the four backend fixes that make it work and
for how to complete the run when the rate window is clear.
