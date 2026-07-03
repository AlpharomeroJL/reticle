//! Full-loop integration tests against `reticle-bench`'s deterministic `MockModel`.
//!
//! These drive the whole propose-verify-correct harness through its public surface,
//! [`run_agent_task`] plus [`MockModel`], with no live API (the tests never touch the
//! network and `ANTHROPIC_API_KEY` is irrelevant here). They prove:
//!
//! - the loop converges on a mock task and writes the four artifacts;
//! - a propose-verify-correct cycle (a first dirty proposal, then a correction) reaches
//!   a clean layout;
//! - a mock task that never corrects is recorded as a failure, not retro-edited;
//! - the written transcript is well-formed JSONL and carries no injected secret.
//!
//! The API-key-never-in-any-artifact guarantee is proven end to end against the real
//! [`AnthropicModel`] transport in the crate's in-tree tests (see
//! `api_key_never_appears_in_any_written_artifact` in `src/run.rs`); here we assert the
//! transcript contract that makes that guarantee structural: it holds only command
//! records, never free text a caller passes in.

use std::path::Path;

use reticle_agent::{
    LoopOptions, Provenance, RefinementFn, run_agent_task, run_agent_task_refined,
};
use reticle_agent_api::AgentCommand;
use reticle_agent_api::args::{LayerArg, PointArg, RectArg};
use reticle_bench::model::{Context, MockModel, ModelClient};
use reticle_bench::{BenchTask, CheckerRegistry, ResultRecord, Tier};

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

/// The tier-1 "clean met1 rectangle" DRC task.
fn drc_task() -> BenchTask {
    BenchTask {
        id: "t1_drc".into(),
        tier: Tier(1),
        prompt: "Create a cell named top and place a met1 rectangle that passes DRC.".into(),
        technology: "sky130.tech".into(),
        checker: "drc_clean".into(),
        intent: None,
        refinement: None,
    }
}

/// A unique scratch directory for a test, keyed by tag and process id.
fn out_dir(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("reticle-agent-it-{tag}-{}", std::process::id()))
}

fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

/// Reads the result artifact back as the one-element record array the writer produces.
fn read_result(path: &Path) -> ResultRecord {
    let text = std::fs::read_to_string(path).expect("read result");
    let mut records: Vec<ResultRecord> = serde_json::from_str(&text).expect("parse result");
    assert_eq!(records.len(), 1, "result artifact holds exactly one record");
    records.pop().unwrap()
}

#[test]
fn loop_converges_and_writes_four_artifacts() {
    // One clean proposal: create the cell and a well-sized met1 rect. Passes on the
    // first try.
    let mut model = MockModel::new().with_script(
        "t1_drc",
        vec![vec![
            AgentCommand::CreateCell { name: "top".into() },
            met1_rect(500),
        ]],
    );
    let dir = out_dir("converge");
    let outcome = run_agent_task(
        &drc_task(),
        &mut model,
        &CheckerRegistry::default(),
        "",
        "0.1.0",
        LoopOptions::default(),
        &dir,
        0,
        &Provenance::new("mock"),
        |_, _| {},
    )
    .expect("run");

    assert!(outcome.record.success, "clean layout must pass drc_clean");
    // All four artifacts exist (the PNG is best-effort; its path is None on a headless
    // machine with no GPU, which is not a failure).
    assert!(outcome.artifacts.transcript.exists(), "transcript written");
    assert!(outcome.artifacts.gds.exists(), "gds written");
    assert!(outcome.artifacts.result.exists(), "result written");
    // The GDS is non-empty for a document that has geometry.
    let gds = std::fs::read(&outcome.artifacts.gds).expect("read gds");
    assert!(!gds.is_empty(), "exported gds should carry bytes");
    // The result on disk agrees with the returned record.
    assert!(read_result(&outcome.artifacts.result).success);
    cleanup(&dir);
}

