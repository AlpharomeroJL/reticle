# Collaboration

`reticle-sync` and `reticle-server` make a document editable by several people at
once, with no central authority resolving conflicts.

## A CRDT over the document

The document is mirrored onto a `yrs` CRDT (the Rust port of Yjs; ADR 0007). Every
create, edit, move, and delete becomes a CRDT update, and the CRDT guarantees that
peers who see the same set of updates converge to the same document regardless of
the order in which the updates arrive. Edits made while offline reconcile
automatically on reconnect.

## Presence and comments

Alongside the document updates, peers exchange lightweight presence messages
(cursor, selection, and viewport) so each person sees where the others are working,
and threaded comments anchored to shapes so a review conversation lives next to the
geometry.

## The relay

`reticle-server` is a thin `axum` and `tokio` WebSocket relay: it holds rooms,
broadcasts updates and awareness to the other peers, sends a new peer the current
state, and offers a persistence hook. It contains no editing logic of its own, so
the same convergence guarantees hold whether peers are connected through it or
exchange updates by any other means.

## Testing

Convergence is tested directly: concurrent operation sets are applied in different
orders across simulated peers, and the final documents must be identical.
