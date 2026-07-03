//! Driving one task through the propose-verify-correct loop.
//!
//! [`run_task`] is the core of the harness: it creates a [`Session`], installs the
//! task's technology, then repeatedly asks the [`ModelClient`] for commands, applies
//! them, and runs the task's [`Checker`]. If the check fails it feeds the failure back
//! and lets the model correct, up to a bounded number of iterations. It records how
//! many iterations ran, the DRC violation count in the first proposal versus the final
//! document, and a deterministic wall time, into a [`ResultRecord`].
//!
//! # Determinism
//!
//! Library code never reads the clock: `wall_ms` is a monotonic *step counter* (one
//! unit per applied command), so a given task and mock always produce byte-identical
//! records. This keeps the results reproducible and the tests exact. The live-model
//! path (not in this lane) would substitute a real duration at the call site.

use reticle_agent_api::{AgentCommand, Session, Transcript};
use reticle_model::RuleSet;

use crate::model::{Context, ModelClient};
use crate::{BenchTask, CheckResult, Checker, CheckerRegistry, ResultRecord};

/// How many propose-verify-correct iterations a task may use before the runner gives
/// up. The first proposal is iteration 0, so this bounds total attempts.
pub const DEFAULT_MAX_ITERATIONS: u32 = 4;

/// Why [`run_task`] could not produce a [`ResultRecord`].
///
/// A task that simply fails its check is *not* an error, it is a well-formed record
/// with `success = false`; these variants are setup failures that prevent running at
/// all.
#[derive(Debug)]
pub enum RunError {
    /// The task's technology file could not be read or parsed.
    Technology {
        /// What went wrong.
        detail: String,
    },
    /// The task named a checker that the registry does not contain.
    UnknownChecker {
        /// The missing checker name.
        name: String,
    },
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Technology { detail } => write!(f, "technology setup failed: {detail}"),
            RunError::UnknownChecker { name } => write!(f, "no checker named `{name}`"),
        }
    }
}

impl std::error::Error for RunError {}

/// Options controlling a run.
#[derive(Clone, Copy, Debug)]
pub struct RunOptions {
    /// Maximum propose-verify-correct iterations.
    pub max_iterations: u32,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            max_iterations: DEFAULT_MAX_ITERATIONS,
        }
    }
}

/// Runs `task` against `model`, verifying with the checker named by the task in
/// `registry`, and returns the resulting [`ResultRecord`].
///
/// The loop: on each iteration ask the model for commands, apply each to the session,
/// count DRC violations on the target cell, then run the checker. The first
/// iteration's violation count becomes `first_proposal_violations`; the last
/// iteration's becomes `final_violations`. The loop stops as soon as the checker
/// passes, when the model proposes nothing, or when `max_iterations` is reached.
///
/// `technology_source` is the technology-file text the task runs against (the caller
/// resolves the task's `technology` path to text so the runner does no IO). Pass an
/// empty string to keep the session's default technology.
///
/// # Errors
///
/// Returns [`RunError::Technology`] if a non-empty `technology_source` fails to apply,
/// or [`RunError::UnknownChecker`] if the task's checker is not in `registry`.
pub fn run_task(
    task: &BenchTask,
    model: &mut dyn ModelClient,
    registry: &CheckerRegistry,
    technology_source: &str,
    suite_version: &str,
    options: RunOptions,
) -> Result<ResultRecord, RunError> {
    run_task_with_transcript(
        task,
        model,
        registry,
        technology_source,
        suite_version,
        options,
    )
    .map(|(record, _transcript)| record)
}

