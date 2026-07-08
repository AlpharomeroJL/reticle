# Leaderboard

This page is generated deterministically from the committed benchmark result records under `benchmarks/results/`. It does not run the suite; it aggregates the `*.result.json` records the runs already wrote. Regenerate it with `cargo run -p reticle-bench -- leaderboard`. The record format is the API: to add a row, run the suite and open a pull request with your records (see [Submitting a run](submitting.md)).

It aggregates **185** committed result record(s) into **3** row(s), one per `backend` / `model` / `quantization` triple. The numbers are exactly what the committed records say and grow as more runs are committed.

## How to read a row

- **Kind** labels a row as a *bare model* (a model driven through Reticle's own propose-verify-correct loop), an *agent system* (a system that brings its own loop and scaffold, such as Claude Code), or a *multi-agent* system. A bare-model row and an agent-system row measure different things and are **not** comparable head to head (see the [methodology](benchmark.md)).
- **Quantization** is carried where the backend reports one (for example `Q4_K_M` on a local GGUF model), so a small quantized local model is never conflated with a full-precision or frontier one.
- **PARTIAL** marks a row that has no result in one or more tiers, so it did not span the full difficulty range and its denominator is not comparable to a full-tier row.
- Each **Tier** cell is `passed/total`, and **Overall** is `passed/total (rate)` over every committed record for that row.

## Rankings

| Kind | Model | Backend | Quantization | Suite | Tier 1 | Tier 2 | Tier 3 | Tier 4 | Tier 5 | Overall | |
| ---- | ----- | ------- | ------------ | ----- | -----: | -----: | -----: | -----: | -----: | ------: | -- |
| agent system | `claude-sonnet-5` | claude-code | - | adhoc | 8/9 | 10/11 | 14/15 | - | - | **32/35 (91%)** | PARTIAL |
| bare model | `gpt-oss:16k` | ollama | MXFP4 | 0.4.0 | 9/9 | 11/11 | 19/34 | 5/11 | 8/10 | **52/75 (69%)** |  |
| bare model | `qwen2.5-coder:16k` | ollama | Q4_K_M | 0.4.0 | 6/9 | 8/11 | 6/34 | 3/11 | 6/10 | **29/75 (39%)** |  |

The labeling rules above are the honest account preserved from the [benchmark methodology](benchmark.md): a machinery baseline, a local model, and an agent system are always distinguishable, and a partial run is never published as a full-suite score.
