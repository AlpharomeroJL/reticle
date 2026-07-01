//! Hand-built unit tests for each DRC rule kind.
//!
//! Each test constructs a tiny cell with a known violation (or a deliberately
//! clean layout) and asserts the engine reports exactly what it should, including
//! the violation's rule name and zoom-to location.

use reticle_drc::DrcEngine;
use reticle_geometry::{LayerId, Path, Point, Polygon, Rect};
use reticle_model::{Cell, Document, DrawShape, Rule, RuleKind, RuleSet, ShapeKind};

const METAL1: LayerId = LayerId::new(1, 0);
const METAL2: LayerId = LayerId::new(2, 0);
const VIA: LayerId = LayerId::new(3, 0);

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

fn rule(name: &str, kind: RuleKind, layer: LayerId, other: Option<LayerId>, value: i64) -> Rule {
    Rule {
        name: name.to_owned(),
        kind,
        layer,
        other_layer: other,
        value,
    }
}

// --- Width -------------------------------------------------------------------

#[test]
fn narrow_wire_triggers_width() {
    // A 4-wide, 100-long wire with a 10 DBU minimum width.
    let doc = doc_with(vec![rect_shape(METAL1, 0, 0, 100, 4)]);
    let engine = DrcEngine::new(vec![rule("m1.width", RuleKind::Width, METAL1, None, 10)]);
    let v = engine.check_cell(&doc, "top");
    assert_eq!(v.len(), 1, "one width violation expected");
    assert_eq!(v[0].rule, "m1.width");
    assert_eq!(
        v[0].location,
        Rect::new(Point::new(0, 0), Point::new(100, 4)),
        "location is the offending shape's bbox"
    );
    assert!(
        v[0].message.contains("width 4"),
        "message: {}",
        v[0].message
    );
}

#[test]
fn wide_wire_passes_width() {
    let doc = doc_with(vec![rect_shape(METAL1, 0, 0, 100, 20)]);
    let engine = DrcEngine::new(vec![rule("m1.width", RuleKind::Width, METAL1, None, 10)]);
    assert!(engine.check_cell(&doc, "top").is_empty());
}

#[test]
fn width_only_applies_to_its_layer() {
    // Narrow shape lives on METAL2, but the rule targets METAL1.
    let doc = doc_with(vec![rect_shape(METAL2, 0, 0, 100, 2)]);
    let engine = DrcEngine::new(vec![rule("m1.width", RuleKind::Width, METAL1, None, 10)]);
    assert!(engine.check_cell(&doc, "top").is_empty());
}

// --- Spacing -----------------------------------------------------------------

#[test]
fn two_rects_a_hair_too_close_trigger_spacing() {
    // Gap of 3 DBU between two boxes; minimum spacing is 5.
    let doc = doc_with(vec![
        rect_shape(METAL1, 0, 0, 10, 10),
        rect_shape(METAL1, 13, 0, 23, 10),
    ]);
    let engine = DrcEngine::new(vec![rule("m1.space", RuleKind::Spacing, METAL1, None, 5)]);
    let v = engine.check_cell(&doc, "top");
    assert_eq!(v.len(), 1, "one spacing violation expected");
    assert_eq!(v[0].rule, "m1.space");
    assert!(
        v[0].message.contains("spacing 3"),
        "message: {}",
        v[0].message
    );
    // Location spans both offending shapes.
    assert_eq!(
        v[0].location,
        Rect::new(Point::new(0, 0), Point::new(23, 10))
    );
}

#[test]
fn adequately_spaced_rects_pass() {
    let doc = doc_with(vec![
        rect_shape(METAL1, 0, 0, 10, 10),
        rect_shape(METAL1, 20, 0, 30, 10), // gap of 10
    ]);
    let engine = DrcEngine::new(vec![rule("m1.space", RuleKind::Spacing, METAL1, None, 5)]);
    assert!(engine.check_cell(&doc, "top").is_empty());
}

#[test]
fn touching_rects_do_not_trigger_spacing() {
    let doc = doc_with(vec![
        rect_shape(METAL1, 0, 0, 10, 10),
        rect_shape(METAL1, 10, 0, 20, 10), // shares an edge: gap 0
    ]);
    let engine = DrcEngine::new(vec![rule("m1.space", RuleKind::Spacing, METAL1, None, 5)]);
    assert!(
        engine.check_cell(&doc, "top").is_empty(),
        "touching shapes must not be flagged as a spacing violation"
    );
}

#[test]
fn overlapping_rects_do_not_trigger_spacing() {
    let doc = doc_with(vec![
        rect_shape(METAL1, 0, 0, 10, 10),
        rect_shape(METAL1, 5, 5, 15, 15), // overlaps
    ]);
    let engine = DrcEngine::new(vec![rule("m1.space", RuleKind::Spacing, METAL1, None, 5)]);
    assert!(engine.check_cell(&doc, "top").is_empty());
}

