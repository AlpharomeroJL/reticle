# 0029, ResultRecord gains a backend and quantization label

## Context

v6.0.0 adds a local (Ollama, OpenAI-compatible) benchmark backend alongside the
existing deterministic mock and the Anthropic backend. The packet is explicit that
the mock baseline, a local baseline, and any future frontier run must never be
conflated: a result row has to say which backend and which quantization produced
it. `ResultRecord` (in `reticle-bench`) is a frozen Wave-0 contract surface, so it
can only be amended by the integration agent at a wave boundary, with a record.

## Decision

Amend `ResultRecord` at the Wave 1 boundary to add `backend: String` (for example
`mock`, `ollama`, `anthropic`) and `quantization: Option<String>`, both with
`#[serde(default)]` so already-recorded result JSON keeps deserializing (an older
row reads back with an empty backend and no quantization). The runner stamps these
onto every row it writes, and the markdown summary gains a Backend column. The
existing `model` field continues to carry the model id.

## Consequences

Every new results table is labeled by backend and, for a local model, by
quantization, so a mock 3-of-63 machinery baseline can never be read as a model
score, and a `gpt-oss:16k` run is distinct from a `qwen2.5-coder:16k` run and from
any later frontier run. Historical result files remain readable through the serde
defaults. This is the only Wave 1 change to a frozen surface; it is made by the
integration agent per [ADR 0028](0028-v6-subagent-worktree-orchestration.md).
