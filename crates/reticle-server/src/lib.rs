//! The Reticle collaboration relay.
//!
//! This crate is a real WebSocket relay built on `axum` 0.8 and `tokio`. It
//! carries no editing logic of its own: it treats every payload
//! ([`reticle_proto::v1::SyncMessage`], [`reticle_proto::v1::CrdtUpdate`],
//! [`reticle_proto::v1::Presence`], [`reticle_proto::v1::Comment`], ...) as an
//! opaque blob of bytes and fans it out to the other peers connected to the
//! same document ("room").
//!
//! # Architecture
//!
//! * [`RelayState`] is the shared, cloneable handle used as axum
//!   [`State`]. It owns a registry of rooms keyed by
//!   document id and a [`Persist`] hook.
//! * Each room owns a [`tokio::sync::broadcast`] channel. Every message a
//!   peer sends is published to that channel so it fans out to all *other*
//!   peers in the room. A message is tagged with the id of the connection that
//!   produced it so the sender can skip its own echo.
//! * Each room also keeps an in-memory **update log**: the ordered list of
//!   payloads seen so far. When a peer joins a non-empty room the relay replays
//!   this log to it (before any live traffic) so it catches up on the current
//!   state. See the `Room` type for the trade-offs of this approach.
//! * [`Persist`] is invoked for every update so a host can durably store room
//!   state; the default [`NoPersist`] does nothing.
//!
//! # Wire protocol
//!
//! The relay listens on `GET /ws/{room}` and upgrades to a WebSocket. Only
//! binary frames are treated as payloads; text frames are ignored, and ping and
//! close frames are handled by axum and this loop respectively. The relay never
//! inspects payload contents.
//!
//! # Example
//!
//! ```no_run
//! # async fn run() {
//! use reticle_server::{serve, RelayState};
//!
//! let state = RelayState::new();
//! let listener = tokio::net::TcpListener::bind("127.0.0.1:3030").await.unwrap();
//! serve(listener, state).await.unwrap();
//! # }
//! ```

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::response::Response;
use axum::routing::any;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;

/// Capacity of each room's broadcast channel, in messages.
///
/// If a peer's receiver lags more than this many messages behind the producer,
/// `tokio::sync::broadcast` drops the oldest messages for that receiver and
/// reports [`broadcast::error::RecvError::Lagged`]. The relay treats a lagging
/// peer as recoverable and keeps forwarding subsequent messages.
const ROOM_CHANNEL_CAPACITY: usize = 1024;

/// The default bind address used by the binary when `RETICLE_SERVER_ADDR` is
/// unset. Exposed so hosts and tests can reference the same default.
pub const DEFAULT_ADDR: &str = "127.0.0.1:3030";

/// The environment variable read by [`bind_address`] to override [`DEFAULT_ADDR`].
pub const ADDR_ENV: &str = "RETICLE_SERVER_ADDR";

/// A persistence hook invoked once per relayed update.
///
/// Implementors receive the room id and the raw payload bytes for every update
/// the relay accepts, in the order the relay accepted them. This lets a host
/// durably store room state (for example, to rebuild the update log after a
/// restart) without the relay itself depending on any storage backend.
///
/// Implementations must be cheap and non-blocking: the hook is called while the
/// room registry lock is *not* held, but on the connection's receive path.
/// Offload heavy or blocking work to a background task or channel.
pub trait Persist: Send + Sync + fmt::Debug {
    /// Called for each update accepted into `room`, with the opaque `payload`.
    fn on_update(&self, room: &str, payload: &[u8]);
}

/// The default [`Persist`] implementation: it does nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPersist;

impl Persist for NoPersist {
    #[inline]
    fn on_update(&self, _room: &str, _payload: &[u8]) {}
}

/// A single message broadcast within a room.
///
/// Carrying the originating connection id lets each peer's forwarding task
/// suppress the echo of its own messages, since a `tokio::sync::broadcast`
/// receiver otherwise observes *every* message published after it subscribed -
/// including those the same connection sent.
#[derive(Clone)]
struct RoomMessage {
    /// The connection that produced this payload.
    origin: u64,
    /// The opaque payload bytes.
    payload: Bytes,
}

