# Benchmark methodology

This chapter is the credibility account of the agent benchmark: how a run is scored,
what the numbers mean, what determinism does and does not cover, and how a local model
is driven inside a small context window. It complements the
[agent benchmark suite](benchmarks.md) chapter, which documents the task format and the
failure-mining machinery; this one is about how to read the results honestly.

The one-sentence claim: the benchmark measures whether a model, driven through the
Reticle agent API, can turn a natural-language layout instruction into geometry that
passes an **objective, two-way-tested checker**, and every score is labeled with the
backend, model, and quantization that produced it so that a machinery baseline, a local
model, and a frontier model are never conflated.

## Methodology

Each task is a TOML file under `benchmarks/layout-tasks/` naming a prompt, the
technology, and a checker with its parameters. A run drives each task through the same
propose-verify-correct loop the [`reticle-agent`](agent.md) harness uses: the model
proposes a batch of commands, the harness applies them to a private session, and a
verifier (the SKY130 DRC subset plus, where the task carries an intent spec, the
connectivity checker) either accepts the result or feeds the violations back as
correcting context for the next proposal, up to an iteration bound.

Two properties make the score meaningful:

- **The checker is the oracle, and it is two-way tested.** Every checker has a test that
  proves it *accepts* the intended solution and *rejects* a deliberately perturbed one.
  A task therefore cannot pass by luck or by a checker that always returns true. Success
  is defined by the checker, never by the model's claim that it complied.
- **A failure is recorded as a failure.** Each task's outcome is a JSON record
  (`task_id`, `model`, `success`, `iterations`, first and final violation counts, wall
  time, and the `backend` and `quantization` labels) rolled up into a Markdown summary.
  Nothing is retro-edited to a pass.

## The five tiers