/// Like [`run_task`], but also returns the [`Transcript`] of the run, so a caller can
/// replay it and check that the document hash is reproducible (see the replay
/// determinism test). `run_task` is this function with the transcript discarded.
///
/// # Errors
///
/// Same as [`run_task`].
pub fn run_task_with_transcript(
    task: &BenchTask,
    model: &mut dyn ModelClient,
    registry: &CheckerRegistry,
    technology_source: &str,
    suite_version: &str,
    options: RunOptions,
) -> Result<(ResultRecord, Transcript), RunError> {
    let checker = registry
        .get(&task.checker)
        .ok_or_else(|| RunError::UnknownChecker {
            name: task.checker.clone(),
        })?;

    let mut session = Session::new();
    let mut clock = StepClock::default();

    if !technology_source.is_empty() {
        session
            .apply(AgentCommand::SetTechnology {
                source: technology_source.to_owned(),
            })
            .map_err(|e| RunError::Technology {
                detail: e.to_string(),
            })?;
        clock.tick();
    }

    let mut iterations = 0_u32;
    let mut first_proposal_violations = 0_u32;
    let mut final_violations = 0_u32;
    let mut success = false;
    let mut feedback: Vec<String> = Vec::new();
    let mut prev_violations = 0_u32;

    while iterations < options.max_iterations {
        let context = Context {
            iteration: iterations,
            prev_violations,
            feedback: feedback.clone(),
        };
        let commands = model.propose(&task.id, &task.prompt, &context);
        // An empty proposal after the first iteration means the model has nothing left
        // to try; stop and report the standing result rather than spinning.
        if commands.is_empty() && iterations > 0 {
            break;
        }

        for command in commands {
            // Applying records into the session transcript even on a command error, so
            // a bad command is visible to the checker rather than silently dropped.
            let _ = session.apply(command);
            clock.tick();
        }

        iterations += 1;
        let violations = drc_violation_count(&session);
        if iterations == 1 {
            first_proposal_violations = violations;
        }
        final_violations = violations;
        prev_violations = violations;

        match run_checker(checker, &session) {
            CheckResult::Pass => {
                success = true;
                break;
            }
            CheckResult::Fail(failures) => {
                feedback = failures.into_iter().map(|f| f.reason).collect();
            }
        }
    }

    let record = ResultRecord {
        task_id: task.id.clone(),
        model: model.id().to_owned(),
        suite_version: suite_version.to_owned(),
        success,
        iterations,
        first_proposal_violations,
        final_violations,
        wall_ms: clock.elapsed_ms(),
        // The bench runner drives the deterministic mock; a live-backend runner stamps
        // these after the fact (see `reticle-bench`'s `--backend` path).
        backend: model.id().to_owned(),
        quantization: None,
    };
    Ok((record, reticle_agent_api::transcript_of(&session)))
}

/// A deterministic stand-in for wall time: a monotonic count of applied commands,
/// reported as milliseconds so it fits [`ResultRecord::wall_ms`] without implying a
/// real duration. Library code uses this instead of the clock so records are exactly
/// reproducible.
#[derive(Clone, Copy, Debug, Default)]
struct StepClock {
    steps: u64,
}

impl StepClock {
    /// Advances the clock by one step (one applied command).
    fn tick(&mut self) {
        self.steps += 1;
    }

    /// The elapsed "time" as a step count.
    fn elapsed_ms(self) -> u64 {
        self.steps
    }
}

/// Runs `checker` over the session's current document and a snapshot transcript.
fn run_checker(checker: &dyn Checker, session: &Session) -> CheckResult {
    let transcript = snapshot_transcript(session);
    checker.check(session.document(), &transcript)
}

/// Builds a [`Transcript`] from the session's recorded commands and current document
/// hash, the value the checker sees.
fn snapshot_transcript(session: &Session) -> Transcript {
    Transcript {
        records: session.transcript().to_vec(),
        final_hash: reticle_model::document_hash(session.document()),
    }
}