/// A collaboration room: the set of peers editing one document.
///
/// Peers are not tracked individually; instead the room owns a
/// [`broadcast::Sender`] and every peer subscribes to it. The room also keeps
/// an ordered `log` of every payload it has accepted.
///
/// # Initial-state approach
///
/// The relay replays the full `log` to each new joiner before forwarding live
/// traffic. This is the simplest *correct* catch-up strategy for opaque CRDT
/// updates: CRDT updates are commutative and idempotent, so replaying the whole
/// ordered log reconstructs the current state on the joiner regardless of
/// interleaving. The cost is unbounded memory growth per room; a production host
/// would compact the log into periodic snapshots (and can drive that from the
/// [`Persist`] hook). Keeping the raw log here keeps the relay free of any
/// payload-format knowledge.
struct Room {
    /// Fan-out channel; a peer publishes here and all other peers receive it.
    sender: broadcast::Sender<RoomMessage>,
    /// Ordered log of every accepted payload, replayed to late joiners.
    log: Vec<Bytes>,
}

impl Room {
    /// Creates an empty room with a fresh broadcast channel.
    fn new() -> Self {
        let (sender, _rx) = broadcast::channel(ROOM_CHANNEL_CAPACITY);
        Self {
            sender,
            log: Vec::new(),
        }
    }
}

impl fmt::Debug for Room {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Room")
            .field("receivers", &self.sender.receiver_count())
            .field("log_len", &self.log.len())
            .finish()
    }
}

/// Shared, cloneable state for the relay.
///
/// This is the value passed as an axum [`State`] extractor. Cloning it
/// is cheap: it shares the underlying room registry and persistence hook via
/// [`Arc`]. All handles to the same `RelayState` see the same rooms.
#[derive(Clone)]
pub struct RelayState {
    /// Registry of rooms keyed by document id.
    ///
    /// A `std::sync::Mutex` is deliberate: the guarded critical sections are
    /// short and synchronous (register a room, clone a `Sender`, snapshot or
    /// push to the log) and the guard is never held across an `.await`.
    rooms: Arc<Mutex<HashMap<String, Room>>>,
    /// Persistence hook invoked for every accepted update.
    persist: Arc<dyn Persist>,
    /// Monotonic source of per-connection ids for echo suppression.
    next_conn_id: Arc<AtomicU64>,
}

impl Default for RelayState {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for RelayState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let room_count = self.rooms.lock().map_or(0, |rooms| rooms.len());
        f.debug_struct("RelayState")
            .field("rooms", &room_count)
            .field("persist", &self.persist)
            .field("next_conn_id", &self.next_conn_id.load(Ordering::Relaxed))
            .finish()
    }
}

impl RelayState {
    /// Creates an empty relay with the no-op [`NoPersist`] hook.
    #[must_use]
    pub fn new() -> Self {
        Self::with_persist(NoPersist)
    }

