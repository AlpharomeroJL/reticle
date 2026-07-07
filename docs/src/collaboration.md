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

## Reconnect and resync

A shared live session runs over a browser WebSocket, and browser sockets drop: a
laptop sleeps, Wi-Fi hiccups, a phone changes networks. The live transport
(`reticle_app::livesync`) treats a drop as recoverable rather than terminal (ADR
0063). When a socket closes or errors while the session is still wanted, the
transport redials with **capped exponential backoff**: the wait doubles each attempt
from a 500 ms base up to a 30 s ceiling, with deterministic jitter (each wait lands
in `[ceiling/2, ceiling]`) so a fleet of tabs recovering from the same outage does
not stampede the relay in lockstep. Attempts are unbounded (only closing the session
stops them) because an outage of any length should heal on its own.

While waiting between tries the status line reads *"reconnecting to the shared session
(attempt N)…"*, counting the tries so a person can see progress; it returns to
*"connected"* the moment a redial succeeds.

Two things resync on reconnect, from the two ends:

* **The sharer** republishes its **whole document as one full-state snapshot** the
  instant its socket reopens, before resuming incremental updates. So any edit made
  while the socket was down still reaches viewers, carried by that snapshot, not lost
  with the dropped socket. Because `yrs` updates are idempotent, a viewer that already
  saw part of the document applies the snapshot without duplicating anything.
* **A viewer** needs no resend logic at all: on rejoining a room the relay replays the
  room's full update log before live traffic, so a reconnecting (or freshly arriving)
  viewer catches up on everything published so far, again idempotently.

The reconnect *schedule* is pure, `cfg`-free logic, unit-tested without a browser
(attempt growth, the cap, and the jitter bounds), and the resync *contract* is proven
headlessly by a relay integration test that kills a sharer's socket, edits offline,
reconnects on a fresh connection, and asserts the viewer materializes the combined
edits exactly once.

## Testing

Convergence is tested directly: concurrent operation sets are applied in different
orders across simulated peers, and the final documents must be identical.
