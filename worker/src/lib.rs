//! Reticle collaboration relay as a Cloudflare Durable Object.
//!
//! This is the second Reticle relay: a Worker routes `GET /ws/{room}` to a
//! per-room Durable Object (`ReticleRoom`) that accepts hibernatable WebSockets
//! and mirrors the native `reticle_server` relay's observable semantics exactly
//! (see `crates/reticle-server/src/lib.rs` and the conformance suite in
//! `crates/reticle-relay-conformance`). The protocol is the asset; the two
//! relays are proven equivalent by one vector table run against both.
//!
//! # Semantics mirrored
//!
//! * Join `GET /ws/{room}`; `?mode=view|viewer|readonly|ro` (case-insensitive)
//!   joins read-only. The mode is stored in the socket's hibernation attachment.
//! * On join the room's full accepted-frame log replays to the joiner in order,
//!   before live traffic. The log is persisted in DO storage in fixed chunks.
//! * A view-mode connection's frames are dropped: not logged, not broadcast.
//! * A sender never receives its own echo (peers are keyed by connection id).
//! * Only binary frames are payloads; text frames are ignored.
//!
//! # Free-tier engineering (where the DO diverges, semantics preserved)
//!
//! Presence frames (first byte `0x12`, the `SyncMessage` presence tag) are
//! coalesced per client to about 10 Hz: within a short window only the newest
//! presence per sender is delivered. This is driven by the DO alarm, which is
//! hibernation-safe (a `setTimeout` would block hibernation). Update frames
//! (first byte `0x0A`) are never coalesced or dropped. The newest presence
//! always converges, so a read-only viewer following a sharer still lands on the
//! sharer's latest cursor and viewport. Room expiry is also driven by the alarm:
//! when it fires with no open sockets, the room's storage is deleted.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use worker::*;

/// The `SyncMessage` presence field tag (frozen in `reticle-proto`): a frame
/// whose first byte is this is a presence update and is coalesced.
const PRESENCE_TAG: u8 = 0x12;

/// Frames per persisted log chunk. The log replays to late joiners in order.
const LOG_CHUNK: u64 = 32;

/// Presence coalescing window: at most one presence per sender is delivered per
/// this interval (about 10 Hz).
const PRESENCE_WINDOW_MS: u64 = 100;

/// Idle time before an unoccupied room's storage is reclaimed by the alarm.
const ROOM_TTL_MS: u64 = 60_000;

/// The largest single binary frame the relay accepts. A CRDT update or presence
/// frame is at most a few KiB; a larger frame is an attempt to exhaust room
/// storage, so it is dropped. Bounds per-frame memory.
const MAX_FRAME_BYTES: usize = 1 << 20; // 1 MiB

/// The largest number of frames the room log retains. Updates cannot be dropped
/// without losing geometry for late joiners, so once the log is full the room stops
/// growing its storage (it keeps broadcasting live, only late-join replay is
/// truncated) rather than letting one editor exhaust storage without bound.
const MAX_LOG_FRAMES: u64 = 100_000;

/// The largest number of simultaneous connections to one room. Caps broadcast
/// fan-out and connection-id growth so one client cannot exhaust a room.
const MAX_CONNS_PER_ROOM: usize = 64;

/// Per-connection state carried in the socket's hibernation attachment, so it
/// survives eviction: a stable id (for echo suppression) and the join mode.
#[derive(Serialize, Deserialize, Clone, Copy)]
struct Conn {
    /// Stable, room-unique connection id.
    id: u64,
    /// Whether this connection joined read-only (`?mode=view`).
    view: bool,
}

/// Worker entry: route `GET /ws/{room}` to the room's Durable Object, forwarding
/// the request (query string included, so the DO reads `?mode=`).
#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let url = req.url()?;
    let path = url.path().to_string();
    match path.strip_prefix("/ws/") {
        Some(room) if !room.is_empty() => {
            let ns = env.durable_object("ROOM")?;
            let stub = ns.id_from_name(room)?.get_stub()?;
            stub.fetch_with_request(req).await
        }
        _ => Response::ok("reticle relay: connect to /ws/{room}"),
    }
}

