//! Integration tests for immutable snapshot permalinks (`reticle_server::snapshot`).
//!
//! Mirrors `tests/relay.rs`'s style: bind the relay on an ephemeral port, drive it
//! with real `tokio-tungstenite` clients, and treat it as a black box over the
//! wire. These are the server-side proof for the lane's two required guarantees: a
//! snapshot permalink round-trips the exact geometry it captured, and a snapshot
//! taken at revision N keeps serving N's geometry even after the live room accepts
//! more edits.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, DrawShape, ShapeKind};
use reticle_server::{RelayState, serve};
use reticle_sync::{Frame, SyncDocument, decode_frame, encode_update_frame};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

/// A connected client socket.
type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Grace period letting the server-side upgrade task subscribe or record before a
/// later step depends on it having done so. Matches `tests/relay.rs`.
const SUBSCRIBE_GRACE: Duration = Duration::from_millis(300);
/// Generous timeout for a frame (or a clean close) that *should* arrive.
const RECV_TIMEOUT: Duration = Duration::from_secs(5);
/// Short timeout for asserting a frame must *not* arrive (used against a *live*
/// join, which never closes on its own, so this bound is really hit).
const NEGATIVE_TIMEOUT: Duration = Duration::from_millis(400);

/// The met1 layer (SKY130 layer 68, datatype 20), used for test rectangles.
fn met1() -> LayerId {
    LayerId::new(68, 20)
}

/// Binds the relay on an ephemeral port and spawns it, returning the bound address
/// and a cloned handle to the exact [`RelayState`] backing it, so a test can cross-
/// check the wire-reported revision against the library surface directly (the same
/// reason `tests/relay.rs`'s persist-hook test keeps its own handle to state it
/// passed into the spawned server).
async fn spawn_relay() -> (SocketAddr, RelayState) {
    let state = RelayState::new();
    let handle = state.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("resolve local addr");
    tokio::spawn(async move {
        serve(listener, state).await.expect("relay serve");
    });
    (addr, handle)
}

/// Connects an editor (read-write) client to `ws://{addr}/ws/{room}`.
async fn connect_editor(addr: SocketAddr, room: &str) -> Client {
    let url = format!("ws://{addr}/ws/{room}");
    let (socket, _response) = connect_async(url).await.expect("editor connect");
    socket
}

/// Connects to the "current revision" route, `GET /snapshot/{room}`: the capture
/// step, [`reticle_server::snapshot::revision_handler`] (private, exercised only
/// through the route here, as the black-box style of this test file intends).
async fn connect_revision(addr: SocketAddr, room: &str) -> Client {
    let url = format!("ws://{addr}/snapshot/{room}");
    let (socket, _response) = connect_async(url).await.expect("revision connect");
    socket
}

/// Connects to one immutable snapshot, `GET /snapshot/{room}/at/{revision}`.
async fn connect_snapshot(addr: SocketAddr, room: &str, revision: usize) -> Client {
    let url = format!("ws://{addr}/snapshot/{room}/at/{revision}");
    let (socket, _response) = connect_async(url).await.expect("snapshot connect");
    socket
}

/// Sends `payload` as one binary WebSocket frame.
async fn send_binary(client: &mut Client, payload: &[u8]) {
    client
        .send(Message::Binary(payload.to_vec().into()))
        .await
        .expect("send binary");
}

/// Best-effort binary send that ignores a failure (used only where the point of
/// the test is what a witness observes afterward, not whether this particular send
/// itself succeeds against a connection that may already be closing).
async fn try_send_binary(client: &mut Client, payload: &[u8]) {
    let _ = client.send(Message::Binary(payload.to_vec().into())).await;
}

/// Awaits the next binary payload on `client`, ignoring control frames, or `None`
/// on timeout or once the stream ends (a clean close included).
async fn recv_binary(client: &mut Client, dur: Duration) -> Option<Vec<u8>> {
    let deadline = tokio::time::Instant::now() + dur;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match timeout(remaining, client.next()).await {
            Ok(Some(Ok(Message::Binary(bytes)))) => return Some(bytes.to_vec()),
            // Close/ping/text are not payloads; loop again (a Close is followed by
            // the stream ending, which the next iteration observes as `None`).
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(_)) | None) | Err(_) => return None,
        }
    }
}

/// Drains binary frames from `client` (bounded by `dur`, see [`recv_binary`]),
/// decoding each as a live-session frame and applying only the update ones into
/// `into` (exactly how `reticle_app::livesync::route_frame` routes a live frame,
/// and how a viewer or a reopened snapshot materializes geometry from the wire).
async fn drain_and_apply(client: &mut Client, into: &mut SyncDocument, dur: Duration) {
    while let Some(bytes) = recv_binary(client, dur).await {
        if let Ok(Frame::Update(raw)) = decode_frame(&bytes) {
            into.apply_update(&raw)
                .expect("a replayed frame is a valid update");
        }
    }
}

