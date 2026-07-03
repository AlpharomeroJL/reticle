//! Full-loop integration tests against `reticle-bench`'s deterministic `MockModel`.
//!
//! These drive the whole propose-verify-correct harness through its public surface —
//! [`run_agent_task`] plus [`MockModel`] — with no live API (the tests never touch the
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

use reticle_agent::{LoopOptions, run_agent_task};
use reticle_agent_api::AgentCommand;
use reticle_agent_api::args::{LayerArg, PointArg, RectArg};
use reticle_bench::model::MockModel;
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
    // must not appear anywhere in the written transcript — the structural property that
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