#[test]
fn cross_layer_spacing() {
    // METAL1 vs METAL2, 4 DBU apart, minimum 6.
    let doc = doc_with(vec![
        rect_shape(METAL1, 0, 0, 10, 10),
        rect_shape(METAL2, 14, 0, 24, 10),
    ]);
    let engine = DrcEngine::new(vec![rule(
        "m1m2.space",
        RuleKind::Spacing,
        METAL1,
        Some(METAL2),
        6,
    )]);
    let v = engine.check_cell(&doc, "top");
    assert_eq!(v.len(), 1);
    assert!(
        v[0].message.contains("spacing 4"),
        "message: {}",
        v[0].message
    );
}

#[test]
fn spacing_counts_each_pair_once() {
    // Three collinear boxes each 3 apart; with minimum 5 the close pairs are
    // (0,1) and (1,2) but not (0,2) which is 13 apart. Exactly two violations.
    let doc = doc_with(vec![
        rect_shape(METAL1, 0, 0, 10, 10),
        rect_shape(METAL1, 13, 0, 23, 10),
        rect_shape(METAL1, 26, 0, 36, 10),
    ]);
    let engine = DrcEngine::new(vec![rule("m1.space", RuleKind::Spacing, METAL1, None, 5)]);
    assert_eq!(engine.check_cell(&doc, "top").len(), 2);
}

#[test]
fn diagonal_spacing_uses_corner_distance() {
    // Corner-to-corner offset (3, 4) -> distance exactly 5; minimum 6 flags it.
    let doc = doc_with(vec![
        rect_shape(METAL1, 0, 0, 10, 10),
        rect_shape(METAL1, 13, 14, 20, 20),
    ]);
    let engine = DrcEngine::new(vec![rule("m1.space", RuleKind::Spacing, METAL1, None, 6)]);
    let v = engine.check_cell(&doc, "top");
    assert_eq!(v.len(), 1);
    assert!(
        v[0].message.contains("spacing 5"),
        "message: {}",
        v[0].message
    );
}

// --- Area --------------------------------------------------------------------

#[test]
fn tiny_rect_triggers_area() {
    // 4x4 = 16 DBU^2 shape; minimum area 100.
    let doc = doc_with(vec![rect_shape(METAL1, 0, 0, 4, 4)]);
    let engine = DrcEngine::new(vec![rule("m1.area", RuleKind::Area, METAL1, None, 100)]);
    let v = engine.check_cell(&doc, "top");
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].rule, "m1.area");
    assert!(
        v[0].message.contains("area 16"),
        "message: {}",
        v[0].message
    );
    assert_eq!(v[0].location, Rect::new(Point::new(0, 0), Point::new(4, 4)));
}

#[test]
fn large_rect_passes_area() {
    let doc = doc_with(vec![rect_shape(METAL1, 0, 0, 50, 50)]);
    let engine = DrcEngine::new(vec![rule("m1.area", RuleKind::Area, METAL1, None, 100)]);
    assert!(engine.check_cell(&doc, "top").is_empty());
}

// --- Enclosure ---------------------------------------------------------------

#[test]
fn insufficient_enclosure_triggers() {
    // A via enclosed by metal2 by only 2 DBU on the tight sides; minimum 5.
    let doc = doc_with(vec![
        rect_shape(METAL2, 0, 0, 20, 20),
        rect_shape(VIA, 2, 2, 18, 18), // 2 DBU margin all around
    ]);
    let engine = DrcEngine::new(vec![rule(
        "via.enc",
        RuleKind::Enclosure,
        VIA,
        Some(METAL2),
        5,
    )]);
    let v = engine.check_cell(&doc, "top");
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].rule, "via.enc");
    assert!(
        v[0].message.contains("enclosure 2"),
        "message: {}",
        v[0].message
    );
    assert_eq!(
        v[0].location,
        Rect::new(Point::new(2, 2), Point::new(18, 18))
    );
}

#[test]
fn sufficient_enclosure_passes() {
    let doc = doc_with(vec![
        rect_shape(METAL2, 0, 0, 20, 20),
        rect_shape(VIA, 8, 8, 12, 12), // 8 DBU margin all around
    ]);
    let engine = DrcEngine::new(vec![rule(
        "via.enc",
        RuleKind::Enclosure,
        VIA,
        Some(METAL2),
        5,
    )]);
    assert!(engine.check_cell(&doc, "top").is_empty());
}

