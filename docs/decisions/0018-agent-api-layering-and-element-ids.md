# 0018, Agent API layering: serde-friendly types and session-owned element ids

## Context

The v5 run adds `reticle-agent-api`, a serializable command surface over the engine
so an external agent (or an MCP tool, or the demo server) can drive layout edits. Two
design questions had to be settled before the surface froze, because every later lane
codes against the answers.

First, the wire types. `reticle-geometry` does not derive serde, and making it do so
would pull a serialization concern into the lowest, most performance-sensitive crate.

Second, addressing. The engine's edit vocabulary (`reticle_model::Edit`) removes shapes
by positional index, and those indices shift when a shape is deleted. A command like
`query_shapes` then `delete_shapes` needs a handle that keeps pointing at the same
element across intervening edits.

## Decision

`reticle-agent-api` owns its own plain-data serde argument types (`PointArg`, `RectArg`,
`LayerArg`, `TransformArg`, and so on) and converts to and from engine types when a
command is applied. `reticle-geometry` stays serde-free.

Elements are addressed by a stable `ElementId(u64)` allocated by the session, not by a
positional index and not by a new id baked into `reticle-model`. The session keeps an
id-to-slot map and reconciles it as edits shift indices; the model is untouched. This
keeps the id scheme contained and independent of the CRDT's own element ids in
`reticle-sync`.

## Consequences

- The command and response enums (`AgentCommand`, `AgentResponse`) round-trip as JSON
  with a stable `op`/`result` tag, tested in the crate, so the MCP schemas and the
  transcript format derive from one frozen source.
- Adding a geometry primitive does not force a serde change in `reticle-geometry`; the
  cost is a small conversion layer in `reticle-agent-api`, paid once per command kind.
- The session must maintain the id map correctly across removals; that logic is the
  agent-api implementation lane's responsibility and is property-tested there (random
  command sequences never panic and never mis-address an element).
