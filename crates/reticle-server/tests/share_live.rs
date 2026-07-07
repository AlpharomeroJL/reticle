//! Two-client live share-transport integration test over the real relay (ADR 0058).
//!
//! This is the authoritative, headless proof of the share-link live browser transport:
//! it binds a real [`reticle_server`] relay on an ephemeral port and connects a
//! **publisher** (Edit mode) and a **viewer** (View mode, `?mode=view`) as in-process
//! `tokio_tungstenite` peers, using exactly the wire framing the browser transport uses
//! (`reticle_sync::frame`: every frame is a `reticle_proto::v1::SyncMessage`). It proves
//! two things the whole feature rests on:
//!
//! 1. **The transport works end to end.** The publisher's `SyncMessage`-framed CRDT
//!    frames *and* its presence frame reach the viewer over the socket; the viewer
//!    decodes them with `frame::decode_frame` and materializes the sharer's geometry
//!    into a `SyncDocument` and the sharer's presence (cursor, selection, viewport).
//!    This is exactly what `reticle_app::livesync::ViewerTransport` does in the browser,
//!    minus the DOM.
//!
//! 2. **Read-only is enforced server-side.** A frame the View client sends is dropped by
//!    the relay: a second Edit peer never receives it, and it never enters the room log
//!    (a late joiner replays only the sharer's frames). This proves the server-side half
//!    of the read-only guarantee over a real socket, independent of any app-side rule.
//!
//! Modeled on `tests/relay.rs` (the opaque-bytes relay tests) and the demo server's
//! `tests_livewire.rs` (which materializes CRDT frames a watcher receives). Deterministic
//! and headless: no browser, no GPU.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, DrawShape, ShapeKind};
use reticle_server::{RelayState, serve};
use reticle_sync::{Frame, Presence, SyncDocument, decode_frame, encode_presence_frame};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

/// A connected client socket.
type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Grace period letting the server-side upgrade task subscribe to the room before a
/// peer publishes (the client handshake completing does not guarantee the spawned
/// `on_upgrade` callback has run). Matches `tests/relay.rs`.
const SUBSCRIBE_GRACE: Duration = Duration::from_millis(300);
/// Generous timeout for a frame that *should* arrive.
const RECV_TIMEOUT: Duration = Duration::from_secs(5);
/// Short timeout for asserting a frame must *not* arrive.
const NEGATIVE_TIMEOUT: Duration = Duration::from_millis(400);

/// The met1 layer (SKY130 layer 68, datatype 20), used for the sharer's rectangle.
fn met1() -> LayerId {
    LayerId::new(68, 20)
}

/// Binds the relay on an ephemeral port and spawns it, returning the bound address.
async fn spawn_relay() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("resolve local addr");
    tokio::spawn(async move {
        serve(listener, RelayState::new())
            .await
            .expect("relay serve");
    });
    addr
}

/// Connects an editor (read-write) WebSocket client to `ws://{addr}/ws/{room}`.
async fn connect_editor(addr: SocketAddr, room: &str) -> Client {
    let url = format!("ws://{addr}/ws/{room}");
    let (socket, _response) = connect_async(url).await.expect("editor connect");
    socket
}

/// Connects a **read-only** viewer client (`?mode=view`) to a room, exactly as
/// `reticle_app::share::viewer_ws_link` composes the browser viewer's URL.
async fn connect_viewer(addr: SocketAddr, room: &str) -> Client {
    let url = format!("ws://{addr}/ws/{room}?mode=view");
    let (socket, _response) = connect_async(url).await.expect("viewer connect");
    socket
}

/// Sends `payload` as one binary WebSocket frame.
async fn send_binary(client: &mut Client, payload: Vec<u8>) {
    client
        .send(Message::Binary(payload.into()))
        .await
        .expect("send binary");
}

/// Awaits the next binary payload on `client`, ignoring pings/pongs, or `None` on
/// timeout.
async fn recv_binary(client: &mut Client, dur: Duration) -> Option<Vec<u8>> {
    let deadline = tokio::time::Instant::now() + dur;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match timeout(remaining, client.next()).await {
            Ok(Some(Ok(Message::Binary(bytes)))) => return Some(bytes.to_vec()),
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(_)) | None) | Err(_) => return None,
        }
    }
}

/// The publisher's document: a cell `top` with one met1 rectangle, as a sharer would
/// have drawn it. Returns the sync document so the caller can produce framed updates.
fn sharer_document() -> SyncDocument {
    let mut doc = SyncDocument::new("sharer");
    let mut cell = Cell::new("top");
    cell.shapes.push(DrawShape::new(
        met1(),
        ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(400, 200))),
    ));
    doc.add_cell(&cell);
    doc
}

