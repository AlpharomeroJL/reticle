//! Rate-limited demo server for Reticle.
//!
//! This crate freezes the HTTP API ([`SubmitRequest`], [`SubmitResponse`],
//! [`StatusResponse`], [`CancelRequest`]) and the mandatory [`LimitConfig`], and
//! implements the axum service that serves them with every limit enforced.
//!
//! # The service
//!
//! [`DemoServer`] is built from a [`LimitConfig`] and exposes three routes:
//!
//! * `POST /submit` accepts a [`SubmitRequest`], validates it, applies the rate
//!   and concurrency limits, starts a session behind the [`Harness`], and returns
//!   a [`SubmitResponse`] with a session id and a watchable room;
//! * `GET /status/{id}` returns the session's [`StatusResponse`];
//! * `POST /cancel` stops a session.
//!
//! There is no way to construct the server without a [`LimitConfig`], so the
//! demo cannot be exposed unbounded.
//!
//! # Limits and their HTTP behaviour
//!
//! | Limit | Field | On breach |
//! | --- | --- | --- |
//! | prompt length | [`LimitConfig::max_prompt_len`] | `400 Bad Request` |
//! | task vocabulary | [`LimitConfig::allowed_vocabulary`] | `400 Bad Request` |
//! | per-IP rate | [`LimitConfig::per_ip_rate_per_min`] | `429 Too Many Requests` |
//! | per-IP concurrency | [`LimitConfig::per_ip_concurrency`] | `409 Conflict` |
//! | global concurrency | [`LimitConfig::global_concurrency`] | `503 Service Unavailable` |
//! | token budget | [`LimitConfig::token_budget`] | session cancelled |
//! | command budget | [`LimitConfig::command_budget`] | session cancelled |
//!
//! # The harness
//!
//! The server does not run an agent directly; it drives sessions through the
//! [`Harness`] trait. [`MockHarness`] is the built-in stand-in that walks a
//! session Queued to Running to Done, honours cancellation, and cancels the
//! session if a budget is exceeded. The real `reticle-agent` loop plugs in behind
//! the same trait in a later wave.
//!
//! # Example
//!
//! ```no_run
//! # async fn run() {
//! use reticle_demo::{DemoServer, LimitConfig};
//!
//! let server = DemoServer::new(LimitConfig::default());
//! let listener = tokio::net::TcpListener::bind("127.0.0.1:3040").await.unwrap();
//! axum::serve(listener, server.into_make_service()).await.unwrap();
//! # }
//! ```

mod api;
mod error;
mod harness;
mod limits;
mod rate;
mod server;
mod vocab;

pub use api::{CancelRequest, SessionState, StatusResponse, SubmitRequest, SubmitResponse};
pub use error::DemoError;
pub use harness::{Budget, CancelToken, Harness, MockHarness, SessionHandle};
pub use limits::LimitConfig;
pub use server::{CLIENT_IP_HEADER, DemoServer, DemoState, UNKNOWN_IP};

#[cfg(test)]
mod tests {
    use super::{LimitConfig, SessionState, StatusResponse, SubmitRequest};

    #[test]
    fn api_and_limits_round_trip_json() {
        let req = SubmitRequest {
            prompt: "place a metal1 rectangle".into(),
        };
        let back: SubmitRequest =
            serde_json::from_str(&serde_json::to_string(&req).unwrap()).unwrap();
        assert_eq!(req, back);

        let status = StatusResponse {
            session_id: "s1".into(),
            state: SessionState::Running,
            iteration: 1,
            violations: 2,
            message: "verifying".into(),
        };
        let back: StatusResponse =
            serde_json::from_str(&serde_json::to_string(&status).unwrap()).unwrap();
        assert_eq!(status, back);

        // The default limits are non-permissive (bounded concurrency and budgets).
        let limits = LimitConfig::default();
        assert!(limits.per_ip_concurrency >= 1);
        assert!(limits.global_concurrency >= limits.per_ip_concurrency);
        let back: LimitConfig =
            serde_json::from_str(&serde_json::to_string(&limits).unwrap()).unwrap();
        assert_eq!(limits, back);
    }
}
