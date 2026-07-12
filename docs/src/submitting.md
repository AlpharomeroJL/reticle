# Submitting a run

The [leaderboard](leaderboard.md) is built from committed result records, and the record
format is the whole API. There is no server, no account, and no upload endpoint: you run
the suite, you get JSON records, you open a pull request that adds them, and the next time
the page is generated your row appears. This chapter is the exact recipe.

## 1. Run the suite

The suite lives under `benchmarks/layout-tasks/` (a `manifest.toml` and one TOML per
task). Pick the backend that matches what you are measuring:

```
just bench-agent                     # the deterministic mock (machinery baseline, no key)
just bench-agent-ollama              # a local model over an Ollama endpoint
just bench-agent-claude-code         # Claude Code as an agent system (consumes your quota)
```

Each recipe takes the same scoping flags, so you can run a tier or a single task while you
iterate:

```
just bench-agent-ollama --tier 1
just bench-agent-ollama --task t1_place_met1_rect
```

The model and its provenance come from the environment and the backend. For a local model
over Ollama, set the model name and (where you know it) the quantization so your row is
labeled honestly:

```
$env:RETICLE_MODEL_NAME = 'gpt-oss:16k'
just bench-agent-ollama --quantization MXFP4
```

The mock is deterministic end to end and solves only the three sample tasks; it is a
**machinery baseline, not a model score**, and the leaderboard labels it as such. A real
score comes from a real model.

## 2. Find your records

A suite run writes an aggregate result file (a JSON array of records) under
`scratch/agent-suite-results/` by default (`suite.json` for a whole-suite run, or
`tier-<n>.json` / `task-<id>.json` for a scoped one). The Claude Code backend also writes
one artifact per task under `scratch/agent-runs/`, including a `<task_id>.result.json`
record and, separately, a `<task_id>.notrun.json` for any task that could not run at all.

A **not-run is never a result.** A task that the backend could not even start (the CLI
missing, the session unauthenticated, out of quota) is recorded as a distinct
`*.notrun.json` artifact and is never counted as a pass or a fail. Only `*.result.json`
records reach the leaderboard.

## 3. The record schema

Every record is a [`ResultRecord`](benchmarks.md) with these fields (the shape is frozen;
`backend` and `quantization` default so older JSON still parses):

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `task_id` | string | The task that ran. Must begin with a `t<N>_` tier prefix (for example `t1_place_met1_rect`); the leaderboard reads the tier from it. |
| `model` | string | The model identifier (or `mock`). |
| `suite_version` | string | The suite version the task came from. |
| `success` | bool | Whether the two-way-tested checker passed. |
| `iterations` | number | Propose-verify-correct iterations used. |
| `first_proposal_violations` | number | DRC violations in the first proposal. |
| `final_violations` | number | DRC violations in the final document. |
| `wall_ms` | number | Wall-clock time for the task, in milliseconds. |
| `backend` | string | The client kind: `mock`, `ollama`, `anthropic`, `claude-code`, ... |
| `quantization` | string or null | The model's quantization when the backend reports one (for example `Q4_K_M`), else null. |

A file is a JSON array of these records, exactly as the runner writes it.

## 4. Validate before you submit

Reject a malformed record before it ever reaches a reviewer. The validator reuses the same
`ResultRecord` shape the runner writes, so a record that validates is a record the
leaderboard can render:

```
cargo run -p reticle-bench -- validate-records scratch/agent-suite-results/suite.json
```

It accepts a single file or a whole directory. A valid set prints a count and exits 0; a
malformed record is rejected with a message naming the file, the record index, and the
reason (an empty `task_id`, a `task_id` with no tier prefix, an empty `model` or
`suite_version`, or a file that is not a JSON array of records), and exits non-zero.

## 5. Open the pull request

Commit your records under `benchmarks/results/`, in a directory that names your run, with a
file name ending in `.result.json` (the extension the leaderboard aggregates; a
`*.notrun.json` is deliberately not aggregated):

```
benchmarks/results/<your-label>/<model-or-run>.result.json
```

Then regenerate the page so your row is included, and commit the regenerated page too:

```
cargo run -p reticle-bench -- leaderboard
```

Open the pull request with both the records and the regenerated `docs/src/leaderboard.md`.
Your row is aggregated per `backend` / `model` / `quantization` triple and `suite_version`
(so a run against a new suite version is always its own row, never blended into an older
one), labeled by
[kind](leaderboard.md#how-to-read-a-row) (a bare model, an agent system, or a multi-agent
system), and marked **PARTIAL** if it does not span all five tiers. A bare-model row and an
agent-system row are not comparable head to head; both are welcome, and the labeling keeps
them honest.