/// The full transport contract: a publisher's `SyncMessage`-framed CRDT frame and
/// presence frame both reach a read-only viewer, which materializes the sharer's
/// geometry and presence exactly as the browser viewer transport does.
#[tokio::test]
async fn publisher_crdt_and_presence_reach_the_read_only_viewer() {
    let addr = spawn_relay().await;
    let room = "share-live";

    // The viewer joins read-only first, so every frame is delivered live (no
    // dependence on room-log replay timing), mirroring tests_livewire.rs.
    let mut viewer = connect_viewer(addr, room).await;
    let mut publisher = connect_editor(addr, room).await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    // The sharer publishes (a) its document as a CRDT frame and (b) its presence, each
    // framed exactly as reticle_app::livesync::SharerTransport frames them.
    let sharer = sharer_document();
    let update_frame = reticle_sync::encode_update_frame(&sharer.encode_state_update());
    send_binary(&mut publisher, update_frame).await;

    let mut presence = Presence::new("sharer");
    presence.display_name = "Sharer".to_owned();
    presence.cursor = Point::new(120, -45);
    presence.selection = vec!["top/shape-1".to_owned()];
    presence.viewport = Rect::new(Point::new(-1000, -2000), Point::new(3000, 4000));
    send_binary(&mut publisher, encode_presence_frame(&presence)).await;

    // The viewer decodes each binary frame as a SyncMessage and routes on the variant,
    // materializing the sharer's geometry into a mirror doc and recording presence.
    // This is exactly ViewerTransport's onmessage -> ViewerSession path, headless.
    let mut mirror = SyncDocument::new("viewer");
    let mut got_geometry = false;
    let mut got_presence: Option<Presence> = None;

    // Two frames were sent; loop until both are classified or we time out.
    while !(got_geometry && got_presence.is_some()) {
        let Some(bytes) = recv_binary(&mut viewer, RECV_TIMEOUT).await else {
            break;
        };
        match decode_frame(&bytes).expect("a viewer frame is a valid SyncMessage") {
            Frame::Update(raw) => {
                mirror.apply_update(&raw).expect("raw yrs bytes apply");
                got_geometry = true;
            }
            Frame::Presence(p) => got_presence = Some(p),
            Frame::Comment(_) => {}
        }
    }

    // The sharer's geometry materialized on the viewer side.
    assert!(got_geometry, "the viewer received the CRDT update frame");
    let cell = mirror
        .document()
        .cell("top")
        .expect("the sharer's cell `top` reached the viewer");
    assert_eq!(
        cell.shapes.len(),
        1,
        "the sharer's one met1 rectangle reached the viewer"
    );

    // The sharer's presence (cursor, selection, viewport) materialized on the viewer.
    let seen = got_presence.expect("the viewer received the presence frame");
    assert_eq!(seen, presence, "the whole presence arrived intact");
    assert_eq!(seen.cursor, Point::new(120, -45));
    assert_eq!(seen.selection, vec!["top/shape-1".to_owned()]);
    assert_eq!(
        seen.viewport,
        Rect::new(Point::new(-1000, -2000), Point::new(3000, 4000)),
        "the viewport that follow-mode rides on arrived intact"
    );
}

/// Read-only, server-side: a frame the View client sends is dropped by the relay. A
/// second Edit peer never receives it, proving the server enforces read-only over a
/// real socket regardless of any app-side rule.
#[tokio::test]
async fn a_viewer_frame_is_dropped_by_the_relay_and_never_reaches_an_editor() {
    let addr = spawn_relay().await;
    let room = "share-live-ro";

    let mut publisher = connect_editor(addr, room).await;
    let mut viewer = connect_viewer(addr, room).await;
    // A second editor observes the room so we can prove the viewer's frame never fans
    // out to a real participant.
    let mut other = connect_editor(addr, room).await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    // The publisher's real frame reaches both the viewer and the other editor.
    let sharer = sharer_document();
    let real_frame = reticle_sync::encode_update_frame(&sharer.encode_state_update());
    send_binary(&mut publisher, real_frame).await;

    let at_viewer = recv_binary(&mut viewer, RECV_TIMEOUT).await;
    assert!(
        at_viewer.is_some_and(|b| matches!(decode_frame(&b), Ok(Frame::Update(_)))),
        "the viewer receives the sharer's live CRDT frame"
    );
    let at_other = recv_binary(&mut other, RECV_TIMEOUT).await;
    assert!(
        at_other.is_some_and(|b| matches!(decode_frame(&b), Ok(Frame::Update(_)))),
        "the other editor also receives the sharer's frame"
    );

    // The viewer attempts to publish a *well-formed* edit frame. Even though it is a
    // valid SyncMessage the relay would forward from an editor, the View mode drops it.
    let mut viewer_doc = SyncDocument::new("sneaky-viewer");
    viewer_doc.add_cell(&Cell::new("viewer-injected"));
    let viewer_frame = reticle_sync::encode_update_frame(&viewer_doc.encode_state_update());
    send_binary(&mut viewer, viewer_frame).await;

    // It must never reach the other editor.
    let leaked = recv_binary(&mut other, NEGATIVE_TIMEOUT).await;
    assert_eq!(
        leaked, None,
        "a read-only viewer's frame must never be broadcast to another editor"
    );
}

