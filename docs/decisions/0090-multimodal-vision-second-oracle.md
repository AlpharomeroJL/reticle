# 0090, a multimodal vision model as a second, best-effort oracle for the agent benchmark

> Placeholder ADR number (assigned at lane start on `lane/v8-6b-vision`). If it collides
> with another wave-6 lane's ADR at the gate, renumber this file and its cross-references,
> exactly as the wave-5 5b/5c collision was resolved.

## Context

The agent benchmark grades a task with an **authoritative** oracle: the SKY130 DRC subset
plus the task's `Checker` (rect-present, intent connectivity, extraction, ...). Every
checker is two-way tested (it accepts the intended solution and rejects a perturbed one),
so a score cannot be won by luck (see the [benchmark methodology](../src/benchmark.md)).
Those oracles all read the same modality: the geometry of the document.

The question this lane explores is whether a *second oracle of a different modality* adds
signal. Reticle already renders a document region to a PNG headlessly (the
`render_png` / `RenderPng` path used by the run writer). A vision-capable language model
can look at that render and answer a yes/no question about it. Two oracles that reach a
verdict by unrelated means, geometry versus pixels, agreeing on the same faithful/corrupt
distinction is stronger evidence than either alone; a disagreement is a signal worth
surfacing. The constraint is that a vision model is heavy and may not be installed or may
be VRAM-bound on a given host, so it must never become a hard dependency of the gate.

The open questions were: which model (VRAM budget), what role the second oracle plays
relative to the authoritative checker, how the render reaches the model, and what happens
when the model cannot run.

## Decision

**A second oracle, never the authority.** `reticle_agent::vision_oracle` renders a task's
layout through the existing `RenderPng` path and asks a local vision model a yes/no
question, reported *beside* the authoritative checker as an agreement rate. The graded
pass/fail of a task remains the deterministic checker's; the vision verdict and the
agreement rate are provenance, not the verdict of record. This mirrors the container-oracle
role established by ADR 0054 (the Tiny Tapeout precheck) and ADR 0088 (the LEF/DEF import
oracle): an independent second reading that corroborates, never one that overrides.

**Model choice and VRAM budget: `llava:7b` on Ollama.** The default vision model is
`llava:7b` (about 4.7 GB resident), pulled with `ollama pull llava:7b`. It fits
comfortably in the lane's 16 GB card, well under the ~8 GB resident budget the packet set,
leaving room for the wgpu render to share the GPU. The model id is overridable with
`RETICLE_VISION_MODEL` (for example `qwen2.5vl:7b` or `moondream`). The request goes to
Ollama's native generate endpoint (`{base}/api/generate`, base overridable with
`RETICLE_VISION_BASE_URL`) as a non-streaming body carrying the base64 PNG in `images`,
with `temperature: 0` so a given render yields a stable verdict rather than drifting at
Ollama's default sampling temperature.

**Honest not-run, never an error.** The oracle probes availability the same cheap way the
container oracles do: the `ollama` CLI must be on the path and the model must already be
pulled (`ollama list`). When the model is absent, when the host has no GPU adapter to
render, or when any transport or parse step fails, the oracle returns
`VisionOutcome::Skipped(reason)`, a printable not-run, never an `Err` and never a panic.
The adapter-gated integration test runs when a model is present and skips honestly with a
printed reason otherwise. Numbers are only ever reported when the oracle actually ran;
nothing fabricates a verdict or an agreement rate.

**A coarse, geometry-anchored question.** The prompt asks a binary "does this render
contain drawn geometry (filled colored shapes), or is it an empty/blank layout?" with the
task intent appended as a trailing hint. This phrasing is deliberate: a 7B `llava`
reliably answers the present-versus-blank question but flips to a spurious "no" when the
same question is framed as "does it *match* the intent", and it *hallucinates* geometry
into a blank image when forced to justify its answer. So the oracle is scoped as a coarse
"non-empty layout consistent with the intent" second opinion, not a fine intent-conformance
judge. That is the honest ceiling of a small local vision model, and it is enough to
corroborate a faithful layout against an empty/corrupt one.

## Consequences

- The benchmark gains a second, independent oracle whose agreement with the authoritative
  checker is a measurable, reported number, without the gate ever depending on a vision
  model being present.
- On the development host the live oracle ran (`llava:7b`) and agreed with the
  authoritative `RectPresent` checker on both fixtures of a faithful-versus-empty pair
  (100% over that pair). This is a small, hand-built fixture pair, not a suite-wide
  measurement; the agreement number is a demonstration of the mechanism, not a headline
  benchmark result.
- The second oracle's discrimination is coarse (present-versus-blank), so it will not catch
  a subtle geometric error a faithful-looking render still contains; the authoritative
  checker remains the only thing that can. The vision oracle's value is corroboration and
  the disagreement signal, not independent grading.
- The renderer is used strictly as-is through `RenderPng`; no GPU-pipeline internals were
  touched. The model outputs are non-deterministic in principle (a different host, model,
  or Ollama version can answer differently), so the agreement rate is reproducible only for
  a fixed render, model, and pinned temperature.
