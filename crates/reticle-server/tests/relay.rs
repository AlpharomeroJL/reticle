//! Integration tests for the `reticle-server` WebSocket relay.
//!
//! Each test binds the relay on an ephemeral port (`127.0.0.1:0`), connects one
//! or more real WebSocket clients with `tokio-tungstenite`, and asserts the
//! relay's forwarding, isolation, and catch-up behaviour. The relay is treated
//! as a black box over the wire; these tests exercise the public
//! [`reticle_server::serve`] / [`reticle_server::RelayState`] surface only.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use reticle_server::{RelayState, serve};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

/// A connected client socket.
type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Binds the relay on an ephemeral port and spawns it, returning the bound
/// address. The server task runs until the test process exits.
async fn spawn_relay() -> SocketAddr {
    spawn_relay_with(RelayState::new()).await
}

/// Like [`spawn_relay`] but serves a caller-provided [`RelayState`].
async fn spawn_relay_with(state: RelayState) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("resolve local addr");
    tokio::spawn(async move {
        serve(listener, state).await.expect("relay serve");
    });
    addr
}

/// Connects a WebSocket client to `ws://{addr}/ws/{room}`.
async fn connect(addr: SocketAddr, room: &str) -> Client {
    let url = format!("ws://{addr}/ws/{room}");
    let (socket, _response) = connect_async(url).await.expect("client connect");
    socket
}

/// Connects a **read-only** WebSocket client (`?mode=view`) to a room.
async fn connect_viewer(addr: SocketAddr, room: &str) -> Client {
    let url = format!("ws://{addr}/ws/{room}?mode=view");
    let (socket, _response) = connect_async(url).await.expect("viewer connect");
    socket
}

/// Sends `payload` as a binary WebSocket frame.
async fn send_binary(client: &mut Client, payload: &[u8]) {
    client
        .send(Message::Binary(payload.to_vec().into()))
        .await
        .expect("send binary");
}

/// Awaits the next binary payload on `client`, returning `None` if no binary
/// frame arrives within `dur` (ignoring pings/pongs).
async fn recv_binary(client: &mut Client, dur: Duration) -> Option<Vec<u8>> {
    let deadline = tokio::time::Instant::now() + dur;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match timeout(remaining, client.next()).await {
            Ok(Some(Ok(Message::Binary(bytes)))) => return Some(bytes.to_vec()),
            // Control/text frames are not relay payloads; keep waiting.
            Ok(Some(Ok(_))) => {}
            // Stream ended, errored, or timed out: no binary frame arrived.
            Ok(Some(Err(_)) | None) | Err(_) => return None,
        }
    }
}

/// Grace period allowing the server-side upgrade task to subscribe to the room
/// before a peer publishes. The WebSocket handshake completing on the client
/// does not guarantee the spawned `on_upgrade` callback has run yet.
const SUBSCRIBE_GRACE: Duration = Duration::from_millis(300);
/// Generous timeout for a message that *should* arrive.
const RECV_TIMEOUT: Duration = Duration::from_secs(5);
/// Short timeout for asserting a message must *not* arrive.
const NEGATIVE_TIMEOUT: Duration = Duration::from_millis(400);

/// A message from one peer reaches the other peer in the same room, and the
/// sender does not receive its own echo.
#[tokio::test]
async fn broadcasts_to_peer_but_not_to_sender() {
    let addr = spawn_relay().await;

    let mut alice = connect(addr, "doc").await;
    let mut bob = connect(addr, "doc").await;

    // Let both server-side connections subscribe before Alice publishes.
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    let payload = b"hello-room".to_vec();
    send_binary(&mut alice, &payload).await;

    // Bob (the other peer) receives it.
    let received = recv_binary(&mut bob, RECV_TIMEOUT).await;
    assert_eq!(
        received.as_deref(),
        Some(payload.as_slice()),
        "peer B must receive A's message"
    );

    // Alice does not receive her own echo.
    let echo = recv_binary(&mut alice, NEGATIVE_TIMEOUT).await;
    assert_eq!(echo, None, "sender A must not receive its own echo");
}

/// Two rooms are isolated: a message published in room 1 is never delivered to a
/// peer in room 2.
#[tokio::test]
async fn rooms_are_isolated() {
    let addr = spawn_relay().await;

    let mut in_room_1 = connect(addr, "room-1").await;
    let mut in_room_2 = connect(addr, "room-2").await;

    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    send_binary(&mut in_room_1, b"room-1-only").await;

    let leaked = recv_binary(&mut in_room_2, NEGATIVE_TIMEOUT).await;
    assert_eq!(leaked, None, "a message in room 1 must not reach room 2");
}

