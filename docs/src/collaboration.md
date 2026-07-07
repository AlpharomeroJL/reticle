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

## Sharing a session

The Share panel turns the current session into something a collaborator can open. The
one-click **Share this session** button does the whole dance at once: it mints a fresh
room (the design name plus a short random suffix), goes live so viewers stream this
session, and copies the read-only viewer link to the clipboard. The advanced fields
(relay host, room name, viewer-page origin) stay below for when you want to name the
room yourself or point at a specific relay. A viewer opens the copied link in a browser
and joins read-only: they see your live edits, pan and zoom independently, and can
follow your view, but never publish (the relay enforces read-only on its side too).

## Permalinks

A **permalink** deep-links a particular view of an opened document, layered on top of
the `?gds=<url>` open link. Beyond the file, it can carry three independent, optional
pieces of view state:

- `?cell=<name>` - the cell to focus (URL-encoded, so spaces and non-ASCII names work).
- `?view=<x>,<y>,<zoom>` - the camera: the world point at the canvas center and the
  zoom in pixels per DBU.
- `?layers=<csv>` - the visible layers as `layer/datatype` specs
  (for example `68/20,69/20`); every other layer is hidden. An empty value hides all.

`?view=` is shared with the start-view selector (`?view=viewer|editor|replay`); the two
are told apart by shape, so a value of exactly three numbers is a camera and anything
else is the start view (see ADR 0067). **Copy permalink to this view** in the Share
panel serializes the current cell, camera, and visible layers into such a link; opening
it re-applies them once the document has loaded. Malformed values (a bad number, an
unknown layer) are ignored rather than failing the open, so a hand-edited link still
works as far as it can.

## Testing

Convergence is tested directly: concurrent operation sets are applied in different
orders across simulated peers, and the final documents must be identical.

The permalink parser and emitter are pure and round-trip tested (`emit` then `parse` is
the identity), including the encoding edge cases (spaces, unicode cell names, an empty
layer list), and an app-level test proves an emitted permalink restores the same cell,
camera, and layers on a freshly opened document.

### Browser-level proof (ADR 0058, 0068)

The read-only contract and the transport are proven twice. The authority is a headless
Rust relay test (`crates/reticle-server/tests/share_live.rs`): a real publisher and a real
viewer connect over an ephemeral port, and it asserts the publisher's frames materialize
in the viewer and that a frame a view-mode client sends is dropped and never enters the
room log. On top of that, a two-context Playwright suite (`just e2e-share`) proves the
*browser* side, reading a wasm instrumentation seam (`window.__reticle_stats`) because the
egui canvas is GPU-painted and has no DOM node to assert on:

- an edit made in the sharer paints in a read-only viewer (a no-edit control room isolates
  the scripted edit to exactly one extra applied shape in the viewer);
- a view-mode socket cannot write: the same captured relay frame is dropped when a browser
  sends it from a `?mode=view` socket but applied when sent from an edit-mode socket, a
  positive control that keeps the drop assertion from passing vacuously;
- a phone navigates a design by touch: on a mobile viewport a two-finger pinch changes the
  zoom and a drag changes the pan, read back from the camera field of the same seam.

What these do not assert is the pixels of the rendered canvas; that stays the job of the
Rust relay test (for the read-only contract) and the pure camera unit tests (for the
pan/zoom transform). The README's share clip is captured from this same two-context flow
by `just capture-share`.
