//! Solvability, grading, and determinism proof for the v0.7.0 Phase-3 depth tasks.
//!
//! `suite.rs` proves the committed suite loads and every checker dispatches;
//! `new_tasks_freeze.rs` proved the v0.6.0 coverage group the same way this file
//! proves the v0.7.0 group: each task is driven through the real
//! [`run_task`](reticle_bench::run_task) propose-verify-correct loop by a
//! deterministic [`MockModel`] scripted solution, and
//!
//! - the solving script grades as a PASS (the task is solvable and graded), while an
//!   empty model grades as a FAIL (the checker is not vacuous), and
//! - running the whole seven-task sub-suite twice yields byte-identical result
//!   records and a byte-identical rendered leaderboard, so nothing (a wall clock, a
//!   hash-map order) leaks nondeterminism into the frozen output.
//!
//! The seven tasks exercise three new checker families added for Phase-3 depth
//! (`crates/reticle-bench/src/net_checkers.rs` and
//! `crates/reticle-bench/src/pcell_checkers.rs`), plus two tasks that reuse existing
//! checkers but require a genuinely multi-step scripted solution:
//!
//! - **Net-trace** (3): `net_trace_connected`/`net_trace_extent`/`net_trace_isolated`,
//!   solved with plain `create_cell`/`add_rect`, one attempt each.
//! - **PCell params** (2): `pcell_box`, one task leaving `margin` to the
//!   `bench.box_pad` PCell's schema default and one overriding it, one attempt each.
//! - **Multi-step edits** (2): `t4_multistep_grow_enclosure` scripts three
//!   iterations that grow a via's enclosing conductors with two successive
//!   `offset_shapes` edits (not a delete-and-redraw); `t4_multistep_reposition_via`
//!   scripts two iterations where the correction is a `transform_shapes` move of a
//!   stranded via into place. Both edit existing shape ids in place rather than
//!   deleting and re-adding, the pattern every earlier correction script used.

use reticle_agent_api::args::{LayerArg, OrientationArg, PointArg, RectArg, TransformArg};
use reticle_agent_api::{AgentCommand, ElementId};
use reticle_bench::{
    BenchTask, CheckerRegistry, MockModel, ResultRecord, RunOptions, load_suite,
    render_leaderboard, run_task,
};
use std::path::PathBuf;

