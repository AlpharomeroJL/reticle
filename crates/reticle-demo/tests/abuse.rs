//! Abuse tests that drive the real demo router end to end.
//!
//! Every test here builds a [`DemoServer`] from a concrete [`LimitConfig`] and
//! exercises the actual axum [`Router`] rather than the internal helpers: most
//! use `tower::ServiceExt::oneshot` to send a real HTTP request in-process, and
//! the lifecycle tests bind a loopback socket and talk to it over TCP with a
//! hand-rolled client so the spawned harness task runs on the server runtime.
//!
//! The point is to prove that each [`LimitConfig`] field is enforced on the wire
//! with the documented status code, and that a session can be started, cancelled,
//! and run to completion through the public endpoints.
//!
//! | Abuse | Config field exercised | Expected status |
//! | --- | --- | --- |
//! | flood one IP | `per_ip_rate_per_min` | `429` |
//! | two sessions, one IP | `per_ip_concurrency` | `409` |
//! | fill the server | `global_concurrency` | `503` |
//! | oversized prompt | `max_prompt_len` | `400` |
//! | off-vocabulary prompt | `allowed_vocabulary` | `400` |
//! | tiny budget | `token_budget` / `command_budget` | session `Cancelled` |

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use reticle_demo::{
    CLIENT_IP_HEADER, DemoServer, LimitConfig, MockHarness, SessionState, StatusResponse,
    SubmitResponse,
};
use tower::ServiceExt;

/// A permissive base config: every limit is wide open so a test can tighten the
/// single field it wants to exercise without another limit tripping first.
fn base_limits() -> LimitConfig {
    LimitConfig {
        per_ip_rate_per_min: 1_000,
        per_ip_concurrency: 1_000,
        global_concurrency: 1_000,
        token_budget: 1_000_000,
        command_budget: 1_000_000,
        max_prompt_len: 400,
        allowed_vocabulary: Vec::new(),
    }
}

