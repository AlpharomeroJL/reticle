//! The propose-verify-correct loop and the four-artifact writer.
//!
//! [`run_agent_task`] drives one task through the loop: create a [`Session`], install
//! the task's technology, then repeatedly ask the model for a batch of commands, apply
//! them (within a command budget), and verify with the task's [`Checker`]. DRC
//! violation counts plus the checker's pass/fail are the oracle. When the check fails
//! and there is iteration and budget left, the verifier's feedback is handed back so
//! the model can correct; the loop stops on a pass, on an empty proposal, or when a cap
//! is hit.
//!
//! Every run writes four artifacts next to each other: the JSONL transcript, the final
//! GDS, a rendered PNG, and the [`ResultRecord`] JSON. A run that never passes its check
//! is recorded with `success = false`; artifacts are never retro-edited to make a
//! failure look like a pass. The transcript is asserted key-free by construction (the
//! session records only commands and outcomes, never the key), and the tests confirm no
//! artifact contains the secret.
//!
//! # Reuse
//!
//! The loop mirrors `reticle-bench`'s [`run_task`](reticle_bench::run_task) core
//! (same [`Context`], [`ModelClient`], and per-iteration verify shape) but adds a
//! command budget, a document-context hook so the model sees the layout, and the
//! artifact writers, none of which `run_task` provides. `reticle-bench` is unmodified.

use std::path::{Path, PathBuf};

use reticle_agent_api::{AgentCommand, AgentResponse, Session, Transcript};
use reticle_bench::model::{Context, ModelClient};
use reticle_bench::{BenchTask, CheckFailure, CheckResult, Checker, CheckerRegistry, ResultRecord};

/// How the loop is bounded.
#[derive(Clone, Copy, Debug)]
pub struct LoopOptions {
    /// Maximum propose-verify-correct iterations before giving up.
    pub max_iterations: u32,
    /// Maximum total commands the model may have applied across all iterations. Once
    /// reached, the loop stops proposing further batches.
    pub command_budget: u32,
}

impl Default for LoopOptions {
    fn default() -> Self {
        Self {
            max_iterations: 4,
            command_budget: 256,
        }
    }
}

/// A source of mid-session user refinement constraints for the loop.
///
/// An iterative refinement session lets the user add new constraints *between*
/// iterations without restarting the run: "make the wire wider", "keep it on met1",
/// and so on. The loop drains this source once per iteration, just before it asks the
/// model, and folds whatever strings it yields into the model's [`Context::feedback`]
/// alongside the checker's own failure reasons. The model therefore sees the new
/// constraint on the very next proposal, the checker re-runs, and the session
/// converges without being torn down and rebuilt (see the agent chapter for the
/// protocol).
///
/// The one required method, [`drain`](RefinementSource::drain), returns the
/// constraints that have arrived since it was last called (an empty vector when none
/// have). A caller backed by a channel drains the receiver; a scripted test yields a
/// pre-set batch on a chosen iteration. The loop owns no channel of its own, so this
/// works the same for a live UI, an HTTP endpoint, or a deterministic test.
pub trait RefinementSource {
    /// The refinement constraints that have arrived since the previous call, in the
    /// order the user supplied them. Returns an empty vector when there is nothing new.
    ///
    /// `iteration` is the zero-based index of the iteration about to run, so a source
    /// can schedule a scripted constraint for a specific point in the sequence; a
    /// channel-backed source ignores it and just drains what is queued.
    fn drain(&mut self, iteration: u32) -> Vec<String>;
}

/// A [`RefinementSource`] that never yields a constraint: the loop runs exactly as it
/// did before refinements existed. Used by [`run_agent_task`] and any caller that has
/// no mid-session constraints to inject.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoRefinements;

impl RefinementSource for NoRefinements {
    fn drain(&mut self, _iteration: u32) -> Vec<String> {
        Vec::new()
    }
}

/// A [`RefinementSource`] adapting any `FnMut(u32) -> Vec<String>` closure, so a caller
/// can supply refinements inline without defining a type. The closure is invoked once
/// per iteration with the iteration index and returns the constraints new since the
/// last call.
pub struct RefinementFn<F>(pub F)
where
    F: FnMut(u32) -> Vec<String>;

// A closure has no meaningful Debug, so print an opaque placeholder rather than derive.
impl<F> std::fmt::Debug for RefinementFn<F>
where
    F: FnMut(u32) -> Vec<String>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("RefinementFn").field(&"<closure>").finish()
    }
}

impl<F> RefinementSource for RefinementFn<F>
where
    F: FnMut(u32) -> Vec<String>,
{
    fn drain(&mut self, iteration: u32) -> Vec<String> {
        (self.0)(iteration)
    }
}

/// Which backend produced a run, stamped onto the [`ResultRecord`].
///
/// Keeps a mock run, a local (Ollama) run, and a frontier (`anthropic`) run distinct in
/// the results, so a summary never conflates them. The `model` id already lives on the
/// record via [`ModelClient::id`]; this adds the backend family and an optional
/// quantization the model id alone does not carry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Provenance {
    /// The backend family: `"mock"`, `"ollama"`, `"anthropic"`, or another label.
    pub backend: String,
    /// The model's quantization, when known (for example `"Q4_K_M"`); `None` otherwise.
    pub quantization: Option<String>,
}

impl Provenance {
    /// A provenance with just a backend label and no quantization.
    #[must_use]
    pub fn new(backend: impl Into<String>) -> Self {
        Self {
            backend: backend.into(),
            quantization: None,
        }
    }

    /// Sets the quantization label.
    #[must_use]
    pub fn with_quantization(mut self, quantization: impl Into<String>) -> Self {
        self.quantization = Some(quantization.into());
        self
    }
}

/// The paths of the four artifacts a run writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifacts {
    /// The JSONL transcript (one command record per line).
    pub transcript: PathBuf,
    /// The exported GDSII of the final document.
    pub gds: PathBuf,
    /// The rendered PNG, when rendering succeeded (a headless run with no GPU records
    /// no PNG; the reason is noted in [`RunOutcome::render_note`]).
    pub png: Option<PathBuf>,
    /// The result-record JSON (a single-element array, matching the bench writer).
    pub result: PathBuf,
}

