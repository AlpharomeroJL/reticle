//! The axum service: submit, status, and cancel, with every demo limit enforced.
//!
//! [`DemoServer`] is constructed from a [`LimitConfig`] and a [`Harness`]. It
//! cannot be built without a limit configuration, which is the point of the demo:
//! there is no code path that exposes the endpoints unbounded. [`DemoServer::router`]
//! produces the axum [`Router`]; [`DemoServer::into_make_service`] produces a
//! make-service with per-connection peer addresses attached for real deployment.
//!
//! # Enforcement summary
//!
//! | Limit | Field | Rejection |
//! | --- | --- | --- |
//! | prompt length | `max_prompt_len` | `400` |
//! | task vocabulary | `allowed_vocabulary` | `400` |
//! | per-IP rate | `per_ip_rate_per_min` | `429` |
//! | per-IP concurrency | `per_ip_concurrency` | `409` |
//! | global concurrency | `global_concurrency` | `503` |
//! | token / command budget | `token_budget`, `command_budget` | session cancelled |
//!
//! The order matters: cheap input validation (length, vocabulary) runs before
//! any stateful counter is touched, so a malformed prompt never consumes a rate
//! token or a concurrency slot. Rate is checked next, then per-IP and global
//! concurrency. Only once all gates pass is a session created and the harness
//! spawned.
//!
//! # Client IP
//!
//! Enforcement is per source IP. The IP is taken from the [`CLIENT_IP_HEADER`]
//! request header when present (the header a trusted front proxy sets, and the
//! hook tests use to simulate distinct clients), otherwise from the connection's
//! peer address. A request with neither is attributed to [`UNKNOWN_IP`] and still
//! rate- and concurrency-limited, so an unidentifiable flood cannot bypass the
//! caps.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::connect_info::IntoMakeServiceWithConnectInfo;
use axum::extract::{ConnectInfo, FromRequestParts, Path, State};
use axum::http::HeaderMap;
use axum::http::request::Parts;
use axum::routing::{get, post};

use crate::api::{CancelRequest, StatusResponse, SubmitRequest, SubmitResponse};
use crate::error::DemoError;
use crate::harness::{Budget, Harness, SessionHandle};
use crate::limits::LimitConfig;
use crate::rate::RateLimiter;
use crate::vocab::offending_words;

/// The header a trusted front proxy sets to the real client IP, and the header
/// integration tests use to drive distinct source IPs through one in-process
/// service.
pub const CLIENT_IP_HEADER: &str = "x-demo-client-ip";

/// The source key used when no client IP can be determined.
pub const UNKNOWN_IP: &str = "unknown";

/// How many offending vocabulary words to include in a rejection message.
const MAX_REPORTED_OFFENDERS: usize = 8;

/// Shared, cloneable server state used as axum [`State`].
///
/// It owns the [`LimitConfig`], the per-IP rate limiter, the per-IP and global
/// concurrency counters, the session registry, and the harness. Cloning is cheap
/// (everything is behind an [`Arc`]); every handler and every spawned session
/// task shares one instance.
#[derive(Clone)]
pub struct DemoState {
    inner: Arc<StateInner>,
}

struct StateInner {
    limits: LimitConfig,
    rate: RateLimiter,
    global_active: AtomicU32,
    per_ip_active: Mutex<HashMap<String, u32>>,
    sessions: Mutex<HashMap<String, SessionHandle>>,
    next_id: AtomicU64,
    harness: Arc<dyn Harness>,
}

impl std::fmt::Debug for DemoState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DemoState")
            .field("limits", &self.inner.limits)
            .field(
                "global_active",
                &self.inner.global_active.load(Ordering::SeqCst),
            )
            .finish_non_exhaustive()
    }
}

impl DemoState {
    /// Builds state from a limit configuration and a harness.
    fn new(limits: LimitConfig, harness: Arc<dyn Harness>) -> Self {
        let rate = RateLimiter::new(limits.per_ip_rate_per_min, Duration::from_secs(60));
        Self {
            inner: Arc::new(StateInner {
                limits,
                rate,
                global_active: AtomicU32::new(0),
                per_ip_active: Mutex::new(HashMap::new()),
                sessions: Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(1),
                harness,
            }),
        }
    }