    /// Creates an empty relay that invokes `persist` for every accepted update.
    #[must_use]
    pub fn with_persist<P>(persist: P) -> Self
    where
        P: Persist + 'static,
    {
        Self {
            rooms: Arc::new(Mutex::new(HashMap::new())),
            persist: Arc::new(persist),
            next_conn_id: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Builds the axum [`Router`] for this relay, with the state attached.
    ///
    /// The single route is `GET /ws/{room}`, which upgrades to a WebSocket and
    /// joins the connection to the named room.
    pub fn router(self) -> Router {
        Router::new()
            .route("/ws/{room}", any(ws_handler))
            .with_state(self)
    }

    /// Allocates a fresh, process-unique connection id.
    fn allocate_conn_id(&self) -> u64 {
        self.next_conn_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Joins `room`, returning a sender for publishing, a receiver for live
    /// traffic, and a snapshot of the room's update log to replay to the joiner.
    ///
    /// The receiver is subscribed *before* the log snapshot is taken and while
    /// the registry lock is held, so no update can slip between the snapshot and
    /// the start of live delivery: any update accepted after this call is seen
    /// on the returned receiver, and everything before it is in the snapshot.
    fn join(
        &self,
        room: &str,
    ) -> (
        broadcast::Sender<RoomMessage>,
        broadcast::Receiver<RoomMessage>,
        Vec<Bytes>,
    ) {
        let mut rooms = self.rooms.lock().expect("room registry mutex poisoned");
        let entry = rooms.entry(room.to_owned()).or_insert_with(Room::new);
        let receiver = entry.sender.subscribe();
        let sender = entry.sender.clone();
        let backlog = entry.log.clone();
        (sender, receiver, backlog)
    }

    /// Records an accepted `payload` in `room`'s log and runs the persist hook.
    ///
    /// The persist hook is invoked after the lock is released so a slow hook
    /// cannot stall other connections' access to the registry.
    fn record(&self, room: &str, payload: &Bytes) {
        {
            let mut rooms = self.rooms.lock().expect("room registry mutex poisoned");
            if let Some(existing) = rooms.get_mut(room) {
                existing.log.push(payload.clone());
            }
        }
        self.persist.on_update(room, payload);
    }
}

/// axum handler for `GET /ws/{room}`: upgrades the connection and joins `room`.
async fn ws_handler(
    ws: WebSocketUpgrade,
    Path(room): Path<String>,
    State(state): State<RelayState>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, room, state))
}

/// Drives one peer connection for the lifetime of its socket.
///
/// The socket is split so reads and writes proceed concurrently:
///
/// * the **forward** task drains this room's broadcast receiver and writes each
///   message to the client, skipping messages this connection itself produced
///   (echo suppression);
/// * the **receive** loop reads binary frames from the client, publishes each to
///   the room, and records it in the log.
///
/// Before live forwarding starts, the room's existing update log is replayed to
/// the client so a late joiner catches up.
async fn handle_socket(socket: WebSocket, room: String, state: RelayState) {
    let conn_id = state.allocate_conn_id();
    let (sender, mut receiver, backlog) = state.join(&room);

    let (mut ws_tx, mut ws_rx) = socket.split();

    // Replay the current room state to this joiner before any live traffic.
    for payload in backlog {
        if ws_tx.send(Message::Binary(payload)).await.is_err() {
            return;
        }
    }

    // Forward live broadcast traffic (from other peers) to this client.
    let forward = tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(msg) => {
                    // Suppress the echo of our own messages.
                    if msg.origin == conn_id {
                        continue;
                    }
                    if ws_tx.send(Message::Binary(msg.payload)).await.is_err() {
                        break;
                    }
                }
                // We lagged behind: the dropped messages are skipped and the
                // loop continues to deliver subsequent ones.
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                // The room is gone (all senders dropped); nothing more to do.
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Receive client frames and publish opaque payloads to the room.
    while let Some(Ok(message)) = ws_rx.next().await {
        match message {
            Message::Binary(payload) => {
                state.record(&room, &payload);
                // A send error means every receiver has dropped; the room is
                // effectively empty, so continue logging without broadcasting.
                let _ = sender.send(RoomMessage {
                    origin: conn_id,
                    payload,
                });
            }
            Message::Close(_) => break,
            // Text/Ping/Pong are not relay payloads; ping/pong are auto-handled.
            Message::Text(_) | Message::Ping(_) | Message::Pong(_) => {}
        }
    }

    // Client is gone: drop our sender and stop forwarding.
    drop(sender);
    forward.abort();
}

/// Serves the relay on `listener` until the process is terminated.
///
/// Builds the router from `state` and runs it with [`axum::serve()`]. This future
/// resolves only if the server encounters an unrecoverable I/O error.
///
/// # Errors
///
/// Returns any [`std::io::Error`] surfaced by the underlying accept loop.
pub async fn serve(listener: tokio::net::TcpListener, state: RelayState) -> std::io::Result<()> {
    axum::serve(listener, state.router()).await
}

/// Resolves the bind address from the [`ADDR_ENV`] environment variable,
/// falling back to [`DEFAULT_ADDR`].
#[must_use]
pub fn bind_address() -> String {
    std::env::var(ADDR_ENV).unwrap_or_else(|_| DEFAULT_ADDR.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_and_new_agree() {
        let a = RelayState::new();
        let b = RelayState::default();
        // Both start with no rooms.
        assert_eq!(a.rooms.lock().unwrap().len(), 0);
        assert_eq!(b.rooms.lock().unwrap().len(), 0);
    }

    #[test]
    fn connection_ids_are_unique_and_monotonic() {
        let state = RelayState::new();
        let first = state.allocate_conn_id();
        let second = state.allocate_conn_id();
        assert_eq!(first, 0);
        assert_eq!(second, 1);
    }

    #[test]
    fn join_creates_room_and_record_logs() {
        let state = RelayState::new();
        let (_tx, _rx, backlog) = state.join("doc-1");
        assert!(backlog.is_empty(), "new room starts with empty log");

        state.record("doc-1", &Bytes::from_static(b"update"));
        let (_tx2, _rx2, backlog2) = state.join("doc-1");
        assert_eq!(backlog2.len(), 1, "recorded update is replayed to joiners");
    }

    #[test]
    fn debug_impls_do_not_panic() {
        let state = RelayState::new();
        let _ = state.join("doc-debug");
        // Exercise the manual Debug impls.
        assert!(format!("{state:?}").contains("RelayState"));
        let rooms = state.rooms.lock().unwrap();
        assert!(format!("{:?}", rooms.get("doc-debug").unwrap()).contains("Room"));
    }
}
