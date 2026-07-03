//! Publishing the agent's CRDT frames into a relay room.
//!
//! A spectator watches a session by joining the room the demo service hands back
//! (`SubmitResponse::room`) on the collaboration relay. The relay
//! ([`reticle_server`]) fans every binary frame out to the other peers in that room
//! and replays the room's log to a late joiner. Peers exchange **raw `yrs` v1
//! update bytes** as binary frames (this is exactly what `reticle-server`'s
//! `echo_latency` example ships, and what a `reticle-sync` peer applies with
//! `SyncDocument::apply_update`).
//!
//! [`RoomPublisher`] is the harness's handle to a room: it owns a background task
//! that holds the WebSocket connection and drains an [`mpsc`] channel, so the
//! blocking agent worker can hand off a frame with a non-blocking `send` and never
//! touches the socket directly.

use futures_util::SinkExt;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

/// How many frames may be queued for the relay before the harness applies
/// backpressure. A demo step ships one small frame, so a short queue is ample.
const FRAME_QUEUE: usize = 64;

/// A handle for publishing binary CRDT frames into one relay room.
///
/// Cloneable senders let the harness push frames from a blocking worker. Dropping
/// every sender closes the channel and ends the background task, which closes the
/// WebSocket.
#[derive(Clone, Debug)]
pub struct RoomPublisher {
    tx: mpsc::Sender<Vec<u8>>,
}

impl RoomPublisher {
    /// Connects to `room` on the relay at `ws_base` (for example
    /// `ws://127.0.0.1:3041`) and spawns the background publisher task.
    ///
    /// Returns the publisher on a successful connect. On failure it returns the
    /// error string so the caller can log it and continue without streaming; a
    /// spectator transport being down never fails the agent loop.
    ///
    /// # Errors
    ///
    /// Returns a human-readable error when the WebSocket connection cannot be
    /// established.
    pub async fn connect(ws_base: &str, room: &str) -> Result<Self, String> {
        let url = format!("{}/ws/{room}", ws_base.trim_end_matches('/'));
        let (ws, _resp) = connect_async(&url)
            .await
            .map_err(|e| format!("connecting to relay {url}: {e}"))?;

        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(FRAME_QUEUE);
        tokio::spawn(async move {
            let mut ws = ws;
            while let Some(frame) = rx.recv().await {
                if ws.send(Message::Binary(frame.into())).await.is_err() {
                    break;
                }
            }
            // Best-effort clean close when the channel ends.
            let _ = ws.close(None).await;
        });

        Ok(Self { tx })
    }

    /// Queues one binary frame for the room. Non-blocking.
    ///
    /// A frame is dropped (with no error) if the queue is full or the background
    /// task has ended, because a lost spectator frame is not worth stalling or
    /// failing the authoritative agent loop over. The room log and the CRDT's
    /// idempotent updates make an occasional drop harmless: the next full-state
    /// frame carries the whole document again.
    pub fn publish(&self, frame: Vec<u8>) {
        // `try_send` never awaits, so it is safe to call from a blocking worker via
        // a runtime handle. A full or closed channel is ignored by design.
        let _ = self.tx.try_send(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::RoomPublisher;
    use reticle_server::{RelayState, serve};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn publish_reaches_a_peer_in_the_room() {
        use futures_util::StreamExt;
        use tokio_tungstenite::connect_async;
        use tokio_tungstenite::tungstenite::Message;

        // A relay on an ephemeral port.
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = serve(listener, RelayState::new()).await;
        });
        let ws_base = format!("ws://{addr}");

        // A spectator joins the room first.
        let (mut spectator, _) = connect_async(format!("{ws_base}/ws/demo-room"))
            .await
            .expect("spectator connect");
        // Give the upgrade a moment to subscribe before the publisher sends.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // The publisher connects and sends one frame.
        let publisher = RoomPublisher::connect(&ws_base, "demo-room")
            .await
            .expect("publisher connect");
        publisher.publish(b"hello-crdt-frame".to_vec());

        // The spectator receives exactly those bytes as a binary frame.
        let got = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                match spectator.next().await {
                    Some(Ok(Message::Binary(bytes))) => return bytes.to_vec(),
                    Some(Ok(_)) => {}
                    _ => panic!("spectator stream ended before the frame arrived"),
                }
            }
        })
        .await
        .expect("frame arrives within the timeout");
        assert_eq!(got, b"hello-crdt-frame");
    }

    #[tokio::test]
    async fn connect_to_a_dead_relay_is_an_error_not_a_panic() {
        // Nothing is listening here; connect must return Err, not panic, so the
        // caller can degrade to no-streaming.
        let err = RoomPublisher::connect("ws://127.0.0.1:1", "room")
            .await
            .expect_err("connecting to a dead relay should fail");
        assert!(err.contains("relay"), "error should name the relay: {err}");
    }
}