/// The result of a run: the record plus where its artifacts landed.
#[derive(Debug)]
pub struct RunOutcome {
    /// The record (also written to [`Artifacts::result`]).
    pub record: ResultRecord,
    /// Where the four artifacts were written.
    pub artifacts: Artifacts,
    /// A note about rendering, e.g. why no PNG was produced (no GPU adapter). Empty
    /// when a PNG was written.
    pub render_note: String,
}

/// Why a run could not be set up or its artifacts could not be written.
///
/// A task that merely fails its check is **not** an error: it produces a well-formed
/// [`RunOutcome`] with `success = false`. These variants are setup/IO failures.
#[derive(Debug)]
pub enum LoopError {
    /// The task's technology source could not be applied.
    Technology {
        /// What went wrong.
        detail: String,
    },
    /// The task named a checker the registry does not contain.
    UnknownChecker {
        /// The missing checker name.
        name: String,
    },
    /// An artifact could not be written to disk.
    Artifact {
        /// The artifact path.
        path: PathBuf,
        /// The IO error message.
        detail: String,
    },
}

impl std::fmt::Display for LoopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoopError::Technology { detail } => write!(f, "technology setup failed: {detail}"),
            LoopError::UnknownChecker { name } => write!(f, "no checker named `{name}`"),
            LoopError::Artifact { path, detail } => {
                write!(f, "writing artifact {}: {detail}", path.display())
            }
        }
    }
}

impl std::error::Error for LoopError {}

/// Runs `task` through the propose-verify-correct loop against `model`, writing the
/// four artifacts under `out_dir` and returning the outcome.
///
/// `context_hook` is called with `(&mut model, &session)` before each proposal so a
/// document-aware model (like [`AnthropicModel`](crate::AnthropicModel)) can be fed a
/// snapshot of the current layout; pass a no-op closure for a model that ignores it.
///
/// `technology_source` is the technology-file text (empty keeps the session default),
/// resolved to text by the caller so the loop does no technology IO. `wall_ms` is a
/// caller-provided duration in milliseconds (the CLI measures real time; tests pass a
/// fixed value so records are reproducible).
///
/// `provenance` labels which backend produced the run (see [`Provenance`]); it is
/// stamped straight onto the [`ResultRecord`] so a mock, a local, and a frontier run are
/// never conflated.
///
/// This runs with no mid-session refinements; it is
/// [`run_agent_task_refined`] with a [`NoRefinements`] source. Use the refined form to
/// fold user constraints in between iterations without restarting the loop.
///
/// # Errors
///
/// Returns [`LoopError::Technology`] or [`LoopError::UnknownChecker`] on a setup
/// failure, or [`LoopError::Artifact`] if an artifact cannot be written. A failed
/// check is not an error.
#[allow(clippy::too_many_arguments)]
pub fn run_agent_task<M: ModelClient>(
    task: &BenchTask,
    model: &mut M,
    registry: &CheckerRegistry,
    technology_source: &str,
    suite_version: &str,
    options: LoopOptions,
    out_dir: &Path,
    wall_ms: u64,
    provenance: &Provenance,
    context_hook: impl FnMut(&mut M, &Session),
) -> Result<RunOutcome, LoopError> {
    run_agent_task_refined(
        task,
        model,
        registry,
        technology_source,
        suite_version,
        options,
        out_dir,
        wall_ms,
        provenance,
        context_hook,
        NoRefinements,
    )
}

/// Like [`run_agent_task`], but folds mid-session user refinement constraints into the
/// loop as they arrive, without restarting the run.
///
/// `refinements` is drained once per iteration, just before the model is asked; each
/// yielded constraint string is added to the model's [`Context::feedback`] alongside
/// the checker's own failure reasons, so the model reacts to it on the next proposal
/// and the checker re-runs against the amended target. Pass [`NoRefinements`] for the
/// original behavior, or a [`RefinementFn`] / custom [`RefinementSource`] to supply
/// constraints from a channel, a UI, or a script. Constraints accumulate across
/// iterations, so a constraint injected on iteration 1 still conditions iteration 3.
///
/// Every other argument behaves exactly as in [`run_agent_task`].
///
/// # Errors
///
/// Same as [`run_agent_task`]: a setup or artifact-write failure; a failed check is not
/// an error.
#[allow(clippy::too_many_arguments)]
pub fn run_agent_task_refined<M: ModelClient>(
    task: &BenchTask,
    model: &mut M,
    registry: &CheckerRegistry,
    technology_source: &str,
    suite_version: &str,
    options: LoopOptions,
    out_dir: &Path,
    wall_ms: u64,
    provenance: &Provenance,
    mut context_hook: impl FnMut(&mut M, &Session),
    mut refinements: impl RefinementSource,
) -> Result<RunOutcome, LoopError> {
    let checker = registry
        .get(&task.checker)
        .ok_or_else(|| LoopError::UnknownChecker {
            name: task.checker.clone(),
        })?;

    let mut session = Session::new();
    if !technology_source.is_empty() {
        session
            .apply(AgentCommand::SetTechnology {
                source: technology_source.to_owned(),
            })
            .map_err(|e| LoopError::Technology {
                detail: e.to_string(),
            })?;
    }

    let loop_result = drive(
        task,
        model,
        checker,
        &mut session,
        options,
        &mut context_hook,
        &mut refinements,
    );

    // Write the four artifacts regardless of pass/fail; a failure is recorded honestly.
    let record = ResultRecord {
        task_id: task.id.clone(),
        model: model.id().to_owned(),
        suite_version: suite_version.to_owned(),
        success: loop_result.success,
        iterations: loop_result.iterations,
        first_proposal_violations: loop_result.first_proposal_violations,
        final_violations: loop_result.final_violations,
        wall_ms,
        backend: provenance.backend.clone(),
        quantization: provenance.quantization.clone(),
    };

    let (artifacts, render_note) = write_artifacts(task, &session, &record, out_dir)?;
    Ok(RunOutcome {
        record,
        artifacts,
        render_note,
    })
}

/// The standing state a loop run reports.
struct LoopResult {
    success: bool,
    iterations: u32,
    first_proposal_violations: u32,
    final_violations: u32,
}