#[test]
fn propose_verify_correct_cycle_reaches_clean() {
    // First proposal is deliberately under-width (violates min width/area); the second
    // deletes it and adds a clean rect. The run must show a dirty first proposal that
    // falls to zero.
    let mut model = MockModel::new().with_script(
        "t1_drc",
        vec![
            vec![
                AgentCommand::CreateCell { name: "top".into() },
                met1_rect(100),
            ],
            vec![
                AgentCommand::DeleteShapes {
                    ids: vec![reticle_agent_api::ElementId(1)],
                },
                met1_rect(500),
            ],
        ],
    );
    let dir = out_dir("cycle");
    let outcome = run_agent_task(
        &drc_task(),
        &mut model,
        &CheckerRegistry::default(),
        "",
        "0.1.0",
        LoopOptions::default(),
        &dir,
        0,
        &Provenance::new("mock"),
        |_, _| {},
    )
    .expect("run");

    assert!(
        outcome.record.success,
        "correction should reach a clean layout"
    );
    assert!(
        outcome.record.first_proposal_violations > 0,
        "the first proposal was deliberately dirty"
    );
    assert_eq!(outcome.record.final_violations, 0, "final layout is clean");
    assert_eq!(outcome.record.iterations, 2, "one correction was needed");
    cleanup(&dir);
}

#[test]
fn failing_mock_task_is_recorded_as_failure() {
    // The model only ever proposes the under-width rect, so the checker never passes.
    let mut model = MockModel::new().with_script(
        "t1_drc",
        vec![vec![
            AgentCommand::CreateCell { name: "top".into() },
            met1_rect(100),
        ]],
    );
    let dir = out_dir("fail");
    let outcome = run_agent_task(
        &drc_task(),
        &mut model,
        &CheckerRegistry::default(),
        "",
        "0.1.0",
        LoopOptions::default(),
        &dir,
        0,
        &Provenance::new("mock"),
        |_, _| {},
    )
    .expect("run");

    assert!(
        !outcome.record.success,
        "an uncorrected violation must fail"
    );
    assert!(
        outcome.record.final_violations > 0,
        "violations still stand"
    );
    // The written record is a failure too: no retro-edit to a pass.
    assert!(!read_result(&outcome.artifacts.result).success);
    cleanup(&dir);
}

#[test]
fn transcript_is_jsonl_and_carries_no_injected_secret() {
    // Drive a run whose task prompt embeds a sentinel "secret" and whose commands never
    // contain it. The transcript records only commands and outcomes, so the sentinel
    // must not appear anywhere in the written transcript, the structural property that
    // keeps an API key out of it.
    const SENTINEL: &str = "sk-ant-SENTINEL-MUST-NOT-APPEAR";
    let mut task = drc_task();
    task.prompt = format!("Draw a clean met1 rect. (context marker {SENTINEL})");

    let mut model = MockModel::new().with_script(
        "t1_drc",
        vec![vec![
            AgentCommand::CreateCell { name: "top".into() },
            met1_rect(500),
        ]],
    );
    let dir = out_dir("secret");
    let outcome = run_agent_task(
        &task,
        &mut model,
        &CheckerRegistry::default(),
        "",
        "0.1.0",
        LoopOptions::default(),
        &dir,
        0,
        &Provenance::new("mock"),
        // Even feeding the "secret" into the model as document context must not leak it
        // to the transcript: the hook drives the model, not the transcript.
        |_, _| {},
    )
    .expect("run");

    let transcript = std::fs::read_to_string(&outcome.artifacts.transcript).expect("read");
    assert!(
        !transcript.contains(SENTINEL),
        "transcript must not carry the prompt/context sentinel"
    );
    // Every non-blank line parses as JSON (command records, then a final_hash trailer).
    let mut command_lines = 0;
    for line in transcript.lines().filter(|l| !l.trim().is_empty()) {
        let value: serde_json::Value =
            serde_json::from_str(line).expect("each transcript line is valid JSON");
        if value.get("command").is_some() {
            command_lines += 1;
        }
    }
    assert!(
        command_lines >= 2,
        "at least the create_cell + add_rect records"
    );
    cleanup(&dir);
}