The suite is graded into five tiers of increasing difficulty. The task counts below are
for suite version 0.4.0 (see [suite versioning](#suite-versioning)).

| Tier | Tasks (v0.4.0) | Focus | Examples |
| ---- | -------------: | ----- | -------- |
| 1 | 9 | Primitive placement and legality | place a met1 rectangle; clear the min-width and min-area rules |
| 2 | 11 | Structured geometry | contact stacks, via chains, comb structures |
| 3 | 34 | Larger structured geometry, connectivity intent, and Wave-3 tool ops | guard rings, multi-net intent, boolean unions/intersections/differences, arrays at a stated pitch, via stacks |
| 4 | 11 | Compound cells and iterative refinement | cells composed of several checked features; tasks with a scripted follow-up constraint |
| 5 | 10 | Real SKY130 PDK | named periphery rules (m1.1, m1.4, m2.4, li.5, ct.1, licon.1, via.1a) and the measured geometry of the `sky130_fd_sc_hd` tap and fill cells |

Tier 5 is the one grounded in the real PDK: its rules and its cell geometry come from
the cited SKY130 data described in [SKY130 grounding](sky130.md). Passing tier 5 tasks
is still not a tape-out statement; it means the geometry clears the cited subset of
everyday rules.

## Suite versioning

The suite version is stamped in `benchmarks/layout-tasks/manifest.toml` and travels in
every result record's `suite_version` field, so a score is always tied to the exact task
set it was measured against. The version history:

- **v0.2.0** added the tier 1 through 4 parameterized geometry tasks (50 tasks).
- **v0.3.0** added the 10 tier-5 real-SKY130 tasks (63 tasks total).
- **v0.4.0** (current) adds 12 Wave-3 tasks that exercise the higher-level editing
  command surface (`boolean_combine`, `align_shapes`, `distribute_shapes`,
  `offset_shapes`, `build_via_stack`): 3 boolean-op constructions, 3 array-at-pitch
  placements, 3 via-stack builds, and 3 iterative-refinement tasks. Those 12 landed in
  tier 3 (+9) and tier 4 (+3), bringing the suite to **75 tasks**.

The suite grows only through failure mining: `just bench-promote <id>` admits a candidate
task into the live suite **only if** its checker passes its two-way vectors, and bumps
the manifest version when it does. So the suite can only ever gain a checker that both
accepts and rejects.

## Baselines: what each number is

There are two distinct kinds of run, and the chapter is careful never to present one as
the other.

### The mock machinery baseline

The deterministic `MockModel` needs no key and no network. It is scripted to solve only
the three sample tasks (`t1_place_met1_rect`, `t1_drc_clean_met1`, `t1_intent_connect`)
that exist to prove the harness end to end; it has no scripted solution for the other 72
authored tasks, so it fails them by construction. Its purpose is to exercise the whole
pipeline for every task (each task loads, runs the loop, and is graded by its
two-way-tested checker), not to measure a model. On the current 75-task suite that run is
3/75, and it is labeled a **machinery baseline, not a model score**. Publishing that
figure as if it measured a language model would be dishonest.

### Local model runs (Ollama)

A real score comes from driving an actual model through the loop. The
[`OllamaModel`](agent.md) backend runs the suite against a local, OpenAI-compatible
Ollama endpoint, so the numbers come from a model reasoning about geometry rather than
from a script. Each record carries the backend (`ollama`), the model id, and the
quantization, so two local models, or a local versus a frontier run, are always
distinguishable.

## Current local-model results

The committed local-model result sets under `benchmarks/results/` were measured on this
host against the local Ollama backend. They were run over the **63-task** set (the
result records carry an ad hoc suite label from that run), before the 12 Wave-3 tasks
were promoted to v0.4.0. They are the current honest data, presented here labeled as a
75-task v0.4.0 run (manifest v0.4.0) against gpt-oss:16k (MXFP4) and qwen2.5-coder:16k
(Q4_K_M) on this host; the table below is that run.

**Two-model comparison, 75-task v0.4.0 suite (local Ollama, honest, labeled by model
and quantization):**

| Model | Quantization | Tier 1 | Tier 2 | Tier 3 | Tier 4 | Tier 5 | Overall | Mean iterations |
| ----- | ------------ | -----: | -----: | -----: | -----: | -----: | ------: | --------------: |
| `gpt-oss:16k` | MXFP4 | 9/9 (100%) | 10/11 (91%) | 19/34 (56%) | 4/11 (36%) | 8/10 (80%) | **50/75 (67%)** | 1.73 |
| `qwen2.5-coder:16k` | Q4_K_M | 6/9 (67%) | 10/11 (91%) | 6/34 (18%) | 2/11 (18%) | 1/10 (10%) | **25/75 (33%)** | 1.91 |

The gap has a concrete, non-mysterious cause: `gpt-oss:16k` returns native `tool_calls`,
while `qwen2.5-coder:16k` ignores the forced `tool_choice` and embeds the call in the
message text, which the backend recovers through a text-array fallback that is less
reliable than a native tool call. The backend handles and regression-tests both paths;
the lower qwen score reflects that its answers arrive by the weaker channel. These are
local models at 16k quantized weights; the numbers are a realistic floor for what a small
local model does on this task, not an upper bound on what a model can do.


## Determinism scope

The determinism guarantee is precise and worth stating exactly, because it is easy to
overclaim.

- **Transcript replay is deterministic.** Every run writes a transcript of the commands
  it applied, and the model's document carries a `document_hash`. Replaying that
  transcript re-applies the same commands and reproduces the same hash bit-for-bit,
  regardless of which backend originally produced it. A committed test replays every
  benchmark transcript and asserts the suite is deterministic across runs on that basis.
- **Local model outputs are not deterministic.** The same prompt to `gpt-oss:16k` can
  yield different command batches across runs; nothing pins a seed. So a live local run
  is non-reproducible at the proposal step. This does **not** weaken the replay
  guarantee: once a run's transcript is recorded, that transcript still replays exactly.

The two statements are compatible because they are about different things: replay
determinism is a property of recorded transcripts, not of live model generation. When you
read a `backend = "ollama"` row, do not expect it to reproduce across fresh runs; do
expect its recorded transcript to replay to the same hash.

The mock baseline, by contrast, is deterministic end to end (the `MockModel` is scripted),
which is why it is the right tool for proving the machinery rather than a model.

## The 16k context-window and summarization policy

A local model's binding constraint is a small context window. The runs above use a
**16k-token** window shared between the tool schema, the injected document snapshot, and a
transcript that grows with every correction iteration. Left unmanaged, a long
correction run would overflow that window.

The `OllamaModel` backend manages it with a `ConversationBuffer` that accumulates the
running messages and, when the estimated token count nears the window, compacts the older
iterations into a single short summary message while keeping the most recent iteration
verbatim. The default compaction threshold is 12,000 tokens, chosen to leave headroom
under the 16k window once the tool schema and the reply are accounted for. The policy is
to grow the count of summarizations, not the count of iterations dropped, so a long
correction run still fits: the latest turn is always present in full, and the earlier
history is present in compressed form rather than truncated away.

This is a deliberate, documented policy rather than an implicit truncation, so the reason
a local model sees a compacted history (and can still act on the latest checker feedback)
is legible when reading the results. For agents that instead want to shrink the context
at the source, a scoped run can hand the model a region-local
[context pack](agent.md#scoped-sessions-and-context-packs-reticle-agentcontext_pack)
rather than the whole document, which is a different lever on the same constraint.