/// The propose-verify-correct core: ask, apply (within budget), verify, feed back.
///
/// `refinements` is drained once per iteration and its constraints accumulate in
/// `user_constraints`, which is folded into every subsequent [`Context::feedback`]
/// ahead of the checker's own reasons. That is how a mid-session user constraint reaches
/// the model without restarting the loop: the checker still re-runs each iteration, and
/// the bounded iteration count still applies, so a constraint that conflicts with the
/// task cannot spin forever (it is capped by `options.max_iterations`).
fn drive<M: ModelClient>(
    task: &BenchTask,
    model: &mut M,
    checker: &dyn Checker,
    session: &mut Session,
    options: LoopOptions,
    context_hook: &mut impl FnMut(&mut M, &Session),
    refinements: &mut impl RefinementSource,
) -> LoopResult {
    let mut iterations = 0_u32;
    let mut first_proposal_violations = 0_u32;
    let mut final_violations = 0_u32;
    let mut success = false;
    let mut feedback: Vec<String> = Vec::new();
    // User refinement constraints, accumulated across iterations. Unlike `feedback`
    // (which the checker replaces each iteration), these persist: a constraint injected
    // once keeps conditioning every later proposal.
    let mut user_constraints: Vec<String> = Vec::new();
    let mut prev_violations = 0_u32;
    let mut commands_applied = 0_u32;

    while iterations < options.max_iterations {
        // Fold in any user constraints that arrived since the previous iteration, before
        // building the context, so the model sees them on this very proposal.
        user_constraints.extend(refinements.drain(iterations));

        // Give the model the current layout before it proposes.
        context_hook(model, session);

        // The model sees the accumulated user constraints first, then the checker's
        // failure reasons from the previous attempt. Combining them here (rather than in
        // a new Context field) keeps `reticle_bench::model::Context` frozen.
        let context = Context {
            iteration: iterations,
            prev_violations,
            feedback: user_constraints
                .iter()
                .cloned()
                .chain(feedback.iter().cloned())
                .collect(),
        };
        let commands = model.propose(&task.id, &task.prompt, &context);

        // An empty proposal after the first iteration means the model has nothing left
        // to try; stop rather than spinning.
        if commands.is_empty() && iterations > 0 {
            break;
        }

        for command in commands {
            if commands_applied >= options.command_budget {
                break;
            }
            // Applying records the command (and any error) into the transcript, so a
            // bad command is visible to the checker rather than silently dropped.
            let _ = session.apply(command);
            commands_applied += 1;
        }

        iterations += 1;
        let violations = drc_violation_count(session);
        if iterations == 1 {
            first_proposal_violations = violations;
        }
        final_violations = violations;
        prev_violations = violations;

        match checker.check(session.document(), &snapshot_transcript(session)) {
            CheckResult::Pass => {
                success = true;
                break;
            }
            CheckResult::Fail(failures) => {
                feedback = failures
                    .into_iter()
                    .map(|f: CheckFailure| f.reason)
                    .collect();
            }
        }

        // If the budget is spent there is no point asking for another batch.
        if commands_applied >= options.command_budget {
            break;
        }
    }

    LoopResult {
        success,
        iterations,
        first_proposal_violations,
        final_violations,
    }
}

/// Builds a [`Transcript`] from the session's records and current document hash.
fn snapshot_transcript(session: &Session) -> Transcript {
    Transcript {
        records: session.transcript().to_vec(),
        final_hash: reticle_model::document_hash(session.document()),
    }
}

/// Counts DRC violations on the session's target cell under the built-in SKY130 rule
/// subset; `0` when the document has no cell yet.
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

/// A compact, human-readable summary of the current document for the model prompt.
///
/// Lists each cell with its shape/instance/array counts and bounding box, so the model
/// knows what already exists (which cells to add to, what not to recreate) without the
/// full geometry. Kept small on purpose; the model edits by command, not by diffing a
/// dump.
#[must_use]
pub fn document_summary(session: &Session) -> String {
    use reticle_model::ShapeKind;
    use std::fmt::Write as _;
    let doc = session.document();
    let cells: Vec<_> = doc.cells().collect();
    if cells.is_empty() {
        return "(empty document, no cells yet)".to_owned();
    }
    let mut out = String::new();
    for cell in cells {
        let (rects, polys, paths) =
            cell.shapes
                .iter()
                .fold((0, 0, 0), |(r, p, pa), s| match s.kind {
                    ShapeKind::Rect(_) => (r + 1, p, pa),
                    ShapeKind::Polygon(_) => (r, p + 1, pa),
                    ShapeKind::Path(_) => (r, p, pa + 1),
                });
        // Writing into a String is infallible; the Result is discarded deliberately.
        let _ = writeln!(
            out,
            "cell {}: {} shapes ({rects} rect, {polys} polygon, {paths} path), \
             {} instances, {} arrays",
            cell.name,
            cell.shapes.len(),
            cell.instances.len(),
            cell.arrays.len(),
        );
    }
    out
}

// ----- artifact writers -----------------------------------------------------

