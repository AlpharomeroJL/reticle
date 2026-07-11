//! Solvability, grading, and determinism proof for the v0.6.0 coverage tasks.
//!
//! `suite.rs` proves the committed suite loads and every checker dispatches; the
//! `wave3_tasks.rs` / `tier5_solvability.rs` tests prove earlier groups satisfiable with
//! hand-built documents. This test covers the five v0.6.0 tasks a third way, the way the
//! brief for this group asks for: each task is driven through the real
//! [`run_task`](reticle_bench::run_task) propose-verify-correct loop by a deterministic
//! [`MockModel`] scripted solution, and
//!
//! - the solving script grades as a PASS (the task is solvable and graded), while an
//!   empty model grades as a FAIL (the checker is not vacuous), and
//! - running the whole five-task sub-suite twice yields byte-identical result records and
//!   a byte-identical rendered leaderboard, so nothing (a wall clock, a hash-map order)
//!   leaks nondeterminism into the frozen output.
//!
//! The five tasks are chosen to reach checker code paths no earlier task exercised:
//! a `shape_count` count-range (min and max), a `layer_area` area-window (min and max), a
//! `contact_stack` on the via2 met2/met3 stack with an enclosure floor, a `boolean_result`
//! pinned to the XOR area (union/intersection/difference were already covered), and a
//! six-via via1 chain. Every scripted solution uses only `create_cell`/`add_rect`, so the
//! geometry it produces is the same known-good geometry the two-way tests draw.

use reticle_agent_api::AgentCommand;
use reticle_agent_api::args::{LayerArg, PointArg, RectArg};
use reticle_bench::{
    BenchTask, CheckerRegistry, MockModel, ResultRecord, RunOptions, load_suite,
    render_leaderboard, run_task,
};
use std::path::PathBuf;

/// The five v0.6.0 task ids, in manifest order.
const NEW_TASK_IDS: [&str; 5] = [
    "t2_shape_count_range_li1",
    "t2_layer_area_window_met2",
    "t3_contact_stack_via2_enc",
    "t3_bool_xor_met2",
    "t3_via_chain_via1_6",
];

/// The committed suite directory, relative to this crate's manifest.
fn suite_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("benchmarks")
        .join("layout-tasks")
}

/// Loads the named task from the committed suite.
fn task(id: &str) -> BenchTask {
    let (_manifest, tasks) =
        load_suite(&suite_dir()).unwrap_or_else(|e| panic!("suite loads: {e}"));
    tasks
        .into_iter()
        .find(|t| t.id == id)
        .unwrap_or_else(|| panic!("task `{id}` is in the suite"))
}

/// A `create_cell top` command; every scripted solution builds its geometry in `top`.
fn create_top() -> AgentCommand {
    AgentCommand::CreateCell { name: "top".into() }
}

/// An axis-aligned rectangle on `layer`/`datatype` in cell `top`.
fn rect(layer: u16, datatype: u16, x0: i32, y0: i32, x1: i32, y1: i32) -> AgentCommand {
    AgentCommand::AddRect {
        cell: "top".into(),
        layer: LayerArg { layer, datatype },
        rect: RectArg {
            min: PointArg { x: x0, y: y0 },
            max: PointArg { x: x1, y: y1 },
        },
    }
}

/// A `MockModel` that scripts a one-attempt solving sequence for every v0.6.0 task.
///
/// Each script is the geometry a correct answer draws, so the checker the runner compiles
/// for the task passes on the first proposal. The scripts are pure `create_cell`/`add_rect`
/// (no boolean or via-stack engine dependency); the geometry mirrors the known-good
/// documents the two-way solvability tests use.
fn solving_mock() -> MockModel {
    MockModel::new()
        // shape_count:layer=67/20,min=2,max=4 -> three disjoint li1 rects (count 3 in [2,4]).
        .with_script(
            "t2_shape_count_range_li1",
            vec![vec![
                create_top(),
                rect(67, 20, 0, 0, 200, 200),
                rect(67, 20, 300, 0, 500, 200),
                rect(67, 20, 600, 0, 800, 200),
            ]],
        )
        // layer_area:layer=69/20,min_area=100000,max_area=300000 -> one 400x400 met2 rect
        // (area 160000, inside the window).
        .with_script(
            "t2_layer_area_window_met2",
            vec![vec![create_top(), rect(69, 20, 0, 0, 400, 400)]],
        )
        // contact_stack:via=69/44,min_enclosure=40 -> met2 and met3 pads with a via2 cut
        // enclosed by 55 dbu on every side, bridging met2 up to met3.
        .with_script(
            "t3_contact_stack_via2_enc",
            vec![vec![
                create_top(),
                rect(69, 20, 0, 0, 300, 300),
                rect(70, 20, 0, 0, 300, 300),
                rect(69, 44, 55, 55, 245, 245),
            ]],
        )
        // boolean_result:layer=69/20,min_area=119000,max_area=121000,cleared=68/20 -> the two
        // 200x300 XOR flanks on met2 (total 120000), with met1 left empty.
        .with_script(
            "t3_bool_xor_met2",
            vec![vec![
                create_top(),
                rect(69, 20, 0, 0, 200, 300),
                rect(69, 20, 300, 0, 500, 300),
            ]],
        )
        // via_chain:via=68/44,vias=6 -> a met1 strap and a met2 strap stitched by six via1
        // cuts, all overlapping both straps so they share one net.
        .with_script(
            "t3_via_chain_via1_6",
            vec![vec![
                create_top(),
                rect(68, 20, 0, 0, 2000, 300),
                rect(69, 20, 0, 0, 2000, 300),
                rect(68, 44, 100, 75, 250, 225),
                rect(68, 44, 400, 75, 550, 225),
                rect(68, 44, 700, 75, 850, 225),
                rect(68, 44, 1000, 75, 1150, 225),
                rect(68, 44, 1300, 75, 1450, 225),
                rect(68, 44, 1600, 75, 1750, 225),
            ]],
        )
}