/// Read-only, server-side: a frame the View client sends is not even recorded in the
/// room log. A peer joining *after* the viewer tried to publish replays only the
/// sharer's frames.
#[tokio::test]
async fn a_viewer_frame_never_enters_the_room_log() {
    let addr = spawn_relay().await;
    let room = "share-live-log";

    let mut publisher = connect_editor(addr, room).await;
    let mut viewer = connect_viewer(addr, room).await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    // The sharer records one real frame; the viewer attempts to record another.
    let sharer = sharer_document();
    let real_frame = reticle_sync::encode_update_frame(&sharer.encode_state_update());
    send_binary(&mut publisher, real_frame).await;

    let mut viewer_doc = SyncDocument::new("sneaky-viewer");
    viewer_doc.add_cell(&Cell::new("viewer-injected"));
    let viewer_frame = reticle_sync::encode_update_frame(&viewer_doc.encode_state_update());
    send_binary(&mut viewer, viewer_frame).await;

    // Let the relay process both before a late joiner reads the log.
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    // A late joiner replays the log and reconstructs the document. It must contain the
    // sharer's cell and NOT the viewer's injected cell.
    let mut latecomer = connect_editor(addr, room).await;
    let mut replayed = SyncDocument::new("latecomer");
    // Drain every logged frame (a short window: the log is replayed up front).
    while let Some(bytes) = recv_binary(&mut latecomer, NEGATIVE_TIMEOUT).await {
        if let Ok(Frame::Update(raw)) = decode_frame(&bytes) {
            replayed.apply_update(&raw).expect("logged frame applies");
        }
    }

    assert!(
        replayed.document().cell("top").is_some(),
        "the log replays the sharer's frame"
    );
    assert!(
        replayed.document().cell("viewer-injected").is_none(),
        "the viewer's dropped frame must never appear in the replayed log"
    );
}

/// A second met1 rectangle, the "offline edit B" a sharer makes while its socket is down.
fn offline_edit_rect() -> DrawShape {
    DrawShape::new(
        met1(),
        ShapeKind::Rect(Rect::new(Point::new(500, 0), Point::new(900, 200))),
    )
}