/// Writes the four artifacts under `out_dir` and returns their paths plus a render
/// note.
///
/// The transcript is JSONL (one [`CommandRecord`](reticle_agent_api::CommandRecord) per
/// line). The GDS and PNG come from applying `ExportGds`/`RenderPng` to a *clone* of the
/// session so the run's own transcript is not extended by artifact production. A
/// headless environment with no GPU yields no PNG; that is recorded in the note, not
/// treated as a failure.
fn write_artifacts(
    task: &BenchTask,
    session: &Session,
    record: &ResultRecord,
    out_dir: &Path,
) -> Result<(Artifacts, String), LoopError> {
    std::fs::create_dir_all(out_dir).map_err(|e| LoopError::Artifact {
        path: out_dir.to_path_buf(),
        detail: e.to_string(),
    })?;

    // 1. Transcript as JSONL.
    let transcript_path = out_dir.join(format!("{}.transcript.jsonl", task.id));
    write_transcript_jsonl(session, &transcript_path)?;

    // 2. Final GDS. Export on a clone so the live transcript is untouched.
    let gds_path = out_dir.join(format!("{}.gds", task.id));
    let mut export_session = clone_session(session);
    match export_session.apply(AgentCommand::ExportGds) {
        Ok(AgentResponse::Blob { bytes, .. }) => {
            std::fs::write(&gds_path, &bytes).map_err(|e| LoopError::Artifact {
                path: gds_path.clone(),
                detail: e.to_string(),
            })?;
        }
        Ok(_) => {
            // ExportGds always returns a Blob; any other shape is unexpected but not
            // fatal to the run. Record an empty file so the artifact path is stable.
            std::fs::write(&gds_path, []).map_err(|e| LoopError::Artifact {
                path: gds_path.clone(),
                detail: e.to_string(),
            })?;
        }
        Err(e) => {
            // A document with no cell cannot export; still leave a stable, empty file
            // and continue. The failure is already reflected in the result record.
            let _ = e;
            std::fs::write(&gds_path, []).map_err(|e| LoopError::Artifact {
                path: gds_path.clone(),
                detail: e.to_string(),
            })?;
        }
    }

    // 3. Rendered PNG (best-effort: needs a GPU adapter and a cell to frame).
    let png_path = out_dir.join(format!("{}.png", task.id));
    let render_note = match render_png_bytes(session) {
        Ok(bytes) => {
            std::fs::write(&png_path, &bytes).map_err(|e| LoopError::Artifact {
                path: png_path.clone(),
                detail: e.to_string(),
            })?;
            String::new()
        }
        Err(note) => note,
    };
    let png = if render_note.is_empty() {
        Some(png_path)
    } else {
        None
    };

    // 4. Result record as a single-element JSON array (matches the bench writer shape).
    let result_path = out_dir.join(format!("{}.result.json", task.id));
    let json = serde_json::to_string_pretty(std::slice::from_ref(record)).map_err(|e| {
        LoopError::Artifact {
            path: result_path.clone(),
            detail: e.to_string(),
        }
    })?;
    std::fs::write(&result_path, json).map_err(|e| LoopError::Artifact {
        path: result_path.clone(),
        detail: e.to_string(),
    })?;

    Ok((
        Artifacts {
            transcript: transcript_path,
            gds: gds_path,
            png,
            result: result_path,
        },
        render_note,
    ))
}

/// Writes the session's command records to `path`, one JSON object per line, followed
/// by a final line carrying the document hash (so a reader can verify a replay).
fn write_transcript_jsonl(session: &Session, path: &Path) -> Result<(), LoopError> {
    use std::fmt::Write as _;
    let mut out = String::new();
    for record in session.transcript() {
        let line = serde_json::to_string(record).map_err(|e| LoopError::Artifact {
            path: path.to_path_buf(),
            detail: format!("serializing transcript record: {e}"),
        })?;
        out.push_str(&line);
        out.push('\n');
    }
    // Trailer: the final document hash the replay must reproduce.
    let _ = writeln!(
        out,
        "{}",
        serde_json::json!({ "final_hash": reticle_model::document_hash(session.document()) })
    );
    std::fs::write(path, out).map_err(|e| LoopError::Artifact {
        path: path.to_path_buf(),
        detail: e.to_string(),
    })
}

/// Renders the session's target cell to PNG bytes via the engine's `RenderPng`
/// command, framing the document's bounding box.
///
/// Returns the note explaining why no PNG was produced (no cell, no GPU adapter) as an
/// `Err`, so the caller can record it without failing the run.
fn render_png_bytes(session: &Session) -> Result<Vec<u8>, String> {
    use reticle_agent_api::args::{PointArg, RectArg};
    use reticle_geometry::Rect;

    let doc = session.document();
    let cell = doc
        .top_cells()
        .first()
        .cloned()
        .or_else(|| doc.cells().next().map(|c| c.name.clone()))
        .ok_or_else(|| "no cell to render".to_owned())?;
    // Frame the target cell's bounding box; fall back to a unit box if empty.
    let bbox = cell_bbox(session, &cell).unwrap_or_else(|| {
        Rect::new(
            reticle_geometry::Point::new(0, 0),
            reticle_geometry::Point::new(1, 1),
        )
    });
    let region = RectArg {
        min: PointArg {
            x: bbox.min.x,
            y: bbox.min.y,
        },
        max: PointArg {
            x: bbox.max.x,
            y: bbox.max.y,
        },
    };
    let mut render_session = clone_session(session);
    match render_session.apply(AgentCommand::RenderPng {
        region,
        width: 512,
        height: 512,
    }) {
        Ok(AgentResponse::Blob { bytes, .. }) => Ok(bytes),
        Ok(_) => Err("render returned no image bytes".to_owned()),
        Err(e) => Err(format!("render skipped: {e}")),
    }
}

/// The bounding box of `cell` in `session`, if it has geometry.
fn cell_bbox(session: &Session, cell: &str) -> Option<reticle_geometry::Rect> {
    use reticle_geometry::Shape as _;
    let c = session.document().cell(cell)?;
    let mut boxes = c.shapes.iter().map(reticle_model::DrawShape::bounding_box);
    let first: reticle_geometry::Rect = boxes.next()?;
    Some(boxes.fold(first, |acc, b| acc.union(&b)))
}

/// Rebuilds an equivalent session by replaying `session`'s recorded commands, for
/// producing an artifact (GDS/PNG) without extending the live session's transcript.
///
/// Uses only the public surface: a fresh [`Session`] and [`Session::apply`] over the
/// public [`transcript`](Session::transcript). Command application is deterministic, so
/// the clone's document, revision, and ids match the original. A command that failed in
/// the original fails again the same way, so the replayed document is identical.
fn clone_session(session: &Session) -> Session {
    let mut clone = Session::new();
    for record in session.transcript() {
        let _ = clone.apply(record.command.clone());
    }
    clone
}

#[cfg(test)]
mod tests {
    use super::{
        Artifacts, LoopOptions, RunOutcome, document_summary, drc_violation_count, run_agent_task,
    };
    use reticle_agent_api::args::{LayerArg, PointArg, RectArg};
    use reticle_agent_api::{AgentCommand, Session};
    use reticle_bench::model::{Context, MockModel, ModelClient};
    use reticle_bench::{BenchTask, CheckerRegistry, Tier};

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

    /// The tier-1 DRC task fixture.
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
    fn document_summary_lists_cells_and_counts() {
        let mut session = Session::new();
        session
            .apply(AgentCommand::CreateCell { name: "top".into() })
            .unwrap();
        session.apply(met1_rect(500)).unwrap();
        let summary = document_summary(&session);
        assert!(summary.contains("cell top"));
        assert!(summary.contains("1 rect"));
    }

    #[test]
    fn empty_document_summary_is_marked() {
        assert!(document_summary(&Session::new()).contains("empty"));
    }

