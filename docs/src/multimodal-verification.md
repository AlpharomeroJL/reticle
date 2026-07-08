# Multimodal verification

The [benchmark methodology](benchmark.md) chapter makes one thing central: the checker is
the oracle, and it is two-way tested. That oracle reads a single modality, the geometry of
the document. This subsection describes a **second oracle of a different modality**: render
the layout to an image and ask a local vision model whether the render shows what the task
intended. It sits *beside* the authoritative checker and never replaces it.

## Why a second modality

Two oracles that reach a verdict by unrelated means, geometry versus pixels, agreeing on
the same faithful-versus-corrupt distinction is stronger evidence than either alone, and a
disagreement is a signal worth surfacing rather than a failure. Reticle already renders a
document region to a PNG headlessly (the `render_png` / `RenderPng` path the run writer
uses for its per-run artifact). The vision oracle reuses that path unchanged, base64-encodes
the PNG, and posts it with a yes/no question to a local vision model over Ollama's native
`{base}/api/generate` endpoint.

The authoritative pass/fail is still the deterministic checker's. The vision verdict and the
**agreement rate** it produces are provenance reported alongside the score, not the verdict
of record. This is the same second-reading role the
[Tiny Tapeout precheck](tapeout.md) and the LEF/DEF import oracle play: an independent
corroboration, never an override.

## Honest not-run

A vision model is heavy and may not be installed, or may be VRAM-bound on a given host, so
it is never a hard dependency of the gate. The oracle probes availability the same cheap way
the container oracles do: the `ollama` CLI must be on the path and the model must already be
pulled. When the model is absent, when the host has no GPU adapter to render, or when any
transport or parse step fails, the oracle returns a printable *skip* reason, never an error
and never a panic. The adapter-gated test runs when a model is present and skips honestly
otherwise, printing why. Numbers are only ever reported when the oracle actually ran;
nothing here fabricates a verdict.

## Model and VRAM budget

The default model is `llava:7b` (about 4.7 GB resident), pulled with
`ollama pull llava:7b`. It fits comfortably in a 16 GB card, well under the ~8 GB resident
budget, leaving room for the wgpu render to share the GPU. It is overridable with
`RETICLE_VISION_MODEL` (for example `qwen2.5vl:7b` or `moondream`), and the endpoint with
`RETICLE_VISION_BASE_URL`. The request pins `temperature: 0` so a given render yields a
stable verdict rather than drifting at Ollama's default sampling temperature.

## What the second oracle can and cannot judge

The prompt asks a binary question, *does this render contain drawn geometry (filled colored
shapes) or is it an empty/blank layout?*, with the task intent appended as a trailing hint.
This phrasing is deliberate. A 7B `llava` reliably answers the present-versus-blank question,
but on this host it flips to a spurious "no" when the same question is framed as "does it
*match* the intent", and it hallucinates geometry into a genuinely blank image when it is
forced to justify its answer. So the oracle is scoped as a coarse "non-empty layout
consistent with the intent" second opinion, not a fine intent-conformance judge. That is the
honest ceiling of a small local vision model, and it is enough to corroborate a faithful
layout against an empty or corrupt one.

Concretely, the oracle is compared against the authoritative checker over a fixture pair: a
faithful layout (a cell with separated metal rectangles, which the `RectPresent` checker
passes) and a corrupt one (the same cell with no geometry, which the checker fails). The
`AgreementTally` records, for each fixture, whether the vision verdict matched the checker's
pass/fail, and reports the fraction. The safety property the pair demonstrates, that a
corrupt layout is caught by *at least one* oracle, holds regardless of the vision model's
answer because the authoritative checker always catches it; the vision oracle's contribution
is the corroboration and the disagreement signal on top of that.

On the development host the live oracle ran (`llava:7b`) and agreed with the authoritative
checker on both fixtures of the pair. That is a demonstration of the mechanism over a small,
hand-built fixture pair, not a suite-wide benchmark headline. See
[ADR 0090](../decisions/0090-multimodal-vision-second-oracle.md) for the model choice, the
VRAM budget, and the honest-not-run policy.