/// Whether a `mode` query value selects a read-only viewer, matching the native
/// relay's `JoinMode::from_query_value` (case-insensitive).
fn is_view_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "view" | "viewer" | "readonly" | "read-only" | "ro"
    )
}

/// Reads the join mode from a request URL's query string.
fn parse_view(url: &Url) -> bool {
    url.query_pairs()
        .any(|(k, v)| k == "mode" && is_view_value(&v))
}

/// The per-room relay. One instance exists per room id; the Workers runtime may
/// hibernate it between events, so all cross-event state lives in storage or in
/// the sockets' attachments.
#[durable_object]
struct ReticleRoom {
    state: State,
}

impl DurableObject for ReticleRoom {
    fn new(state: State, _env: Env) -> Self {
        Self { state }
    }

    async fn fetch(&self, req: Request) -> Result<Response> {
        let view = parse_view(&req.url()?);
        let storage = self.state.storage();

        // Per-room connection cap: refuse a join that would exceed it, so one client
        // cannot open unbounded sockets and exhaust the room's fan-out and id space.
        if self.state.get_websockets().len() >= MAX_CONNS_PER_ROOM {
            return Response::error("room is full", 429);
        }

        let pair = WebSocketPair::new()?;
        let server = pair.server;
        self.state.accept_web_socket(&server);

        // Tag the connection with a stable id and its mode, so both survive
        // hibernation and echo suppression can key off the id.
        let id: u64 = get_or(&storage, "next_conn").await;
        storage.put("next_conn", id + 1).await?;
        server.serialize_attachment(Conn { id, view })?;

        // Replay the full room log to this joiner, in order, then each sender's
        // last-known presence, so a late viewer lands on the current cursors and
        // viewports (matching the native relay, which logs and replays presence).
        replay_log(&storage, &server).await;
        replay_presence(&storage, &server).await;

        // Ensure an alarm exists so an emptied room is eventually reclaimed.
        if storage.get_alarm().await?.is_none() {
            storage.set_alarm(Duration::from_millis(ROOM_TTL_MS)).await?;
        }

        Response::from_websocket(pair.client)
    }

    async fn websocket_message(
        &self,
        ws: WebSocket,
        message: WebSocketIncomingMessage,
    ) -> Result<()> {
        // Only binary frames are payloads; text/ping/pong are ignored.
        let WebSocketIncomingMessage::Binary(bytes) = message else {
            return Ok(());
        };
        // Reject an oversized frame: a real update or presence frame is small, so a
        // large one is an attempt to exhaust room storage. Drop it.
        if bytes.len() > MAX_FRAME_BYTES {
            return Ok(());
        }
        let Some(conn) = ws.deserialize_attachment::<Conn>()? else {
            return Ok(());
        };
        // A read-only viewer's frames are dropped: never logged, never broadcast.
        if conn.view {
            return Ok(());
        }

        let storage = self.state.storage();
        if bytes.first() == Some(&PRESENCE_TAG) {
            // Record the sender's last-known presence for late-join replay (the native
            // relay logs and replays presence; the DO keeps one entry per sender rather
            // than logging every frame), then coalesce for live delivery via the alarm.
            upsert_presence(&storage, "last_presence", conn.id, &bytes).await?;
            upsert_presence(&storage, "pending", conn.id, &bytes).await?;
            if !get_or::<bool>(&storage, "presence_alarm").await {
                storage
                    .set_alarm(Duration::from_millis(PRESENCE_WINDOW_MS))
                    .await?;
                storage.put("presence_alarm", true).await?;
            }
        } else {
            // An update or comment. Flush any pending presence first so presence sent
            // before this update is delivered before it (strict order, matching the
            // native relay's FIFO). Log (bounded) and broadcast live regardless.
            flush_pending(&self.state, &storage).await?;
            append_log(&storage, &bytes).await?;
            broadcast_except(&self.state, conn.id, &bytes);
        }
        Ok(())
    }

