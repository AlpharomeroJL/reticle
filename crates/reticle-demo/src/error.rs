//! The demo server's rejection reasons and their HTTP status codes.
//!
//! Every way a submission can be refused is one [`DemoError`] variant, and each
//! variant carries exactly one HTTP status so the mapping from a limit to its
//! wire behaviour lives in one place (see [`DemoError::status`]). The response
//! body is a small JSON object `{ "error": "..." }` so a client can show the
//! reason without parsing prose.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// A reason a request was refused, together with its HTTP status.
///
/// The status codes are chosen so a caller can tell a *retryable* refusal (rate
/// or capacity: `429`, `409`, `503`) from a *permanent* one (a malformed or
/// disallowed prompt: `400`, or an unknown session: `404`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DemoError {
    /// The source IP has submitted more than `per_ip_rate_per_min` times in the
    /// current window. Maps to `429 Too Many Requests`.
    RateLimited,
    /// The source IP already holds `per_ip_concurrency` live sessions. Maps to
    /// `409 Conflict`: retryable once one of the caller's sessions finishes.
    PerIpConcurrency,
    /// The server already holds `global_concurrency` live sessions. Maps to
    /// `503 Service Unavailable`: retryable once the server drains.
    GlobalConcurrency,
    /// The prompt is longer than `max_prompt_len` characters. Maps to
    /// `400 Bad Request`.
    PromptTooLong {
        /// The prompt's length in characters.
        len: usize,
        /// The configured maximum.
        max: usize,
    },
    /// The prompt uses words outside the allowed task vocabulary. Maps to
    /// `400 Bad Request`. The rejected words are listed so the caller can see
    /// which tokens were out of scope.
    OffVocabulary {
        /// The offending words, in first-seen order (capped for brevity).
        words: Vec<String>,
    },
    /// No session with the given id exists. Maps to `404 Not Found`.
    UnknownSession {
        /// The id that was looked up.
        session_id: String,
    },
}

impl DemoError {
    /// The HTTP status this rejection maps to.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            DemoError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            DemoError::PerIpConcurrency => StatusCode::CONFLICT,
            DemoError::GlobalConcurrency => StatusCode::SERVICE_UNAVAILABLE,
            DemoError::PromptTooLong { .. } | DemoError::OffVocabulary { .. } => {
                StatusCode::BAD_REQUEST
            }
            DemoError::UnknownSession { .. } => StatusCode::NOT_FOUND,
        }
    }

    /// A short, stable human-readable reason for this rejection.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            DemoError::RateLimited => "rate limit exceeded for this source IP".to_owned(),
            DemoError::PerIpConcurrency => {
                "too many concurrent sessions for this source IP".to_owned()
            }
            DemoError::GlobalConcurrency => "the demo is at capacity; try again shortly".to_owned(),
            DemoError::PromptTooLong { len, max } => {
                format!("prompt is {len} characters; the maximum is {max}")
            }
            DemoError::OffVocabulary { words } => {
                format!(
                    "prompt uses words outside the demo task set: {}",
                    words.join(", ")
                )
            }
            DemoError::UnknownSession { session_id } => {
                format!("no session with id {session_id}")
            }
        }
    }
}

/// The JSON body returned for a rejected request.
#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl IntoResponse for DemoError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            error: self.message(),
        };
        (self.status(), Json(body)).into_response()
    }
}