/// The refinement task fixture: mirrors the committed `t4_refine_add_shape_met1` task.
/// The initial prompt asks for one met1 rectangle; the scripted `refinement` asks for
/// a second, and the `shape_count` checker enforces the post-refinement bar (>= 2).
fn refine_add_shape_task() -> BenchTask {
    BenchTask {
        id: "t4_refine_add_shape_met1".into(),
        tier: Tier(4),
        prompt: "Create a cell named top and place one met1 rectangle.".into(),
        technology: "sky130.tech".into(),
        checker: "shape_count:layer=68/20,min=2".into(),
        intent: None,
        refinement: Some(
            "Refinement: add a second, separate met1 rectangle so the cell holds at \
             least two met1 shapes."
                .into(),
        ),
    }
}

/// A mock that places one met1 rect on iteration 0, then places a second rect once it
/// sees an "add a second" refinement folded into its feedback. This mirrors a real
/// model reacting to the scripted follow-up constraint, and proves the loop delivered
/// the `BenchTask.refinement` through the refinement seam without a restart.
#[derive(Clone, Debug, Default)]
struct AddSecondShapeMock;

impl ModelClient for AddSecondShapeMock {
    #[allow(clippy::unnecessary_literal_bound)]
    fn id(&self) -> &str {
        "add-second-shape-mock"
    }

    fn propose(&mut self, _task_id: &str, _prompt: &str, context: &Context) -> Vec<AgentCommand> {
        if context.iteration == 0 {
            // Initial proposal: the cell and the first met1 rect.
            return vec![
                AgentCommand::CreateCell { name: "top".into() },
                met1_rect(300),
            ];
        }
        // The scripted refinement asks for a second shape; add one clear of the first.
        let asked_for_second = context
            .feedback
            .iter()
            .any(|line| line.contains("second") && line.contains("met1"));
        if asked_for_second {
            vec![AgentCommand::AddRect {
                cell: "top".into(),
                layer: LayerArg {
                    layer: 68,
                    datatype: 20,
                },
                rect: RectArg {
                    min: PointArg { x: 500, y: 0 },
                    max: PointArg { x: 800, y: 300 },
                },
            }]
        } else {
            Vec::new()
        }
    }
}

#[test]
fn benchtask_refinement_is_folded_in_and_converges() {
    // Drive the committed-style refinement task exactly as the CLI's `drive_one` does:
    // the scripted `refinement` string is yielded on iteration 1 (after the first
    // proposal) through a `RefinementFn`, and folded into the model's feedback. The
    // model then adds the second shape and the `shape_count` checker passes, all in one
    // continuous run (no restart).
    let task = refine_add_shape_task();
    let refinement = task
        .refinement
        .clone()
        .expect("fixture carries a refinement");
    let registry = CheckerRegistry::for_task(&task).expect("checker compiles");
    let mut model = AddSecondShapeMock;
    let dir = out_dir("refine-benchtask");
    let outcome = run_agent_task_refined(
        &task,
        &mut model,
        &registry,
        "",
        "0.4.0",
        LoopOptions::default(),
        &dir,
        0,
        &Provenance::new("mock"),
        |_, _| {},
        RefinementFn(move |iteration: u32| {
            if iteration == 1 {
                vec![refinement.clone()]
            } else {
                Vec::new()
            }
        }),
    )
    .expect("run");

    assert!(
        outcome.record.success,
        "the loop must converge once the second-shape refinement is applied"
    );
    assert_eq!(
        outcome.record.iterations, 2,
        "iteration 0 (one shape, fails min=2) then 1 (second shape) converges"
    );
    cleanup(&dir);
}
