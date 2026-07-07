# Lane v8-7a-leaderboard: RESULT

Status: **GREEN.** A static, deterministic leaderboard generated from the committed
benchmark result records, plus a documented submission harness and a record-schema
validator. All additive: the runner, the task TOML format, and the `ResultRecord` field
set are untouched.

## What shipped

- **`crates/reticle-bench/src/leaderboard.rs`** (new): reads the committed
  `*.result.json` records under `benchmarks/results/`, aggregates per
  `backend` / `model` / `quantization` triple with a per-tier breakdown, and renders a
  byte-stable Markdown page. The tier is derived from the `t<N>_` task-id prefix, so the
  leaderboard depends only on committed records (never the frozen task TOMLs). A
  `validate_records` gate rejects a malformed submission with a clear message.
- **`crates/reticle-bench/src/main.rs`** (additive subcommands): `leaderboard` (reads
  records, writes `docs/src/leaderboard.md`; `--out -` prints to stdout) and
  `validate-records` (validates a file or directory; a malformed record exits 1, an
  unreadable path exits 2, a valid set exits 0). The `run` and `promote` paths are
  unchanged.
- **`docs/src/leaderboard.md`** (generated): the leaderboard chapter, wired into
  `SUMMARY.md`. Honest labeling preserved: a **Kind** column (bare model vs agent system
  vs multi-agent), a **Quantization** column, and a **PARTIAL** marker for a row that does
  not span all five tiers.
- **`docs/src/submitting.md`** (new): the submission harness. Exactly how an external
  contributor runs the suite (`just bench-agent-ollama` etc.), where records land, the
  record schema, how to validate before submitting, and how to PR a row. Wired into
  `SUMMARY.md`.
- **`docs/decisions/0068-static-leaderboard-record-format-as-api.md`** (new ADR) plus its
  row in `docs/decisions/README.md`.

## Determinism

The render is a pure function of the record set: rows sort by a float-free total order
(pass rate in permille, then task count, then the provenance triple), percentages are
integer-rounded, and nothing time-varying is emitted. Same records produce the same bytes.

Pinned by **`crates/reticle-bench/tests/leaderboard_deterministic.rs`** over a fixed
fixture record set (`tests/fixtures/leaderboard/records/`, independent of the live results
tree the bench workers write to):

- `render_matches_the_committed_golden_page` (byte-for-byte vs `golden.md`),
- `render_is_byte_stable_and_order_independent` (reverse and rotate the input; identical
  bytes),
- `render_carries_no_timestamp`,
- `a_valid_record_set_builds` and two rejection tests (a task id with no tier prefix; a
  file that is not a record array), the two-way submission check.

## Gate (all green)

- `cargo nextest run -p reticle-bench` -> 136 passed, 0 failed.
- `cargo clippy -p reticle-bench --all-targets -- -D warnings` -> clean.
- `cargo doc -p reticle-bench --no-deps` -> clean (0 warnings).
- Deterministic leaderboard build (golden byte-stability test) -> passes.
- `powershell -File scripts/check-style.ps1` -> OK (no em-dashes).

## Submission harness doc

`docs/src/submitting.md` (also linked from the leaderboard page and `SUMMARY.md`).
Validate a submission with:

```
cargo run -p reticle-bench -- validate-records <file-or-dir>
```

## Honest gaps

- The page renders **the records committed at generation time** (currently 185 records:
  `gpt-oss:16k`/MXFP4 52/75, `qwen2.5-coder:16k`/Q4_K_M 29/75, and a PARTIAL Claude Code
  `claude-sonnet-5` row 32/35 over tiers 1 through 3). The row set and denominators grow as
  the bench workers commit more records; a regeneration
  (`cargo run -p reticle-bench -- leaderboard`) refreshes the page. The page therefore
  can lag the newest committed records until it is regenerated.
- The committed record counts (75 per local model) differ from the 83-task narrative in
  `benchmarks.md`/`benchmark.md`; the leaderboard reports exactly what is committed, not
  the prose figures, which is the point of generating it from records.
- **PARTIAL** is defined from the records alone (a missing tier), not from the suite
  manifest, to keep the page self-contained; a run that covers all five tiers but omits
  individual tasks within a tier is not flagged. This is documented on the page and in
  ADR 0068.

## Commits (branch `lane/v8-7a-leaderboard`, not pushed)

- `ff86ad8` feat(bench): deterministic leaderboard generator and validate-records subcommand
- `2f13879` test(bench): golden byte-stability test and fixture record set
- `7512406` docs(bench): leaderboard chapter, submission harness, and ADR 0068
