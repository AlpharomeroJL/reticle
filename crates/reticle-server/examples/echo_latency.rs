//! Measures collaboration echo latency: the wall-clock time for one client's
//! CRDT edit to travel through the relay and apply on a second client, on
//! localhost. Run with `cargo run -p reticle-server --example echo_latency
//! --release`.
//!
//! Two real WebSocket clients join one room on an ephemeral-port relay. Each
//! iteration, one client adds a shape, encodes just that edit as a `yrs` v1 diff
//! against the peer's state vector, and ships it; the timer runs from the send to
//! the moment the peer has applied it. This is the "remote echo on localhost"
//! path the performance target names; the local edit is applied synchronously and
//! is timed separately.

use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, DrawShape, ShapeKind};
use reticle_server::{RelayState, serve};
use reticle_sync::SyncDocument;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Awaits the next binary frame, skipping control/text frames.
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

#[tokio::main]
async fn main() {
    // Relay on an ephemeral port.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        serve(listener, RelayState::new()).await.expect("serve");
    });

    // Two clients in one room.
    let (mut alice_ws, _) = connect_async(format!("ws://{addr}/ws/bench"))
        .await
        .expect("alice connect");
    let (mut bob_ws, _) = connect_async(format!("ws://{addr}/ws/bench"))
        .await
        .expect("bob connect");
    // Let both server-side upgrades subscribe before the first publish.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Seed both CRDT docs with a shared base cell (out of band).
    let mut alice = SyncDocument::new("alice");
    alice.add_cell(&Cell::new("top"));
    let mut bob = SyncDocument::new("bob");
    bob.apply_update(&alice.encode_state_update())
        .expect("seed bob");

    let iters = 2000usize;
    let warmup = 200usize;
    let mut samples: Vec<Duration> = Vec::with_capacity(iters - warmup);
    let payload_len;
    {
        // Measure one representative payload size for the report.
        let sv = bob.state_vector();
        let probe = alice.encode_update(&sv).expect("probe");
        payload_len = probe.len();
    }

    for i in 0..iters {
        let sv_bob = bob.state_vector();
        let x = i as i32;
        let rect = Rect::new(Point::new(x, 0), Point::new(x + 10, 10));
        alice.add_shape(
            "top",
            &DrawShape::new(LayerId::new(1, 0), ShapeKind::Rect(rect)),
        );
        let update = alice.encode_update(&sv_bob).expect("encode diff");

        let t0 = Instant::now();
        alice_ws
            .send(Message::Binary(update.into()))
            .await
            .expect("send");
        let frame = recv_binary(&mut bob_ws, Duration::from_secs(5))
            .await
            .expect("peer received the edit");
        bob.apply_update(&frame).expect("apply on bob");
        let dt = t0.elapsed();
        if i >= warmup {
            samples.push(dt);
        }
    }

    samples.sort_unstable();
    let n = samples.len();
    let mean = samples.iter().sum::<Duration>() / (n as u32);
    let median = samples[n / 2];
    let p95 = samples[(n * 95) / 100];
    let max = samples[n - 1];
    let bob_shapes = bob.document().cell("top").map_or(0, |c| c.shapes.len());

    // Local echo: applying an edit to the local document is synchronous.
    let mut solo = SyncDocument::new("solo");
    solo.add_cell(&Cell::new("top"));
    let local_t = Instant::now();
    solo.add_shape(
        "top",
        &DrawShape::new(
            LayerId::new(1, 0),
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(1, 1))),
        ),
    );
    let local = local_t.elapsed();

    println!("collab echo over localhost relay (send -> peer applied)");
    println!("  samples: {n} (after {warmup} warmup), payload ~{payload_len} bytes/edit");
    println!("  mean   {mean:?}");
    println!("  median {median:?}");
    println!("  p95    {p95:?}");
    println!("  max    {max:?}");
    println!("  correctness: bob applied {bob_shapes} shapes (expected {iters})");
    println!("local edit apply (single add_shape, no network): {local:?}");
}
