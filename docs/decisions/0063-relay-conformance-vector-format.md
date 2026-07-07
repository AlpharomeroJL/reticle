# 0063, the relay conformance vector format: one table, two targets

## Context

With two relays (the native `reticle-server` and the Durable Object of ADR 0062)
that must be observably interchangeable, the risk is drift: a change to one relay,
or a subtle difference in the DO's storage-backed replay or alarm-driven
coalescing, that the existing per-relay tests would not catch. The brief called for
a conformance suite that runs identical frame sequences against each relay and
freezes its vector format at the 1B merge (ADR 0061). The format has to express the
whole relay contract once, run against two very different targets (an in-process
axum server and a networked wasm DO under miniflare), and be able to *fail* a relay
that breaks the contract, not just pass a correct one.

## Decision

A conformance **vector** is a named list of scripted `Action`s
(`crates/reticle-relay-conformance`): connect a client (edit or view) to a room,
send an update/presence/text frame, expect a specific frame or silence at a client,
burst presence, disconnect. Clients and rooms are named by string so a vector reads
as a script; frames carry a distinguishable marker (an ASCII tag in the update
payload, a sequence number in `presence.cursor.x`) recovered from whatever the far
side receives. One `run_vector(target, vector)` executor drives a vector with
`tokio-tungstenite` and returns the first assertion that did not hold. A `Target` is
just a base WebSocket URL plus timing and one semantic knob: `Target::native`
spawns the axum relay on an ephemeral port (the `share_live.rs` pattern);
`Target::external` addresses any relay by URL (the DO under `wrangler dev`, or a
deployed `wss://...workers.dev`).

The only target-aware branch is presence coalescing. Byte-for-byte delivery is not
the invariant, because the DO coalesces presence and the native relay does not;
the shared invariant is convergence. A presence burst asserts, on both relays, that
the observer receives a strictly increasing run whose newest sequence is last; the
coalescing target additionally asserts it received strictly fewer than were sent
(within a generous bound), and the non-coalescing target that it received all of
them. One vector, one verdict per relay, both PASS. Every other clause of the
contract is a plain vector shared unchanged: late-join log replay in order,
view-mode frame drop (broadcast and log), echo suppression, uncoalesced updates,
full-log replay (the room cap observable), two-room isolation, and the binary-only
rule. A negative vector (which expects a dropped view frame to be forwarded) must
FAIL against either real relay, proving the harness has teeth.

The suite is split so `just ci` stays Node-free: the native half is always-on
`cargo nextest`; the DO half (`RETICLE_CONFORMANCE_DO=1`) spawns `wrangler dev
--local`, runs the same table, and kills the process tree. `just conformance` runs
both, bootstrapping `worker/node_modules` with `npm ci`.

## Consequences

The contract is expressed once and checked against both relays with identical
verdicts, so a future change to either relay that breaks a semantic fails a named
vector rather than silently diverging. The vector format is now frozen (ADR 0061):
new coverage adds vectors; the `Action`/`Payload`/`Target` shape does not change
under a lane. The cost is that the DO half needs Node and a wrangler spawn, which is
why it is a wave-gate recipe and not part of `just ci`. The one place the two
relays are not byte-identical (presence coalescing) is explicit in the runner and
documented, not hidden: the suite proves equivalence of the semantics the protocol
guarantees, and measures (does not paper over) the free-tier optimization layered
on top.