#[test]
fn unenclosed_shape_triggers() {
    // A via with no metal2 covering it at all.
    let doc = doc_with(vec![rect_shape(VIA, 2, 2, 18, 18)]);
    let engine = DrcEngine::new(vec![rule(
        "via.enc",
        RuleKind::Enclosure,
        VIA,
        Some(METAL2),
        5,
    )]);
    let v = engine.check_cell(&doc, "top");
    assert_eq!(v.len(), 1);
    assert!(
        v[0].message.contains("not enclosed"),
        "message: {}",
        v[0].message
    );
}

// --- Extension ---------------------------------------------------------------

#[test]
fn insufficient_extension_triggers() {
    // METAL1 (the extending layer) overlaps a VIA but only overhangs it by 1 DBU
    // on the right; minimum extension 5.
    let doc = doc_with(vec![
        rect_shape(METAL1, 0, 0, 21, 10),
        rect_shape(VIA, 5, 0, 20, 10), // metal1 right edge 21 vs via 20 -> overhang 1
    ]);
    let engine = DrcEngine::new(vec![rule(
        "m1.ext",
        RuleKind::Extension,
        METAL1,
        Some(VIA),
        5,
    )]);
    let v = engine.check_cell(&doc, "top");
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].rule, "m1.ext");
    assert!(
        v[0].message.contains("extension"),
        "message: {}",
        v[0].message
    );
}

// --- Notch -------------------------------------------------------------------

#[test]
fn narrow_notch_triggers() {
    // Two same-layer boxes forming a 2 DBU notch; minimum notch 5.
    let doc = doc_with(vec![
        rect_shape(METAL1, 0, 0, 10, 10),
        rect_shape(METAL1, 12, 0, 22, 10),
    ]);
    let engine = DrcEngine::new(vec![rule("m1.notch", RuleKind::Notch, METAL1, None, 5)]);
    let v = engine.check_cell(&doc, "top");
    assert_eq!(v.len(), 1);
    assert!(
        v[0].message.contains("notch 2"),
        "message: {}",
        v[0].message
    );
}

// --- Density -----------------------------------------------------------------

#[test]
fn low_density_triggers() {
    // METAL1 covers 100 of a 400 DBU^2 window = 250 permille; minimum 500 permille.
    let doc = doc_with(vec![
        rect_shape(METAL1, 0, 0, 10, 10), // area 100
        rect_shape(METAL2, 0, 0, 20, 20), // sets the window to 20x20 = 400
    ]);
    let engine = DrcEngine::new(vec![rule(
        "m1.density",
        RuleKind::Density,
        METAL1,
        None,
        500,
    )]);
    let v = engine.check_cell(&doc, "top");
    assert_eq!(v.len(), 1);
    assert!(v[0].message.contains("250"), "message: {}", v[0].message);
}

#[test]
fn adequate_density_passes() {
    let doc = doc_with(vec![
        rect_shape(METAL1, 0, 0, 18, 20), // area 360 of 400 = 900 permille
        rect_shape(METAL2, 0, 0, 20, 20),
    ]);
    let engine = DrcEngine::new(vec![rule(
        "m1.density",
        RuleKind::Density,
        METAL1,
        None,
        500,
    )]);
    assert!(engine.check_cell(&doc, "top").is_empty());
}

// --- Angle -------------------------------------------------------------------

#[test]
fn non_manhattan_polygon_flagged() {
    // A triangle: inherently has a diagonal edge.
    let tri = Polygon::new(vec![Point::new(0, 0), Point::new(10, 0), Point::new(0, 10)]);
    let doc = doc_with(vec![DrawShape::new(METAL1, ShapeKind::Polygon(tri))]);
    let engine = DrcEngine::new(vec![rule("m1.angle", RuleKind::Angle, METAL1, None, 0)]);
    let v = engine.check_cell(&doc, "top");
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].rule, "m1.angle");
}

#[test]
fn rectangle_passes_angle() {
    let doc = doc_with(vec![rect_shape(METAL1, 0, 0, 10, 10)]);
    let engine = DrcEngine::new(vec![rule("m1.angle", RuleKind::Angle, METAL1, None, 0)]);
    assert!(engine.check_cell(&doc, "top").is_empty());
}

// --- Polygon / path bounding-box behaviour -----------------------------------

#[test]
fn polygon_width_uses_bounding_box_with_note() {
    // A thin polygon: bbox is 3 wide, 40 tall -> feature width 3 < min 10.
    let poly = Polygon::new(vec![
        Point::new(0, 0),
        Point::new(3, 0),
        Point::new(3, 40),
        Point::new(0, 40),
    ]);
    let doc = doc_with(vec![DrawShape::new(METAL1, ShapeKind::Polygon(poly))]);
    let engine = DrcEngine::new(vec![rule("m1.width", RuleKind::Width, METAL1, None, 10)]);
    let v = engine.check_cell(&doc, "top");
    assert_eq!(v.len(), 1);
    assert!(
        v[0].message.contains("bounding-box estimate"),
        "polygon result should be documented as approximate: {}",
        v[0].message
    );
}