/// A peer joining a non-empty room receives the room's existing state (the
/// replayed update log) before any live traffic.
#[tokio::test]
async fn late_joiner_receives_initial_state() {
    let addr = spawn_relay().await;

    // Alice joins and publishes two updates that become the room's state.
    let mut alice = connect(addr, "history").await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;
    send_binary(&mut alice, b"update-1").await;
    send_binary(&mut alice, b"update-2").await;

    // Give the relay time to record both updates into the room log.
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    // Bob joins late and must receive the full log, in order, up front.
    let mut bob = connect(addr, "history").await;

    let first = recv_binary(&mut bob, RECV_TIMEOUT).await;
    let second = recv_binary(&mut bob, RECV_TIMEOUT).await;
    assert_eq!(first.as_deref(), Some(b"update-1".as_slice()));
    assert_eq!(second.as_deref(), Some(b"update-2".as_slice()));
}

/// A read-only viewer (`?mode=view`) receives the sharer's live frames, but the
/// frames the viewer itself sends are dropped: they never reach another peer, so
/// a viewer cannot mutate the shared session.
#[tokio::test]
async fn read_only_viewer_receives_but_cannot_publish() {
    let addr = spawn_relay().await;

    // The sharer edits; the viewer joins read-only; a second editor observes the
    // room so we can prove the viewer's frame never fans out.
    let mut sharer = connect(addr, "shared").await;
    let mut viewer = connect_viewer(addr, "shared").await;
    let mut other = connect(addr, "shared").await;

    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    // The sharer's edit reaches the read-only viewer.
    send_binary(&mut sharer, b"sharer-frame").await;
    let seen = recv_binary(&mut viewer, RECV_TIMEOUT).await;
    assert_eq!(
        seen.as_deref(),
        Some(b"sharer-frame".as_slice()),
        "a read-only viewer must receive the sharer's live frames"
    );
    // The other editor also saw it (drain it so its buffer is clean below).
    let other_seen = recv_binary(&mut other, RECV_TIMEOUT).await;
    assert_eq!(other_seen.as_deref(), Some(b"sharer-frame".as_slice()));

    // The viewer tries to publish an edit. It must not reach the other editor.
    send_binary(&mut viewer, b"viewer-edit").await;
    let leaked = recv_binary(&mut other, NEGATIVE_TIMEOUT).await;
    assert_eq!(
        leaked, None,
        "a read-only viewer's frame must never be broadcast to other peers"
    );
}

/// A read-only viewer's frame is not even recorded in the room log: a peer that
/// joins *after* the viewer tried to publish replays only the sharer's frame.
#[tokio::test]
async fn read_only_viewer_frame_is_not_recorded_in_the_log() {
    let addr = spawn_relay().await;

    let mut sharer = connect(addr, "log-room").await;
    let mut viewer = connect_viewer(addr, "log-room").await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    // The sharer records one real frame; the viewer attempts to record another.
    send_binary(&mut sharer, b"real-frame").await;
    send_binary(&mut viewer, b"ignored-viewer-frame").await;

    // Let the relay process both before a late joiner reads the log.
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    // A late joiner replays the log: it must contain exactly the sharer's frame,
    // and nothing from the viewer.
    let mut latecomer = connect(addr, "log-room").await;
    let first = recv_binary(&mut latecomer, RECV_TIMEOUT).await;
    assert_eq!(
        first.as_deref(),
        Some(b"real-frame".as_slice()),
        "the log replays the sharer's frame"
    );
    let second = recv_binary(&mut latecomer, NEGATIVE_TIMEOUT).await;
    assert_eq!(
        second, None,
        "the viewer's dropped frame must not appear in the replayed log"
    );
}

/// A no-op custom [`reticle_server::Persist`] hook is accepted via
/// [`RelayState::with_persist`] and the relay still forwards messages.
#[tokio::test]
async fn custom_persist_hook_is_invoked_and_relay_works() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug)]
    struct Counter(Arc<AtomicUsize>);
    impl reticle_server::Persist for Counter {
        fn on_update(&self, _room: &str, _payload: &[u8]) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let count = Arc::new(AtomicUsize::new(0));
    let state = RelayState::with_persist(Counter(Arc::clone(&count)));
    let addr = spawn_relay_with(state).await;

    let mut alice = connect(addr, "persisted").await;
    let mut bob = connect(addr, "persisted").await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    send_binary(&mut alice, b"persist-me").await;
    let received = recv_binary(&mut bob, RECV_TIMEOUT).await;
    assert_eq!(received.as_deref(), Some(b"persist-me".as_slice()));

    // The persist hook observed exactly one update.
    assert_eq!(count.load(Ordering::SeqCst), 1, "persist hook ran once");
}
