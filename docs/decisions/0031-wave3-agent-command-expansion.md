# 0031, Wave 3 expands the AgentCommand surface for the Wave 2 tools

## Context

Wave 3 Lane 3A exposes the Wave 2 editor capabilities (boolean union, intersection,
difference, and xor; align and distribute; offset and sizing; the via-stack builder)
to the agent, so a model can construct them in one tool call rather than a long batch
of primitives, which matters for the 16k-window local models. `AgentCommand` is a
frozen Wave-0 contract surface. The Wave 2 logic lives in `reticle-app`
(`ops.rs`, `productivity.rs`), which `reticle-agent` does not depend on, so the agent
layer cannot call it; the same constructions must be implemented at the command layer.

## Decision

Amend `AgentCommand` at the Wave 3 boundary to add the higher-level construction
commands (a boolean-combine over a set of element ids on a layer, align and
distribute, an offset/size, and a via-stack builder). The enum is already
`#[non_exhaustive]` and serde-tagged by `op`, so the new variants are additive and do
not break existing recorded transcripts. Their `apply.rs` arms reuse the same
`reticle-geometry` primitives the UI uses (`polygon_boolean`, `offset`, `Transform`,
and the `Rule`/`RuleKind::Enclosure` reading for via enclosures), so the UI and the
agent share one geometry engine without a crate dependency between them. Each new
command gets an MCP tool with a tight model-facing description and a two-way schema
test, matching the existing tools. The integration agent authorizes this amendment
per the contract-change rules; the exact variant set is fixed by Lane 3A and recorded
in `docs/TASKS.md` at the Wave 3 merge.

## Consequences

The agent and the benchmark (Lane 3F) can pose and solve boolean, array, and
via-stack constructions directly. The duplication of construction logic between the
`reticle-app` UI and the agent `apply.rs` is deliberate: there is no cross-dependency,
and both are pinned to `reticle-geometry`, checked by that crate's own oracle tests.
Older transcripts and results remain valid because the change is purely additive.
This is the Wave 3 frozen-surface amendment, the counterpart to
[ADR 0029](0029-result-record-backend-label.md) in Wave 1.
