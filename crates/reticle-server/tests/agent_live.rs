//! The agent as a live collaborator in a real relay room (ADR 0022, lane v8-1d).
//!
//! This is the headless, in-process proof that [`reticle_agent::live::run_in_room`]
//! makes the agent a genuine room participant, not a private mirror. It binds a real
//! [`reticle_server`] relay on an ephemeral port, connects a plain test peer (an editor,
//! exactly as `share_live.rs` does), and runs the agent client against the same room with
//! a deterministic scripted step (no model, no key). It proves two directions over one
//! real socket each:
//!
//! 1. **The agent's frames reach the room.** The peer receives the agent's step as a
//!    `SyncMessage` CRDT frame *and* the agent's presence frame, decodes them with
//!    `reticle_sync::decode_frame`, and materializes the agent's geometry and cursor.
//!
//! 2. **Peer frames reach the agent.** A concurrent edit the peer publishes is applied
//!    back into the agent's own `SyncDocument`, so after the run the agent's document
//!    holds the peer's cell.
//!
//! Modeled on `share_live.rs`: deterministic, headless, no browser.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use reticle_agent::live::{LiveConfig, run_in_room};
use reticle_agent::{AgentCollaborator, Pacing};
use reticle_agent_api::AgentCommand;
use reticle_agent_api::args::{LayerArg, PointArg, RectArg};
use reticle_geometry::Point;
use reticle_model::Cell;
use reticle_server::{RelayState, serve};
use reticle_sync::{Frame, SyncDocument, decode_frame};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

/// A connected client socket.
type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Grace period letting the server-side upgrade task subscribe a peer to the room before
/// a publish. Matches `share_live.rs`.
const SUBSCRIBE_GRACE: Duration = Duration::from_millis(300);
/// Generous timeout for a frame that should arrive.
const RECV_TIMEOUT: Duration = Duration::from_secs(5);

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

/// Connects an editor (read-write) client to `ws://{addr}/ws/{room}`.
async fn connect_editor(addr: SocketAddr, room: &str) -> Client {
    let url = format!("ws://{addr}/ws/{room}");
    let (socket, _response) = connect_async(url).await.expect("editor connect");
    socket
}

/// Sends `payload` as one binary WebSocket frame.
async fn send_binary(client: &mut Client, payload: Vec<u8>) {
    client
        .send(Message::Binary(payload.into()))
        .await
        .expect("send binary");
}

/// Awaits the next binary payload on `client`, ignoring pings/pongs, or `None` on timeout.
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

/// The scripted single propose step the agent publishes: create a cell and draw one met1
/// rectangle. Deterministic and model-free (the mock/scripted path the bench harness
/// uses), so the test is reproducible.
fn agent_step() -> Vec<AgentCommand> {
    vec![
        AgentCommand::CreateCell {
            name: "agent_cell".to_owned(),
        },
        AgentCommand::AddRect {
            cell: "agent_cell".to_owned(),
            layer: LayerArg {
                layer: 68,
                datatype: 20,
            },
            rect: RectArg {
                min: PointArg { x: 0, y: 0 },
                max: PointArg { x: 400, y: 200 },
            },
        },
    ]
}

/// End to end: the agent's scripted step reaches a peer as a CRDT frame plus a presence
/// frame, and the peer's concurrent edit reaches the agent's document.
#[tokio::test]
async fn agent_frames_reach_a_peer_and_a_peer_edit_reaches_the_agent() {
    let addr = spawn_relay().await;
    let room = "agent-live";

    // The peer joins first and is given time to subscribe, so it receives the agent's
    // frames live.
    let mut peer = connect_editor(addr, room).await;
    tokio::time::sleep(SUBSCRIBE_GRACE).await;

    // Run the agent client in the background: it connects, waits out a startup grace so
    // its own subscription settles (so the peer's later edit reaches it), publishes one
    // scripted step, then lingers to absorb inbound frames.
    let url = format!("ws://{addr}/ws/{room}");
    let config = LiveConfig {
        doc_id: room.to_owned(),
        publish_presence: true,
        startup_grace: SUBSCRIBE_GRACE,
        linger: Duration::from_secs(3),
    };
    let collab = AgentCollaborator::new(Pacing::Instant).with_display_name("Reticle agent");
    let agent_task =
        tokio::spawn(async move { run_in_room(collab, &url, vec![agent_step()], &config).await });

    // The peer receives the agent's CRDT update frame and presence frame, materializing
    // the agent's geometry and its cursor.
    let mut mirror = SyncDocument::new("peer");
    let mut got_geometry = false;
    let mut got_presence = false;
    while !(got_geometry && got_presence) {
        let Some(bytes) = recv_binary(&mut peer, RECV_TIMEOUT).await else {
            break;
        };
        match decode_frame(&bytes).expect("a room frame is a valid SyncMessage") {
            Frame::Update(raw) => {
                mirror.apply_update(&raw).expect("agent update applies");
                got_geometry = true;
            }
            Frame::Presence(p) => {
                assert_eq!(
                    p.actor,
                    reticle_agent_api::AGENT_ACTOR,
                    "presence is the agent's"
                );
                // The cursor sits at the rectangle center (200, 100).
                assert_eq!(
                    p.cursor,
                    Point::new(200, 100),
                    "the agent's cursor is at its placement"
                );
                got_presence = true;
            }
            Frame::Comment(_) => {}
        }
    }
    assert!(
        got_geometry,
        "the peer received the agent's CRDT update frame"
    );
    assert!(got_presence, "the peer received the agent's presence frame");
    let cell = mirror
        .document()
        .cell("agent_cell")
        .expect("the agent's cell reached the peer");
    assert_eq!(
        cell.shapes.len(),
        1,
        "the agent's rectangle reached the peer"
    );

    // The peer now publishes its own concurrent edit; it must reach the agent's document.
    let mut peer_doc = SyncDocument::new("peer-editor");
    peer_doc.add_cell(&Cell::new("peer_cell"));
    let peer_frame = reticle_sync::encode_update_frame(&peer_doc.encode_state_update());
    send_binary(&mut peer, peer_frame).await;

    // The agent lingers, applies the peer's frame, and returns its collaborator.
    let collab = agent_task
        .await
        .expect("agent task joins")
        .expect("run_in_room succeeds");

    assert!(
        collab.document().cell("peer_cell").is_some(),
        "the peer's concurrent edit reached the agent's document"
    );
    // The agent still holds its own geometry too.
    assert!(
        collab.document().cell("agent_cell").is_some(),
        "the agent kept its own cell",
    );
}