/// Builds a `POST /submit` request for `prompt`, attributed to `ip` via the
/// trusted client-IP header.
fn submit_request(ip: &str, prompt: &str) -> Request<Body> {
    let body = format!(r#"{{"prompt":{}}}"#, json_string(prompt));
    Request::builder()
        .method("POST")
        .uri("/submit")
        .header("content-type", "application/json")
        .header(CLIENT_IP_HEADER, ip)
        .body(Body::from(body))
        .expect("build submit request")
}

/// JSON-encodes a string, so prompts with quotes or backslashes stay valid.
fn json_string(s: &str) -> String {
    serde_json::to_string(s).expect("encode string as json")
}

/// Sends one request through the router and returns the status and raw body.
async fn send(server: &DemoServer, req: Request<Body>) -> (StatusCode, Vec<u8>) {
    let response = server
        .clone()
        .router()
        .oneshot(req)
        .await
        .expect("router handled request");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect response body")
        .to_bytes()
        .to_vec();
    (status, bytes)
}

/// Sends a submit and asserts it succeeded, returning the decoded response.
async fn submit_ok(server: &DemoServer, ip: &str, prompt: &str) -> SubmitResponse {
    let (status, body) = send(server, submit_request(ip, prompt)).await;
    assert_eq!(status, StatusCode::OK, "submit should be accepted");
    serde_json::from_slice(&body).expect("decode submit response")
}

// ---- Rate limiting -------------------------------------------------------

#[tokio::test]
async fn floods_one_ip_get_429_after_the_per_minute_cap() {
    // Only the rate limit is tight; concurrency is wide so it never trips first.
    let limits = LimitConfig {
        per_ip_rate_per_min: 3,
        ..base_limits()
    };
    // A long-running harness keeps accepted sessions alive, proving the 429 comes
    // from the rate window and not from sessions finishing and freeing capacity.
    let server = DemoServer::with_harness(
        limits,
        MockHarness::with_profile(1_000, Duration::from_secs(30), 1, 1),
    );

    // The first three submissions from one IP are accepted.
    for i in 0..3 {
        let (status, _) = send(&server, submit_request("10.0.0.1", "draw")).await;
        assert_eq!(status, StatusCode::OK, "submission {i} should be accepted");
    }

    // The fourth in the same window is rejected with 429.
    let (status, _) = send(&server, submit_request("10.0.0.1", "draw")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    // A different IP is unaffected: the window is per source.
    let (status, _) = send(&server, submit_request("10.0.0.2", "draw")).await;
    assert_eq!(status, StatusCode::OK, "a fresh IP has its own budget");
}

// ---- Per-IP concurrency --------------------------------------------------

#[tokio::test]
async fn second_live_session_from_one_ip_gets_409() {
    // One session per IP; rate and global caps are wide.
    let limits = LimitConfig {
        per_ip_concurrency: 1,
        ..base_limits()
    };
    let server = DemoServer::with_harness(
        limits,
        MockHarness::with_profile(1_000, Duration::from_secs(30), 1, 1),
    );

    // The first session for this IP takes the only per-IP slot and stays live.
    let _first = submit_ok(&server, "10.0.0.1", "draw").await;
    assert_eq!(server.state().global_active(), 1);

    // A second concurrent submit from the same IP is refused with 409.
    let (status, _) = send(&server, submit_request("10.0.0.1", "draw")).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // A different IP still gets in under the wide global cap.
    let (status, _) = send(&server, submit_request("10.0.0.2", "draw")).await;
    assert_eq!(status, StatusCode::OK, "the cap is per IP, not global");
}

// ---- Global concurrency --------------------------------------------------

#[tokio::test]
async fn filling_the_server_gets_503() {
    // Two sessions server-wide; per-IP allows several so global is the binding cap.
    let limits = LimitConfig {
        per_ip_concurrency: 5,
        global_concurrency: 2,
        ..base_limits()
    };
    let server = DemoServer::with_harness(
        limits,
        MockHarness::with_profile(1_000, Duration::from_secs(30), 1, 1),
    );

    // Two distinct IPs fill the global capacity of two.
    let _a = submit_ok(&server, "10.0.0.1", "draw").await;
    let _b = submit_ok(&server, "10.0.0.2", "draw").await;
    assert_eq!(server.state().global_active(), 2);

    // A third submit, even from a brand-new IP under its per-IP cap, is refused
    // with 503 because the server as a whole is full.
    let (status, _) = send(&server, submit_request("10.0.0.3", "draw")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

// ---- Prompt length -------------------------------------------------------

#[tokio::test]
async fn oversized_prompt_gets_400_without_consuming_capacity() {
    let limits = LimitConfig {
        max_prompt_len: 16,
        ..base_limits()
    };
    let server = DemoServer::new(limits);

    let long = "draw a very long shape indeed".repeat(4);
    let (status, _) = send(&server, submit_request("10.0.0.1", &long)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Length is checked before any slot is reserved: nothing was spent.
    assert_eq!(
        server.state().global_active(),
        0,
        "a rejected oversized prompt must not reserve a concurrency slot"
    );
}

// ---- Task vocabulary -----------------------------------------------------

#[tokio::test]
async fn off_vocabulary_prompt_gets_400() {
    let limits = LimitConfig {
        allowed_vocabulary: ["place", "metal1", "rectangle"]
            .into_iter()
            .map(String::from)
            .collect(),
        ..base_limits()
    };
    let server = DemoServer::new(limits);

    // A prompt using only allowed words (plus stoplist glue and numbers) passes.
    let (status, _) = send(
        &server,
        submit_request("10.0.0.1", "place a metal1 rectangle at 10 20"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "in-vocabulary prompt is accepted");

    // A prompt straying off the task set is rejected with 400, and the offending
    // word is named in the error body.
    let (status, body) = send(
        &server,
        submit_request("10.0.0.2", "write me a poem about metal1"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let text = String::from_utf8(body).expect("utf8 error body");
    assert!(
        text.contains("poem"),
        "the rejection should name the off-vocabulary word, got: {text}"
    );
}

// ---- Lifecycle over a real socket ---------------------------------------

/// Binds the server to an ephemeral loopback port and serves it on the current
/// runtime, returning the bound address and the serving task's handle.
///
/// A live socket (rather than an in-process `oneshot`) is used for the lifecycle
/// tests because they need the spawned harness task to keep running and advancing
/// the session while the client polls; `oneshot` drives only a single request's
/// future.
async fn serve(server: DemoServer) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, server.into_make_service()).await;
    });
    (addr, handle)
}

/// A minimal one-shot HTTP/1.1 client: sends `method path` with `body` and
/// returns the numeric status and the response body text.
///
/// The demo has no client crate, and pulling a full HTTP client into the dev
/// dependencies would be heavier than the tests need. Reading the status line and
/// the body after the header terminator is enough to assert lifecycle behaviour.
async fn http(addr: std::net::SocketAddr, method: &str, path: &str, body: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to server");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len(),
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write request");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.expect("read response");
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("parse status code");
    let payload = text
        .split_once("\r\n\r\n")
        .map_or_else(String::new, |(_, rest)| rest.to_owned());
    (status, payload)
}

/// Polls `GET /status/{id}` until the session reaches `want` or the attempt
/// budget is exhausted, returning the final observed status.
async fn poll_until(addr: std::net::SocketAddr, id: &str, want: SessionState) -> StatusResponse {
    let path = format!("/status/{id}");
    let mut last = None;
    for _ in 0..200 {
        let (code, body) = http(addr, "GET", &path, "").await;
        assert_eq!(code, 200, "status should be found");
        let status: StatusResponse = serde_json::from_str(&body).expect("decode status");
        let reached = status.state == want;
        last = Some(status);
        if reached {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    last.expect("at least one status poll")
}

#[tokio::test]
async fn a_normal_submit_reaches_done() {
    // A quick harness so the session completes promptly.
    let server = DemoServer::with_harness(
        base_limits(),
        MockHarness::with_profile(3, Duration::from_millis(5), 10, 1),
    );
    let (addr, task) = serve(server).await;

    let (code, body) = http(addr, "POST", "/submit", r#"{"prompt":"draw"}"#).await;
    assert_eq!(code, 200);
    let submitted: SubmitResponse = serde_json::from_str(&body).expect("decode submit");

    let status = poll_until(addr, &submitted.session_id, SessionState::Done).await;
    assert_eq!(
        status.state,
        SessionState::Done,
        "the session should finish successfully"
    );

    task.abort();
}

#[tokio::test]
async fn cancel_stops_a_running_session() {
    // A long harness so the session is comfortably Running when we cancel it.
    let server = DemoServer::with_harness(
        base_limits(),
        MockHarness::with_profile(1_000, Duration::from_millis(20), 1, 1),
    );
    let (addr, task) = serve(server).await;

    let (code, body) = http(addr, "POST", "/submit", r#"{"prompt":"draw"}"#).await;
    assert_eq!(code, 200);
    let submitted: SubmitResponse = serde_json::from_str(&body).expect("decode submit");

    // Wait for it to actually be Running before cancelling.
    let running = poll_until(addr, &submitted.session_id, SessionState::Running).await;
    assert_eq!(
        running.state,
        SessionState::Running,
        "session should be live"
    );

    // Cancel it and confirm it settles into Cancelled.
    let cancel_body = format!(r#"{{"session_id":{}}}"#, json_string(&submitted.session_id));
    let (code, _) = http(addr, "POST", "/cancel", &cancel_body).await;
    assert_eq!(code, 200, "cancel of a known session succeeds");

    let cancelled = poll_until(addr, &submitted.session_id, SessionState::Cancelled).await;
    assert_eq!(
        cancelled.state,
        SessionState::Cancelled,
        "a cancelled session ends in the Cancelled state"
    );

    task.abort();
}

#[tokio::test]
async fn a_tiny_budget_cancels_the_session() {
    // The command budget (1) is smaller than one iteration's charge (2), so the
    // very first iteration overruns and the harness cancels the session.
    let limits = LimitConfig {
        command_budget: 1,
        ..base_limits()
    };
    let server = DemoServer::with_harness(
        limits,
        MockHarness::with_profile(3, Duration::from_millis(5), 10, 2),
    );
    let (addr, task) = serve(server).await;

    let (code, body) = http(addr, "POST", "/submit", r#"{"prompt":"draw"}"#).await;
    assert_eq!(code, 200);
    let submitted: SubmitResponse = serde_json::from_str(&body).expect("decode submit");

    let status = poll_until(addr, &submitted.session_id, SessionState::Cancelled).await;
    assert_eq!(
        status.state,
        SessionState::Cancelled,
        "exceeding the command budget cancels the session"
    );
    assert!(
        status.message.contains("budget"),
        "the status should attribute the stop to the budget, got: {}",
        status.message
    );

    task.abort();
}
