# The relay: two implementations, one protocol

Reticle ships two collaboration relays, and treats the protocol between them as
the asset rather than either relay. The first is a native binary
(`crates/reticle-server`): axum and tokio, one broadcast channel and an in-memory
log per room, the relay you run yourself. The second is a Cloudflare **Durable
Object** (`worker/`): a Worker routes `GET /ws/{room}` to a per-room object that
holds the WebSockets, hosted on the free plan with no server to operate. A single
conformance suite proves the two are observably interchangeable.

## The wire invariant the relay keys off

Every frame on a live session is a `reticle_proto::v1::SyncMessage`, a protobuf
`oneof` of three payloads. Because a `oneof` field is encoded length-delimited,
the **first byte** of every frame is the field tag of the active variant:

| First byte | Variant  | Meaning              |
|-----------:|----------|----------------------|
| `0x0A`     | `update`   | a CRDT document delta |
| `0x12`     | `presence` | cursor, selection, viewport |
| `0x1A`     | `comment`  | a threaded comment    |

This is frozen by a unit test in `reticle-proto` and a doc comment on
`SyncMessage`. It lets a relay classify a frame with no protobuf decoder at all:
the Durable Object reads one byte to tell a presence update (which it coalesces)
from a document update (which it never drops). Neither relay ever inspects the
rest of the payload; both treat it as opaque bytes to fan out.

## The semantics both relays honor

The native relay defines the contract; the Durable Object mirrors it exactly:

- **Join and replay.** `GET /ws/{room}` upgrades to a WebSocket;
  `?mode=view|viewer|readonly|ro` (case-insensitive) joins read-only. On join the
  room's full accepted-frame log replays to the joiner, in order, before any live
  traffic, so a late joiner catches up.
- **Read-only is enforced server-side.** A frame from a view-mode connection is
  dropped: never logged, never broadcast. A viewer cannot mutate the document
  even if it sends a well-formed edit frame.
- **No self-echo.** A sender never receives the echo of its own frame.
- **Binary only.** Only binary frames are payloads; text, ping, and pong are not.

## Free-tier limits engineering

The free plan rewards doing less work, so the Durable Object adds three things the
native relay does not need, each preserving the observable semantics.

**Presence coalescing.** A live cursor can move at screen refresh rates, but a
follower only needs the *latest* position. The DO keeps only the newest presence
per client within a short window (about 10 Hz) and delivers that, driven by the
object's **alarm** rather than a `setTimeout` (a timer callback would keep the
isolate awake and defeat hibernation). Update frames are never coalesced: a
dropped CRDT delta would corrupt the document, while a dropped intermediate cursor
is invisible. The newest presence always converges, so read-only follow-mode still
lands on the sharer's current viewport. This is the one place the two relays are
not byte-identical: for a burst the DO delivers strictly fewer presence frames than
the native relay, by design.

**Log chunking.** The room log is persisted in Durable Object storage in fixed
chunks and replayed to late joiners in order, so a room survives the object being
evicted between events. A production host would compact old chunks into snapshots;
the raw chunked log keeps the relay free of any payload-format knowledge.

**Caps and expiry.** The native relay's per-room broadcast channel is bounded
(`ROOM_CHANNEL_CAPACITY` = 1024); a lagging peer skips dropped messages rather than
stalling the room. The DO bounds its own work the same way and reclaims an idle
room from the same alarm it uses for coalescing: when the alarm fires with no open
sockets, the room's storage is deleted. Hibernation-critical state (each
connection's mode and id) rides the socket's *attachment*, which survives eviction,
so a room can hibernate and wake without losing the read-only guarantee or
self-echo suppression.

A fourth optimization, **client-side packing** (batching several pending CRDT
updates into one frame per flush in the sharer's publish path), belongs to the app
transport rather than the relay and is tracked as a follow-up.

## Proving the two equivalent: the conformance suite

`crates/reticle-relay-conformance` expresses the whole contract once, as a table of
scripted vectors, and runs the identical table against either relay through one
`tokio-tungstenite` driver:

- `Target::native` spawns the axum relay in-process on an ephemeral port.
- `Target::external` addresses any relay by URL: the Durable Object under
  `wrangler dev --local` (miniflare, no Cloudflare auth), or a deployed
  `wss://...workers.dev`.

Each vector covers one clause: late-join log replay in order, view-mode frame drop,
echo suppression, presence coalescing, uncoalesced updates, full-log replay, two-
room isolation, and the binary-only rule. Byte-for-byte delivery is *not* the
invariant, because the DO coalesces presence; the shared invariant is
**convergence**. A presence burst asserts on both relays that the observer receives
a strictly increasing run whose newest value arrives last; the coalescing target
additionally asserts it received strictly fewer than were sent, and the native
target that it received all of them. One vector, both relays PASS, the verdicts
identical.

A deliberately-broken vector (one that expects a dropped view-mode frame to be
forwarded) must **fail** against either real relay: that is how the suite proves it
would catch a relay that broke the contract, rather than vacuously passing. Run the
whole suite with `just conformance`, which runs the native half in-process always
and the Durable Object half against `wrangler dev` when `worker/node_modules`
exists. `just ci` stays Node-free.
