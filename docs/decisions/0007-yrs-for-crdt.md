# 0007, `yrs` for the collaborative CRDT

## Context

Real-time collaboration needs a CRDT so concurrent create/edit/move/delete
operations converge regardless of arrival order, with presence (awareness) and
offline reconcile. The two mature Rust options are `yrs` (the Rust port of Yjs) and
`automerge`. The spec names `yrs` with `automerge` as an acceptable alternative.

## Decision

Use `yrs`. It is battle-tested via the Yjs ecosystem, has a compact binary update
format ideal for a WebSocket relay, ships an awareness protocol for presence, and
supports the shared map/array/text types needed to model a hierarchical layout
document. `reticle-sync` wraps `yrs` so the model is not tightly coupled to it.

## Consequences

We get a proven CRDT with an efficient wire format and presence for free, and the
`reticle-server` relay can be pure transport (no business logic). We own the mapping
from the hierarchical model (cells/instances/arrays/shapes) onto `yrs` types and
the convergence tests that assert order-independence. Switching to `automerge`
later would mean re-implementing that mapping behind the same `reticle-sync` API.
