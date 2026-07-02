//! Rate-limited demo server for Reticle.
//!
//! This crate freezes the HTTP API ([`SubmitRequest`], [`SubmitResponse`],
//! [`StatusResponse`], [`CancelRequest`]) and the mandatory [`LimitConfig`]. The
//! axum service that implements them, and the enforcement of every limit (per-IP
//! rate and concurrency, a global cap, token and command budgets, a maximum prompt
//! length, and a task-vocabulary input filter), lands in a later wave.

mod api;
mod limits;

pub use api::{CancelRequest, SessionState, StatusResponse, SubmitRequest, SubmitResponse};
pub use limits::LimitConfig;

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
