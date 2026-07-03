//! Two-way solvability proof for the Wave-3 (v0.4.0) task group.
//!
//! `suite.rs` proves the committed suite loads and every checker dispatches; this
//! test goes one step further for the 12 Wave-3 tasks the same way
//! `tier5_solvability.rs` does for the tier-5 group: each task, as loaded from
//! `benchmarks/layout-tasks` (checker compiled exactly as the runner compiles it),
//! is run against a reference document that satisfies the prompt and must PASS, then
//! against a deliberately spoiled variant and must FAIL. That keeps every new task
//! honest in both directions: it is satisfiable, and it is not vacuous.
//!
//! The four families exercised:
//!
//! - boolean-op constructions (`boolean_result`): the union / intersection /
//!   difference of the same two 300x300 met1 squares produce three distinct result
//!   areas (150000 / 30000 / 60000) on met2, with the met1 inputs consumed. A good
//!   doc carries the right result area with met1 empty; a bad doc carries the wrong
//!   op's area (or leaves an input behind).
//! - array-with-pitch (`array_pitch`): an array at the right count and pitch passes;
//!   the wrong pitch or too few instances fails.
//! - via-stack (`contact_stack`): a cut bridging both conductors with enough
//!   enclosure passes; a cut that misses the top conductor fails.
//! - iterative-refinement (`layer_area` / `shape_count`): the checker enforces the
//!   *post-refinement* bar, so a doc that meets only the loose initial requirement
//!   fails while one that meets the tightened requirement passes. The refinement
//!   string itself is asserted present and non-empty.

use std::path::PathBuf;

use reticle_agent_api::Transcript;
use reticle_bench::{BenchTask, CheckResult, CheckerRegistry, load_suite};
use reticle_geometry::{LayerId, Point, Polygon, Rect, Transform};
use reticle_model::{ArrayInstance, Cell, Document, DrawShape, ShapeKind};

// The SKY130 layers the Wave-3 tasks draw on.
const POLY: LayerId = LayerId::new(66, 20);
const LICON1: LayerId = LayerId::new(66, 44);
const LI1: LayerId = LayerId::new(67, 20);
const MCON: LayerId = LayerId::new(67, 44);
const MET1: LayerId = LayerId::new(68, 20);
const VIA1: LayerId = LayerId::new(68, 44);
const MET2: LayerId = LayerId::new(69, 20);

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

/// Runs `task`'s own checker (compiled exactly as the runner compiles it) over `doc`.
fn check(task: &BenchTask, doc: &Document) -> CheckResult {
    let registry = CheckerRegistry::for_task(task)
        .unwrap_or_else(|e| panic!("task `{}` checker compiles: {e}", task.id));
    let checker = registry
        .get(&task.checker)
        .unwrap_or_else(|| panic!("task `{}` checker resolves", task.id));
    checker.check(doc, &Transcript::default())
}

/// Asserts the reference document passes and the spoiled document fails.
fn assert_two_way(id: &str, good: &Document, bad: &Document) {
    let task = task(id);
    let pass = check(&task, good);
    assert!(
        pass.is_pass(),
        "`{id}` must pass its reference solution, got {pass:?}"
    );
    let fail = check(&task, bad);
    assert!(
        matches!(fail, CheckResult::Fail(ref f) if !f.is_empty()),
        "`{id}` must reject the spoiled document"
    );
}

/// A rectangle shape on `layer` spanning `(x0, y0)` to `(x1, y1)`.
fn rect(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
    DrawShape::new(
        layer,
        ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
    )
}

/// A polygon shape on `layer` for the rectangle `(x0,y0)-(x1,y1)`, standing in for a
/// planar boolean's polygon output (booleans write polygons, not rectangles).
fn poly_rect(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
    DrawShape::new(
        layer,
        ShapeKind::Polygon(Polygon::from_rect(Rect::new(
            Point::new(x0, y0),
            Point::new(x1, y1),
        ))),
    )
}

/// A one-cell document named `top`, marked as the top cell, holding `shapes`.
fn doc_with(shapes: Vec<DrawShape>) -> Document {
    let mut cell = Cell::new("top");
    cell.shapes = shapes;
    let mut doc = Document::new();
    doc.insert_cell(cell);
    doc.set_top_cells(vec!["top".into()]);
    doc
}

