//! Solvability proof for the tier-5 (real SKY130) tasks.
//!
//! `suite.rs` proves the committed suite loads and every checker dispatches; this
//! test goes one step further for the tier-5 group: each task, as loaded from
//! `benchmarks/layout-tasks`, is run against a reference document drawn with
//! correct SKY130 geometry (real rule values, real `sky130_fd_sc_hd` dimensions)
//! and must PASS, then against a deliberately spoiled variant and must FAIL. That
//! keeps the tasks honest in both directions: they are satisfiable, and they are
//! not vacuous.
//!
//! The reference geometry mirrors the numbers the prompts cite: m1.1/m1.6 minimum
//! width and area, the fixed ct.1/licon.1/via.1a cut sizes, the m1.4/li.5/m2.4
//! enclosures, and the measured rails, taps, and nwell overhang of the
//! `sky130_fd_sc_hd` fill and tap cells.

use std::path::PathBuf;

use reticle_agent_api::Transcript;
use reticle_bench::{BenchTask, CheckResult, CheckerRegistry, load_suite};
use reticle_geometry::{LayerId, Point, Rect, Transform};
use reticle_model::{Cell, Document, DrawShape, Instance, ShapeKind};

// The real SKY130 layers the tier-5 tasks draw on.
const NWELL: LayerId = LayerId::new(64, 20);
const DIFF: LayerId = LayerId::new(65, 20);
const TAP: LayerId = LayerId::new(65, 44);
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

/// A one-cell document named `top`, marked as the top cell, holding `shapes`.
fn doc_with(shapes: Vec<DrawShape>) -> Document {
    let mut cell = Cell::new("top");
    cell.shapes = shapes;
    let mut doc = Document::new();
    doc.insert_cell(cell);
    doc.set_top_cells(vec!["top".into()]);
    doc
}

#[test]
fn met1_min_wire_is_solvable() {
    // 140 x 593: exactly m1.1 minimum width, area 83020 >= m1.6's 83000.
    let good = doc_with(vec![rect(MET1, 0, 0, 593, 140)]);
    // Same width but short: area 70000 violates m1.6.
    let bad = doc_with(vec![rect(MET1, 0, 0, 500, 140)]);
    assert_two_way("t5_met1_min_wire", &good, &bad);
}

#[test]
fn fill_rail_pair_is_solvable() {
    // The measured sky130_fd_sc_hd__fill_1 rails: 940 x 480 each, 902400 total.
    let good = doc_with(vec![
        rect(MET1, -240, -240, 700, 240),
        rect(MET1, -240, 2480, 700, 2960),
    ]);
    // Rails trimmed to the bare 460 tile: 441600 total, under the 900000 floor.
    let bad = doc_with(vec![
        rect(MET1, 0, -240, 460, 240),
        rect(MET1, 0, 2480, 460, 2960),
    ]);
    assert_two_way("t5_fill_rail_pair", &good, &bad);
}

#[test]
fn mcon_stack_is_solvable() {
    // A ct.1-sized (170) mcon centered in 300 x 300 pads: 65 dbu margin >= m1.4's 30.
    let good = doc_with(vec![
        rect(LI1, 0, 0, 300, 300),
        rect(MET1, 0, 0, 300, 300),
        rect(MCON, 65, 65, 235, 235),
    ]);
    // The cut still connects but sits flush with the pad corner: 0 enclosure < 30.
    let bad = doc_with(vec![
        rect(LI1, 0, 0, 300, 300),
        rect(MET1, 0, 0, 300, 300),
        rect(MCON, 0, 0, 170, 170),
    ]);
    assert_two_way("t5_mcon_stack_m1_enc", &good, &bad);
}

#[test]
fn licon_tap_stack_is_solvable() {
    // A licon.1-sized (170) cut centered in 330 x 330 pads: exactly li.5's 80 margin.
    let good = doc_with(vec![
        rect(DIFF, 0, 0, 330, 330),
        rect(LI1, 0, 0, 330, 330),
        rect(LICON1, 80, 80, 250, 250),
    ]);
    // Off-center cut: 40 dbu on the near sides, under the 80 the task requires.
    let bad = doc_with(vec![
        rect(DIFF, 0, 0, 330, 330),
        rect(LI1, 0, 0, 330, 330),
        rect(LICON1, 40, 40, 210, 210),
    ]);
    assert_two_way("t5_licon_tap_li1_enc", &good, &bad);
}

/// The VGND rail pair of an eight-tile row with one mcon per 460 dbu tile, the
/// measured fill-cell cut position (x 145..315 within each tile).
fn rail_chain(mcons_on_rail: usize) -> Vec<DrawShape> {
    let mut shapes = vec![rect(LI1, 0, -85, 3680, 85), rect(MET1, 0, -240, 3680, 240)];
    for k in 0..mcons_on_rail {
        let x = 460 * i32::try_from(k).expect("small tile index");
        shapes.push(rect(MCON, x + 145, -85, x + 315, 85));
    }
    shapes
}