/// Reads the one revision reply `GET /snapshot/{room}` sends and parses it as a
/// decimal integer.
async fn read_revision(client: &mut Client) -> usize {
    let bytes = recv_binary(client, RECV_TIMEOUT)
        .await
        .expect("the revision route replies with one binary frame");
    std::str::from_utf8(&bytes)
        .expect("revision is ASCII decimal")
        .parse()
        .expect("revision parses as an integer")
}

/// A `SyncDocument` publishing a cell `top` with one met1 rectangle: "edit A".
fn edit_a() -> SyncDocument {
    let mut doc = SyncDocument::new("sharer");
    let mut cell = Cell::new("top");
    cell.shapes.push(DrawShape::new(
        met1(),
        ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(400, 200))),
    ));
    doc.add_cell(&cell);
    doc
}

/// Adds a second, distinct cell to an existing sync document: "edit B".
fn add_edit_b(doc: &mut SyncDocument) {
    let mut cell = Cell::new("added-later");
    cell.shapes.push(DrawShape::new(
        met1(),
        ShapeKind::Rect(Rect::new(Point::new(500, 0), Point::new(900, 200))),
    ));
    doc.add_cell(&cell);
}

/// The flagship immutability proof: a snapshot captured at revision N keeps
/// serving exactly N's geometry after the live room accepts further edits, while a
/// fresh live joiner (which was never frozen) sees everything. The wire-reported
/// revision also agrees with the library-level `RelayState::current_revision`.
#[tokio::test]
async fn snapshot_at_revision_still_serves_that_revision_after_further_edits() {
    let (addr, state) = spawn_relay().await;
    let room = "immutable-doc";

    let mut editor = connect_editor(addr, room).await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    // Edit A lands, and the client "captures" a snapshot right after it.
    let mut doc = edit_a();
    send_binary(
        &mut editor,
        &encode_update_frame(&doc.encode_state_update()),
    )
    .await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    let mut revision_client = connect_revision(addr, room).await;
    let revision_at_a = read_revision(&mut revision_client).await;
    assert_eq!(
        revision_at_a,
        state.current_revision(room),
        "the wire-reported revision agrees with the library surface"
    );
    assert!(revision_at_a > 0, "edit A was recorded before the capture");

    // The live room keeps changing after the capture: edit B lands afterward.
    add_edit_b(&mut doc);
    send_binary(
        &mut editor,
        &encode_update_frame(&doc.encode_state_update()),
    )
    .await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    let mut revision_client_b = connect_revision(addr, room).await;
    let revision_at_b = read_revision(&mut revision_client_b).await;
    assert!(
        revision_at_b > revision_at_a,
        "the live room's revision must advance past the captured one"
    );

    // Opening the permalink captured at revision_at_a must reproduce ONLY edit A,
    // no matter that the live room has since moved on to revision_at_b.
    let mut snapshot = connect_snapshot(addr, room, revision_at_a).await;
    let mut reconstructed = SyncDocument::new("opened-snapshot");
    drain_and_apply(&mut snapshot, &mut reconstructed, RECV_TIMEOUT).await;

    assert!(
        reconstructed.document().cell("top").is_some(),
        "the snapshot has edit A's geometry"
    );
    assert!(
        reconstructed.document().cell("added-later").is_none(),
        "edit B, recorded after the snapshot was captured, must never appear in it"
    );

    // A fresh live joiner, by contrast, sees both edits: the room itself was never
    // frozen, only the snapshot's own frozen copy of history is.
    let mut live_joiner = connect_editor(addr, room).await;
    let mut live_view = SyncDocument::new("live-joiner");
    drain_and_apply(&mut live_joiner, &mut live_view, NEGATIVE_TIMEOUT).await;
    assert!(live_view.document().cell("top").is_some());
    assert!(
        live_view.document().cell("added-later").is_some(),
        "a live joiner sees edit B; only the snapshot is frozen before it"
    );
}