/// A document whose top cell places one array of `columns`x`rows` leaf instances at
/// the given pitches.
fn doc_with_array(columns: u32, rows: u32, column_pitch: i32, row_pitch: i32) -> Document {
    let mut leaf = Cell::new("leaf");
    leaf.shapes.push(rect(MET1, 0, 0, 200, 200));
    let mut top = Cell::new("top");
    top.arrays.push(ArrayInstance {
        cell: "leaf".into(),
        transform: Transform::IDENTITY,
        columns,
        rows,
        column_pitch,
        row_pitch,
    });
    let mut doc = Document::new();
    doc.insert_cell(leaf);
    doc.insert_cell(top);
    doc.set_top_cells(vec!["top".into()]);
    doc
}

// ---- boolean-op constructions -----------------------------------------------

#[test]
fn bool_union_is_solvable() {
    // Union of the two 300x300 met1 squares is the 500x300 rectangle = 150000, written
    // to met2, with met1 consumed.
    let good = doc_with(vec![poly_rect(MET2, 0, 0, 500, 300)]);
    // The intersection-sized result (30000) is the wrong op for a union task.
    let bad = doc_with(vec![poly_rect(MET2, 200, 0, 300, 300)]);
    assert_two_way("t3_bool_union_met2", &good, &bad);
}

#[test]
fn bool_intersect_is_solvable() {
    // Intersection is the 100x300 overlap strip = 30000 on met2, met1 consumed.
    let good = doc_with(vec![poly_rect(MET2, 200, 0, 300, 300)]);
    // The union-sized result (150000) is the wrong op for an intersection task.
    let bad = doc_with(vec![poly_rect(MET2, 0, 0, 500, 300)]);
    assert_two_way("t3_bool_intersect_met2", &good, &bad);
}

#[test]
fn bool_difference_is_solvable() {
    // Difference (A minus B) is the 200x300 remainder = 60000 on met2, met1 consumed.
    let good = doc_with(vec![poly_rect(MET2, 0, 0, 200, 300)]);
    // A correct-area result, but an untouched met1 input remains: the boolean did not
    // consume its inputs, so `cleared=68/20` rejects it.
    let bad = doc_with(vec![
        poly_rect(MET2, 0, 0, 200, 300),
        rect(MET1, 0, 0, 300, 300),
    ]);
    assert_two_way("t3_bool_difference_met2", &good, &bad);
}

// ---- array-with-pitch -------------------------------------------------------

#[test]
fn array_row4_pitch_is_solvable() {
    // 1x4 row at column pitch 800.
    let good = doc_with_array(4, 1, 800, 0);
    // Right count, wrong column pitch (500, not 800).
    let bad = doc_with_array(4, 1, 500, 0);
    assert_two_way("t3_array_row4_pitch", &good, &bad);
}

#[test]
fn array_grid_pitch_is_solvable() {
    // 3x2 grid at 800x600.
    let good = doc_with_array(3, 2, 800, 600);
    // Right shape and column pitch, wrong row pitch (700, not 600).
    let bad = doc_with_array(3, 2, 800, 700);
    assert_two_way("t3_array_grid_pitch", &good, &bad);
}

#[test]
fn array_col3_pitch_is_solvable() {
    // 1-column, 3-row column array at row pitch 700.
    let good = doc_with_array(1, 3, 0, 700);
    // Only two rows: short of the three instances the task demands.
    let bad = doc_with_array(1, 2, 0, 700);
    assert_two_way("t3_array_col3_pitch", &good, &bad);
}

// ---- via-stack --------------------------------------------------------------

#[test]
fn via_stack_m1_m2_is_solvable() {
    // A via1 cut centered in overlapping 300x300 met1/met2 pads: 55 dbu enclosure on
    // every side, above the 40 the task requires, and it bridges both metals.
    let good = doc_with(vec![
        rect(MET1, 0, 0, 300, 300),
        rect(MET2, 0, 0, 300, 300),
        rect(VIA1, 55, 55, 245, 245),
    ]);
    // The cut lands on met1 but met2 is elsewhere: the stack never bridges.
    let bad = doc_with(vec![
        rect(MET1, 0, 0, 300, 300),
        rect(MET2, 1000, 1000, 1300, 1300),
        rect(VIA1, 55, 55, 245, 245),
    ]);
    assert_two_way("t3_via_stack_m1_m2", &good, &bad);
}