/// Runs one committed task against `model`, compiling its checker exactly as the runner
/// does, and returns the result record. Technology is left at the session default (empty
/// source): the v0.6.0 checkers read layers, areas, and the baked SKY130 connection rules,
/// none of which need a technology installed.
fn run_one(id: &str, model: &mut MockModel) -> ResultRecord {
    let t = task(id);
    let registry = CheckerRegistry::for_task(&t)
        .unwrap_or_else(|e| panic!("task `{id}` checker compiles: {e}"));
    run_task(&t, model, &registry, "", "0.6.0", RunOptions::default())
        .unwrap_or_else(|e| panic!("task `{id}` runs: {e}"))
}

/// Runs the whole five-task sub-suite against a fresh solving mock, in manifest order.
fn run_new_suite() -> Vec<ResultRecord> {
    let mut model = solving_mock();
    NEW_TASK_IDS
        .iter()
        .map(|id| run_one(id, &mut model))
        .collect()
}

#[test]
fn every_new_task_is_solvable_and_graded_pass() {
    let mut model = solving_mock();
    for id in NEW_TASK_IDS {
        let record = run_one(id, &mut model);
        assert!(
            record.success,
            "`{id}` must grade PASS under its scripted solution, got {record:?}"
        );
        assert_eq!(record.model, "mock", "`{id}` records the mock model id");
        assert_eq!(
            record.suite_version, "0.6.0",
            "`{id}` records the suite version"
        );
        // These tasks are graded by geometric/connectivity checkers, not `drc_clean`, so
        // the record's DRC violation count is incidental and deliberately not asserted.
    }
}

#[test]
fn every_new_task_checker_is_not_vacuous() {
    // An empty model proposes nothing, so the document stays empty and the checker must
    // reject it: the task is graded, not a free pass.
    let mut empty = MockModel::new();
    for id in NEW_TASK_IDS {
        let record = run_one(id, &mut empty);
        assert!(
            !record.success,
            "`{id}` must grade FAIL for an empty solution (the checker is not vacuous)"
        );
    }
}

#[test]
fn result_records_are_byte_identical_across_two_runs() {
    let first = run_new_suite();
    let second = run_new_suite();
    assert_eq!(first.len(), NEW_TASK_IDS.len(), "one record per new task");
    assert!(first.iter().all(|r| r.success), "every new task solved");

    // The frozen output is the serialized record set. Two runs executed at different wall
    // times must serialize to the same bytes: a leaked clock or a hash-map iteration order
    // would show up here as a diff.
    let json_first = serde_json::to_string_pretty(&first).expect("serialize run 1");
    let json_second = serde_json::to_string_pretty(&second).expect("serialize run 2");
    assert_eq!(
        json_first, json_second,
        "the same suite and MockModel must yield byte-identical result records twice"
    );
    // wall_ms is a monotonic step count, not a clock, so no year-like timestamp appears.
    assert!(
        !json_first.contains("202"),
        "no wall-clock timestamp leaked into the frozen records"
    );
}

#[test]
fn leaderboard_render_from_new_records_is_byte_identical_across_two_runs() {
    // Render the leaderboard from records produced by two independent runs of the suite.
    // The records carry `t2_`/`t3_` tier prefixes, so every row aggregates; the render is a
    // pure function of the record set, so the two pages must be byte-for-byte equal.
    let first = render_leaderboard(&run_new_suite());
    let second = render_leaderboard(&run_new_suite());
    assert_eq!(
        first, second,
        "the leaderboard rendered from a fresh suite run must be byte-stable"
    );
    assert!(
        first.contains("# Leaderboard"),
        "a leaderboard page was rendered"
    );
    assert!(
        !first.contains("2025") && !first.contains("2026"),
        "no timestamp in the rendered leaderboard"
    );
}