/// Kill-and-reconnect resync (ADR 0063): a sharer publishes edit A; its socket drops; it
/// makes edit B offline in its own [`SyncDocument`]; it reconnects with a *new* connection
/// and publishes one full-state snapshot. A viewer that stayed connected materializes A+B
/// **exactly once** (two shapes, not three: the snapshot re-carries A's content but `yrs`
/// updates are idempotent), and a viewer joining fresh afterward sees the same A+B via the
/// relay's log replay (no client-side reconnect code, just the snapshot frame plus replay).
#[tokio::test]
async fn sharer_reconnect_full_state_resyncs_the_viewer_exactly_once() {
    let addr = spawn_relay().await;
    let room = "share-live-reconnect";

    // The viewer joins first so every live frame reaches it (no replay-timing dependence).
    let mut viewer = connect_viewer(addr, room).await;
    let mut publisher = connect_editor(addr, room).await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    // Edit A: the sharer's cell `top` with one met1 rectangle, framed as the transport does.
    let mut sharer = sharer_document();
    let frame_a = reticle_sync::encode_update_frame(&sharer.encode_full_state());
    send_binary(&mut publisher, frame_a).await;

    // The viewer materializes A: one shape.
    let mut mirror = SyncDocument::new("viewer");
    let a = recv_binary(&mut viewer, RECV_TIMEOUT)
        .await
        .expect("the viewer receives edit A");
    let Frame::Update(raw_a) = decode_frame(&a).expect("A is a SyncMessage") else {
        panic!("expected an update frame for A");
    };
    mirror.apply_update(&raw_a).expect("A applies");
    assert_eq!(
        mirror.document().cell("top").expect("top").shapes.len(),
        1,
        "the viewer has edit A before the drop"
    );

    // The socket drops: the sharer's client goes away, but the room and its log persist.
    drop(publisher);
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    // Offline edit B: a second rectangle into the sharer's own SyncDocument, no socket.
    sharer.add_shape("top", &offline_edit_rect());

    // The sharer reconnects with a NEW connection and publishes one full-state snapshot.
    let mut reconnected = connect_editor(addr, room).await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;
    let full_state = reticle_sync::encode_update_frame(&sharer.encode_full_state());
    send_binary(&mut reconnected, full_state).await;

    // The still-connected viewer applies the snapshot and now has A+B, exactly once: two
    // shapes, not three, because re-carrying A's content is an idempotent no-op.
    let snapshot = recv_binary(&mut viewer, RECV_TIMEOUT)
        .await
        .expect("the viewer receives the full-state snapshot");
    let Frame::Update(raw_full) = decode_frame(&snapshot).expect("snapshot is a SyncMessage")
    else {
        panic!("expected an update frame for the snapshot");
    };
    mirror.apply_update(&raw_full).expect("snapshot applies");
    assert_eq!(
        mirror.document().cell("top").expect("top").shapes.len(),
        2,
        "the viewer materializes A+B exactly once (no duplication)"
    );

    // A viewer joining fresh replays the room log (frame A + the snapshot) and reconstructs
    // the same A+B, proving late joiners resync via replay without any reconnect client code.
    let mut latecomer = connect_viewer(addr, room).await;
    let mut replayed = SyncDocument::new("latecomer");
    while let Some(bytes) = recv_binary(&mut latecomer, NEGATIVE_TIMEOUT).await {
        if let Ok(Frame::Update(raw)) = decode_frame(&bytes) {
            replayed.apply_update(&raw).expect("logged frame applies");
        }
    }
    assert_eq!(
        replayed.document().cell("top").expect("top").shapes.len(),
        2,
        "a fresh viewer sees A+B via the relay's log replay"
    );
}

/// Negative control for the resync (ADR 0063): if the reconnecting sharer *skips* the
/// full-state snapshot (the pre-hardening behavior, where a dropped socket silently lost
/// the offline edit), the viewer is left missing edit B. This is what makes the snapshot
/// frame load-bearing: remove it and the reconnect test above would fail.
#[tokio::test]
async fn without_the_full_state_frame_the_viewer_misses_the_offline_edit() {
    let addr = spawn_relay().await;
    let room = "share-live-no-resync";

    let mut viewer = connect_viewer(addr, room).await;
    let mut publisher = connect_editor(addr, room).await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    // Edit A reaches the viewer, as before.
    let mut sharer = sharer_document();
    let frame_a = reticle_sync::encode_update_frame(&sharer.encode_full_state());
    send_binary(&mut publisher, frame_a).await;

    let mut mirror = SyncDocument::new("viewer");
    let a = recv_binary(&mut viewer, RECV_TIMEOUT)
        .await
        .expect("the viewer receives edit A");
    if let Ok(Frame::Update(raw)) = decode_frame(&a) {
        mirror.apply_update(&raw).expect("A applies");
    }
    assert_eq!(mirror.document().cell("top").expect("top").shapes.len(), 1);

    // The socket drops; edit B is made offline but the reconnected sharer publishes only
    // presence (the buggy path that skips the snapshot), never the document.
    drop(publisher);
    tokio::time::sleep(SUBSCRIBE_GRACE).await;
    sharer.add_shape("top", &offline_edit_rect());

    let mut reconnected = connect_editor(addr, room).await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;
    let mut presence = Presence::new("sharer");
    presence.cursor = Point::new(1, 2);
    send_binary(&mut reconnected, encode_presence_frame(&presence)).await;

    // No CRDT frame arrives after reconnect, so the viewer never learns about B.
    let mut got_update_after_reconnect = false;
    while let Some(bytes) = recv_binary(&mut viewer, NEGATIVE_TIMEOUT).await {
        if let Ok(Frame::Update(raw)) = decode_frame(&bytes) {
            mirror.apply_update(&raw).expect("applies");
            got_update_after_reconnect = true;
        }
    }
    assert!(
        !got_update_after_reconnect,
        "skipping the snapshot means no geometry resync frame is sent"
    );
    assert_eq!(
        mirror.document().cell("top").expect("top").shapes.len(),
        1,
        "without the full-state frame the viewer is left missing offline edit B"
    );
}
