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
    mut context_hook: impl FnMut(&mut M, &Session),
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
fn drive<M: ModelClient>(
    task: &BenchTask,
    model: &mut M,
    checker: &dyn Checker,
    session: &mut Session,
    options: LoopOptions,
    context_hook: &mut impl FnMut(&mut M, &Session),
) -> LoopResult {
    let mut iterations = 0_u32;
    let mut first_proposal_violations = 0_u32;
    let mut final_violations = 0_u32;
    let mut success = false;
    let mut feedback: Vec<String> = Vec::new();
    let mut prev_violations = 0_u32;
    let mut commands_applied = 0_u32;

    while iterations < options.max_iterations {
        // Give the model the current layout before it proposes.
        context_hook(model, session);

        let context = Context {
            iteration: iterations,
            prev_violations,
            feedback: feedback.clone(),
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
        return "(empty document — no cells yet)".to_owned();
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
            |_, _| {},
        )
        .expect_err("missing checker");
        assert!(matches!(err, super::LoopError::UnknownChecker { .. }));
        cleanup(&out_dir);
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
