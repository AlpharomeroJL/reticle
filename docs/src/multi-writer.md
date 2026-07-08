# Multi-writer collaboration

The [collaboration](collaboration.md) layer began as one-writer-broadcasts: a sharer
published, viewers consumed. Multi-writer turns that into true co-editing, where
several editors' edits merge and converge, a read-only viewer cannot write, and each
editor's undo affects only their own edits (ADR 0081).

## Convergence

Every editor keeps a `SyncDocument` backed by a `yrs` CRDT. Local edits become binary
updates; editors exchange them both ways and converge to a byte-identical document
regardless of the order updates arrive in. Disjoint edits (each editor working on a
different cell) and conflicting edits (both adding to the same cell) both converge:
records are keyed by a globally-unique `actor:counter` id, so two editors never write
the same key, and every record simply coexists as a union.

## Roles: editor and viewer

A session has two roles, and read-only is a structural property rather than a matter
of discipline:

- An **editor** joins in Edit mode. It both receives peers' updates and may publish
  its own.
- A **viewer** joins in View mode (`?mode=view`). It receives everything but cannot
  write, enforced in two independent places:
  - **The relay** drops a view-mode connection's frames outright: they are neither
    added to the room log nor broadcast, so a viewer's attempted edit never reaches
    another peer. Both the native relay and the Cloudflare Durable Object worker do
    this, and the conformance suite proves they agree.
  - **The client** viewer transport exposes *no* method that sends a document frame,
    so a viewer is structurally unable to publish even if the relay backstop were
    removed.

An end-to-end relay test drives two editors and one viewer at once: the editors
converge, the viewer materializes exactly the union the editors reached, and the
frame the viewer tries to publish is dropped and never reaches either editor.

## Selective undo

In a shared document a single global undo stack is wrong: undoing should reverse *my*
last edit, not whatever edit happened most recently, and it must never disturb a
concurrent editor's work.

Each editor tags its local edits with a per-actor **origin** (a stable actor id on the
underlying CRDT transaction) and drives undo through a `yrs` undo manager scoped to
that origin. So:

- `undo` reverts only this editor's own most recent edit; a peer's edits (applied with
  no local origin) are never on this editor's undo stack.
- The undo is itself an ordinary CRDT change, so after the editors exchange updates
  again they still converge, now with that one edit removed.
- `redo` re-applies only this editor's undone edit, and likewise reconverges.

So two editors can interleave edits, and each can undo and redo their own work
independently while both documents stay in agreement.

## What makes it converge, and one sharp edge

Three details are load-bearing:

- **Client ids stay below 2^53.** `yrs` (following Yjs) round-trips client ids through
  a JavaScript-safe-integer representation; a larger id is silently corrupted on the
  wire, which makes peers disagree on which client owns a struct. That is invisible to
  a document comparison but breaks the precise identity undo/redo depends on, so the
  derived client id is masked to 32 bits.
- **Deleted structs are kept, not garbage-collected.** Redo re-inserts a
  previously-deleted item; keeping tombstones lets that re-insertion converge across
  peers.
- **The undo manager is `Send + Sync`.** A document is driven from a background task in
  the live-agent path, so it must cross threads.

## Status

The sync-layer contract above (convergence, per-actor selective undo) and the relay's
view-mode enforcement are delivered and tested. The in-app *editor* still edits a
separate document as its source of truth and publishes a reconciled mirror; making the
live editor CRDT-backed, so it merges inbound peer edits and routes its undo button
through the per-actor undo manager, is a larger app change deferred to a later step.
