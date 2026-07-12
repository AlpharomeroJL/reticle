# 0123, leaderboard rows are keyed per suite version, not just the provenance triple

## Context

ADR 0092 made the static leaderboard a pure function of the committed result records,
aggregated per `backend` / `model` / `quantization` triple. That keying predates the
frozen suite versions: it assumed a given model was measured against one suite. Once a
model has records from two suite versions (for example an earlier ad-hoc `claude-code`
run and a run against the frozen v0.7.0 suite), the triple keying folds both into one row
and sums their pass counts into a single denominator, and the `Suite` cell lists both
versions (`0.7.0, adhoc`).

That blends two different task universes into one number. The v0.7.0 suite is 95 tasks;
the earlier run measured a different, smaller set. A single `120/134 (90%)` figure over
the union is not a score against either suite, and the campaign's honesty rule is explicit:
every benchmark row carries its own suite version and its own denominator, and rows are
never compared across denominators. A row that internally blends two suite denominators
breaks that rule even though every underlying record is real.

## Decision

Add `suite_version` to the leaderboard's `GroupKey`, so a row is one
`backend` / `model` / `quantization` / `suite_version` combination. A model measured
against two suite versions now renders as two rows, each carrying a single suite version
and its own pass/total, sorted by the same float-free total order (pass rate, then task
count, then the provenance fields with suite version as the final tiebreak so the order
stays deterministic). Single-suite rows (every existing row) are unchanged; only a model
with records from more than one suite splits.

This amends, but does not reverse, ADR 0092: the leaderboard is still a pure, deterministic
function of the record set, submission is still "commit records and regenerate," and the
byte-stability test still pins the render. The render text and the `docs/src/submitting.md`
recipe now say "per triple and suite version."

## Consequences

- No row ever blends two suite denominators; the honesty rule holds by construction.
- The v8.2.0 leaderboard shows the real v0.7.0 `claude-code` run (48/53, all five tiers) as
  its own row, distinct from the earlier ad-hoc run (72/81), rather than a merged
  `0.7.0, adhoc` row.
- The determinism fixture set is single-suite per triple, so its golden page is unchanged;
  the change is exercised by the live record set.
- A future contributor who runs a new suite version gets a new row automatically, which is
  the intended behavior: suites are frozen and versioned, and their scores stay separable.
