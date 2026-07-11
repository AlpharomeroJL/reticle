//! The real, native-only plan/approve/execute agent runner.
//!
//! On native, the agent panel can drive a genuine propose-verify-correct loop against a
//! live model through `reticle-agent`'s [`run_agent_task`]: the user writes a prompt,
//! reviews the plan (the run configuration to be authorized), approves, and the loop runs
//! on a background worker thread against the configured backend. The run writes the same
//! JSONL transcript the replay theater plays, so a real run flows straight into the
//! theater as a faithful replay.
//!
//! This module is native-only for the reasons `reticle-agent` is: the loop uses blocking
//! `ureq` HTTP, filesystem artifact writes, and (for the live-room mode it does not use
//! here) `tokio`, none of which build for wasm32. On the web the panel keeps the
//! model-free scripted preview, so the browser never claims to run a live model.
//!
//! # Honesty
//!
//! Nothing here fabricates a run. When no backend is configured ([`detect_backend`]
//! returns `None`) the panel shows the scripted preview and says so; the API key is read
//! from the environment only, never surfaced; a run that fails its check is reported as a
//! failure ([`RunReport::success`] `= false`), never retro-edited to a pass.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use reticle_agent::run::document_summary;
use reticle_agent::{AnthropicModel, LoopOptions, OllamaModel, Provenance, run_agent_task};
use reticle_bench::{BenchTask, CheckerRegistry, Tier};

/// The command budget one interactive run may apply. Small: an interactive prompt is a
/// single focused edit, not a whole benchmark suite.
const COMMAND_BUDGET: u32 = 64;

/// Maximum propose-verify-correct iterations for one interactive run.
const MAX_ITERATIONS: u32 = 4;

/// The checker an interactive prompt is verified against: DRC-clean, the honest default
/// for "draw me something that passes the rules".
const INTERACTIVE_CHECKER: &str = "drc_clean";

/// Which model backend a real run drives, chosen from the environment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AgentBackend {
    /// The Anthropic Messages API (key from `ANTHROPIC_API_KEY`).
    Anthropic,
    /// An OpenAI-compatible endpoint (Ollama), configured from `RETICLE_MODEL_*`.
    Ollama,
}

impl AgentBackend {
    /// The provenance label stamped on the run's result record.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            AgentBackend::Anthropic => "anthropic",
            AgentBackend::Ollama => "ollama",
        }
    }

    /// A short human-readable name for the panel.
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            AgentBackend::Anthropic => "Anthropic",
            AgentBackend::Ollama => "Ollama (local)",
        }
    }
}

/// Chooses a backend from availability flags, preferring the frontier (Anthropic) over a
/// local model, matching the CLI's default backend order. Split out from [`detect_backend`]
/// so the choice is unit-tested without touching the environment.
#[must_use]
fn backend_from_flags(anthropic_key: bool, ollama_model: bool) -> Option<AgentBackend> {
    if anthropic_key {
        Some(AgentBackend::Anthropic)
    } else if ollama_model {
        Some(AgentBackend::Ollama)
    } else {
        None
    }
}

/// Detects which real backend is configured in the environment, if any.
///
/// Prefers Anthropic (a key in `ANTHROPIC_API_KEY`) over a local Ollama model
/// (`RETICLE_MODEL_NAME`). Returns `None` when neither is configured, so the panel
/// honestly falls back to the scripted preview rather than offering a Run control that
/// cannot run. Reads the environment only; the key value is never returned or logged.
#[must_use]
pub fn detect_backend() -> Option<AgentBackend> {
    backend_from_flags(
        reticle_agent::ApiKey::from_env().is_some(),
        OllamaModel::from_env().has_model(),
    )
}

/// The interactive [`BenchTask`] for a prompt: a tier-0 ad-hoc task verified DRC-clean
/// against the session's built-in technology (no technology file). Deterministic and
/// model-free, so it is unit-tested.
#[must_use]
pub fn build_task(prompt: &str) -> BenchTask {
    BenchTask {
        id: "interactive".to_owned(),
        tier: Tier(0),
        prompt: prompt.to_owned(),
        technology: String::new(),
        checker: INTERACTIVE_CHECKER.to_owned(),
        intent: None,
        refinement: None,
    }
}

