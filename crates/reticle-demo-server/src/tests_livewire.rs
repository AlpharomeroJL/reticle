//! Live-wiring tests for the composed demo server (Wave 3, item 1).
//!
//! These prove the whole path works end to end, not just the pieces:
//!
//! * `live_wiring_streams_the_drawn_geometry_to_a_watcher` runs the real
//!   [`AgentHarness`] against the in-process relay while a watcher (a plain
//!   WebSocket peer, exactly what a browser `reticle-sync` client is) is joined to
//!   the room, then decodes the binary frames it received and materializes them
//!   into a document, asserting the geometry the agent drew actually arrived over
//!   the wire.
//! * `demo_server_drives_the_harness_and_cancel_stops_it` drives the composed
//!   `DemoServer` through its HTTP API: a submit starts the real harness, and a
//!   `POST /cancel` moves the running session to `Cancelled` server-side.
//!
//! The watcher joins before the harness runs (as `reticle-server`'s own relay test
//! does), so delivery is live and the test does not depend on room-log retention.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use tower::ServiceExt;

use reticle_agent_api::AgentCommand;
use reticle_agent_api::args::{LayerArg, PointArg, RectArg};
use reticle_bench::model::MockModel;
use reticle_demo::{
    Budget, CLIENT_IP_HEADER, DemoServer, Harness, SessionHandle, SessionState, StatusResponse,
    SubmitResponse,
};

use crate::config::demo_limits;
use crate::harness::{AgentHarness, LoopBounds, ModelSource, demo_script};

/// met1 in the SKY130 tech (layer 68, datatype 20).
fn met1() -> LayerArg {
    LayerArg {
        layer: 68,
        datatype: 20,
    }
}

#[tokio::test]
async fn live_wiring_streams_the_drawn_geometry_to_a_watcher() {
    use reticle_server::{RelayState, serve};
    use reticle_sync::SyncDocument;
    use tokio::net::TcpListener;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message;

    // A relay on an ephemeral port.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind relay");
    let addr = listener.local_addr().expect("relay addr");
    tokio::spawn(async move {
        let _ = serve(listener, RelayState::new()).await;
    });
    let ws_base = format!("ws://{addr}");
    let room = "live-wiring-room";

    // The watcher joins the room first, so every frame the harness ships is
    // delivered live (no dependence on room-log replay timing).
    let (mut watcher, _) = connect_async(format!("{ws_base}/ws/{room}"))
        .await
        .expect("watcher connects");
    tokio::time::sleep(Duration::from_millis(150)).await;

    // The real harness (offline scripted model) draws a clean met1 rectangle and
    // streams each atomic step to the room as a raw yrs frame.
    let harness = AgentHarness::new(
        ModelSource::Scripted(demo_script()),
        LoopBounds {
            max_iterations: 4,
            tokens_per_iteration: 1_000,
            step_delay: Duration::from_millis(40),
        },
        Some(ws_base.clone()),
    );
    let handle = SessionHandle::new("live".into());
    harness
        .run(
            "place a clean met1 rectangle".into(),
            room.into(),
            Budget {
                token_budget: 1_000_000,
                command_budget: 1_000,
            },
            handle.clone(),
        )
        .await;
    assert_eq!(
        handle.status().state,
        SessionState::Done,
        "the scripted run converges: {}",
        handle.status().message
    );

    // Drain the frames the watcher received and apply them to a fresh CRDT peer,
    // exactly as a browser reticle-sync client does.
    let mut peer = SyncDocument::new("watcher");
    let mut frames = 0usize;
    while let Ok(Some(Ok(msg))) =
        tokio::time::timeout(Duration::from_millis(750), watcher.next()).await
    {
        if let Message::Binary(bytes) = msg {
            peer.apply_update(&bytes)
                .expect("frame is a valid yrs update");
            frames += 1;
        }
    }
    assert!(frames > 0, "the watcher received at least one CRDT frame");

    // The materialized document carries the geometry the agent drew: a cell `top`
    // with the single met1 rectangle.
    let doc = peer.to_document();
    let cell = doc
        .cell("top")
        .expect("the drawn cell `top` reached the watcher");
    assert_eq!(
        cell.shapes.len(),
        1,
        "the one drawn rectangle reached the watcher (frames applied: {frames})"
    );
}

#[tokio::test]
async fn demo_server_drives_the_harness_and_cancel_stops_it() {
    // A model that never satisfies DRC (a met1 rectangle narrower than the 140 nm
    // minimum width), so the loop keeps running until it is cancelled rather than
    // converging. A slow step pace leaves a wide window to cancel mid-run.
    let create = AgentCommand::CreateCell { name: "top".into() };
    let narrow = AgentCommand::AddRect {
        cell: "top".into(),
        layer: met1(),
        rect: RectArg {
            min: PointArg { x: 0, y: 0 },
            max: PointArg { x: 100, y: 400 },
        },
    };
    let mut attempts = vec![vec![create, narrow.clone()]];
    for _ in 0..6 {
        attempts.push(vec![narrow.clone()]);
    }
    let harness = AgentHarness::new(
        ModelSource::Scripted(MockModel::new().with_default(attempts)),
        LoopBounds {
            max_iterations: 8,
            tokens_per_iteration: 1_000,
            step_delay: Duration::from_millis(300),
        },
        None, // no relay needed to prove server-side cancel
    );
    let server = DemoServer::with_harness(demo_limits(), harness);

    // Submit: the demo server starts the real harness behind the session.
    let (code, body) = send(
        &server,
        "POST",
        "/submit",
        "10.0.0.7",
        r#"{"prompt":"place a met1 rectangle"}"#,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "submit accepted");
    let submitted: SubmitResponse = serde_json::from_slice(&body).expect("submit response");

    // Let it enter Running, then cancel it over HTTP.
    let running = poll_until(&server, &submitted.session_id, SessionState::Running).await;
    assert_eq!(running.state, SessionState::Running);
    let cancel_body = format!(
        r#"{{"session_id":{}}}"#,
        serde_json::to_string(&submitted.session_id).unwrap()
    );
    let (code, _) = send(&server, "POST", "/cancel", "10.0.0.7", &cancel_body).await;
    assert_eq!(code, StatusCode::OK, "cancel accepted");

    // The running session reaches Cancelled server-side.
    let cancelled = poll_until(&server, &submitted.session_id, SessionState::Cancelled).await;
    assert_eq!(cancelled.state, SessionState::Cancelled);
    assert_eq!(server.state().global_active(), 0, "the slot is released");
}

/// Drives the composed router with one in-process request.
async fn send(
    server: &DemoServer,
    method: &str,
    uri: &str,
    ip: &str,
    body: &str,
) -> (StatusCode, Vec<u8>) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header(CLIENT_IP_HEADER, ip)
        .body(Body::from(body.to_owned()))
        .expect("build request");
    let resp = server
        .clone()
        .router()
        .oneshot(req)
        .await
        .expect("router handled request");
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes()
        .to_vec();
    (status, bytes)
}

/// Polls `GET /status/{id}` until the session reaches `want`, or panics after a
/// generous deadline.
async fn poll_until(server: &DemoServer, id: &str, want: SessionState) -> StatusResponse {
    for _ in 0..200 {
        let (_code, body) = send(server, "GET", &format!("/status/{id}"), "10.0.0.7", "").await;
        if let Ok(status) = serde_json::from_slice::<StatusResponse>(&body)
            && status.state == want
        {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("session {id} did not reach {want:?} within the deadline");
}
