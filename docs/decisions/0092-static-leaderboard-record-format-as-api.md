# 0092, A deterministic static leaderboard with the record format as its API

## Context

The v8 packet asks for a public leaderboard of the agent benchmark results, plus a
documented way for an outside contributor to run the suite and add their own row. The
constraint is that there is no server and no account budget: the Pages book is a static
bundle. The benchmark run already commits its outcomes as `ResultRecord` JSON under
`benchmarks/results/`, and that record shape is a frozen Wave-0 contract surface (its
fields are set by [ADR 0029](0029-result-record-backend-label.md) and may not change
here). A `ResultRecord` does not carry the tier it ran at, and the task TOMLs that do
carry the tier are owned by the runner and being written to concurrently by the bench
workers, so the leaderboard cannot depend on them.

## Decision

Generate the leaderboard **statically and deterministically from the committed records
alone**, and treat the record format as the entire submission API.

- A new additive `leaderboard` subcommand of `reticle-bench` walks
  `benchmarks/results/` for `*.result.json` files, aggregates the records per
  `backend` / `model` / `quantization` triple with a per-tier breakdown, and writes a
  Markdown book page. The runner, the task format, and the `ResultRecord` field set are
  untouched; the subcommand only reads records.
- The render is a **pure function of the record set**: rows are sorted by a float-free
  total order (pass rate, then task count, then the provenance triple), percentages are
  integer-rounded, and nothing time-varying is emitted. Same records produce the same
  bytes. A golden byte-stability test (`tests/leaderboard_deterministic.rs`) pins this
  over a fixed fixture record set that is independent of the live results tree.
- The **tier is derived from the task id's `t<N>_` prefix**, not from the task TOMLs, so
  the leaderboard stays self-contained and depends only on committed records.
- A row is marked **PARTIAL** when it has no result in one or more of the five tiers, a
  rule computed from the records alone, so a run that did not span the full difficulty
  range (for example the tiers 1 through 3 Claude Code run) is never published as a
  full-suite score.
- The honest labeling of [ADR 0029](0029-result-record-backend-label.md) and
  [ADR 0052](0052-claude-code-agent-backend.md) is preserved on the page: a **Kind**
  column separates a bare model (driven through Reticle's own loop) from an agent system
  (which brings its own loop), and a quantization column keeps a small local model
  distinct from a frontier one.
- The submission harness is documented in `docs/src/submitting.md`: run the suite, find
  the `*.result.json` records, validate them with a new additive `validate-records`
  subcommand, and open a pull request that commits the records plus the regenerated page.
  `validate-records` reuses the `ResultRecord` shape, so it accepts exactly what the
  leaderboard can render and rejects a malformed record (empty `task_id`, a `task_id`
  with no tier prefix, an empty `model` or `suite_version`, or a file that is not a JSON
  array) with a message naming the file, the record index, and the reason.

## Consequences

The leaderboard is a static page anyone can regenerate with one command and reproduce
byte-for-byte, so it can live in the Pages book with no server. Because the record format
is the API, submitting a row is just committing records and re-running the generator, and
the same validation the project uses on its own records is what a contributor runs before
opening a pull request. The page reflects exactly the records committed at generation time
and grows as the bench workers commit more, so a regeneration is the whole maintenance
cost. The tier-from-prefix and PARTIAL rules are conventions the validator enforces and the
ADR records, so a future record that breaks them fails loudly at submission rather than
rendering incorrectly and silently. No frozen surface changed: the leaderboard and validator are
additive read-only subcommands over the existing `ResultRecord`.