/// Permalink round-trip (success bar #1): capture the current revision, serialize
/// it as the permalink URL, and open that URL fresh; the reconstructed document
/// must equal exactly what was captured.
#[tokio::test]
async fn permalink_round_trip_capture_serialize_open_reproduces_the_same_geometry() {
    let (addr, _state) = spawn_relay().await;
    let room = "permalink-room";

    // Capture: publish the geometry, then read the room's current revision.
    let mut editor = connect_editor(addr, room).await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;
    let doc = edit_a();
    send_binary(
        &mut editor,
        &encode_update_frame(&doc.encode_state_update()),
    )
    .await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    let mut revision_client = connect_revision(addr, room).await;
    let revision = read_revision(&mut revision_client).await;

    // Serialize: the permalink names nothing but (room, revision) as two URL path
    // segments (`reticle_app::snapshot::snapshot_ws_link` composes the same shape
    // app-side; this test proves the route it names actually serves what was
    // captured).
    let permalink = format!("ws://{addr}/snapshot/{room}/at/{revision}");

    // Open: dial the permalink fresh and reconstruct the document from the replay.
    let (mut opened, _response) = connect_async(&permalink).await.expect("open permalink");
    let mut reconstructed = SyncDocument::new("opened");
    drain_and_apply(&mut opened, &mut reconstructed, RECV_TIMEOUT).await;

    assert_eq!(
        reconstructed.document(),
        doc.document(),
        "opening the permalink reproduces exactly the captured geometry"
    );
}

/// A snapshot connection cannot mutate the room: a frame sent over it is never
/// recorded in the room log and never reaches another peer, because the handler
/// never reads an inbound frame from a snapshot connection at all.
#[tokio::test]
async fn a_snapshot_connections_frames_are_never_recorded_or_broadcast() {
    let (addr, _state) = spawn_relay().await;
    let room = "snap-readonly";

    let mut editor = connect_editor(addr, room).await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;
    send_binary(&mut editor, b"real-frame").await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    // Dial the snapshot route and attempt to publish through it.
    let mut sneaky = connect_snapshot(addr, room, 1).await;
    try_send_binary(&mut sneaky, b"sneaky-from-snapshot").await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    // A fresh live joiner replays the room log; it must contain only the real
    // editor's frame, never anything from the snapshot connection.
    let mut witness = connect_editor(addr, room).await;
    let first = recv_binary(&mut witness, RECV_TIMEOUT).await;
    assert_eq!(first.as_deref(), Some(b"real-frame".as_slice()));
    let second = recv_binary(&mut witness, NEGATIVE_TIMEOUT).await;
    assert_eq!(
        second, None,
        "a snapshot connection's frame must never enter the room log"
    );
}

/// A snapshot of a room nobody has ever joined replays nothing (an empty log) and
/// does not create the room, mirroring `RelayState::current_revision`'s "never
/// vivifies" contract.
#[tokio::test]
async fn snapshot_of_a_never_joined_room_replays_nothing() {
    let (addr, state) = spawn_relay().await;

    let mut snapshot = connect_snapshot(addr, "ghost-room", 5).await;
    let frame = recv_binary(&mut snapshot, NEGATIVE_TIMEOUT).await;
    assert_eq!(frame, None, "a room nobody joined has nothing to replay");
    assert_eq!(
        state.current_revision("ghost-room"),
        0,
        "checking a snapshot must not itself create the room"
    );
}

/// The `GET /snapshot/{room}` capture route reports `0` for an untouched room and
/// tracks accepted frames one for one afterward.
#[tokio::test]
async fn revision_route_reports_zero_then_tracks_edits() {
    let (addr, _state) = spawn_relay().await;
    let room = "counted";

    let mut fresh = connect_revision(addr, room).await;
    assert_eq!(
        read_revision(&mut fresh).await,
        0,
        "an untouched room starts at revision 0"
    );

    let mut editor = connect_editor(addr, room).await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;
    send_binary(&mut editor, b"one-frame").await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    let mut after = connect_revision(addr, room).await;
    assert_eq!(read_revision(&mut after).await, 1);
}

/// A revision requested beyond the room's current log clamps to the whole log
/// (over the wire, complementing the `log_prefix` unit test in the crate).
#[tokio::test]
async fn a_snapshot_requested_past_the_current_log_serves_the_whole_log() {
    let (addr, _state) = spawn_relay().await;
    let room = "clamped";

    let mut editor = connect_editor(addr, room).await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;
    let doc = edit_a();
    send_binary(
        &mut editor,
        &encode_update_frame(&doc.encode_state_update()),
    )
    .await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    // Ask for a revision far beyond anything recorded.
    let mut snapshot = connect_snapshot(addr, room, 999_999).await;
    let mut reconstructed = SyncDocument::new("opened");
    drain_and_apply(&mut snapshot, &mut reconstructed, RECV_TIMEOUT).await;

    assert_eq!(
        reconstructed.document(),
        doc.document(),
        "an out-of-range revision clamps to the whole log rather than erroring"
    );
}
