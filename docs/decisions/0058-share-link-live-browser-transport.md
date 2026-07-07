# 0058, The share-link live browser transport: one SyncMessage framing, two transports, read-only enforced twice

## Context

ADR 0038 built the read-only viewer as a socket-free state machine
(`reticle_app::viewer::ViewerSession`) and the share links that address a relay room,
but nothing dialed the socket in the browser: a shared read-only link generated a URL
and streamed nothing. The relay (`reticle-server`, ADR 0007/0038) already fans opaque
binary frames out to a room and enforces `?mode=view` read-only server-side; the CRDT
(`reticle-sync`, ADR 0007) already encodes and applies `yrs` update bytes; the
`SyncMessage` proto envelope already exists (spec Section 6). What was missing was the
wasm `web_sys::WebSocket` glue that connects a browser tab to a room and pumps frames,
and a decision about what actually flows over that socket, since a viewer must
materialize *both* the sharer's geometry and the sharer's live cursor, selection, and
viewport over one undistinguished binary stream.

Three sub-decisions had to be pinned together so the sharer, the viewer, and the one
pre-existing publisher (the demo agent harness, ADR 0022) all agree.

## Decision

**1. One wire framing: every published frame is a `reticle_proto::v1::SyncMessage`
envelope.** A document delta travels as `SyncMessage{ Update(CrdtUpdate{ update: raw
yrs v1 bytes }) }`; a presence update travels as `SyncMessage{ Presence(..) }`. The
receiver decodes each binary frame as a `SyncMessage` and routes on the oneof variant:
an update's raw bytes go to `SyncDocument::apply_update` (equivalently
`ViewerSession::apply_frame`), a presence goes into the awareness map. This is the only
way to multiplex the two message types the viewer needs over the single opaque channel
the relay hands over; it reuses the frozen proto, so it adds no schema. The codec is a
new pure module, `reticle_sync::frame` (`encode_update_frame`, `encode_presence_frame`,
`decode_frame` returning a `Frame` enum), so both sides and both targets share exactly
one implementation. It builds for `wasm32`.

The one pre-existing publisher, the demo harness (`reticle-demo-server`), previously
shipped **raw `yrs` bytes** directly (its presence never actually crossed the socket,
it was written only into a local awareness map). To keep the whole system on one
coherent format rather than a split wire, its `stream_frame` now wraps the CRDT bytes
with `frame::encode_update_frame`, and the live-wiring test decodes with
`frame::decode_frame`. This is the "update that path consistently" option: a
mixed-format wire would be a latent bug the first time a browser viewer joined a
demo room.

**2. Two transports, structurally asymmetric.** A new `reticle_app::livesync` module
(cfg-gated for wasm with native no-op stubs, mirroring `webopen.rs`) owns two types:

* `ViewerTransport` opens a `web_sys::WebSocket` to `viewer_ws_link(relay, room)` (which
  carries `?mode=view`), sets `binary_type = Arraybuffer`, and on each message decodes a
  `SyncMessage` and posts a routed `LiveEvent` into a shared inbox the egui loop drains
  each frame (the same inbox pattern `webopen.rs` uses). It has **no send-document path
  at all**: the type exposes no publish method and holds no `SyncDocument`, so it is
  structurally impossible for a viewer to publish. It may send nothing.
* `SharerTransport` opens a `web_sys::WebSocket` to `room_link(relay, room)` (Edit mode)
  and exposes `publish_update(bytes)` and `publish_presence(&Presence)`, each of which
  frames via `reticle_sync::frame` and sends one binary frame.

**3. Read-only is enforced twice, independently.** Server-side, the relay drops any
binary frame from a `?mode=view` connection (never logged, never broadcast; ADR 0038,
`handle_socket`). App-side, the viewer transport has no code path that sends a document
frame, and the `ViewerSession` is only ever fed, never drained to a socket. Either
guarantee alone would suffice; both together make a shared session safe to hand to an
untrusted viewer. The authoritative proof is a headless Rust relay integration test
(`crates/reticle-server/tests/share_live.rs`) that connects a real publisher (Edit) and
a real viewer (View) as in-process `tokio_tungstenite` peers over an ephemeral port and
asserts (a) the publisher's `SyncMessage`-framed CRDT and presence frames reach the
viewer and materialize the sharer's geometry and presence, and (b) a frame a View client
sends is dropped by the relay, never reaching a second Edit peer and never entering the
room log.

## Consequences

* The wire format is now uniform and self-describing: any peer, in any target, frames
  and decodes through `reticle_sync::frame`. A browser viewer can join a demo-harness
  room and a human-shared room identically.
* The demo harness and its `tests_livewire` were updated in lockstep; the change is
  additive to the harness behavior (same CRDT content, now enveloped) and its
  convergence assertions are unchanged in intent.
* The app-side viewer renders the mirror by installing `ViewerSession::document()`
  through the existing `install_document` document-swap seam and feeding the sharer's
  presence into the awareness map the existing `draw_presence` already reads, so the
  live view reuses the whole render pipeline and the app.rs change stays small. Because
  `App::document` (the collaboration mirror) is not kept in step with local edits (it is
  rebuilt only on a document swap), the sharer transport re-encodes state from the
  editable `history.document()` when publishing rather than trusting that mirror.
* The read-only contract has a hard, deterministic, headless test that does not depend
  on a browser. The Playwright two-context e2e is the browser-level proof that the wasm
  bundle boots as a viewer and the socket carries frames; where headless WebGL2 limits
  what can be asserted about pixels, the e2e asserts what is genuinely observable (the
  bundle boots as a viewer, the socket opens, frames arrive) and the Rust test remains
  the authority on the transport and read-only behavior.
* Cost: `reticle-sync` gains a direct `prost` dependency (already in the lockfile via
  `reticle-proto`) to encode and decode the envelope. The viewer applies raw `yrs` bytes
  regardless of the `CrdtUpdate.doc_id`/`actor` fields, which are informational.