    #[test]
    fn drc_count_zero_on_empty() {
        assert_eq!(drc_violation_count(&Session::new()), 0);
    }

    #[test]
    fn command_budget_caps_applied_commands() {
        // A model that keeps proposing shapes; with a tiny budget only the first few
        // apply. Use the default script so every iteration returns two commands.
        let model = MockModel::new().with_default(vec![vec![
            AgentCommand::CreateCell { name: "top".into() },
            met1_rect(100),
            met1_rect(100),
            met1_rect(100),
        ]]);
        let mut model = model;
        let out_dir = unique_dir("budget");
        let outcome = run_agent_task(
            &drc_task(),
            &mut model,
            &CheckerRegistry::default(),
            "",
            "0.1.0",
            LoopOptions {
                max_iterations: 4,
                command_budget: 2,
            },
            &out_dir,
            0,
            &crate::run::Provenance::new("mock"),
            |_, _| {},
        )
        .expect("run");
        // Budget of 2 means exactly two commands were applied (create_cell + one rect);
        // the two extra rects in the batch are cut off. The transcript JSONL has one
        // line per applied command plus a final_hash trailer line.
        let text = std::fs::read_to_string(&outcome.artifacts.transcript).unwrap();
        let command_lines = text.lines().filter(|l| l.contains("\"op\"")).count();
        assert_eq!(command_lines, 2, "command budget must cap applied commands");
        cleanup(&out_dir);
    }

    #[test]
    fn writes_four_artifacts_and_records_success() {
        let create = AgentCommand::CreateCell { name: "top".into() };
        let model = MockModel::new().with_script("t1_drc", vec![vec![create, met1_rect(500)]]);
        let mut model = model;
        let out_dir = unique_dir("ok");
        let outcome: RunOutcome = run_agent_task(
            &drc_task(),
            &mut model,
            &CheckerRegistry::default(),
            "",
            "0.1.0",
            LoopOptions::default(),
            &out_dir,
            0,
            &crate::run::Provenance::new("mock"),
            |_, _| {},
        )
        .expect("run");

        assert!(outcome.record.success, "clean rect should pass drc_clean");
        let Artifacts {
            transcript,
            gds,
            result,
            ..
        } = &outcome.artifacts;
        assert!(transcript.exists(), "transcript written");
        assert!(gds.exists(), "gds written");
        assert!(result.exists(), "result written");
        // The result file round-trips as a one-element record array.
        let text = std::fs::read_to_string(result).unwrap();
        let records: Vec<reticle_bench::ResultRecord> = serde_json::from_str(&text).unwrap();
        assert_eq!(records.len(), 1);
        assert!(records[0].success);
        cleanup(&out_dir);
    }

    #[test]
    fn failing_task_is_recorded_as_failure_not_retro_edited() {
        // The model only ever proposes the bad (too-narrow) shape, so the check never
        // passes; the run must be recorded as a failure with violations standing.
        let model = MockModel::new().with_script(
            "t1_drc",
            vec![vec![
                AgentCommand::CreateCell { name: "top".into() },
                met1_rect(100),
            ]],
        );
        let mut model = model;
        let out_dir = unique_dir("fail");
        let outcome = run_agent_task(
            &drc_task(),
            &mut model,
            &CheckerRegistry::default(),
            "",
            "0.1.0",
            LoopOptions::default(),
            &out_dir,
            0,
            &crate::run::Provenance::new("mock"),
            |_, _| {},
        )
        .expect("run");

        assert!(!outcome.record.success, "must not pass");
        assert!(outcome.record.final_violations > 0, "violations stand");
        // The written record agrees: no retro-edit to a pass.
        let text = std::fs::read_to_string(&outcome.artifacts.result).unwrap();
        let records: Vec<reticle_bench::ResultRecord> = serde_json::from_str(&text).unwrap();
        assert!(!records[0].success);
        cleanup(&out_dir);
    }

    #[test]
    fn context_hook_receives_the_session() {
        // The hook is invoked once per iteration with the live session; prove it sees
        // the document growing.
        let create = AgentCommand::CreateCell { name: "top".into() };
        let model = MockModel::new().with_script("t1_drc", vec![vec![create, met1_rect(500)]]);
        let mut model = model;
        let out_dir = unique_dir("hook");
        let mut hook_calls = 0_u32;
        let _ = run_agent_task(
            &drc_task(),
            &mut model,
            &CheckerRegistry::default(),
            "",
            "0.1.0",
            LoopOptions::default(),
            &out_dir,
            0,
            &crate::run::Provenance::new("mock"),
            |_model, session| {
                hook_calls += 1;
                // On the first call the document is still empty.
                if hook_calls == 1 {
                    assert!(document_summary(session).contains("empty"));
                }
            },
        )
        .expect("run");
        assert!(hook_calls >= 1);
        cleanup(&out_dir);
    }

    /// A transport that returns a fixed `emit_commands` batch, so a full loop can run
    /// through the real [`AnthropicModel`] without a network. It ignores the request
    /// (and thus the API key header) entirely.
    struct CannedTransport(String);

    impl crate::model::HttpTransport for CannedTransport {
        fn post_json(
            &self,
            _url: &str,
            _api_key: &str,
            _body: &serde_json::Value,
        ) -> Result<String, String> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn api_key_never_appears_in_any_written_artifact() {
        use crate::model::AnthropicModel;
        use crate::redact::ApiKey;

        // A distinctive key that would be unmistakable if it leaked.
        const KEY: &str = "sk-ant-LEAKTEST-0123456789abcdef";
        // The model "returns" a clean batch that converges on the first try.
        let response = serde_json::json!({
            "content": [ { "type": "tool_use", "name": "emit_commands", "input": {
                "commands": [
                    { "op": "create_cell", "name": "top" },
                    { "op": "add_rect", "cell": "top",
                      "layer": { "layer": 68, "datatype": 20 },
                      "rect": { "min": { "x": 0, "y": 0 }, "max": { "x": 500, "y": 500 } } }
                ]
            }}]
        })
        .to_string();

        let mut model = AnthropicModel::for_test(ApiKey::from_raw(KEY))
            .with_transport(Box::new(CannedTransport(response)));
        let out_dir = unique_dir("keyleak");
        let outcome = run_agent_task(
            &drc_task(),
            &mut model,
            &CheckerRegistry::default(),
            "",
            "0.1.0",
            LoopOptions::default(),
            &out_dir,
            0,
            &crate::run::Provenance::new("anthropic"),
            // Drive the real document-context path the CLI uses.
            |m, session| m.set_document_context(document_summary(session)),
        )
        .expect("run");

        assert!(outcome.record.success, "the canned clean batch should pass");

        // The key must not appear in ANY written artifact: transcript, gds, result.
        for path in [
            &outcome.artifacts.transcript,
            &outcome.artifacts.gds,
            &outcome.artifacts.result,
        ] {
            let bytes = std::fs::read(path).expect("read artifact");
            let needle = KEY.as_bytes();
            let leaked = bytes.windows(needle.len()).any(|w| w == needle);
            assert!(!leaked, "API key leaked into {}", path.display());
        }
        if let Some(png) = &outcome.artifacts.png {
            let bytes = std::fs::read(png).expect("read png");
            assert!(
                !bytes.windows(KEY.len()).any(|w| w == KEY.as_bytes()),
                "API key leaked into the PNG"
            );
        }
        cleanup(&out_dir);
    }