/// A one-line, model-free summary of the run a Plan stages, for the user to review before
/// approving. Names the backend, the prompt, the checker, and the caps, so approval is
/// informed. Pure, so it is unit-tested.
#[must_use]
pub fn plan_summary(prompt: &str, backend: AgentBackend) -> String {
    format!(
        "Plan: run \"{}\" on {} for up to {MAX_ITERATIONS} iteration(s) / {COMMAND_BUDGET} command(s), \
         verified by the {INTERACTIVE_CHECKER} checker. Approve to execute against the live model.",
        prompt.trim(),
        backend.display_name(),
    )
}

/// The outcome of a finished real run, handed back from the worker thread.
#[derive(Clone, Debug)]
pub struct RunReport {
    /// Whether the run passed its checker.
    pub success: bool,
    /// The number of DRC violations remaining at the end.
    pub final_violations: u32,
    /// The propose-verify-correct iterations the loop took.
    pub iterations: u32,
    /// The JSONL transcript path, ready to load into the replay theater.
    pub transcript: PathBuf,
}

/// Runs one interactive prompt through the propose-verify-correct loop against `backend`,
/// writing artifacts under `out_dir`, and returns a [`RunReport`].
///
/// Blocking: this makes real HTTP calls and writes files, so callers run it on a worker
/// thread, never the UI thread. A setup failure (missing key, unknown checker, IO) is a
/// clean `Err`; a run that merely fails its check is `Ok(RunReport { success: false, .. })`,
/// never an error and never a fabricated pass.
///
/// # Errors
///
/// Returns the stringified setup/IO error when the backend cannot be built, the checker
/// is unknown, or an artifact cannot be written.
pub fn execute(prompt: &str, backend: AgentBackend, out_dir: &Path) -> Result<RunReport, String> {
    let task = build_task(prompt);
    let registry = CheckerRegistry::for_task(&task)?;
    let options = LoopOptions {
        max_iterations: MAX_ITERATIONS,
        command_budget: COMMAND_BUDGET,
    };
    let provenance = Provenance::new(backend.label());
    std::fs::create_dir_all(out_dir).map_err(|e| format!("creating {}: {e}", out_dir.display()))?;

    // Feed the model the current layout before each proposal, exactly as the CLI does.
    let outcome = match backend {
        AgentBackend::Anthropic => {
            let mut model = AnthropicModel::from_env().map_err(|e| e.to_string())?;
            run_agent_task(
                &task,
                &mut model,
                &registry,
                "",
                "interactive",
                options,
                out_dir,
                0,
                &provenance,
                |m: &mut AnthropicModel, s: &_| m.set_document_context(document_summary(s)),
            )
        }
        AgentBackend::Ollama => {
            let mut model = OllamaModel::from_env();
            if !model.has_model() {
                return Err("no Ollama model configured (set RETICLE_MODEL_NAME)".to_owned());
            }
            run_agent_task(
                &task,
                &mut model,
                &registry,
                "",
                "interactive",
                options,
                out_dir,
                0,
                &provenance,
                |m: &mut OllamaModel, s: &_| m.set_document_context(document_summary(s)),
            )
        }
    }
    .map_err(|e| e.to_string())?;

    Ok(RunReport {
        success: outcome.record.success,
        final_violations: outcome.record.final_violations,
        iterations: outcome.record.iterations,
        transcript: outcome.artifacts.transcript,
    })
}

/// The phase of the interactive agent run.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Phase {
    /// No run: awaiting a prompt and a Plan.
    #[default]
    Idle,
    /// A plan is staged for the user to review before approving.
    Planned,
    /// A run is executing on the worker thread.
    Running,
    /// A run finished (it may have passed or failed its check; see the status).
    Done,
    /// The setup or run errored, or the user stopped it before it finished.
    Failed,
}