/// Counts DRC violations on the session's target cell under the built-in SKY130 rule
/// subset. Used for the first-versus-final violation columns; returns `0` when the
/// document has no cell yet.
fn drc_violation_count(session: &Session) -> u32 {
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

#[cfg(test)]
mod tests {
    use super::{RunError, RunOptions, run_task};
    use crate::model::MockModel;
    use crate::{BenchTask, CheckerRegistry, Tier};
    use reticle_agent_api::AgentCommand;
    use reticle_agent_api::args::{LayerArg, PointArg, RectArg};

    /// A met1 rectangle command from the origin to `(size, size)` in cell `top`.
    fn met1_rect(size: i32) -> AgentCommand {
        AgentCommand::AddRect {
            cell: "top".into(),
            layer: LayerArg {
                layer: 68,
                datatype: 20,
            },
            rect: RectArg {
                min: PointArg { x: 0, y: 0 },
                max: PointArg { x: size, y: size },
            },
        }
    }

    /// The tier-1 DRC task fixture: draws a met1 rect, checked by `drc_clean`.
    fn drc_task() -> BenchTask {
        BenchTask {
            id: "t1_drc".into(),
            tier: Tier(1),
            prompt: "Draw a clean met1 rectangle.".into(),
            technology: "sky130.tech".into(),
            checker: "drc_clean".into(),
            intent: None,
        }
    }

    #[test]
    fn converges_after_correcting_first_violation() {
        // First attempt: an under-width 100x100 met1 rect (violates min width/area).
        // Second attempt: delete it and add a clean 500x500 rect.
        let create = AgentCommand::CreateCell { name: "top".into() };
        let model = MockModel::new().with_script(
            "t1_drc",
            vec![
                vec![create.clone(), met1_rect(100)],
                // Correct by deleting the bad shape (id 1) and adding a good one.
                vec![
                    AgentCommand::DeleteShapes {
                        ids: vec![reticle_agent_api::ElementId(1)],
                    },
                    met1_rect(500),
                ],
            ],
        );
        let mut model: Box<dyn crate::model::ModelClient> = Box::new(model);
        let registry = CheckerRegistry::default();
        let record = run_task(
            &drc_task(),
            model.as_mut(),
            &registry,
            "",
            "0.1.0",
            RunOptions::default(),
        )
        .expect("run");

        assert!(record.success, "task should converge to a clean layout");
        assert_eq!(record.final_violations, 0, "final layout must be clean");
        assert!(
            record.first_proposal_violations > 0,
            "the first proposal was deliberately dirty"
        );
        assert_eq!(record.iterations, 2, "one correction was needed");
        assert_eq!(record.model, "mock");
        assert_eq!(record.suite_version, "0.1.0");
        assert_eq!(record.task_id, "t1_drc");
    }

    #[test]
    fn passes_on_first_try_when_proposal_is_clean() {
        let model = MockModel::new().with_script(
            "t1_drc",
            vec![vec![
                AgentCommand::CreateCell { name: "top".into() },
                met1_rect(500),
            ]],
        );
        let mut model: Box<dyn crate::model::ModelClient> = Box::new(model);
        let record = run_task(
            &drc_task(),
            model.as_mut(),
            &CheckerRegistry::default(),
            "",
            "0.1.0",
            RunOptions::default(),
        )
        .expect("run");
        assert!(record.success);
        assert_eq!(record.iterations, 1);
        assert_eq!(record.first_proposal_violations, 0);
        assert_eq!(record.final_violations, 0);
    }

    #[test]
    fn reports_failure_when_never_corrected() {
        // The model only ever proposes the bad shape, so the checker never passes and
        // the run exhausts its iterations with success = false.
        let model = MockModel::new().with_script(
            "t1_drc",
            vec![vec![
                AgentCommand::CreateCell { name: "top".into() },
                met1_rect(100),
            ]],
        );
        let mut model: Box<dyn crate::model::ModelClient> = Box::new(model);
        let record = run_task(
            &drc_task(),
            model.as_mut(),
            &CheckerRegistry::default(),
            "",
            "0.1.0",
            RunOptions::default(),
        )
        .expect("run");
        assert!(!record.success);
        assert!(record.final_violations > 0);
        // After the first proposal the model returns nothing, so the loop stops early.
        assert_eq!(record.iterations, 1);
    }

    #[test]
    fn unknown_checker_is_an_error() {
        let task = BenchTask {
            checker: "no_such_checker".into(),
            ..drc_task()
        };
        let mut model: Box<dyn crate::model::ModelClient> = Box::new(MockModel::new());
        let err = run_task(
            &task,
            model.as_mut(),
            &CheckerRegistry::default(),
            "",
            "0.1.0",
            RunOptions::default(),
        )
        .expect_err("missing checker must error");
        assert!(matches!(err, RunError::UnknownChecker { .. }));
    }
}