    async fn alarm(&self) -> Result<Response> {
        let storage = self.state.storage();

        // Flush the newest pending presence per sender to the other peers.
        flush_pending(&self.state, &storage).await?;

        // Room expiry: reclaim an emptied room; otherwise keep a heartbeat so a
        // room that empties later is still reclaimed.
        if self.state.get_websockets().is_empty() {
            storage.delete_all().await?;
        } else {
            storage.set_alarm(Duration::from_millis(ROOM_TTL_MS)).await?;
        }
        Response::ok("ok")
    }
}

/// Reads a storage value, defaulting when absent or on any decode error.
async fn get_or<T: serde::de::DeserializeOwned + Default>(storage: &Storage, key: &str) -> T {
    storage.get::<T>(key).await.ok().flatten().unwrap_or_default()
}

/// Appends one accepted frame to the room's chunked log, up to [`MAX_LOG_FRAMES`].
/// Once the log is full this is a no-op: the room keeps broadcasting live frames but
/// stops growing storage, so one editor cannot exhaust the room (only late-join replay
/// is truncated past the cap, which no real session reaches).
async fn append_log(storage: &Storage, frame: &[u8]) -> Result<()> {
    let len: u64 = get_or(storage, "log_len").await;
    if len >= MAX_LOG_FRAMES {
        return Ok(());
    }
    let key = format!("log:{}", len / LOG_CHUNK);
    let mut chunk: Vec<Vec<u8>> = get_or(storage, &key).await;
    chunk.push(frame.to_vec());
    storage.put(&key, chunk).await?;
    storage.put("log_len", len + 1).await?;
    Ok(())
}

/// Replays the full room log to `ws`, in order.
async fn replay_log(storage: &Storage, ws: &WebSocket) {
    let len: u64 = get_or(storage, "log_len").await;
    if len == 0 {
        return;
    }
    let chunks = len.div_ceil(LOG_CHUNK);
    for c in 0..chunks {
        let chunk: Vec<Vec<u8>> = get_or(storage, &format!("log:{c}")).await;
        for frame in chunk {
            let _ = ws.send_with_bytes(&frame);
        }
    }
}

/// Upserts `bytes` as the newest presence for `id` in the per-sender map at `key`
/// ("pending" for coalesced live delivery, "last_presence" for late-join replay).
async fn upsert_presence(storage: &Storage, key: &str, id: u64, bytes: &[u8]) -> Result<()> {
    let mut map: Vec<(u64, Vec<u8>)> = get_or(storage, key).await;
    match map.iter_mut().find(|(k, _)| *k == id) {
        Some(entry) => entry.1 = bytes.to_vec(),
        None => map.push((id, bytes.to_vec())),
    }
    storage.put(key, map).await
}

/// Flushes the newest pending presence per sender to the other peers, then clears the
/// pending set. Shared by the alarm and by an incoming update (so a presence sent
/// before an update is delivered before it, preserving the native relay's FIFO order).
async fn flush_pending(state: &State, storage: &Storage) -> Result<()> {
    let pending: Vec<(u64, Vec<u8>)> = get_or(storage, "pending").await;
    if pending.is_empty() {
        return Ok(());
    }
    for (sender_id, frame) in &pending {
        broadcast_except(state, *sender_id, frame);
    }
    storage.delete("pending").await?;
    storage.put("presence_alarm", false).await?;
    Ok(())
}

/// Replays each sender's last-known presence to a joiner, so a late viewer lands on the
/// current cursors and viewports instead of a blank state until the next movement.
async fn replay_presence(storage: &Storage, ws: &WebSocket) {
    let last: Vec<(u64, Vec<u8>)> = get_or(storage, "last_presence").await;
    for (_id, frame) in &last {
        let _ = ws.send_with_bytes(frame);
    }
}

/// Sends `bytes` to every open socket except the one whose connection id is
/// `sender_id` (echo suppression).
fn broadcast_except(state: &State, sender_id: u64, bytes: &[u8]) {
    for peer in state.get_websockets() {
        let peer_id = peer
            .deserialize_attachment::<Conn>()
            .ok()
            .flatten()
            .map(|c| c.id);
        if peer_id != Some(sender_id) {
            let _ = peer.send_with_bytes(bytes);
        }
    }
}
