# 0048, Wave 2D adds a RunGenerator command to drive the generators from the agent

## Context

Wave 2 built six parameterized layout generators behind
[`reticle_gen::Registry`](../../crates/reticle-gen/src/registry.rs) (guard ring, via
farm, pad ring, seal ring, density fill, test structure), each a pure function from
JSON parameters plus a technology to DRC-clean-by-construction geometry. Wave 2D
exposes them on the three product surfaces: the app Generate panel (ADR 0050), the
MCP tool catalog (ADR 0049), and the benchmark (a `generator` checker and 8 tasks).

The app and the MCP both drive a generator by asking a `Session` to run it and record
the result, so that a generator run is a first-class, replayable, undoable command
rather than a side channel that bypasses the transcript. But `AgentCommand` is a
frozen Wave-0 contract surface: there is no existing command that runs a generator.
`reticle-agent-api` already depends on `reticle-model`/`reticle-drc`, and `reticle-gen`
is pure geometry that compiles for wasm, so the agent layer can call the registry
directly once a variant exists.

## Decision

Amend `AgentCommand` at the Wave 2D boundary with one additive variant,
`RunGenerator { cell, generator_id, params }`: the target cell, the registry id, and
the generator's own JSON parameter object. The enum is already `#[non_exhaustive]` and
serde-tagged by `op`, so the new variant is additive and does not break existing
recorded transcripts (a transcript written before it still deserializes; the
round-trip and replay tests are extended to cover it). This is the Wave 2D
frozen-surface amendment, authorized for this lane the same way ADR 0031 authorized
the Wave 3 editor-op expansion.

The `apply.rs` arm builds `Registry::with_builtins()`, generates into a **scratch
cell** using the live document's technology (so validation and geometry building
happen before anything touches the document), then commits each produced shape as a
normal `Edit::AddShape` and allocates a stable element id per shape, exactly as
`BuildViaStack` does. A generator error (unknown id, malformed or out-of-range params)
maps to an `InvalidArgument` error carrying the generator's own field-naming message,
and nothing is committed. Because the geometry lands as ordinary edits, a generator
run is transcript-replayable (the replay reproduces the document hash) and undoable
like every other command; the app layer groups the per-shape edits into one logical
undo step.

## Consequences

The agent, the MCP (ADR 0049), and the benchmark can pose and solve generator
constructions through one command, and a recorded session that ran a generator
replays deterministically. The duplication of the "run the generator" call between the
`reticle-app` Generate panel and the agent `apply.rs` is deliberate and thin: both
call the same `reticle_gen::Registry`, there is no cross-dependency, and the geometry
is pinned to the generators' own DRC-clean-by-construction guarantee. Older transcripts
and results stay valid because the change is purely additive. This is the counterpart
to ADR 0029 (Wave 1) and ADR 0031 (Wave 3): one authorized, additive, serde-back-
compatible amendment per wave.
