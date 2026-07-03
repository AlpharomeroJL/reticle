//! The agent harness abstraction and a mock implementation.
//!
//! The demo server does not run an agent itself. It drives sessions through the
//! [`Harness`] trait, which the real `reticle-agent` propose-verify-correct loop
//! will implement in a later wave. Until then [`MockHarness`] stands in: it walks
//! a session through [`SessionState::Queued`] to [`SessionState::Running`] to
//! [`SessionState::Done`], respects cancellation at every step, and honours the
//! per-session token and command budgets by cancelling the session if either is
//! exceeded.
//!
//! # Session handle
//!
//! A running session is represented by a [`SessionHandle`]: a shared, mutable
//! [`StatusResponse`] the harness updates and the status endpoint reads, plus a
//! [`CancelToken`] the cancel endpoint (or a budget overrun) trips. The handle is
//! cheap to clone; both the server and the spawned harness task hold one.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::api::{SessionState, StatusResponse};

/// A one-way cancellation flag shared between the server and a session task.
///
/// Cancellation is cooperative: the harness checks [`CancelToken::is_cancelled`]
/// at each step and stops promptly. Tripping the token is idempotent.
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
}

impl CancelToken {
    /// Creates an untripped token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation. Idempotent.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Reports whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// The per-session budgets handed to the harness.
///
/// The harness must stop and cancel the session if a step would take cumulative
/// token or command usage past either ceiling, so a runaway agent cannot exhaust
/// the host. The mock harness models modest usage per iteration to exercise this.
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    /// Maximum tokens the session may consume.
    pub token_budget: u64,
    /// Maximum commands (tool calls) the session may issue.
    pub command_budget: u32,
}

/// A shared, mutable view of one session's status plus its cancel token.
#[derive(Clone, Debug)]
pub struct SessionHandle {
    status: Arc<Mutex<StatusResponse>>,
    cancel: CancelToken,
}

impl SessionHandle {
    /// Creates a handle in the [`SessionState::Queued`] state.
    #[must_use]
    pub fn new(session_id: String) -> Self {
        let status = StatusResponse {
            session_id,
            state: SessionState::Queued,
            iteration: 0,
            violations: 0,
            message: "queued".to_owned(),
        };
        Self {
            status: Arc::new(Mutex::new(status)),
            cancel: CancelToken::new(),
        }
    }

    /// Returns a clone of the current status.
    #[must_use]
    pub fn status(&self) -> StatusResponse {
        self.status.lock().expect("status mutex poisoned").clone()
    }

    /// Returns the current lifecycle state.
    #[must_use]
    pub fn state(&self) -> SessionState {
        self.status.lock().expect("status mutex poisoned").state
    }

    /// A clone of this session's cancel token.
    #[must_use]
    pub fn cancel_token(&self) -> CancelToken {
        self.cancel.clone()
    }

    /// Requests cancellation of this session.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Sets the session's state and status line under the lock.
    fn set_state(&self, state: SessionState, message: impl Into<String>) {
        let mut guard = self.status.lock().expect("status mutex poisoned");
        guard.state = state;
        guard.message = message.into();
    }

    /// Records the current iteration and status line while Running, under the
    /// lock.
    fn set_iteration(&self, iteration: u32, message: impl Into<String>) {
        let mut guard = self.status.lock().expect("status mutex poisoned");
        guard.state = SessionState::Running;
        guard.iteration = iteration;
        guard.message = message.into();
    }

    /// Reports live progress from a [`Harness`] implemented outside this crate.
    ///
    /// Marks the session [`SessionState::Running`] and records the iteration, the
    /// current verification violation count, and a human-readable status line, all
    /// under the status lock. The built-in [`MockHarness`] uses the private helpers
    /// above; the real `reticle-agent`-backed harness (which lives in a separate
    /// crate) drives the session through this public method and
    /// [`finish`](Self::finish).
    pub fn report(&self, iteration: u32, violations: u32, message: impl Into<String>) {
        let mut guard = self.status.lock().expect("status mutex poisoned");
        guard.state = SessionState::Running;
        guard.iteration = iteration;
        guard.violations = violations;
        guard.message = message.into();
    }

