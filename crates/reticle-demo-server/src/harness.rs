//! The real `reticle-agent`-backed demo harness.
//!
//! [`AgentHarness`] implements [`reticle_demo::Harness`] by driving the
//! propose-verify-correct loop directly over the public seams of `reticle-agent`
//! and `reticle-agent-api`, so each atomic step can be mirrored onto the
//! collaboration room a spectator watches (ADR 0025):
//!
//! 1. Ask a [`ModelClient`] for a batch of [`AgentCommand`]s.
//! 2. Charge the batch against the session's token and command budget; stop and
//!    cancel if either would be exceeded.
//! 3. Apply the batch to a private [`Session`] (the authoritative document and
//!    transcript), and count DRC violations under the SKY130 rule subset for the
//!    status line and the correcting feedback.
//! 4. Mirror the same batch through an [`AgentCollaborator`] as one atomic
//!    `SyncDocument::step`, publish presence and an [`AgentStatus`], and ship the
//!    resulting `yrs` update to the relay room as one binary frame.
//! 5. Update the [`SessionHandle`] and honour the cancel token throughout.
//!
//! The model client is blocking (the real one makes a synchronous HTTP request),
//! so the loop runs on a `tokio::task::spawn_blocking` worker; frames are handed to
//! the async [`RoomPublisher`] with a non-blocking `try_send`.

use std::time::Duration;

use reticle_agent::collab::{AgentCollaborator, Pacing};
use reticle_agent::run::document_summary;
use reticle_agent_api::{AgentCommand, AgentStatus, Session};
use reticle_bench::model::{Context, MockModel, ModelClient};
use reticle_demo::{Budget, Harness, SessionHandle};

use crate::relay::RoomPublisher;

/// The SKY130 technology installed at the start of every demo session, so DRC
/// verification is meaningful. Embedded at build time so the running binary needs
/// no data files on disk (the Docker image and a bare `just demo-up` both work).
const SKY130_TECH: &str = include_str!("../../../tech/sky130.tech");

/// How a session's model client is produced. A factory (rather than a live client)
/// keeps the [`AgentHarness`] `Send + Sync` and lets each session build its own
/// client on the blocking worker.
#[derive(Clone, Debug)]
pub enum ModelSource {
    /// The real Anthropic-backed client. The key is read from the environment on
    /// the worker; `base_url` and `model` are the endpoint and model id.
    Anthropic {
        /// The Anthropic-compatible base URL (`/v1/messages` is appended).
        base_url: String,
        /// The model id to request.
        model: String,
    },
    /// A deterministic scripted client, for the offline path and tests.
    Scripted(MockModel),
}

/// How the loop is bounded, independent of the per-session token/command budget the
/// server enforces.
#[derive(Clone, Copy, Debug)]
pub struct LoopBounds {
    /// Maximum propose-verify-correct iterations before the harness gives up.
    pub max_iterations: u32,
    /// Estimated tokens charged per iteration when the client does not report real
    /// usage (the scripted client). The real path charges the same estimate; the
    /// point is that the server's `token_budget` still bounds a runaway loop.
    pub tokens_per_iteration: u64,
    /// Inter-step delay so a live spectator can follow the drawing. Instant in
    /// tests.
    pub step_delay: Duration,
}

impl Default for LoopBounds {
    fn default() -> Self {
        Self {
            max_iterations: 4,
            tokens_per_iteration: 2_000,
            step_delay: Duration::from_millis(400),
        }
    }
}

/// A [`Harness`] that runs the real agent loop and streams each step to the relay.
#[derive(Clone, Debug)]
pub struct AgentHarness {
    source: ModelSource,
    bounds: LoopBounds,
    /// The relay WebSocket base URL (for example `ws://127.0.0.1:3041`), or `None`
    /// to run the loop without streaming (status, limits, and cancel still apply).
    relay_ws_base: Option<String>,
}