#[test]
fn path_participates_in_width() {
    // A width-2 path: its bbox is 2 across, below a 10 minimum.
    let path = Path::new(
        vec![Point::new(0, 5), Point::new(50, 5)],
        2,
        reticle_geometry::Endcap::Flat,
    );
    let doc = doc_with(vec![DrawShape::new(METAL1, ShapeKind::Path(path))]);
    let engine = DrcEngine::new(vec![rule("m1.width", RuleKind::Width, METAL1, None, 10)]);
    let v = engine.check_cell(&doc, "top");
    assert_eq!(v.len(), 1);
}

// --- Clean layout & missing cell ---------------------------------------------

#[test]
fn clean_layout_has_zero_violations() {
    // Well-spaced, wide, large, properly enclosed geometry against a full rule set.
    let doc = doc_with(vec![
        rect_shape(METAL1, 0, 0, 50, 50),
        rect_shape(METAL1, 100, 0, 150, 50), // gap 50
        rect_shape(METAL2, -5, -5, 55, 55),
        rect_shape(VIA, 10, 10, 40, 40), // enclosed by metal2 by >= 15
    ]);
    let engine = DrcEngine::new(vec![
        rule("m1.width", RuleKind::Width, METAL1, None, 10),
        rule("m1.space", RuleKind::Spacing, METAL1, None, 10),
        rule("m1.area", RuleKind::Area, METAL1, None, 100),
        rule("via.enc", RuleKind::Enclosure, VIA, Some(METAL2), 5),
    ]);
    assert!(
        engine.check_cell(&doc, "top").is_empty(),
        "a clean layout must produce no violations"
    );
}

#[test]
fn missing_cell_yields_no_violations() {
    let doc = Document::new();
    let engine = DrcEngine::new(vec![rule("m1.width", RuleKind::Width, METAL1, None, 10)]);
    assert!(engine.check_cell(&doc, "nope").is_empty());
}

// --- Incremental re-check ----------------------------------------------------

#[test]
fn check_region_filters_to_the_edited_area() {
    // Two independent width violations far apart. A region around the first should
    // report only that one; the full pass reports both.
    let doc = doc_with(vec![
        rect_shape(METAL1, 0, 0, 100, 4),         // narrow, near origin
        rect_shape(METAL1, 10_000, 0, 10_100, 4), // narrow, far away
    ]);
    let engine = DrcEngine::new(vec![rule("m1.width", RuleKind::Width, METAL1, None, 10)]);

    assert_eq!(
        engine.check_cell(&doc, "top").len(),
        2,
        "full pass sees both"
    );

    let region = Rect::new(Point::new(-50, -50), Point::new(200, 200));
    let local = engine.check_region(&doc, "top", region);
    assert_eq!(local.len(), 1, "region pass sees only the nearby violation");
    assert_eq!(
        local[0].location,
        Rect::new(Point::new(0, 0), Point::new(100, 4))
    );
}

#[test]
fn check_region_matches_check_cell_when_region_covers_everything() {
    let doc = doc_with(vec![
        rect_shape(METAL1, 0, 0, 10, 10),
        rect_shape(METAL1, 13, 0, 23, 10),  // spacing 3
        rect_shape(METAL1, 0, 100, 4, 200), // width 4
    ]);
    let engine = DrcEngine::new(vec![
        rule("m1.space", RuleKind::Spacing, METAL1, None, 5),
        rule("m1.width", RuleKind::Width, METAL1, None, 10),
    ]);
    let full = engine.check_cell(&doc, "top");
    let region = Rect::new(Point::new(-1000, -1000), Point::new(1000, 1000));
    let all = engine.check_region(&doc, "top", region);
    assert_eq!(
        full.len(),
        all.len(),
        "covering region reproduces the full pass"
    );
}

#[test]
fn missing_cell_region_yields_no_violations() {
    let doc = Document::new();
    let engine = DrcEngine::new(vec![rule("m1.width", RuleKind::Width, METAL1, None, 10)]);
    let region = Rect::new(Point::new(0, 0), Point::new(100, 100));
    assert!(engine.check_region(&doc, "nope", region).is_empty());
}

// --- RuleSet contract --------------------------------------------------------

#[test]
fn rules_accessor_returns_configured_rules() {
    let rules = vec![
        rule("m1.width", RuleKind::Width, METAL1, None, 10),
        rule("m1.space", RuleKind::Spacing, METAL1, None, 5),
    ];
    let engine = DrcEngine::new(rules.clone());
    assert_eq!(engine.rules(), rules.as_slice());
}
