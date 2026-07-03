//! The Reticle public demo server.
//!
//! Composes three pieces into one runnable process (ADR 0024):
//!
//! * the rate-limited [`reticle_demo`] service, built from a non-permissive
//!   [`LimitConfig`](reticle_demo::LimitConfig) so it is safe to expose;
//! * an in-process [`reticle_server`] collaboration relay, so a spectator can watch
//!   the room each session draws into;
//! * a harness: the real `reticle-agent` propose-verify-correct loop when
//!   `ANTHROPIC_API_KEY` is set (streaming its atomic steps to the room, ADR 0025),
//!   otherwise the offline [`MockHarness`](reticle_demo::MockHarness) so a bare
//!   `just demo-up` runs with no key and no network.
//!
//! Everything is configured through the environment (see [`config`]); there is no
//! config file to mount. The service binds `HOST:PORT` (default `127.0.0.1:3040`)
//! and the relay binds `RETICLE_RELAY_ADDR` (default `127.0.0.1:3041`).
//!
//! Run it with `just demo-up`.

mod config;
mod harness;
mod relay;

use config::DemoConfig;
use harness::{AgentHarness, LoopBounds, ModelSource, demo_script};
use reticle_demo::DemoServer;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let cfg = DemoConfig::from_env();

    // Bring up the in-process relay first (if enabled) so the first session's
    // publisher can connect immediately.
    let relay_ws_base = resolve_relay(&cfg).await;

    // Choose the harness. Real agent loop when a key is present, else the mock.
    let server = if cfg.have_api_key {
        println!("reticle-demo-server: ANTHROPIC_API_KEY present, using the reticle-agent harness");
        let harness = AgentHarness::new(
            ModelSource::Anthropic {
                base_url: reticle_agent::DEFAULT_BASE_URL.to_owned(),
                model: reticle_agent::DEFAULT_MODEL.to_owned(),
            },
            LoopBounds::default(),
            relay_ws_base.clone(),
        );
        DemoServer::with_harness(cfg.limits.clone(), harness)
    } else {
        println!(
            "reticle-demo-server: no ANTHROPIC_API_KEY, using the offline scripted harness \
             (set the key for the live model)"
        );
        // Offline, but still the real streaming loop: a deterministic scripted model
        // drives the same propose-verify-correct path and streams to the room.
        let harness = AgentHarness::new(
            ModelSource::Scripted(demo_script()),
            LoopBounds::default(),
            relay_ws_base.clone(),
        );
        DemoServer::with_harness(cfg.limits.clone(), harness)
    };

    let bind = cfg.bind_addr();
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("reticle-demo-server: demo service listening on http://{bind}");
    print_limits(&cfg);

    axum::serve(listener, server.into_make_service()).await
}

/// Resolves the relay: brings up the in-process relay when configured and returns
/// the `ws://` base a publisher connects to, or `None` when the relay is disabled
/// or fails to bind (in which case the loop runs without a watchable room).
async fn resolve_relay(cfg: &DemoConfig) -> Option<String> {
    let Some(addr) = &cfg.relay_addr else {
        println!(
            "reticle-demo-server: in-process relay disabled; \
             compose with an external reticle-server (see docs/deployment.md)"
        );
        return None;
    };
    match start_relay(addr).await {
        Ok(base) => {
            println!("reticle-demo-server: relay listening on {base}/ws/{{room}}");
            Some(base)
        }
        Err(e) => {
            // A relay that will not bind is not fatal: the demo still serves and runs
            // the loop, just without a watchable room. Report it plainly.
            eprintln!("reticle-demo-server: relay disabled ({e})");
            None
        }
    }
}

/// Binds and spawns the collaboration relay on `addr`, returning the `ws://` base
/// URL a publisher connects to.
async fn start_relay(addr: &str) -> std::io::Result<String> {
    use reticle_server::{RelayState, serve};
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    tokio::spawn(async move {
        if let Err(e) = serve(listener, RelayState::new()).await {
            eprintln!("reticle-demo-server: relay stopped: {e}");
        }
    });
    Ok(format!("ws://{bound}"))
}