/// The native plan/approve/execute agent runner: the phase, the staged plan, the live
/// status line, and the worker-thread channel.
///
/// The runner never blocks the UI thread: [`approve`](Self::approve) spawns a worker
/// running [`execute`] and stores the receiver; [`poll`](Self::poll) drains it each frame
/// and returns the finished transcript for the app to load into the theater.
#[derive(Debug)]
pub struct AgentRunner {
    /// The backend detected at construction, if any. `None` means no model is configured,
    /// so the panel shows the scripted preview instead.
    backend: Option<AgentBackend>,
    phase: Phase,
    plan: String,
    status: String,
    out_dir: PathBuf,
    rx: Option<Receiver<Result<RunReport, String>>>,
}

impl Default for AgentRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentRunner {
    /// Creates an idle runner, detecting the configured backend once from the
    /// environment. Artifacts are written under the OS temp directory.
    #[must_use]
    pub fn new() -> Self {
        Self {
            backend: detect_backend(),
            phase: Phase::Idle,
            plan: String::new(),
            status: String::new(),
            out_dir: std::env::temp_dir().join("reticle-agent-runs"),
            rx: None,
        }
    }

    /// The detected backend, if a real model is configured.
    #[must_use]
    pub fn backend(&self) -> Option<AgentBackend> {
        self.backend
    }

    /// The current run phase.
    #[must_use]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The staged plan text (non-empty only in [`Phase::Planned`] onward).
    #[must_use]
    pub fn plan(&self) -> &str {
        &self.plan
    }

    /// The live status line for the panel.
    #[must_use]
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Stages a plan for `prompt`, moving to [`Phase::Planned`]. A no-op with no backend
    /// or a blank prompt (the caller keeps the scripted preview in that case).
    pub fn plan_run(&mut self, prompt: &str) {
        let Some(backend) = self.backend else {
            return;
        };
        if prompt.trim().is_empty() {
            "Enter a prompt to plan a run.".clone_into(&mut self.status);
            return;
        }
        self.plan = plan_summary(prompt, backend);
        "Plan staged. Review it, then Approve to run.".clone_into(&mut self.status);
        self.phase = Phase::Planned;
    }