#[test]
fn mcon_rail_chain_is_solvable() {
    let good = doc_with(rail_chain(8));
    // Seven cuts on the rail and one stray: the largest net carries only seven.
    let mut broken = rail_chain(7);
    broken.push(rect(MCON, 5000, 5000, 5170, 5170));
    let bad = doc_with(broken);
    assert_two_way("t5_mcon_rail_chain_8", &good, &bad);
}

/// A met1/met2 strap pair joined by `vias` via.1a-sized (150) cuts, every cut
/// enclosed by at least 55 dbu of metal in y (75) and x (100 at the tightest).
fn via_stitch(vias: usize) -> Vec<DrawShape> {
    let mut shapes = vec![rect(MET1, 0, 0, 2000, 300), rect(MET2, 0, 0, 2000, 300)];
    for k in 0..vias {
        let x = 100 + 300 * i32::try_from(k).expect("small via index");
        shapes.push(rect(VIA1, x, 75, x + 150, 225));
    }
    shapes
}

#[test]
fn via1_stitch_is_solvable() {
    let good = doc_with(via_stitch(6));
    // One cut short of the six the task demands.
    let bad = doc_with(via_stitch(5));
    assert_two_way("t5_via1_stitch_6", &good, &bad);
}

#[test]
fn route_min_pitch_pair_is_solvable() {
    // Two 140-wide met1 straps on the 280 pitch: a at y 0..140, b at y 280..420,
    // leaving exactly the m1.2 minimum 140 gap and no short.
    let good = doc_with(vec![
        rect(MET1, 0, 0, 1000, 140),
        rect(MET1, 0, 280, 1000, 420),
    ]);
    // One blob covering both rails connects every terminal: a forbidden short.
    let bad = doc_with(vec![rect(MET1, 0, 0, 1000, 420)]);
    assert_two_way("t5_route_min_pitch_pair", &good, &bad);
}

/// The reference escape route: met1 wire to a landing pad, a via.1a cut with 55
/// dbu of met2 enclosure (m2.4), then met2 up and across to the far corner.
fn escape_route(with_via: bool) -> Vec<DrawShape> {
    let mut shapes = vec![
        rect(MET1, 0, 0, 1500, 140),        // pin wire east from the origin
        rect(MET1, 1400, -60, 1710, 250),   // via landing pad
        rect(MET2, 1450, -35, 1710, 2720),  // vertical met2 run
        rect(MET2, 1450, 2580, 1840, 2720), // horizontal met2 arm to the corner
    ];
    if with_via {
        shapes.push(rect(VIA1, 1505, 20, 1655, 170));
    }
    shapes
}

#[test]
fn route_cell_escape_is_solvable() {
    let good = doc_with(escape_route(true));
    // Without the via the met1 and met2 halves never join: the net is open.
    let bad = doc_with(escape_route(false));
    assert_two_way("t5_route_cell_escape", &good, &bad);
}

#[test]
fn tap_cell_core_is_solvable() {
    // The measured sky130_fd_sc_hd__tap_1 strips: 170 wide, centered at x = 230.
    let good = doc_with(vec![
        rect(TAP, 145, 320, 315, 845),
        rect(TAP, 145, 1525, 315, 2400),
    ]);
    // Only the substrate strip: one tap shape where the spec demands exactly two.
    let bad = doc_with(vec![rect(TAP, 145, 320, 315, 845)]);
    assert_two_way("t5_tap_cell_core", &good, &bad);
}

/// A document with a five-shape filler tile (measured `sky130_fd_sc_hd__fill_1`
/// nwell, met1 rails, li1 rails) placed `instances` times on the 460 pitch.
fn fill_row(instances: usize) -> Document {
    let mut tile = Cell::new("fill_tile");
    tile.shapes = vec![
        rect(NWELL, -190, 1305, 650, 2910),
        rect(MET1, -240, -240, 700, 240),
        rect(MET1, -240, 2480, 700, 2960),
        rect(LI1, -85, -85, 545, 85),
        rect(LI1, -85, 2635, 545, 2805),
    ];
    let mut top = Cell::new("top");
    for k in 0..instances {
        let x = 460 * i32::try_from(k).expect("small tile index");
        top.instances.push(Instance {
            cell: "fill_tile".into(),
            transform: Transform::translate(x, 0),
        });
    }
    let mut doc = Document::new();
    doc.insert_cell(tile);
    doc.insert_cell(top);
    doc.set_top_cells(vec!["top".into()]);
    doc
}

#[test]
fn fill_tile_row_is_solvable() {
    // Four tiles of five shapes flatten to 20, meeting both bounds.
    let good = fill_row(4);
    // Three placements fall short of the four-instance row.
    let bad = fill_row(3);
    assert_two_way("t5_fill_tile_row", &good, &bad);
}