/// Prints the enforced limits at startup so an operator can see the demo is bounded.
fn print_limits(cfg: &DemoConfig) {
    let l = &cfg.limits;
    println!("reticle-demo-server: limits enforced:");
    println!("  per-IP rate         {} / min", l.per_ip_rate_per_min);
    println!("  per-IP concurrency  {}", l.per_ip_concurrency);
    println!("  global concurrency  {}", l.global_concurrency);
    println!("  token budget        {} / session", l.token_budget);
    println!("  command budget      {} / session", l.command_budget);
    println!("  max prompt length   {} chars", l.max_prompt_len);
    println!("  vocabulary words    {}", l.allowed_vocabulary.len());
}

#[cfg(test)]
mod tests {
    //! A smoke test that drives the composed demo router in-process, proving the
    //! binary's service responds and enforces its limits, without leaving a
    //! blocking server behind (the router is driven with a `tower` oneshot, exactly
    //! as `reticle-demo`'s abuse tests do).

    use crate::config::demo_limits;
    use crate::harness::{AgentHarness, LoopBounds, ModelSource, demo_script};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use reticle_demo::{CLIENT_IP_HEADER, DemoServer, SubmitResponse};
    use std::time::Duration;
    use tower::ServiceExt;

    /// A harness with no relay and instant pacing, for the smoke test.
    fn test_harness() -> AgentHarness {
        AgentHarness::new(
            ModelSource::Scripted(demo_script()),
            LoopBounds {
                max_iterations: 4,
                tokens_per_iteration: 1_000,
                step_delay: Duration::ZERO,
            },
            None,
        )
    }

    async fn send(server: &DemoServer, req: Request<Body>) -> (StatusCode, Vec<u8>) {
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

    fn submit(ip: &str, prompt: &str) -> Request<Body> {
        let body = format!(r#"{{"prompt":{}}}"#, serde_json::to_string(prompt).unwrap());
        Request::builder()
            .method("POST")
            .uri("/submit")
            .header("content-type", "application/json")
            .header(CLIENT_IP_HEADER, ip)
            .body(Body::from(body))
            .expect("build request")
    }

    #[tokio::test]
    async fn composed_router_accepts_an_in_vocabulary_submit() {
        // The composed server (demo limits + the real streaming harness) accepts a
        // valid submission and returns a session id and a watchable room.
        let server = DemoServer::with_harness(demo_limits(), test_harness());
        let (status, body) =
            send(&server, submit("10.0.0.1", "place a clean met1 rectangle")).await;
        assert_eq!(status, StatusCode::OK, "in-vocabulary submit is accepted");
        let resp: SubmitResponse = serde_json::from_slice(&body).expect("decode submit response");
        assert!(!resp.session_id.is_empty(), "a session id is returned");
        assert!(
            resp.room.contains(&resp.session_id),
            "the room names the session"
        );
    }

    #[tokio::test]
    async fn composed_router_rejects_off_vocabulary_with_400() {
        // The demo vocabulary filter is active on the composed server: a prompt that
        // strays off the task words is rejected before any session is created.
        let server = DemoServer::with_harness(demo_limits(), test_harness());
        let (status, _) = send(
            &server,
            submit("10.0.0.2", "write me a poem about the weather"),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "off-vocabulary prompt is 400"
        );
    }

    #[tokio::test]
    async fn composed_router_rejects_oversized_prompt_with_400() {
        let server = DemoServer::with_harness(demo_limits(), test_harness());
        let long = "place a rectangle ".repeat(40); // over 400 chars
        let (status, _) = send(&server, submit("10.0.0.3", &long)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "oversized prompt is 400");
        // Nothing was spent: the input check runs before any slot is reserved.
        assert_eq!(server.state().global_active(), 0);
    }
}
