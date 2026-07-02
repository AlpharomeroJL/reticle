//! Per-rule fixture tests for the SKY130 DRC subset.
//!
//! Every test runs the FULL loaded rule set from [`sky130_drc_rules`] against a
//! tiny hand-crafted layout, in both directions per rule: a layout built to
//! violate exactly one rule (asserting the engine flags that rule and nothing
//! else) and a clean layout that satisfies the whole subset (asserting zero
//! violations). Coordinates are database units, 1 dbu = 1 nm, matching the
//! committed `tech/sky130-drc-subset.toml`.

use reticle_drc::{DrcEngine, sky130_drc_rules};
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, Document, DrawShape, RuleKind, RuleSet, ShapeKind, Violation};

// SKY130 GDS layers used by the fixtures (see tech/sky130.tech).
const DIFF: LayerId = LayerId::new(65, 20);
const POLY: LayerId = LayerId::new(66, 20);
const LI1: LayerId = LayerId::new(67, 20);
const MCON: LayerId = LayerId::new(67, 44);
const MET1: LayerId = LayerId::new(68, 20);
const VIA: LayerId = LayerId::new(68, 44);
const MET2: LayerId = LayerId::new(69, 20);

/// Builds a single-cell document named `top` from the given shapes.
fn doc_with(shapes: Vec<DrawShape>) -> Document {
    let mut cell = Cell::new("top");
    cell.shapes = shapes;
    let mut doc = Document::new();
    doc.insert_cell(cell);
    doc
}

fn rect_shape(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
    DrawShape::new(
        layer,
        ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
    )
}

/// Runs the whole loaded SKY130 subset over `shapes`.
fn sky130_check(shapes: Vec<DrawShape>) -> Vec<Violation> {
    let doc = doc_with(shapes);
    DrcEngine::new(sky130_drc_rules()).check_cell(&doc, "top")
}

/// Asserts the layout broke exactly one rule, `rule`, and returns the violation.
fn assert_single(mut violations: Vec<Violation>, rule: &str) -> Violation {
    let names: Vec<&str> = violations.iter().map(|v| v.rule.as_str()).collect();
    assert_eq!(
        names,
        vec![rule],
        "expected exactly one {rule} violation, got {names:?}"
    );
    violations.remove(0)
}

/// Asserts the layout is clean against the entire subset.
fn assert_clean(violations: &[Violation]) {
    let names: Vec<&str> = violations.iter().map(|v| v.rule.as_str()).collect();
    assert!(names.is_empty(), "expected a clean layout, got {names:?}");
}

// --- m1.1: min width of met1 (140) ---------------------------------------------

#[test]
fn m1_1_narrow_met1_flagged() {
    // 130 wide: 10 dbu under the 140 minimum. Long enough that its area passes
    // m1.6, so the width rule is the only one that fires.
    let v = sky130_check(vec![rect_shape(MET1, 0, 0, 130, 700)]);
    let v = assert_single(v, "m1.1");
    assert_eq!(v.kind, RuleKind::Width);
    assert_eq!(v.measured, 130);
    assert_eq!(v.required, 140);
    assert_eq!(v.layer, MET1);
}

#[test]
fn m1_1_wide_met1_clean() {
    // 300 x 300: width and area both comfortably above minimum.
    assert_clean(&sky130_check(vec![rect_shape(MET1, 0, 0, 300, 300)]));
}

// --- m1.2: min spacing of met1 to met1 (140) ------------------------------------

#[test]
fn m1_2_close_met1_pair_flagged() {
    // Two DRC-clean met1 blocks 100 apart: 40 dbu under the 140 minimum.
    let v = sky130_check(vec![
        rect_shape(MET1, 0, 0, 300, 300),
        rect_shape(MET1, 400, 0, 700, 300),
    ]);
    let v = assert_single(v, "m1.2");
    assert_eq!(v.kind, RuleKind::Spacing);
    assert_eq!(v.measured, 100);
    assert_eq!(v.required, 140);
}

#[test]
fn m1_2_spaced_met1_pair_clean() {
    // Gap of exactly 140: the minimum itself must pass.
    assert_clean(&sky130_check(vec![
        rect_shape(MET1, 0, 0, 300, 300),
        rect_shape(MET1, 440, 0, 740, 300),
    ]));
}

// --- m1.4: mcon enclosed by met1 by 30 ------------------------------------------

#[test]
fn m1_4_thin_enclosure_flagged() {
    // met1 surrounds the mcon by only 20 on the left and bottom (30 required).
    // The met1 block is large enough to satisfy its own width and area rules.
    let v = sky130_check(vec![
        rect_shape(MCON, 0, 0, 170, 170),
        rect_shape(MET1, -20, -20, 500, 500),
    ]);
    let v = assert_single(v, "m1.4");
    assert_eq!(v.kind, RuleKind::Enclosure);
    assert_eq!(v.measured, 20);
    assert_eq!(v.required, 30);
    assert_eq!(v.other_layer, Some(MET1));
}

#[test]
fn m1_4_unenclosed_mcon_flagged() {
    // An mcon with no met1 over it at all: flagged with the absent-feature
    // sentinel rather than a measured margin.
    let v = sky130_check(vec![rect_shape(MCON, 0, 0, 170, 170)]);
    let v = assert_single(v, "m1.4");
    assert_eq!(v.measured, i64::MIN);
}

#[test]
fn m1_4_good_enclosure_clean() {
    // Margin of exactly 30 on the tight sides; met1 is 330 x 330 so width and
    // area pass too.
    assert_clean(&sky130_check(vec![
        rect_shape(MCON, 0, 0, 170, 170),
        rect_shape(MET1, -30, -30, 300, 300),
    ]));
}

// --- m1.6: min area of met1 (83000 dbu^2) ---------------------------------------