impl AgentHarness {
    /// Builds a harness with the given model source, loop bounds, and optional relay
    /// base URL.
    #[must_use]
    pub fn new(source: ModelSource, bounds: LoopBounds, relay_ws_base: Option<String>) -> Self {
        Self {
            source,
            bounds,
            relay_ws_base,
        }
    }
}

impl Harness for AgentHarness {
    fn run(
        &self,
        prompt: String,
        room: String,
        budget: Budget,
        handle: SessionHandle,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        let source = self.source.clone();
        let bounds = self.bounds;
        let relay_ws_base = self.relay_ws_base.clone();

        Box::pin(async move {
            let cancel = handle.cancel_token();
            handle.report(0, 0, "starting");

            // Connect the room publisher (best-effort). A dead relay is logged and
            // the loop runs without streaming; it never fails the session.
            let publisher = match &relay_ws_base {
                Some(base) => match RoomPublisher::connect(base, &room).await {
                    Ok(p) => Some(p),
                    Err(e) => {
                        eprintln!("demo: room streaming disabled ({e})");
                        None
                    }
                },
                None => None,
            };

            // The loop is blocking (the model client makes a synchronous request and
            // the collaborator paces with a blocking sleep), so run it off-runtime.
            // Clone the handle so this task keeps one to recover from a worker panic.
            let worker_handle = handle.clone();
            let result = tokio::task::spawn_blocking(move || {
                run_loop(
                    &source,
                    &prompt,
                    bounds,
                    &budget,
                    &worker_handle,
                    publisher.as_ref(),
                );
            })
            .await;

            if let Err(join_err) = result {
                // The worker panicked; surface it as an error terminal state rather
                // than leaving the session stuck Running.
                handle.finish(
                    reticle_demo::SessionState::Error,
                    format!("agent worker failed: {join_err}"),
                );
            }
            let _ = cancel; // token is observed inside the loop; keep it alive here.
        })
    }
}

/// What one iteration decided the loop should do next.
enum Flow {
    /// Keep going to the next iteration.
    Continue,
    /// Stop the loop; the terminal state has already been written.
    Stop,
    /// Stop the loop without a terminal state yet (the caller finishes it).
    Break,
}

/// The mutable state carried across iterations of one session's loop.
struct LoopState<'a> {
    session: Session,
    collab: AgentCollaborator,
    handle: &'a SessionHandle,
    publisher: Option<&'a RoomPublisher>,
    tokens: u64,
    commands: u32,
    prev_violations: u32,
    feedback: Vec<String>,
}

/// One session's synchronous propose-verify-correct loop. Returns nothing; all
/// progress and the terminal state are written through `handle`.
fn run_loop(
    source: &ModelSource,
    prompt: &str,
    bounds: LoopBounds,
    budget: &Budget,
    handle: &SessionHandle,
    publisher: Option<&RoomPublisher>,
) {
    use reticle_demo::SessionState;

    // Build the model client and seed the session with the demo technology.
    let mut client = match LoopClient::build(source) {
        Ok(c) => c,
        Err(e) => {
            handle.finish(SessionState::Error, e);
            return;
        }
    };
    let mut session = Session::new();
    if let Err(e) = session.apply(AgentCommand::SetTechnology {
        source: SKY130_TECH.to_owned(),
    }) {
        handle.finish(SessionState::Error, format!("technology setup failed: {e}"));
        return;
    }

    let mut state = LoopState {
        session,
        collab: AgentCollaborator::new(pacing(bounds.step_delay)),
        handle,
        publisher,
        tokens: 0,
        commands: 0,
        prev_violations: 0,
        feedback: Vec::new(),
    };

    for iteration in 0..bounds.max_iterations {
        match state.run_iteration(&mut client, prompt, iteration, bounds, budget) {
            Flow::Continue => {}
            Flow::Stop => return,
            Flow::Break => break,
        }
    }

    // Ran out of iterations (or broke on an empty proposal) without a terminal state.
    let last_iter = bounds.max_iterations.saturating_sub(1);
    if state.handle.cancel_token().is_cancelled() {
        state.handle.finish(SessionState::Cancelled, "cancelled");
    } else {
        state.handle.finish(
            SessionState::Done,
            format!(
                "stopped after the iteration limit with {} violation(s)",
                state.prev_violations
            ),
        );
    }
    let v = state.prev_violations;
    state.status(last_iter, "stopped", v, false);
}

