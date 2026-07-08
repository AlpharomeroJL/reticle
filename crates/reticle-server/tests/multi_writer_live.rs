//! End-to-end multi-writer collaboration over the live relay (ADR 0081).
//!
//! This proves the write-multi lane's three properties against the real
//! `reticle-server` relay, with real [`SyncDocument`]s on both ends:
//!
//! * **Two editors converge.** Editor A and editor B (both Edit mode) each make a
//!   disjoint edit, publish their CRDT state through the relay, apply the other's
//!   frame, and reach an identical materialized document.
//! * **A viewer sees the edits.** A read-only viewer (`?mode=view`) receives both
//!   editors' frames and materializes the union.
//! * **A viewer cannot write.** The frame a view-mode client sends is dropped by the
//!   relay: it never reaches either editor, so a viewer cannot mutate the shared doc.
//!
//! The relay forwards opaque binary frames, so these tests ship raw `yrs` update
//! bytes ([`SyncDocument::encode_state_update`]) and apply them with
//! [`SyncDocument::apply_update`] on the other side, exactly the payload the browser
//! transport wraps in a `SyncMessage` envelope.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, DrawShape, ShapeKind};
use reticle_server::{RelayState, serve};
use reticle_sync::SyncDocument;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Grace period so a server-side connection subscribes before a peer publishes.
const SUBSCRIBE_GRACE: Duration = Duration::from_millis(300);
/// Generous timeout for a frame that should arrive.
const RECV_TIMEOUT: Duration = Duration::from_secs(5);
/// Short timeout for asserting a frame must NOT arrive.
const NEGATIVE_TIMEOUT: Duration = Duration::from_millis(400);

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

async fn connect_editor(addr: SocketAddr, room: &str) -> Client {
    let (socket, _) = connect_async(format!("ws://{addr}/ws/{room}"))
        .await
        .expect("editor connect");
    socket
}

async fn connect_viewer(addr: SocketAddr, room: &str) -> Client {
    let (socket, _) = connect_async(format!("ws://{addr}/ws/{room}?mode=view"))
        .await
        .expect("viewer connect");
    socket
}

async fn send_binary(client: &mut Client, payload: &[u8]) {
    client
        .send(Message::Binary(payload.to_vec().into()))
        .await
        .expect("send binary");
}

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

/// A rectangle shape on layer `l`.
fn rect(l: u16, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
    DrawShape::new(
        LayerId::new(l, 0),
        ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
    )
}

/// A `SyncDocument` for `actor` with a single-shape cell named `cell`.
fn editor_doc(actor: &str, cell: &str, layer: u16) -> SyncDocument {
    let mut doc = SyncDocument::new(actor);
    let mut c = Cell::new(cell);
    c.shapes.push(rect(layer, 0, 0, 10, 10));
    doc.add_cell(&c);
    doc
}

/// Two editors and one read-only viewer share a room. Both editors' edits converge
/// on every editor, the viewer materializes the union, and the viewer's own frame
/// is dropped by the relay so it never reaches either editor.
#[tokio::test]
async fn two_editors_converge_and_a_viewer_sees_but_cannot_write() {
    let addr = spawn_relay().await;
    let room = "multi";

    let mut alice = connect_editor(addr, room).await;
    let mut bob = connect_editor(addr, room).await;
    let mut viewer = connect_viewer(addr, room).await;

    // Let all three server-side connections subscribe before anyone publishes.
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    // Editor A and editor B each make a disjoint edit and publish full state.
    let mut a_doc = editor_doc("alice", "alpha", 1);
    let mut b_doc = editor_doc("bob", "beta", 2);
    send_binary(&mut alice, &a_doc.encode_state_update()).await;
    send_binary(&mut bob, &b_doc.encode_state_update()).await;

    // Each editor receives the OTHER editor's frame (no self-echo) and applies it.
    let to_a = recv_binary(&mut alice, RECV_TIMEOUT)
        .await
        .expect("A receives B's frame");
    a_doc.apply_update(&to_a).expect("A applies B's update");
    let to_b = recv_binary(&mut bob, RECV_TIMEOUT)
        .await
        .expect("B receives A's frame");
    b_doc.apply_update(&to_b).expect("B applies A's update");

    // Both editors converge to the union of both edits.
    assert_eq!(a_doc.document(), b_doc.document(), "editors converge");
    assert!(a_doc.document().cell("alpha").is_some());
    assert!(a_doc.document().cell("beta").is_some());
    assert_eq!(a_doc.document().cell_count(), 2);

    // The viewer receives both editors' frames and materializes the union.
    let mut v_doc = SyncDocument::new("viewer");
    for _ in 0..2 {
        let frame = recv_binary(&mut viewer, RECV_TIMEOUT)
            .await
            .expect("viewer receives an editor frame");
        v_doc.apply_update(&frame).expect("viewer applies frame");
    }
    assert_eq!(
        v_doc.document(),
        a_doc.document(),
        "the viewer materializes exactly what the editors converged on"
    );

    // The viewer attempts to write: it publishes its own CRDT edit.
    let rogue = editor_doc("viewer", "rogue", 9);
    send_binary(&mut viewer, &rogue.encode_state_update()).await;

    // The relay drops it: neither editor receives the viewer's frame, so neither
    // editor's converged document gains the rogue cell.
    assert!(
        recv_binary(&mut alice, NEGATIVE_TIMEOUT).await.is_none(),
        "a viewer's write must never reach editor A"
    );
    assert!(
        recv_binary(&mut bob, NEGATIVE_TIMEOUT).await.is_none(),
        "a viewer's write must never reach editor B"
    );
    assert!(
        a_doc.document().cell("rogue").is_none(),
        "the viewer's rogue cell never entered the shared document"
    );
    // (Touch `rogue` so the doc it built is observably unused by the peers.)
    assert!(rogue.document().cell("rogue").is_some());
}