    #[test]
    fn ollama_backend_drives_loop_end_to_end_and_scrubs_key() {
        use crate::ollama::OllamaModel;
        use crate::redact::ApiKey;

        // A distinctive optional key that would be unmistakable if it leaked.
        const KEY: &str = "sk-local-LEAKTEST-0123456789abcdef";
        // A recorded OpenAI-compatible response: an emit_commands tool call whose
        // `arguments` are a JSON string (as OpenAI encodes them) with a clean batch.
        let args = serde_json::json!({
            "commands": [
                { "op": "create_cell", "name": "top" },
                { "op": "add_rect", "cell": "top",
                  "layer": { "layer": 68, "datatype": 20 },
                  "rect": { "min": { "x": 0, "y": 0 }, "max": { "x": 500, "y": 500 } } }
            ]
        })
        .to_string();
        let response = serde_json::json!({
            "choices": [ { "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [ { "type": "function", "function": {
                    "name": "emit_commands",
                    "arguments": args
                }}]
            }}]
        })
        .to_string();

        let mut model = OllamaModel::for_test(
            Some(ApiKey::from_raw(KEY)),
            "http://localhost:11434/v1",
            "gpt-oss:16k",
        )
        .with_transport(Box::new(CannedTransport(response)));
        let out_dir = unique_dir("ollama-e2e");
        let outcome = run_agent_task(
            &drc_task(),
            &mut model,
            &CheckerRegistry::default(),
            "",
            "0.1.0",
            LoopOptions::default(),
            &out_dir,
            0,
            &crate::run::Provenance::new("ollama").with_quantization("Q4_K_M"),
            // Drive the real document-context path the CLI uses.
            |m, session| m.set_document_context(document_summary(session)),
        )
        .expect("run");

        assert!(
            outcome.record.success,
            "the canned clean batch should pass drc_clean"
        );
        // Provenance was stamped onto the record.
        assert_eq!(outcome.record.backend, "ollama");
        assert_eq!(outcome.record.quantization.as_deref(), Some("Q4_K_M"));
        assert_eq!(outcome.record.model, "gpt-oss:16k");

        // The optional key must not appear in ANY written artifact.
        for path in [
            &outcome.artifacts.transcript,
            &outcome.artifacts.gds,
            &outcome.artifacts.result,
        ] {
            let bytes = std::fs::read(path).expect("read artifact");
            let needle = KEY.as_bytes();
            let leaked = bytes.windows(needle.len()).any(|w| w == needle);
            assert!(!leaked, "optional key leaked into {}", path.display());
        }
        cleanup(&out_dir);
    }

    #[test]
    fn ollama_backend_text_array_fallback_drives_loop() {
        use crate::ollama::OllamaModel;

        // A recorded response where the model ignored tool_choice and answered in prose
        // with a JSON command array in `content` (the fallback path).
        let response = serde_json::json!({
            "choices": [ { "message": {
                "role": "assistant",
                "content": "Here is the batch: [\
                    {\"op\":\"create_cell\",\"name\":\"top\"},\
                    {\"op\":\"add_rect\",\"cell\":\"top\",\
                     \"layer\":{\"layer\":68,\"datatype\":20},\
                     \"rect\":{\"min\":{\"x\":0,\"y\":0},\"max\":{\"x\":500,\"y\":500}}}\
                ] applied now."
            }}]
        })
        .to_string();

        let mut model =
            OllamaModel::for_test(None, "http://localhost:11434/v1", "qwen2.5-coder:16k")
                .with_transport(Box::new(CannedTransport(response)));
        let out_dir = unique_dir("ollama-fallback");
        let outcome = run_agent_task(
            &drc_task(),
            &mut model,
            &CheckerRegistry::default(),
            "",
            "0.1.0",
            LoopOptions::default(),
            &out_dir,
            0,
            &crate::run::Provenance::new("ollama"),
            |m, session| m.set_document_context(document_summary(session)),
        )
        .expect("run");

        assert!(
            outcome.record.success,
            "the text-array fallback batch should pass drc_clean"
        );
        cleanup(&out_dir);
    }

    #[test]
    fn ollama_backend_error_body_yields_failure_not_panic() {
        use crate::ollama::OllamaModel;

        // A recorded OpenAI-compatible error body (non-2xx shape). The loop must record a
        // failure (no commands applied), never panic.
        let response = serde_json::json!({
            "error": { "type": "invalid_request_error", "message": "model not found" }
        })
        .to_string();

        let mut model = OllamaModel::for_test(None, "http://localhost:11434/v1", "missing:16k")
            .with_transport(Box::new(CannedTransport(response)));
        let out_dir = unique_dir("ollama-error");
        let outcome = run_agent_task(
            &drc_task(),
            &mut model,
            &CheckerRegistry::default(),
            "",
            "0.1.0",
            LoopOptions::default(),
            &out_dir,
            0,
            &crate::run::Provenance::new("ollama"),
            |m, session| m.set_document_context(document_summary(session)),
        )
        .expect("run");

        assert!(
            !outcome.record.success,
            "an error body must be recorded as a failure"
        );
        // The model surfaced a scrubbed error describing the API failure.
        assert!(
            model
                .last_error()
                .is_some_and(|e| e.contains("invalid_request_error")),
            "the API error should be surfaced on last_error"
        );
        cleanup(&out_dir);
    }

    #[test]
    fn unknown_checker_is_an_error() {
        let task = BenchTask {
            checker: "no_such".into(),
            ..drc_task()
        };
        let mut model = MockModel::new();
        let out_dir = unique_dir("unknown");
        let err = run_agent_task(
            &task,
            &mut model,
            &CheckerRegistry::default(),
            "",
            "0.1.0",
            LoopOptions::default(),
            &out_dir,
            0,
            &crate::run::Provenance::new("mock"),
            |_, _| {},
        )
        .expect_err("missing checker");
        assert!(matches!(err, super::LoopError::UnknownChecker { .. }));
        cleanup(&out_dir);
    }

    // ----- refinement-protocol convergence tests -----------------------------
    //
    // These prove the mid-session refinement loop: a user constraint that arrives
    // *between* iterations is folded into the model input (via Context::feedback),
    // the checker re-runs each iteration, and the session converges without a restart.
    // A conflicting constraint is bounded by max_iterations rather than spinning.

    use super::{NoRefinements, RefinementFn, RefinementSource, run_agent_task_refined};

    /// A checker that passes iff the target cell holds a met1 rectangle at least
    /// `min_width` wide. It stands in for a task requirement the *user* tightens
    /// mid-session ("make the wire wider"): the refinement raises the bar the checker
    /// enforces, so convergence is observable and deterministic.
    #[derive(Clone, Copy, Debug)]
    struct MinWidth {
        min_width: i64,
    }

    impl crate::run::Checker for MinWidth {
        fn check(
            &self,
            doc: &reticle_model::Document,
            _transcript: &reticle_agent_api::Transcript,
        ) -> crate::run::CheckResult {
            use reticle_model::ShapeKind;
            let widest = doc
                .cells()
                .flat_map(|cell| cell.shapes.iter())
                .filter(|s| s.layer.layer == 68 && s.layer.datatype == 20)
                .filter_map(|s| match &s.kind {
                    ShapeKind::Rect(r) => Some(r.width()),
                    _ => None,
                })
                .max()
                .unwrap_or(0);
            if widest >= self.min_width {
                crate::run::CheckResult::Pass
            } else {
                crate::run::CheckResult::Fail(vec![crate::run::CheckFailure::new(format!(
                    "widest met1 rect is {widest}, want >= {}",
                    self.min_width
                ))])
            }
        }
    }

    /// A registry whose `min_width` checker enforces `min_width`.
    fn min_width_registry(min_width: i64) -> CheckerRegistry {
        CheckerRegistry::default().with("min_width", Box::new(MinWidth { min_width }))
    }

    /// The min-width task fixture, checked by `min_width`.
    fn min_width_task() -> BenchTask {
        BenchTask {
            checker: "min_width".into(),
            ..drc_task()
        }
    }

    /// A deterministic model that reacts to a refinement constraint in the context.
    ///
    /// It looks for a `min_width=N` string anywhere in `context.feedback` (which is
    /// exactly where the loop folds user refinements) and, on the *next* iteration,
    /// replaces its rectangle with one `N` wide. Before any such constraint arrives it
    /// draws a default rectangle. This mirrors a real model that widens a wire when the
    /// user asks, and it proves the loop delivered the constraint without a restart.
    #[derive(Clone, Debug)]
    struct RefiningMock {
        /// Width used until a `min_width=` constraint is seen.
        default_width: i32,
        /// The rect element id to delete before drawing the wider one (`1` here, the
        /// first added shape), so successive proposals do not stack rectangles.
        rect_id: u64,
    }

    impl RefiningMock {
        fn new(default_width: i32) -> Self {
            Self {
                default_width,
                rect_id: 1,
            }
        }

        /// Parses the requested minimum width from any `min_width=N` token in feedback,
        /// taking the largest if several arrived.
        fn requested_width(context: &Context) -> Option<i32> {
            context
                .feedback
                .iter()
                .filter_map(|line| line.split("min_width=").nth(1))
                .filter_map(|rest| {
                    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                    digits.parse::<i32>().ok()
                })
                .max()
        }
    }

    impl ModelClient for RefiningMock {
        // The trait borrows the id from owned state; the mock's is a literal but must
        // keep the trait's `&str` return (mirrors `MockModel::id`).
        #[allow(clippy::unnecessary_literal_bound)]
        fn id(&self) -> &str {
            "refining-mock"
        }

        fn propose(
            &mut self,
            _task_id: &str,
            _prompt: &str,
            context: &Context,
        ) -> Vec<AgentCommand> {
            if context.iteration == 0 {
                // First proposal: create the cell and a default-width rect.
                return vec![
                    AgentCommand::CreateCell { name: "top".into() },
                    met1_rect(self.default_width),
                ];
            }
            match Self::requested_width(context) {
                // A refinement arrived: delete the old rect and draw the requested width.
                Some(width) => vec![
                    AgentCommand::DeleteShapes {
                        ids: vec![reticle_agent_api::ElementId(self.rect_id)],
                    },
                    met1_rect(width),
                ],
                // No constraint to act on and nothing already failing: propose nothing,
                // which the loop reads as "done".
                None => Vec::new(),
            }
        }
    }

    #[test]
    fn refinement_source_traits_drain_as_expected() {
        // NoRefinements yields nothing on every iteration.
        let mut none = NoRefinements;
        assert!(none.drain(0).is_empty());
        assert!(none.drain(7).is_empty());

        // RefinementFn forwards the iteration index and returns the closure's output.
        let mut seen = Vec::new();
        let mut src = RefinementFn(|iteration: u32| {
            if iteration == 1 {
                vec!["make the wire wider (min_width=800)".to_owned()]
            } else {
                Vec::new()
            }
        });
        for i in 0..3 {
            seen.push(src.drain(i));
        }
        assert!(seen[0].is_empty());
        assert_eq!(seen[1].len(), 1);
        assert!(seen[2].is_empty());
    }

    #[test]
    fn refinement_is_folded_in_and_loop_converges_without_restart() {
        // The base task only needs a >=140 rect; the model draws a 200-wide rect that
        // passes on iteration 0. Between iterations 0 and 1 the user tightens the
        // requirement to 800 ("make the wire wider"); the checker enforces >=800 from the
        // start, so iteration 0 fails the tightened bar and the injected constraint drives
        // iteration 1 to a 800-wide rect that passes. No restart: it is one continuous run.
        let mut model = RefiningMock::new(200);
        let out_dir = unique_dir("refine-converge");
        let outcome = run_agent_task_refined(
            &min_width_task(),
            &mut model,
            &min_width_registry(800),
            "",
            "0.1.0",
            LoopOptions::default(),
            &out_dir,
            0,
            &crate::run::Provenance::new("mock"),
            |_, _| {},
            // The refinement arrives before iteration 1, not at task start.
            RefinementFn(|iteration: u32| {
                if iteration == 1 {
                    vec!["user: make the wire wider (min_width=800)".to_owned()]
                } else {
                    Vec::new()
                }
            }),
        )
        .expect("run");

        assert!(
            outcome.record.success,
            "the loop must converge once the widen refinement is applied"
        );
        assert_eq!(
            outcome.record.iterations, 2,
            "iteration 0 (too narrow for the tightened bar) then 1 (widened) converges"
        );
        // The final layout actually carries an 800-wide met1 rect: the constraint was
        // satisfied, not just declared.
        let widest = widest_met1_width(&min_width_task(), &out_dir);
        assert_eq!(
            widest, 800,
            "the widened rect is present in the final gds path"
        );
        cleanup(&out_dir);
    }

    #[test]
    fn refinement_injected_at_start_converges_on_first_iteration() {
        // A constraint present from iteration 0 (the source yields it immediately) is
        // folded into the very first proposal's context. The model, seeing min_width=600
        // on iteration 0, would still draw its default first; the checker fails; the
        // accumulated constraint then drives iteration 1. This proves the constraint
        // persists across iterations (it conditions iteration 1 though it arrived at 0).
        let mut model = RefiningMock::new(200);
        let out_dir = unique_dir("refine-start");
        let outcome = run_agent_task_refined(
            &min_width_task(),
            &mut model,
            &min_width_registry(600),
            "",
            "0.1.0",
            LoopOptions::default(),
            &out_dir,
            0,
            &crate::run::Provenance::new("mock"),
            |_, _| {},
            RefinementFn(|iteration: u32| {
                if iteration == 0 {
                    vec!["min_width=600".to_owned()]
                } else {
                    Vec::new()
                }
            }),
        )
        .expect("run");
        assert!(
            outcome.record.success,
            "converges with the persisted constraint"
        );
        assert_eq!(widest_met1_width(&min_width_task(), &out_dir), 600);
        cleanup(&out_dir);
    }

    #[test]
    fn conflicting_refinement_is_bounded_by_max_iterations_not_infinite() {
        // The user demands a width the model can satisfy, but the checker is set one dbu
        // higher, so no proposal the model makes can ever pass: the constraint conflicts
        // with what is achievable. The loop must terminate at max_iterations with a
        // recorded failure, never spin forever.
        let mut model = RefiningMock::new(200);
        let out_dir = unique_dir("refine-conflict");
        let outcome = run_agent_task_refined(
            &min_width_task(),
            &mut model,
            // Checker wants >= 1001, but the refinement only ever asks for (and the model
            // only ever draws) 1000: an unsatisfiable pairing.
            &min_width_registry(1001),
            "",
            "0.1.0",
            LoopOptions {
                max_iterations: 4,
                command_budget: 256,
            },
            &out_dir,
            0,
            &crate::run::Provenance::new("mock"),
            |_, _| {},
            // A fresh conflicting constraint every iteration, so the model keeps trying
            // (never proposes an empty batch) yet can never satisfy the checker.
            RefinementFn(|_iteration: u32| vec!["min_width=1000".to_owned()]),
        )
        .expect("run");

        assert!(
            !outcome.record.success,
            "an unsatisfiable refinement must be recorded as a failure"
        );
        assert_eq!(
            outcome.record.iterations, 4,
            "the loop is bounded by max_iterations, it does not loop forever"
        );
        cleanup(&out_dir);
    }

    #[test]
    fn no_refinements_matches_the_unrefined_entry_point() {
        // run_agent_task_refined with NoRefinements is exactly run_agent_task: a clean
        // first proposal passes on iteration 0.
        let create = AgentCommand::CreateCell { name: "top".into() };
        let mut model = MockModel::new().with_script("t1_drc", vec![vec![create, met1_rect(500)]]);
        let out_dir = unique_dir("no-refine");
        let outcome = run_agent_task_refined(
            &drc_task(),
            &mut model,
            &CheckerRegistry::default(),
            "",
            "0.1.0",
            LoopOptions::default(),
            &out_dir,
            0,
            &crate::run::Provenance::new("mock"),
            |_, _| {},
            NoRefinements,
        )
        .expect("run");
        assert!(outcome.record.success);
        assert_eq!(outcome.record.iterations, 1);
        cleanup(&out_dir);
    }

    /// Reads back the widest met1 rect width from the exported gds path's document by
    /// replaying the written transcript, so a test can assert the *final* geometry, not
    /// just the record. Returns `0` if there is no met1 rect.
    fn widest_met1_width(task: &BenchTask, out_dir: &std::path::Path) -> i64 {
        use reticle_model::ShapeKind;
        // Reconstruct the document from the JSONL transcript's command records.
        let path = out_dir.join(format!("{}.transcript.jsonl", task.id));
        let text = std::fs::read_to_string(&path).expect("read transcript");
        let mut session = Session::new();
        for line in text.lines() {
            let value: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(command) = value.get("command")
                && let Ok(cmd) = serde_json::from_value::<AgentCommand>(command.clone())
            {
                let _ = session.apply(cmd);
            }
        }
        session
            .document()
            .cells()
            .flat_map(|cell| cell.shapes.iter())
            .filter(|s| s.layer.layer == 68 && s.layer.datatype == 20)
            .filter_map(|s| match &s.kind {
                ShapeKind::Rect(r) => Some(r.width()),
                _ => None,
            })
            .max()
            .unwrap_or(0)
    }

    /// A unique scratch directory for one test, keyed by name and process id.
    fn unique_dir(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("reticle-agent-run-{tag}-{}", std::process::id()))
    }

    fn cleanup(dir: &std::path::Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    // Keep the imports honest across cfg variants.
    #[allow(dead_code)]
    fn _uses(_: &Context, _: &dyn ModelClient) {}
}