    /// The limit configuration in force.
    #[must_use]
    pub fn limits(&self) -> &LimitConfig {
        &self.inner.limits
    }

    /// The number of sessions currently counted as active across the server.
    #[must_use]
    pub fn global_active(&self) -> u32 {
        self.inner.global_active.load(Ordering::SeqCst)
    }

    /// Validates `prompt` against the length and vocabulary limits.
    fn validate_prompt(&self, prompt: &str) -> Result<(), DemoError> {
        let len = prompt.chars().count();
        if len > self.inner.limits.max_prompt_len {
            return Err(DemoError::PromptTooLong {
                len,
                max: self.inner.limits.max_prompt_len,
            });
        }
        let mut bad = offending_words(prompt, &self.inner.limits.allowed_vocabulary);
        if !bad.is_empty() {
            bad.truncate(MAX_REPORTED_OFFENDERS);
            return Err(DemoError::OffVocabulary { words: bad });
        }
        Ok(())
    }

    /// Reserves a per-IP and global concurrency slot for `ip`, or returns the
    /// rejection that applies.
    ///
    /// The reservation is atomic with respect to the caps: the global counter is
    /// taken with a compare-and-swap loop and rolled back if the per-IP cap then
    /// fails, so two racing submits can never jointly exceed a cap. The returned
    /// [`ConcurrencySlot`] releases both counters on drop.
    fn reserve_slot(&self, ip: &str) -> Result<ConcurrencySlot, DemoError> {
        // Reserve a global slot first (CAS loop), so the global cap is exact.
        let mut current = self.inner.global_active.load(Ordering::SeqCst);
        loop {
            if current >= self.inner.limits.global_concurrency {
                return Err(DemoError::GlobalConcurrency);
            }
            match self.inner.global_active.compare_exchange_weak(
                current,
                current + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(observed) => current = observed,
            }
        }

        // Then reserve a per-IP slot under the map lock.
        {
            let mut per_ip = self
                .inner
                .per_ip_active
                .lock()
                .expect("per-ip mutex poisoned");
            let count = per_ip.entry(ip.to_owned()).or_insert(0);
            if *count >= self.inner.limits.per_ip_concurrency {
                // Roll back the global reservation taken above.
                self.inner.global_active.fetch_sub(1, Ordering::SeqCst);
                return Err(DemoError::PerIpConcurrency);
            }
            *count += 1;
        }

        Ok(ConcurrencySlot {
            state: self.clone(),
            ip: ip.to_owned(),
        })
    }

    /// Allocates a process-unique session id.
    fn allocate_session_id(&self) -> String {
        let n = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        format!("sess-{n:08x}")
    }

    /// Registers a session handle under its id.
    fn insert_session(&self, id: &str, handle: SessionHandle) {
        self.inner
            .sessions
            .lock()
            .expect("sessions mutex poisoned")
            .insert(id.to_owned(), handle);
    }

    /// Looks up a session handle by id.
    fn get_session(&self, id: &str) -> Option<SessionHandle> {
        self.inner
            .sessions
            .lock()
            .expect("sessions mutex poisoned")
            .get(id)
            .cloned()
    }
}

/// An RAII reservation of one per-IP and one global concurrency slot.
///
/// Held by the spawned session task for the session's lifetime; dropping it (when
/// the task ends, whether the session finished, was cancelled, or errored)
/// releases both counters, freeing capacity for waiting callers.
#[derive(Debug)]
struct ConcurrencySlot {
    state: DemoState,
    ip: String,
}

impl Drop for ConcurrencySlot {
    fn drop(&mut self) {
        self.state
            .inner
            .global_active
            .fetch_sub(1, Ordering::SeqCst);
        let mut per_ip = self
            .state
            .inner
            .per_ip_active
            .lock()
            .expect("per-ip mutex poisoned");
        if let Some(count) = per_ip.get_mut(&self.ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                per_ip.remove(&self.ip);
            }
        }
    }
}

/// The Reticle demo service.
///
/// Construct with [`DemoServer::new`] (default [`MockHarness`]) or
/// [`DemoServer::with_harness`]. There is no constructor that omits the
/// [`LimitConfig`], so the endpoints are never exposed unbounded.
///
/// [`MockHarness`]: crate::harness::MockHarness
#[derive(Clone, Debug)]
pub struct DemoServer {
    state: DemoState,
}