impl LoopState<'_> {
    /// Publishes an [`AgentStatus`] and streams the resulting frame.
    fn status(&mut self, iteration: u32, step: &str, violations: u32, running: bool) {
        publish_status(
            &mut self.collab,
            self.publisher,
            iteration,
            step,
            violations,
            running,
        );
    }

    /// Runs one propose-apply-verify iteration, returning the control flow.
    fn run_iteration(
        &mut self,
        client: &mut LoopClient,
        prompt: &str,
        iteration: u32,
        bounds: LoopBounds,
        budget: &Budget,
    ) -> Flow {
        use reticle_demo::SessionState;

        if self.handle.cancel_token().is_cancelled() {
            self.handle.finish(SessionState::Cancelled, "cancelled");
            let v = self.prev_violations;
            self.status(iteration, "cancelled", v, false);
            return Flow::Stop;
        }

        // Charge the (estimated) token cost of this turn up front; overrun cancels.
        self.tokens = self.tokens.saturating_add(bounds.tokens_per_iteration);
        if self.tokens > budget.token_budget {
            self.handle
                .finish(SessionState::Cancelled, "cancelled: token budget exceeded");
            let v = self.prev_violations;
            self.status(iteration, "budget", v, false);
            return Flow::Stop;
        }

        self.handle.report(
            iteration,
            self.prev_violations,
            format!("proposing (iteration {iteration})"),
        );
        let v = self.prev_violations;
        self.status(iteration, "proposing", v, true);

        // Ask for the next batch, feeding the real model the current layout.
        let context = Context {
            iteration,
            prev_violations: self.prev_violations,
            feedback: self.feedback.clone(),
        };
        let batch = client.propose(prompt, &context, &self.session);
        if batch.is_empty() && iteration > 0 {
            return Flow::Break;
        }

        // Apply the batch within the command budget, keeping the exact applied slice
        // so the mirrored step matches the authoritative session.
        let mut applied: Vec<AgentCommand> = Vec::with_capacity(batch.len());
        for command in batch {
            if self.commands >= budget.command_budget {
                break;
            }
            let _ = self.session.apply(command.clone());
            self.commands += 1;
            applied.push(command);
        }
        self.collab.apply_step(&applied);
        stream_frame(&self.collab, self.publisher);

        // Verify.
        let violations = drc_violation_count(&self.session);
        self.prev_violations = violations;
        self.handle.report(
            iteration,
            violations,
            format!("verified: {violations} violation(s)"),
        );
        self.status(iteration, "verifying", violations, true);

        // Converged only with real geometry that is clean.
        if violations == 0 && has_geometry(&self.session) {
            self.handle.finish(SessionState::Done, "done: 0 violations");
            self.status(iteration, "done", 0, false);
            return Flow::Stop;
        }

        self.feedback = vec![format!(
            "{violations} DRC violation(s) remain; widen or respace shapes"
        )];

        if self.commands >= budget.command_budget {
            self.handle.finish(
                SessionState::Cancelled,
                "cancelled: command budget exceeded",
            );
            self.status(iteration, "budget", violations, false);
            return Flow::Stop;
        }
        Flow::Continue
    }
}

/// The loop's model client: either the real Anthropic-backed one or the scripted
/// mock. A concrete enum (rather than `dyn ModelClient`) is deliberate, so the real
/// path can feed the document snapshot to
/// [`set_document_context`](reticle_agent::AnthropicModel::set_document_context)
/// before each proposal without a downcast.
enum LoopClient {
    /// The real client. Fed the current layout before each proposal.
    Anthropic(reticle_agent::AnthropicModel),
    /// The deterministic scripted client, which ignores the document.
    Scripted(MockModel),
}

