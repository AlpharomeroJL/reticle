# Benchmark result records (suite v0.4.0, 75 tasks)

Raw per-task records and command transcripts from running two local models through the
whole 75-task `layout-tasks` suite over Ollama, on an RTX 4060 Ti (Windows 11).

| Model | Quantization | Overall |
|---|---|---:|
| `gpt-oss:16k` (20B) | MXFP4 | 52/75 (69%) |
| `qwen2.5-coder:16k` (14B) | Q4_K_M | 29/75 (39%) |

`gpt-oss-16k/` and `qwen2.5-coder-16k/` each hold, per task, a `<task>.result.json` (one
`ResultRecord`: task id, model, success, iterations, first and final violation counts, wall
time, backend, quantization) and a `<task>.transcript.jsonl` (the command transcript the run
produced). `suite.json` is the aggregate of that model's records.

The per-tier breakdown is in the main [README](../../README.md#the-agent-benchmark) and the
[benchmark chapter](../../docs/src/benchmarks.md). Local model outputs are not deterministic,
so regenerating a run shifts the counts slightly:

```
$env:RETICLE_MODEL_NAME = 'gpt-oss:16k'
just bench-agent-ollama --quantization MXFP4 --suite-version 0.4.0
```