#[test]
fn m1_6_small_met1_flagged() {
    // 200 x 200 = 40000 dbu^2, under the 83000 minimum; width passes at 200.
    let v = sky130_check(vec![rect_shape(MET1, 0, 0, 200, 200)]);
    let v = assert_single(v, "m1.6");
    assert_eq!(v.kind, RuleKind::Area);
    assert_eq!(v.measured, 40_000);
    assert_eq!(v.required, 83_000);
}

#[test]
fn m1_6_large_met1_clean() {
    // 290 x 290 = 84100 dbu^2, just over the minimum.
    assert_clean(&sky130_check(vec![rect_shape(MET1, 0, 0, 290, 290)]));
}

// --- poly.8: poly endcap extension past diff (130) ------------------------------

#[test]
fn poly_8_short_endcap_flagged() {
    // Poly overhangs the diff it covers by only 100 on every side (130 required).
    // Both shapes satisfy their own width rules.
    let v = sky130_check(vec![
        rect_shape(POLY, 0, 0, 1000, 460),
        rect_shape(DIFF, 100, 100, 900, 360),
    ]);
    let v = assert_single(v, "poly.8");
    assert_eq!(v.kind, RuleKind::Extension);
    assert_eq!(v.measured, 100);
    assert_eq!(v.required, 130);
    assert_eq!(v.other_layer, Some(DIFF));
}

#[test]
fn poly_8_full_endcap_clean() {
    // Poly extends exactly 130 past the diff on every side.
    assert_clean(&sky130_check(vec![
        rect_shape(POLY, 0, 0, 1060, 560),
        rect_shape(DIFF, 130, 130, 930, 430),
    ]));
}

// --- li.6: min area of li1 (56100 dbu^2) ----------------------------------------

#[test]
fn li_6_small_li1_flagged() {
    // 170 x 200 = 34000 dbu^2 under the 56100 minimum; li.1 width passes at 170.
    let v = sky130_check(vec![rect_shape(LI1, 0, 0, 170, 200)]);
    let v = assert_single(v, "li.6");
    assert_eq!(v.measured, 34_000);
    assert_eq!(v.required, 56_100);
}

#[test]
fn li_6_large_li1_clean() {
    // 250 x 250 = 62500 dbu^2.
    assert_clean(&sky130_check(vec![rect_shape(LI1, 0, 0, 250, 250)]));
}

// --- difftap.3: min spacing of diff to diff (270) --------------------------------

#[test]
fn difftap_3_close_diff_pair_flagged() {
    // Two diff regions 200 apart (270 required).
    let v = sky130_check(vec![
        rect_shape(DIFF, 0, 0, 200, 300),
        rect_shape(DIFF, 400, 0, 600, 300),
    ]);
    let v = assert_single(v, "difftap.3");
    assert_eq!(v.measured, 200);
    assert_eq!(v.required, 270);
}

#[test]
fn difftap_3_spaced_diff_pair_clean() {
    // Gap of exactly 270.
    assert_clean(&sky130_check(vec![
        rect_shape(DIFF, 0, 0, 200, 300),
        rect_shape(DIFF, 470, 0, 670, 300),
    ]));
}

// --- via.1a: via size 0.15 x 0.15 encoded as min width (150) ---------------------

#[test]
fn via_1a_undersized_via_flagged() {
    // A 140 via (150 required), correctly enclosed by met2 so only the size
    // rule fires.
    let v = sky130_check(vec![
        rect_shape(VIA, 0, 0, 140, 140),
        rect_shape(MET2, -55, -55, 195, 195),
    ]);
    let v = assert_single(v, "via.1a");
    assert_eq!(v.kind, RuleKind::Width);
    assert_eq!(v.measured, 140);
    assert_eq!(v.required, 150);
}

#[test]
fn via_1a_sized_via_clean() {
    // A 150 via with the m2.4 enclosure of exactly 55.
    assert_clean(&sky130_check(vec![
        rect_shape(VIA, 0, 0, 150, 150),
        rect_shape(MET2, -55, -55, 205, 205),
    ]));
}

// --- m2.4: via enclosed by met2 by 55 --------------------------------------------

#[test]
fn m2_4_thin_enclosure_flagged() {
    // met2 surrounds the via by only 40 (55 required); the via itself is legal.
    let v = sky130_check(vec![
        rect_shape(VIA, 0, 0, 150, 150),
        rect_shape(MET2, -40, -40, 190, 190),
    ]);
    let v = assert_single(v, "m2.4");
    assert_eq!(v.kind, RuleKind::Enclosure);
    assert_eq!(v.measured, 40);
    assert_eq!(v.required, 55);
}

#[test]
fn m2_4_good_enclosure_clean() {
    // Margin of 60 on every side.
    assert_clean(&sky130_check(vec![
        rect_shape(VIA, 0, 0, 200, 200),
        rect_shape(MET2, -60, -60, 260, 260),
    ]));
}

// --- Whole-subset sanity ----------------------------------------------------------

#[test]
fn multi_layer_clean_snippet_passes_the_whole_subset() {
    // A little legal stack: a poly gate strip over diff with full endcaps, an
    // mcon properly enclosed by a met1 block, and a via properly enclosed by
    // met2, all far enough apart that no spacing rule engages.
    assert_clean(&sky130_check(vec![
        rect_shape(POLY, 0, 0, 1060, 560),
        rect_shape(DIFF, 130, 130, 930, 430),
        rect_shape(MCON, 5000, 0, 5170, 170),
        rect_shape(MET1, 4970, -30, 5300, 300),
        rect_shape(VIA, 10_000, 0, 10_150, 150),
        rect_shape(MET2, 9945, -55, 10_205, 205),
    ]));
}