impl DemoServer {
    /// Builds a demo server enforcing `limits`, using the default mock harness.
    #[must_use]
    pub fn new(limits: LimitConfig) -> Self {
        Self::with_harness(limits, crate::harness::MockHarness::default())
    }

    /// Builds a demo server enforcing `limits` with a specific harness.
    pub fn with_harness<H: Harness>(limits: LimitConfig, harness: H) -> Self {
        Self {
            state: DemoState::new(limits, Arc::new(harness)),
        }
    }

    /// The shared state, exposed for assertions in tests.
    #[must_use]
    pub fn state(&self) -> DemoState {
        self.state.clone()
    }

    /// Builds the axum [`Router`] with state attached.
    ///
    /// Routes: `POST /submit`, `GET /status/{id}`, `POST /cancel`. This router
    /// can be driven directly in-process (for example with
    /// `tower::ServiceExt::oneshot`); for a live deployment prefer
    /// [`DemoServer::into_make_service`], which attaches peer addresses.
    pub fn router(self) -> Router {
        Router::new()
            .route("/submit", post(submit))
            .route("/status/{id}", get(status))
            .route("/cancel", post(cancel))
            .with_state(self.state)
    }

    /// Builds a make-service that attaches each connection's [`SocketAddr`], so
    /// the client IP is available for enforcement in a real deployment.
    pub fn into_make_service(self) -> IntoMakeServiceWithConnectInfo<Router, SocketAddr> {
        self.router()
            .into_make_service_with_connect_info::<SocketAddr>()
    }
}

/// The connection's peer address if one was attached, or `None`.
///
/// A plain [`ConnectInfo`] extractor is mandatory and rejects a request that has
/// no peer info (as happens when the router is driven in-process by a test).
/// This infallible wrapper reads the peer address from the request extensions if
/// present, so the same handler works both behind [`DemoServer::into_make_service`]
/// (peer attached) and under a direct oneshot request (peer absent, IP taken from
/// the header instead).
#[derive(Clone, Copy, Debug)]
struct MaybePeer(Option<SocketAddr>);

impl<S> FromRequestParts<S> for MaybePeer
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let addr = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0);
        Ok(MaybePeer(addr))
    }
}

/// Resolves the source IP for `headers`, falling back to the peer `addr`.
fn client_ip(headers: &HeaderMap, addr: Option<SocketAddr>) -> String {
    if let Some(value) = headers.get(CLIENT_IP_HEADER)
        && let Ok(s) = value.to_str()
        && !s.trim().is_empty()
    {
        return s.trim().to_owned();
    }
    addr.map_or_else(|| UNKNOWN_IP.to_owned(), |a| a.ip().to_string())
}

/// `POST /submit`: validate, rate- and concurrency-limit, then start a session.
async fn submit(
    State(state): State<DemoState>,
    MaybePeer(peer): MaybePeer,
    headers: HeaderMap,
    Json(req): Json<SubmitRequest>,
) -> Result<Json<SubmitResponse>, DemoError> {
    let ip = client_ip(&headers, peer);

    // 1. Input validation (no state touched): length then vocabulary -> 400.
    state.validate_prompt(&req.prompt)?;

    // 2. Per-IP rate -> 429.
    if !state.inner.rate.check(&ip) {
        return Err(DemoError::RateLimited);
    }

    // 3. Per-IP and global concurrency -> 409 / 503. Held by the session task.
    let slot = state.reserve_slot(&ip)?;

    // 4. Create the session and spawn the harness.
    let session_id = state.allocate_session_id();
    let room = format!("demo-{session_id}");
    let handle = SessionHandle::new(session_id.clone());
    state.insert_session(&session_id, handle.clone());

    let budget = Budget {
        token_budget: state.inner.limits.token_budget,
        command_budget: state.inner.limits.command_budget,
    };
    let harness = Arc::clone(&state.inner.harness);
    let run_prompt = req.prompt;
    let run_room = room.clone();
    tokio::spawn(async move {
        // The slot lives for the whole task; dropping it here releases capacity.
        let _slot = slot;
        harness.run(run_prompt, run_room, budget, handle).await;
    });

    Ok(Json(SubmitResponse { session_id, room }))
}

