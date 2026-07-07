# 0066, The agent in a real relay room: id-addressed mirroring and a native live client

## Context

ADR 0022 made the agent a live CRDT collaborator: an `AgentCollaborator` mirrors each
propose-iteration step onto a `reticle-sync` `SyncDocument` under `AGENT_ACTOR`, as one
atomic transaction per step, and publishes cursor/selection presence. Two gaps kept it
from being a *real* collaborator in a *real* room:

1. **Id-addressed edits were not mirrored.** ADR 0022 mirrored the geometry-*creating*
   commands (`CreateCell`, `AddRect`, `AddPolygon`, `AddPath`, `PlaceInstance`,
   `PlaceArray`, `DeleteCell`). The two edits that address *existing* elements,
   `TransformShapes` and `DeleteShapes`, name the command surface's stable
   `ElementId`s, which are **not** the CRDT's `actor:counter` element ids. With no
   mapping between them, ADR 0022 left those edits unmirrored: an agent that moved or
   deleted a shape did so only in its private session, and a human watching the room
   saw the stale shape.

2. **There was no way to join an actual room.** The collaborator only mirrored into an
   in-process `SyncDocument`; nothing connected it to a `reticle-server` relay room
   beside browser humans.

## Decision

Two additive changes in `reticle-agent`, plus one generic, additive pair of methods in
`reticle-sync`. No frozen type is touched; `reticle-agent-api`'s command surface is
unchanged.

### Closing the id gap: learn the association at create time

The collaborator now drives an **authoritative internal `Session`** in lockstep with
the mirror. For each command in a step it:

- applies the command to the session, which both validates it and, for every element it
  creates, returns the stable `ElementId` it assigned (`AgentResponse::Ok.affected`);
- mirrors the geometry-creating commands onto the CRDT exactly as before, and for each
  created **shape** records `ElementId -> (CRDT id, cell, geometry)` in a map.

A later `TransformShapes` resolves each addressed `ElementId` through that map and
overwrites the CRDT record's geometry **in place** (same CRDT id, new coordinates), so
the shape keeps its identity and a converged peer sees it *move* rather than blink out
and reappear. A `DeleteShapes` removes the record by its CRDT id. Both mirror only what
the session actually applied: the session's `TransformShapes`/`DeleteShapes` validate
every id up front and are all-or-nothing, so the mirror gates on a non-empty `affected`.

The geometry-creating commands still mirror **unconditionally** (a peer may hold a cell
this agent's private session does not, e.g. an instance of a cell another peer created),
so mirroring is never gated on the private session accepting a create. Only the
id-addressed edits, which *need* the map, consult it.

**Honest failure, never a silent one.** An `ElementId` the collaborator never learned,
a shape created before it attached, or one the internal session rejected, cannot be
resolved. Rather than apply it incorrectly or drop it silently, the mirror skips that id
and records a warning in the new additive `StepReport.skipped` field. A skipped id does
not poison the step: the rest of the batch still commits. A test clears the id map
(`AgentCollaborator::forget_element_ids`, modelling a mid-session attach) and asserts the
still-valid session edit yields a skip, not a silent no-op.

### `reticle-sync`: two generic per-shape step ops

Mirroring a move or delete needs to overwrite or remove a *single* shape record by its
CRDT id, which `SyncDocument`/`StepEdit` did not expose (the only removal was
`remove_cell`, which would also delete a concurrent peer's shapes in the same cell).
`StepEdit` gains two generic methods, `set_shape(cell, id, shape)` and
`remove_shape(id)`, backed by additive helpers in `reticle-sync`'s `mapping` module.
They carry no agent types (the layering that ADR 0022 protects is intact) and converge
because the CRDT id is a unique `actor:counter` string only the agent writes: overwriting
or removing it is a single-writer update that merges cleanly with any concurrent edit on
a *different* id.

### A native live-room client

`reticle-agent::live::run_in_room` connects an `AgentCollaborator` to `ws://host/ws/{room}`
with `tokio-tungstenite`, and for each scripted step: applies the batch (one atomic
`SyncDocument::step`), ships that step's `yrs` delta as **one** binary
`SyncMessage`-framed frame (`encode_update_frame_for`, so one step is one frame and a
watcher never sees a half-step), publishes the agent's presence, and applies inbound peer
frames back into its own document. The framing is exactly the browser transport's
(ADR 0058), so the agent is indistinguishable from any other room participant. The module
is native-only (`tokio` does not build for `wasm32`).

## Consequences

- The ADR 0022 gap is closed: an agent that transforms or deletes a shape it created is
  seen to do so by every peer in the room, and the edit converges in either exchange
  order (asserted by convergence tests with a concurrent human peer).
- An in-process integration test (`reticle-server/tests/agent_live.rs`, the `share_live.rs`
  pattern) proves both directions over a real socket: the agent's step and presence
  frames reach a peer and materialize, and a peer's concurrent edit reaches the agent's
  document.
- A committed, deterministic demo transcript
  (`examples/collab/agent_drc_fix.transcript.jsonl`) records the agent fixing a seeded
  met1 spacing violation (`RunDrc` -> `TransformShapes` to a legal gap -> `RunDrc` clean)
  in a live room, and replays to a pinned document hash. It is labelled, in the trailer,
  the book, and every doc comment, as a **deterministic scripted run with no live model**:
  the commands are a fixed script, not an LLM's output.
- The internal session is a second application of each command (once to validate/allocate
  ids, and the mirror re-derives geometry from the command). This is deliberate: it keeps
  the collaborator a faithful, self-contained mirror without threading session outcomes
  through the public `apply_step` signature (so the existing `reticle-demo-server` caller
  is unchanged and gains id-addressed mirroring for free).
- The mirror does **not** carry the technology (DRC is a session concern), so a mirrored
  CRDT document matches a peer's shape-for-shape, not by whole-document equality.
