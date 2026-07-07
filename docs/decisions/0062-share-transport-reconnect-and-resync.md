# 0062, Live share reconnect: capped-backoff redial and a full-state snapshot on reopen

## Context

ADR 0058 shipped the wasm live-share transport: a read-only `ViewerTransport` and a
publishing `SharerTransport` over `web_sys::WebSocket`, each frame a
`reticle_proto::v1::SyncMessage`. It could open a socket and stream, but it could not
survive one dropping. A closed or errored socket landed the session in
`LiveStatus::Closed`/`Failed` and stayed there: the shared view froze with no path
back short of the user reloading. Browser sockets drop routinely (a laptop sleeps,
Wi-Fi blips, a phone roams), so a live session that cannot heal is barely live.

Two problems had to be solved together. **(1) Redial:** when should the transport try
again, and how often, without hammering the relay or synchronizing a stampede of tabs?
**(2) Resync:** a sharer keeps editing while its socket is down; those edits live only
in its local `SyncDocument`. When it reconnects, how do they reach viewers, given the
old socket (and any incremental deltas queued on it) is gone?

## Decision

**1. Redial with capped exponential backoff and deterministic jitter, unbounded except
by user cancel.** On a `close`/`error` while the session is still wanted, the transport
schedules a redial through `window.setTimeout`. The wait is `next_backoff(attempt)`:
`500 ms * 2^(attempt-1)` clamped to a **30 s ceiling**, then *equal jitter* — half the
ceiling as a floor plus a seed-derived amount in `[0, ceiling/2]`, so every wait lands
in `[ceiling/2, ceiling]`. The floor keeps a redial storm off the relay; the jitter
(seeded per session from `Math.random`) de-correlates many tabs recovering from one
outage. Attempts are **capped only by the user closing the session** (dropping the
transport): an outage of any length should heal unattended. The schedule is pure,
`cfg`-free logic (`next_backoff_seeded(attempt, seed)`), exhaustively unit-tested for
growth, the cap, overflow safety, and jitter bounds — no browser, no clock. Each wait
posts `LiveStatus::Reconnecting { attempt }`, which the existing status line renders as
"reconnecting… (attempt N)"; `Reconnecting` is explicitly **not** terminal.

**2. Resync by publishing a full-state snapshot on reopen, not a state-vector diff.**
The considered alternative was a state-vector diff: on reconnect, learn the receiver's
`StateVector` and send only what it lacks. We rejected it. A reconnecting sharer cannot
trust any *remembered* remote state vector — the room may have gained or lost peers, the
relay may have restarted, the previous viewer may be gone and a new one present — and
negotiating a fresh state vector adds a round trip and a new wire exchange over a
channel that carries opaque frames. A **full-state snapshot** (`SyncDocument::
encode_full_state`, i.e. `yrs` `encode_state_as_update` against the empty state vector)
sidesteps all of it: it is self-contained, correct for *every* receiver regardless of
what they have seen, and idempotent — a viewer that already has part of the document
applies it without duplicating a single shape. The cost (re-sending the whole document
on each reconnect) is bounded by document size and paid only at the reconnect edge, not
per update; for the session sizes this transport targets that is the right trade.

Mechanically the sharer needs no new reconnect code: when its socket reopens it posts
`LiveStatus::Open`, and the app already re-arms a full-document publish on `Open` (ADR
0058, to cover the first connecting-socket publish). So the offline edit — which bumped
the editor's history revision — is republished as one full-state frame ahead of
resumed incremental updates. **Viewers** need no client code either: the relay already
replays a room's full update log to any (re)joiner before live traffic (ADR 0007/0038),
so a reconnecting or freshly arriving viewer catches up idempotently by that replay.

**3. Read-only stays structural.** `ViewerTransport` gains reconnect but still exposes
no send method of any kind: the reconnect machinery lives on a shared `Core` whose
`send` helper is reachable only through `SharerTransport::publish_*`. A viewer redials
and re-consumes; it can no more publish after reconnect than before.

## Consequences

* A dropped live session recovers on its own, within 30 s of the network returning,
  and the sharer's offline edits materialize on viewers exactly once — proven by a
  headless relay test that kills the sharer's socket, edits offline, reconnects on a
  fresh connection, and asserts the viewer's shape count (A+B, not A+A+B), plus a
  negative control that omits the snapshot and shows the viewer left missing the edit.
* The reconnect *schedule* is decoupled from the DOM and tested as pure logic, the same
  seam ADR 0058 established for `route_frame` and `LiveStatus`. The wasm redial glue
  (setTimeout, generation-guarded close/error handling, cancel-on-drop with
  weak-captured closures so nothing leaks) is exercised by the Playwright e2e (lane
  v8-1e) and kept minimal.
* No schema change: `Reconnecting` is a new `LiveStatus` variant (app-local), and the
  snapshot rides the frozen `SyncMessage`/`CrdtUpdate` envelope. `encode_full_state` is
  an additive `SyncDocument` method, deliberately equivalent to `encode_state_update`;
  the distinct name marks the reconnect-resync contract at the call site.
* Trade accepted: full-state on every reconnect is heavier than a diff for a large
  document, and the room log grows unbounded (ADR 0007's standing note; a production
  host compacts it via the persist hook). Both are acceptable at current session sizes
  and revisitable if a session outgrows them.