/// The seven v0.7.0 task ids, in manifest order.
const NEW_TASK_IDS: [&str; 7] = [
    "t3_net_trace_connected_met1",
    "t3_net_trace_extent_met2",
    "t3_net_trace_isolated_met1",
    "t3_pcell_box_pad_default_margin",
    "t3_pcell_box_pad_custom_margin",
    "t4_multistep_grow_enclosure",
    "t4_multistep_reposition_via",
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

/// Grows (or shrinks, for a negative `delta`) the shapes named by `ids` in place,
/// keeping their element ids (`reticle-agent-api`'s `offset_shapes` dispatch: the
/// first result polygon always inherits the input shape's id).
fn offset(ids: &[u64], delta: i32) -> AgentCommand {
    AgentCommand::OffsetShapes {
        ids: ids.iter().map(|&n| ElementId(n)).collect(),
        delta,
    }
}

/// Translates the shape named by `id` by `(dx, dy)`, keeping its element id.
fn translate(id: u64, dx: i32, dy: i32) -> AgentCommand {
    AgentCommand::TransformShapes {
        ids: vec![ElementId(id)],
        transform: TransformArg {
            orientation: OrientationArg::R0,
            mag_num: 1,
            mag_den: 1,
            dx,
            dy,
        },
    }
}

/// A `MockModel` that scripts a solving sequence for every v0.7.0 task.
///
/// The net-trace and PCell-params tasks solve in one attempt, pure
/// `create_cell`/`add_rect`. The two multi-step tasks solve over several iterations
/// through edits to shapes placed in the first attempt, proving the runner's
/// propose-verify-correct loop actually converges rather than the checker having
/// simply been satisfied on the first try every time.
fn solving_mock() -> MockModel {
    MockModel::new()
        // net_trace_connected:a=50/100,b=950/100,min_shapes=2 -> two overlapping met1
        // rects (0,0)-(600,200) and (500,0)-(1000,200); probe (50,100) lands on the
        // first, probe (950,100) on the second, and they share one net of 2 shapes.
        .with_script(
            "t3_net_trace_connected_met1",
            vec![vec![
                create_top(),
                rect(68, 20, 0, 0, 600, 200),
                rect(68, 20, 500, 0, 1000, 200),
            ]],
        )
        // net_trace_extent:probe=50/100,min_width=1500,min_height=200 -> two
        // overlapping met2 rects merging into one net spanning x:[0,1600] y:[0,200].
        .with_script(
            "t3_net_trace_extent_met2",
            vec![vec![
                create_top(),
                rect(69, 20, 0, 0, 800, 200),
                rect(69, 20, 700, 0, 1600, 200),
            ]],
        )
        // net_trace_isolated:a=50/50,b=950/50 -> two disjoint met1 pads, an 800 dbu
        // gap apart, so the probes land on two distinct nets.
        .with_script(
            "t3_net_trace_isolated_met1",
            vec![vec![
                create_top(),
                rect(68, 20, 0, 0, 100, 100),
                rect(68, 20, 900, 0, 1000, 100),
            ]],
        )
        // pcell_box:outer=68/20,inner=67/20,width=400 (margin left to the schema
        // default, 20) -> outer 400x400 square at the origin, inner square inset 20
        // dbu on every side: (20,20)-(380,380).
        .with_script(
            "t3_pcell_box_pad_default_margin",
            vec![vec![
                create_top(),
                rect(68, 20, 0, 0, 400, 400),
                rect(67, 20, 20, 20, 380, 380),
            ]],
        )
        // pcell_box:outer=69/20,inner=68/20,width=600,margin=50 (explicit override)
        // -> outer 600x600 square, inner square inset 50 dbu: (50,50)-(550,550).
        .with_script(
            "t3_pcell_box_pad_custom_margin",
            vec![vec![
                create_top(),
                rect(69, 20, 0, 0, 600, 600),
                rect(68, 20, 50, 50, 550, 550),
            ]],
        )
        // contact_stack:via=68/44,min_enclosure=60 -> a via1 cut centered in small
        // met1/met2 pads (enclosure 20, well short of 60), corrected over two
        // `offset_shapes` grows of 20 dbu each (enclosure 40, then 60) rather than a
        // delete-and-redraw. Converges on the third iteration.
        .with_script(
            "t4_multistep_grow_enclosure",
            vec![
                vec![
                    create_top(),
                    rect(68, 20, 60, 60, 140, 140), // met1 pad -> id 1
                    rect(69, 20, 60, 60, 140, 140), // met2 pad -> id 2
                    rect(68, 44, 80, 80, 120, 120), // via1 cut -> id 3
                ],
                vec![offset(&[1, 2], 20)], // enclosure 20 -> 40, still short of 60
                vec![offset(&[1, 2], 20)], // enclosure 40 -> 60, now meets the floor
            ],
        )
        // via_chain:via=68/44,vias=4 -> a met1/met2 strap pair with three vias
        // landing on both plus a fourth via stranded far away (only 3 of 4 vias on
        // the continuous net); corrected by `transform_shapes`-moving the stranded
        // via into the strap footprint rather than deleting and redrawing it.
        .with_script(
            "t4_multistep_reposition_via",
            vec![
                vec![
                    create_top(),
                    rect(68, 20, 0, 0, 1000, 300), // met1 strap -> id 1
                    rect(69, 20, 0, 0, 1000, 300), // met2 strap -> id 2
                    rect(68, 44, 100, 75, 250, 225), // via -> id 3
                    rect(68, 44, 400, 75, 550, 225), // via -> id 4
                    rect(68, 44, 700, 75, 850, 225), // via -> id 5
                    rect(68, 44, 100, 2000, 250, 2150), // stranded via -> id 6
                ],
                // Move id 6 from (100,2000)-(250,2150) to (850,75)-(1000,225), inside
                // both strap footprints, joining the continuous net.
                vec![translate(6, 750, -1925)],
            ],
        )
}

/// Runs one committed task against `model`, compiling its checker exactly as the
/// runner does, and returns the result record. Technology is left at the session
/// default (empty source): the v0.7.0 checkers read layers, connectivity, and the
/// PCell's own schema, none of which need a technology installed.
fn run_one(id: &str, model: &mut MockModel) -> ResultRecord {
    let t = task(id);
    let registry = CheckerRegistry::for_task(&t)
        .unwrap_or_else(|e| panic!("task `{id}` checker compiles: {e}"));
    run_task(&t, model, &registry, "", "0.7.0", RunOptions::default())
        .unwrap_or_else(|e| panic!("task `{id}` runs: {e}"))
}

/// Runs the whole seven-task sub-suite against a fresh solving mock, in manifest
/// order.
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
            record.suite_version, "0.7.0",
            "`{id}` records the suite version"
        );
    }
}

#[test]
fn multistep_tasks_actually_use_more_than_one_iteration() {
    // The whole point of the two multi-step tasks is that they are not solved on the
    // first proposal; this asserts the runner really drove the loop rather than the
    // checker happening to pass early despite the multi-turn script.
    let mut model = solving_mock();
    let enclosure = run_one("t4_multistep_grow_enclosure", &mut model);
    assert_eq!(
        enclosure.iterations, 3,
        "the enclosure fix converges on the third offset_shapes edit"
    );
    let via = run_one("t4_multistep_reposition_via", &mut model);
    assert_eq!(
        via.iterations, 2,
        "the stranded via is repositioned on the second iteration"
    );
}

#[test]
fn every_new_task_checker_is_not_vacuous() {
    // An empty model proposes nothing, so the document stays empty and the checker
    // must reject it: the task is graded, not a free pass.
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

    // The frozen output is the serialized record set. Two runs executed at different
    // wall times must serialize to the same bytes: a leaked clock or a hash-map
    // iteration order would show up here as a diff.
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
    // Render the leaderboard from records produced by two independent runs of the
    // suite. The records carry `t3_`/`t4_` tier prefixes, so every row aggregates;
    // the render is a pure function of the record set, so the two pages must be
    // byte-for-byte equal.
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