    /// Approves the staged plan and starts the run on a worker thread, moving to
    /// [`Phase::Running`]. A no-op unless a plan is staged. The prompt is re-passed so the
    /// worker owns its own copy.
    ///
    /// The UI notices completion by polling [`poll`](Self::poll) each frame; the app keeps
    /// the frame loop awake while [`is_running`](Self::is_running) so the result is picked
    /// up promptly. This keeps the native module free of any egui/UI coupling.
    pub fn approve(&mut self, prompt: &str) {
        if self.phase != Phase::Planned {
            return;
        }
        let Some(backend) = self.backend else {
            return;
        };
        let prompt = prompt.to_owned();
        let out_dir = self.out_dir.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = execute(&prompt, backend, &out_dir);
            // The receiver may already be gone (the user pressed Stop); ignore that.
            let _ = tx.send(result);
        });
        self.rx = Some(rx);
        self.phase = Phase::Running;
        "Running against the live model...".clone_into(&mut self.status);
    }

    /// Whether a run is executing on the worker thread right now.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.phase == Phase::Running
    }

    /// Stops watching the current run and returns to idle.
    ///
    /// The worker thread cannot be interrupted mid-HTTP-call, so it runs to completion;
    /// dropping the receiver just means its result is discarded (the send fails silently),
    /// which is honest: no partial run is presented as a finished one.
    pub fn stop(&mut self) {
        if self.phase == Phase::Running {
            "Stopped watching the run.".clone_into(&mut self.status);
        }
        self.rx = None;
        self.phase = Phase::Idle;
        self.plan.clear();
    }

    /// Drains the worker channel. Returns the finished run's transcript path (for the app
    /// to load into the replay theater) when a run completes successfully or fails its
    /// check; `None` while still running, on a setup error, or when there is nothing to do.
    ///
    /// A run that failed its check still returns a transcript: the failure is real and
    /// worth replaying. A setup/IO error carries no transcript and moves to
    /// [`Phase::Failed`] with the error on the status line.
    pub fn poll(&mut self) -> Option<PathBuf> {
        let rx = self.rx.as_ref()?;
        match rx.try_recv() {
            Ok(Ok(report)) => {
                self.status = if report.success {
                    format!(
                        "Run complete: passed in {} iteration(s), {} violation(s) left.",
                        report.iterations, report.final_violations
                    )
                } else {
                    format!(
                        "Run complete: did NOT pass ({} violation(s) left after {} iteration(s)). Replaying it.",
                        report.final_violations, report.iterations
                    )
                };
                self.phase = Phase::Done;
                self.rx = None;
                Some(report.transcript)
            }
            Ok(Err(error)) => {
                self.status = format!("Run failed to start: {error}");
                self.phase = Phase::Failed;
                self.rx = None;
                None
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                "The run worker stopped unexpectedly.".clone_into(&mut self.status);
                self.phase = Phase::Failed;
                self.rx = None;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_choice_prefers_anthropic_then_ollama_then_none() {
        assert_eq!(
            backend_from_flags(true, true),
            Some(AgentBackend::Anthropic),
            "a key wins even if Ollama is also configured"
        );
        assert_eq!(
            backend_from_flags(true, false),
            Some(AgentBackend::Anthropic)
        );
        assert_eq!(backend_from_flags(false, true), Some(AgentBackend::Ollama));
        assert_eq!(
            backend_from_flags(false, false),
            None,
            "no model configured means no real backend"
        );
    }

    #[test]
    fn labels_are_distinct_and_stable() {
        assert_eq!(AgentBackend::Anthropic.label(), "anthropic");
        assert_eq!(AgentBackend::Ollama.label(), "ollama");
        assert_ne!(
            AgentBackend::Anthropic.display_name(),
            AgentBackend::Ollama.display_name()
        );
    }

    #[test]
    fn build_task_is_a_drc_clean_tier0_prompt_task() {
        let task = build_task("draw a clean met1 wire");
        assert_eq!(task.checker, "drc_clean");
        assert_eq!(task.tier, Tier(0));
        assert_eq!(task.prompt, "draw a clean met1 wire");
        assert!(task.intent.is_none() && task.refinement.is_none());
        // The task the runner builds resolves to a real checker registry.
        assert!(
            CheckerRegistry::for_task(&task).is_ok(),
            "the interactive task must resolve its checker"
        );
    }

    #[test]
    fn plan_summary_names_the_prompt_and_backend() {
        let s = plan_summary("route a net", AgentBackend::Anthropic);
        assert!(s.contains("route a net"));
        assert!(s.contains("Anthropic"));
        assert!(s.contains("drc_clean"));
    }

    #[test]
    fn a_configured_runner_plans_and_a_blank_prompt_is_rejected() {
        // Construct a runner with a known backend directly, so the test does not depend on
        // the ambient environment.
        let mut runner = AgentRunner {
            backend: Some(AgentBackend::Anthropic),
            phase: Phase::Idle,
            plan: String::new(),
            status: String::new(),
            out_dir: std::env::temp_dir().join("reticle-agent-runs-test"),
            rx: None,
        };
        runner.plan_run("   ");
        assert_eq!(runner.phase(), Phase::Idle, "a blank prompt does not plan");
        runner.plan_run("draw a wire");
        assert_eq!(runner.phase(), Phase::Planned);
        assert!(runner.plan().contains("draw a wire"));
        // Approve requires a running-capable path; stopping returns to idle and clears.
        runner.stop();
        assert_eq!(runner.phase(), Phase::Idle);
        assert!(runner.plan().is_empty());
    }

    #[test]
    fn an_unconfigured_runner_never_plans() {
        let mut runner = AgentRunner {
            backend: None,
            phase: Phase::Idle,
            plan: String::new(),
            status: String::new(),
            out_dir: std::env::temp_dir(),
            rx: None,
        };
        runner.plan_run("draw a wire");
        assert_eq!(
            runner.phase(),
            Phase::Idle,
            "with no backend the panel stays on the scripted preview"
        );
        assert!(runner.plan().is_empty());
    }
}