#[test]
fn via_stack_li_m1_is_solvable() {
    // An mcon cut centered in overlapping 300x300 li1/met1 pads: 55 dbu enclosure,
    // above the 30 the task requires.
    let good = doc_with(vec![
        rect(LI1, 0, 0, 300, 300),
        rect(MET1, 0, 0, 300, 300),
        rect(MCON, 55, 55, 245, 245),
    ]);
    // No cut at all: nothing bridges li1 to met1.
    let bad = doc_with(vec![rect(LI1, 0, 0, 300, 300), rect(MET1, 0, 0, 300, 300)]);
    assert_two_way("t3_via_stack_li_m1", &good, &bad);
}

#[test]
fn via_stack_poly_li_is_solvable() {
    // A licon1 cut centered in overlapping 300x300 poly/li1 pads: 55 dbu enclosure,
    // above the 30 the task requires.
    let good = doc_with(vec![
        rect(POLY, 0, 0, 300, 300),
        rect(LI1, 0, 0, 300, 300),
        rect(LICON1, 55, 55, 245, 245),
    ]);
    // The cut is flush with the pad corner: 0 enclosure, under the 30 required, even
    // though it still connects the two conductors.
    let bad = doc_with(vec![
        rect(POLY, 0, 0, 300, 300),
        rect(LI1, 0, 0, 300, 300),
        rect(LICON1, 0, 0, 190, 190),
    ]);
    assert_two_way("t3_via_stack_poly_li", &good, &bad);
}

// ---- iterative-refinement ---------------------------------------------------

#[test]
fn refine_widen_met1_is_solvable() {
    // The checker enforces the post-refinement bar (met1 area >= 250000). A 500x500
    // rect (250000) meets it.
    let good = doc_with(vec![rect(MET1, 0, 0, 500, 500)]);
    // A rect that meets only the loose initial requirement (>= 90000) but not the
    // tightened one: 300x300 = 90000 < 250000.
    let bad = doc_with(vec![rect(MET1, 0, 0, 300, 300)]);
    assert_two_way("t4_refine_widen_met1", &good, &bad);
}

#[test]
fn refine_add_shape_met1_is_solvable() {
    // The checker enforces the post-refinement bar (>= 2 met1 shapes). Two rects meet it.
    let good = doc_with(vec![
        rect(MET1, 0, 0, 300, 300),
        rect(MET1, 500, 0, 800, 300),
    ]);
    // Only one shape: the initial single rect, before the refinement's second shape.
    let bad = doc_with(vec![rect(MET1, 0, 0, 300, 300)]);
    assert_two_way("t4_refine_add_shape_met1", &good, &bad);
}

#[test]
fn refine_li1_grow_is_solvable() {
    // The checker enforces the post-refinement bar (li1 area >= 200000). A 500x500 li1
    // rect (250000) meets it.
    let good = doc_with(vec![rect(LI1, 0, 0, 500, 500)]);
    // Only the loose initial size (>= 60000) is met: 250x250 = 62500 < 200000.
    let bad = doc_with(vec![rect(LI1, 0, 0, 250, 250)]);
    assert_two_way("t4_refine_li1_grow", &good, &bad);
}

#[test]
fn refinement_tasks_carry_a_nonempty_refinement() {
    // The three refinement tasks must declare a scripted follow-up constraint; every
    // other task must not (the field is absent for single-shot tasks).
    let refinement_ids = [
        "t4_refine_widen_met1",
        "t4_refine_add_shape_met1",
        "t4_refine_li1_grow",
    ];
    for id in refinement_ids {
        let t = task(id);
        let refinement = t
            .refinement
            .as_deref()
            .unwrap_or_else(|| panic!("refinement task `{id}` must carry a refinement string"));
        assert!(
            !refinement.trim().is_empty(),
            "refinement task `{id}` has an empty refinement string"
        );
    }

    // No non-refinement task accidentally carries the field.
    let (_manifest, tasks) = load_suite(&suite_dir()).expect("suite loads");
    for t in &tasks {
        if !refinement_ids.contains(&t.id.as_str()) {
            assert!(
                t.refinement.is_none(),
                "task `{}` unexpectedly carries a refinement",
                t.id
            );
        }
    }
}