impl LoopClient {
    /// Builds the client for `source` on the current worker.
    fn build(source: &ModelSource) -> Result<Self, String> {
        match source {
            ModelSource::Anthropic { base_url, model } => {
                let m = reticle_agent::AnthropicModel::from_env()
                    .map_err(|e| e.to_string())?
                    .with_base_url(base_url.clone())
                    .with_model(model.clone());
                Ok(LoopClient::Anthropic(m))
            }
            ModelSource::Scripted(mock) => Ok(LoopClient::Scripted(mock.clone())),
        }
    }

    /// Proposes the next batch, feeding the real model the current document summary
    /// first (the scripted client ignores it).
    fn propose(&mut self, prompt: &str, context: &Context, session: &Session) -> Vec<AgentCommand> {
        match self {
            LoopClient::Anthropic(m) => {
                m.set_document_context(document_summary(session));
                m.propose("demo", prompt, context)
            }
            LoopClient::Scripted(m) => m.propose("demo", prompt, context),
        }
    }
}

/// The pacing for the collaborator: instant when the delay is zero, else a delay.
fn pacing(step_delay: Duration) -> Pacing {
    if step_delay.is_zero() {
        Pacing::Instant
    } else {
        Pacing::Delay(step_delay)
    }
}

/// Encodes the collaborator's whole document as a `yrs` update and ships it as one
/// binary frame to the room. A no-op when there is no publisher.
fn stream_frame(collab: &AgentCollaborator, publisher: Option<&RoomPublisher>) {
    if let Some(p) = publisher {
        let frame = collab.sync().encode_state_update();
        p.publish(frame);
    }
}

/// Publishes an [`AgentStatus`] over the awareness channel and, if streaming, ships
/// the resulting frame so a watcher's narration updates too.
fn publish_status(
    collab: &mut AgentCollaborator,
    publisher: Option<&RoomPublisher>,
    iteration: u32,
    step: &str,
    violations: u32,
    running: bool,
) {
    collab.publish_status(&AgentStatus {
        iteration,
        step: step.to_owned(),
        violations,
        running,
    });
    stream_frame(collab, publisher);
}

/// Whether the session's target cell has any geometry, so an empty cell (which
/// trivially passes DRC) is not mistaken for a converged result.
fn has_geometry(session: &Session) -> bool {
    let doc = session.document();
    let target = doc
        .top_cells()
        .first()
        .cloned()
        .or_else(|| doc.cells().next().map(|c| c.name.clone()));
    target
        .and_then(|name| doc.cell(&name))
        .is_some_and(|cell| !cell.shapes.is_empty())
}

/// Counts DRC violations on the session's target cell under the SKY130 rule subset;
/// `0` when the document has no cell yet. Mirrors `reticle-agent`'s verify step.
fn drc_violation_count(session: &Session) -> u32 {
    use reticle_model::RuleSet as _;
    let doc = session.document();
    let Some(cell) = doc
        .top_cells()
        .first()
        .cloned()
        .or_else(|| doc.cells().next().map(|c| c.name.clone()))
    else {
        return 0;
    };
    let engine = reticle_drc::DrcEngine::new(reticle_drc::sky130_drc_rules());
    engine.check_cell(doc, &cell).len() as u32
}

/// The scripted client the offline default and tests use.
///
/// It converges a demo prompt on a single DRC-clean met1 rectangle in a cell named
/// `top`, in one step: create the cell, then add a 400 x 400 nm met1 rect, which
/// clears the met1 min width (140 nm) and min area (83000 nm^2) rules. The offline
/// path is deterministic and needs no network; the *live* model (with a key) is what
/// exercises the multi-iteration correct-and-retry arc against real feedback.
#[must_use]
pub fn demo_script() -> MockModel {
    use reticle_agent_api::args::{LayerArg, PointArg, RectArg};

    let met1 = LayerArg {
        layer: 68,
        datatype: 20,
    };
    let create = AgentCommand::CreateCell { name: "top".into() };
    let clean = AgentCommand::AddRect {
        cell: "top".into(),
        layer: met1,
        rect: RectArg {
            min: PointArg { x: 0, y: 0 },
            max: PointArg { x: 400, y: 400 },
        },
    };
    // One clean attempt; a later empty proposal ends the loop.
    MockModel::new().with_default(vec![vec![create, clean]])
}

