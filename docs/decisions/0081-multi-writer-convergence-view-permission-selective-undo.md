# 0081, Multi-writer convergence, view-mode permission, and per-actor selective undo

## Context

Live collaboration began as one-writer-broadcasts: a sharer published its document
and viewers consumed it (ADR 0038, 0058, 0063). Convergence already held at the CRDT
layer (a `yrs` document over the model, ADR 0007), but three properties a true
multi-writer session needs were unproven or absent:

1. **Several editors' edits must converge.** Two people editing the same document,
   exchanging updates both ways, must reach a byte-identical result.
2. **Undo must be per-actor.** In a shared document a global undo stack is wrong: my
   undo must revert *my* last edit, not whatever edit happened most recently, and it
   must not touch a concurrent peer's work. After my undo the peers must still
   converge.
3. **A view-mode participant must not be able to write.** Read-only has to be
   enforced, not merely conventional.

## Decision

**Tag every local edit with a per-actor `yrs` transaction origin, drive undo through
a `yrs` `UndoManager` scoped to that origin, and keep view-mode read-only enforced on
both the relay and the client.**

1. **Per-actor origin.** Every local mutation on a `SyncDocument` runs in a
   transaction tagged with the actor's id as its origin. A remote peer's update is
   applied with *no* origin, so the two are distinguishable at the CRDT layer.

2. **Selective undo.** Each `SyncDocument` owns a `UndoManager` that tracks *only*
   this actor's origin and is scoped to every record map. `undo`/`redo` therefore
   affect only this peer's own edits; a peer's applied updates (no local origin) are
   never captured onto this stack. The undo/redo is itself an ordinary CRDT change,
   so after exchanging updates the peers still converge. A zero capture-timeout makes
   each edit its own undo step deterministically (no wall-clock grouping), and a
   constant clock keeps that working on `wasm32`, where the default options are
   unavailable.

3. **View-mode permission, defence in depth.** The relay drops a `?mode=view`
   connection's frames: they are neither logged nor broadcast, so a viewer's write
   never reaches another peer (native `reticle-server` and the Cloudflare Durable
   Object worker both enforce this, proven by the conformance suite). On the client,
   the viewer transport exposes *no* publish method at all, so a viewer is
   structurally unable to send a document frame even if the relay backstop were
   removed.

## Consequences

Three correctness details were load-bearing and are recorded so they are not
"simplified" away:

- **Client ids must stay below 2^53.** `yrs` (following Yjs) round-trips client ids
  through a JS-safe-integer representation; a full 64-bit hash silently corrupts
  struct ownership on the wire. That corruption is invisible to a materialized
  document comparison (records are keyed by stable `actor:counter` strings, not by
  client id) but it breaks the precise struct identity the undo manager needs, so
  **redo across peers diverged** until the derived client id was masked to 32 bits.
- **Garbage collection is disabled** on the document. Redo re-inserts a
  previously-deleted item; if a peer had GC'd that tombstone, the redo and a
  re-exchange would diverge. Keeping tombstones makes undo/redo converge across
  peers.
- **The `UndoManager` must be `Send + Sync`.** A `SyncDocument` is driven from a
  `tokio` task in the live-agent path, so the owned undo manager (and thus the
  document) has to cross threads; `yrs`' `sync` feature provides the `Send + Sync`
  observers this needs.

The review-critical properties are proven natively: two-writer disjoint-and-conflicting
convergence (byte-identical), selective undo that reverts only the local actor's edit
and reconverges, per-actor independent undo, redo convergence, and undo leaving an
integrated remote edit intact. An end-to-end relay test drives two Edit-mode peers and
one View-mode peer: the editors converge, the viewer materializes the union, and the
viewer's published frame is dropped by the relay.

**Scope not taken (honest gap).** The in-app *editor* still edits a separate
`EditableDocument`/`History` as its source of truth and publishes a reconciled
`SyncDocument` mirror; it does not yet accept and merge inbound peer edits into the
live editable document, nor route its undo button through the per-actor
`UndoManager`. Wiring that is a larger app rearchitecture (making the editor
CRDT-backed) and is deferred; the sync-layer contract and the relay enforcement that
rearchitecture would build on are delivered and tested here.