    /// Settles the session into a terminal state with a final status line.
    ///
    /// The final violation count from the last [`report`](Self::report) is
    /// preserved. Intended for an out-of-crate [`Harness`] to end a run in
    /// [`SessionState::Done`], [`SessionState::Cancelled`], or
    /// [`SessionState::Error`].
    pub fn finish(&self, state: SessionState, message: impl Into<String>) {
        self.set_state(state, message);
    }
}

/// Drives a session to completion.
///
/// Implementations run asynchronously (spawned by the server) and communicate
/// solely through the [`SessionHandle`]: they mutate its status and watch its
/// cancel token. `run` must return promptly after the session reaches a terminal
/// state ([`SessionState::Done`], [`SessionState::Cancelled`], or
/// [`SessionState::Error`]).
pub trait Harness: Send + Sync + 'static {
    /// Runs the session identified by `handle` for `prompt`, observing `budget`.
    ///
    /// `room` is the collaboration room a spectator can watch; the mock does not
    /// use it, but the real harness streams draw operations there.
    fn run(
        &self,
        prompt: String,
        room: String,
        budget: Budget,
        handle: SessionHandle,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;
}

/// A stand-in harness that walks a session Queued -> Running -> Done.
///
/// It performs a fixed number of short "iterations", sleeping briefly between
/// each so a concurrent cancel can interrupt it, and charges `tokens_per_iter`
/// and `commands_per_iter` against the budget each step. If either budget would
/// be exceeded it cancels the session (terminal [`SessionState::Cancelled`] with
/// a budget message) rather than finishing.
#[derive(Clone, Debug)]
pub struct MockHarness {
    iterations: u32,
    step: Duration,
    tokens_per_iter: u64,
    commands_per_iter: u32,
}

impl Default for MockHarness {
    fn default() -> Self {
        Self {
            iterations: 3,
            step: Duration::from_millis(20),
            tokens_per_iter: 1_000,
            commands_per_iter: 2,
        }
    }
}

impl MockHarness {
    /// A mock with explicit pacing and per-iteration usage, for tests that need
    /// to force a budget overrun or a longer-lived running session.
    #[must_use]
    pub fn with_profile(
        iterations: u32,
        step: Duration,
        tokens_per_iter: u64,
        commands_per_iter: u32,
    ) -> Self {
        Self {
            iterations,
            step,
            tokens_per_iter,
            commands_per_iter,
        }
    }

    /// Sleeps for one step unless cancellation is requested first.
    ///
    /// Returns `true` if the sleep completed, `false` if it was cut short by a
    /// cancel (so the caller should stop).
    async fn step_or_cancel(&self, cancel: &CancelToken) -> bool {
        if cancel.is_cancelled() {
            return false;
        }
        tokio::time::sleep(self.step).await;
        !cancel.is_cancelled()
    }
}

impl Harness for MockHarness {
    fn run(
        &self,
        _prompt: String,
        _room: String,
        budget: Budget,
        handle: SessionHandle,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        let this = self.clone();
        Box::pin(async move {
            let cancel = handle.cancel_token();

            // Move from Queued to Running (unless cancelled while queued).
            if !this.step_or_cancel(&cancel).await {
                handle.set_state(SessionState::Cancelled, "cancelled while queued");
                return;
            }
            handle.set_state(SessionState::Running, "running");

            let mut tokens: u64 = 0;
            let mut commands: u32 = 0;
            for iter in 1..=this.iterations {
                if cancel.is_cancelled() {
                    handle.set_state(SessionState::Cancelled, "cancelled");
                    return;
                }

                // Charge this iteration against the budget; overrun cancels.
                tokens = tokens.saturating_add(this.tokens_per_iter);
                commands = commands.saturating_add(this.commands_per_iter);
                if tokens > budget.token_budget || commands > budget.command_budget {
                    cancel.cancel();
                    handle.set_state(SessionState::Cancelled, "cancelled: budget exceeded");
                    return;
                }

                handle.set_iteration(iter, format!("iteration {iter}"));

                if !this.step_or_cancel(&cancel).await {
                    handle.set_state(SessionState::Cancelled, "cancelled");
                    return;
                }
            }

            handle.set_state(SessionState::Done, "done");
        })
    }
}