#[cfg(test)]
mod tests {
    use super::{AgentHarness, LoopBounds, ModelSource, demo_script, drc_violation_count};
    use reticle_agent_api::args::{LayerArg, PointArg, RectArg};
    use reticle_agent_api::{AgentCommand, Session};
    use reticle_demo::{Budget, Harness, SessionHandle, SessionState};
    use std::time::Duration;

    fn instant_bounds() -> LoopBounds {
        LoopBounds {
            max_iterations: 4,
            tokens_per_iteration: 1_000,
            step_delay: Duration::ZERO,
        }
    }

    fn wide_budget() -> Budget {
        Budget {
            token_budget: 1_000_000,
            command_budget: 1_000,
        }
    }

    /// The scripted harness, with no relay, drives a session to Done with a clean
    /// document.
    #[tokio::test]
    async fn scripted_run_reaches_done_clean() {
        let harness =
            AgentHarness::new(ModelSource::Scripted(demo_script()), instant_bounds(), None);
        let handle = SessionHandle::new("s-clean".into());
        harness
            .run(
                "place a clean met1 rectangle".into(),
                "demo-room".into(),
                wide_budget(),
                handle.clone(),
            )
            .await;
        let status = handle.status();
        assert_eq!(
            status.state,
            SessionState::Done,
            "message: {}",
            status.message
        );
        assert_eq!(status.violations, 0, "the converged run is DRC-clean");
    }

    /// A tiny command budget cancels the session before it can converge.
    #[tokio::test]
    async fn tiny_command_budget_cancels() {
        let harness =
            AgentHarness::new(ModelSource::Scripted(demo_script()), instant_bounds(), None);
        let handle = SessionHandle::new("s-budget".into());
        let budget = Budget {
            token_budget: 1_000_000,
            command_budget: 1, // smaller than the first batch (create + rect)
        };
        harness
            .run(
                "place a met1 rectangle".into(),
                "demo-room".into(),
                budget,
                handle.clone(),
            )
            .await;
        let status = handle.status();
        assert_eq!(status.state, SessionState::Cancelled);
        assert!(
            status.message.contains("budget"),
            "message should cite the budget: {}",
            status.message
        );
    }

    /// A pre-cancelled session settles into Cancelled without drawing.
    #[tokio::test]
    async fn pre_cancelled_session_ends_cancelled() {
        let harness =
            AgentHarness::new(ModelSource::Scripted(demo_script()), instant_bounds(), None);
        let handle = SessionHandle::new("s-cancel".into());
        handle.cancel();
        harness
            .run(
                "place a met1 rectangle".into(),
                "demo-room".into(),
                wide_budget(),
                handle.clone(),
            )
            .await;
        assert_eq!(handle.status().state, SessionState::Cancelled);
    }

    /// The DRC counter agrees with a hand-built clean rectangle.
    #[test]
    fn drc_counter_zero_on_clean_rect() {
        let mut session = Session::new();
        session
            .apply(AgentCommand::CreateCell { name: "top".into() })
            .unwrap();
        session
            .apply(AgentCommand::AddRect {
                cell: "top".into(),
                layer: LayerArg {
                    layer: 68,
                    datatype: 20,
                },
                rect: RectArg {
                    min: PointArg { x: 0, y: 0 },
                    max: PointArg { x: 400, y: 400 },
                },
            })
            .unwrap();
        assert_eq!(drc_violation_count(&session), 0);
    }
}