/// `GET /status/{id}`: return the session's current status, or `404`.
async fn status(
    State(state): State<DemoState>,
    Path(id): Path<String>,
) -> Result<Json<StatusResponse>, DemoError> {
    match state.get_session(&id) {
        Some(handle) => Ok(Json(handle.status())),
        None => Err(DemoError::UnknownSession { session_id: id }),
    }
}

/// `POST /cancel`: request cancellation of a session, or `404` if unknown.
///
/// Returns the session's status immediately after tripping its cancel token; the
/// harness observes the token and moves the session to
/// [`crate::api::SessionState::Cancelled`] promptly.
async fn cancel(
    State(state): State<DemoState>,
    Json(req): Json<CancelRequest>,
) -> Result<Json<StatusResponse>, DemoError> {
    match state.get_session(&req.session_id) {
        Some(handle) => {
            handle.cancel();
            Ok(Json(handle.status()))
        }
        None => Err(DemoError::UnknownSession {
            session_id: req.session_id,
        }),
    }
}

// A small direct-body extraction path lets integration tests read typed
// responses without a network client.
impl DemoState {
    /// Constructs a state directly for unit tests in this crate.
    #[cfg(test)]
    fn for_test(limits: LimitConfig) -> Self {
        Self::new(limits, Arc::new(crate::harness::MockHarness::default()))
    }
}

#[cfg(test)]
mod tests {
    use super::{DemoState, client_ip};
    use crate::error::DemoError;
    use crate::limits::LimitConfig;
    use axum::http::{HeaderMap, HeaderValue};

    fn small_vocab_limits() -> LimitConfig {
        LimitConfig {
            per_ip_rate_per_min: 3,
            per_ip_concurrency: 1,
            global_concurrency: 2,
            token_budget: 10_000,
            command_budget: 50,
            max_prompt_len: 32,
            allowed_vocabulary: ["place", "metal1", "rectangle"]
                .into_iter()
                .map(String::from)
                .collect(),
        }
    }

    #[test]
    fn client_ip_prefers_header_then_peer_then_unknown() {
        let mut headers = HeaderMap::new();
        headers.insert(super::CLIENT_IP_HEADER, HeaderValue::from_static("9.9.9.9"));
        assert_eq!(client_ip(&headers, None), "9.9.9.9");

        let empty = HeaderMap::new();
        let peer = "1.2.3.4:5555".parse().ok();
        assert_eq!(client_ip(&empty, peer), "1.2.3.4");
        assert_eq!(client_ip(&empty, None), super::UNKNOWN_IP);
    }

    #[test]
    fn prompt_length_is_enforced() {
        let state = DemoState::for_test(small_vocab_limits());
        let long = "place ".repeat(20); // well over 32 chars
        let err = state.validate_prompt(&long).unwrap_err();
        assert!(matches!(err, DemoError::PromptTooLong { .. }));
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn vocabulary_is_enforced() {
        let state = DemoState::for_test(small_vocab_limits());
        let err = state.validate_prompt("delete everything").unwrap_err();
        assert!(matches!(err, DemoError::OffVocabulary { .. }));
    }

    #[test]
    fn valid_prompt_passes_validation() {
        let state = DemoState::for_test(small_vocab_limits());
        assert!(state.validate_prompt("place metal1 rectangle").is_ok());
    }

    #[test]
    fn concurrency_slots_reserve_and_release() {
        let state = DemoState::for_test(small_vocab_limits());
        let a = state.reserve_slot("ip-a").unwrap();
        // per_ip_concurrency is 1, so the same IP is refused a second slot.
        assert!(matches!(
            state.reserve_slot("ip-a").unwrap_err(),
            DemoError::PerIpConcurrency
        ));
        // A different IP still fits under the global cap of 2.
        let _b = state.reserve_slot("ip-b").unwrap();
        // Global cap of 2 is now full: a third IP is refused with 503.
        assert!(matches!(
            state.reserve_slot("ip-c").unwrap_err(),
            DemoError::GlobalConcurrency
        ));
        // Releasing one frees a global slot.
        drop(a);
        let _c = state.reserve_slot("ip-c").unwrap();
    }
}
