# 0022, The agent as a live CRDT collaborator: a step-atomic bridge

## Context

The propose-verify-correct harness (`reticle-agent`) drives a model against the
`reticle-agent-api` command surface and produces a `Document`, but it does so on a
private, single-player `Session`. To let a human watch (and edit alongside) the agent
in real time, its edits have to land on the collaboration layer (`reticle-sync`, a
`yrs` CRDT) under a distinct identity, `reticle_agent_api::AGENT_ACTOR`.

Two constraints shape the design:

1. Each of `SyncDocument`'s existing mutators (`add_cell`, `add_rect`, ...) opens its
   own `yrs` transaction, so a step that draws several shapes would ship as several
   separate updates. A concurrent peer could then observe a half-drawn step (one
   shape on the wire, the rest not yet). An agent "step" (one propose iteration, which
   may emit many commands) must instead land as **one** atomic CRDT update.

2. `reticle-sync` must not depend on `reticle-agent` or `reticle-agent-api` (that
   would invert the layering: the agent is the higher-level consumer). So the bridge
   itself lives in `reticle-agent`, and any support `reticle-sync` needs must be
   generic (no agent types leaking down).

## Decision

Two additive changes, no frozen type touched:

- **`reticle-sync`: a public grouped-edit API.** Expose `SyncDocument::step`, which
  runs a caller closure inside the single existing private `edit()` transaction and
  hands it a `StepEdit` handle offering the same operations (`add_cell`,
  `add_empty_cell`, `add_shape`, `add_rect`, `add_instance`, `add_array`,
  `set_top_cell`, `remove_cell`). Everything the closure does commits as one update,
  so a multi-shape step is atomic on the wire. No `yrs` type appears in the public
  signature. The awareness map also gains a generic per-actor **status** slot
  (`set_status`/`status`/`statuses`, an opaque `String`) so a status channel can ride
  the awareness layer without `reticle-sync` knowing what `AgentStatus` is.

- **`reticle-agent`: the `collab` bridge module** (depends on `reticle-sync`). An
  `AgentCollaborator` wraps a `SyncDocument` created for `AGENT_ACTOR`. It translates a
  batch of `AgentCommand`s (one logical step) into a single `SyncDocument::step`, then
  publishes presence (cursor at the last edit location, selection = the ids placed this
  step) and an `AgentStatus` (serialized to JSON) into the awareness status slot. A
  `Pacing` setting gives either a fixed inter-step delay or an `instant` mode for tests
  and replay.

## Consequences

- A multi-shape agent step is one CRDT transaction: a concurrent human peer never
  observes a partially-applied step. This is asserted by a convergence test that snap-
  shots the wire update mid-run and materializes it in isolation.
- The bridge reuses the harness's own command vocabulary, so it stays in lockstep with
  what the agent actually does; no second, drifting translation of edits.
- `reticle-sync` gained one generic grouped-edit method and a generic status slot;
  both are useful beyond the agent (any batched editor, any presence-carried status).
- Pacing is a pure data setting the caller drives; the library sleeps only when asked,
  so tests and replay stay instant and deterministic.
