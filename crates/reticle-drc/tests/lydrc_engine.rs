//! End-to-end test of the `.lydrc` compatibility path: parse the committed
//! `subset.lydrc` deck with [`reticle_drc::parse_lydrc`], run the resulting rules
//! through [`DrcEngine`], and assert the per-rule verdicts on the `subset.gds`
//! geometry (reconstructed here as a [`Document`]).
//!
//! This is the reticle side of the `KLayout` verdict comparison in
//! `scripts/lydrc-compare.ps1`. The geometry below is the single source of truth
//! shared with `scripts/lydrc-fixture-gen.rb` (which emits the identical
//! `subset.gds` `KLayout` reads); keep the two in lock-step. Coordinates are DBU
//! (1 dbu = 1 nm). Each supported rule fires exactly once except `m2.1`, which is
//! clean, so `fired` verdicts are unambiguous.

use reticle_drc::DrcEngine;
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, Document, DrawShape, RuleKind, RuleSet, ShapeKind};

const MET1: LayerId = LayerId::new(68, 20);
const MET2: LayerId = LayerId::new(69, 20);
const MCON: LayerId = LayerId::new(67, 44);
const LI1: LayerId = LayerId::new(67, 20);

const DECK: &str = include_str!("fixtures/subset.lydrc");

fn rect(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
    DrawShape::new(
        layer,
        ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
    )
}

/// The `subset.gds` layout, reconstructed shape-for-shape. See the file header and
/// `scripts/lydrc-fixture-gen.rb` for why each shape is placed where it is.
fn subset_layout() -> Document {
    let mut cell = Cell::new("SUBSET");
    cell.shapes = vec![
        // met1: one narrow wire (width 100 < 140 -> m1.1) far from everything else.
        rect(MET1, 0, 0, 1000, 100),
        // met1: a close pair, 100 DBU apart (< 140 -> m1.2), each 200x200 so width is fine.
        rect(MET1, 0, 10_000, 200, 10_200),
        rect(MET1, 300, 10_000, 500, 10_200),
        // met1: a 220x220 pad enclosing the mcon below by only 10 DBU (< 30 -> m1.4).
        rect(MET1, 9_990, -10, 10_210, 210),
        // mcon: a 200x200 cut enclosed by the pad above.
        rect(MCON, 10_000, 0, 10_200, 200),
        // li1: a 200x200 pad, area 40_000 < 56_100 -> li.6.
        rect(LI1, 0, 20_000, 200, 20_200),
        // met2: a 500x500 pad, wide and large -> m2.1 stays clean.
        rect(MET2, 0, 30_000, 500, 30_500),
    ];
    let mut doc = Document::new();
    doc.insert_cell(cell);
    doc
}

/// The number of violations the engine reports per rule name.
fn counts_by_rule(deck: &str, doc: &Document) -> std::collections::HashMap<String, usize> {
    let rules = reticle_drc::parse_lydrc(deck).expect("subset.lydrc must parse");
    let engine = DrcEngine::new(rules);
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for v in engine.check_cell(doc, "SUBSET") {
        *counts.entry(v.rule).or_default() += 1;
    }
    counts
}

#[test]
fn subset_deck_parses_to_the_expected_rules() {
    let rules = reticle_drc::parse_lydrc(DECK).expect("subset.lydrc must parse");
    let kinds: Vec<(&str, RuleKind)> = rules.iter().map(|r| (r.name.as_str(), r.kind)).collect();
    assert_eq!(
        kinds,
        vec![
            ("m1.1", RuleKind::Width),
            ("m1.2", RuleKind::Spacing),
            ("m1.4", RuleKind::Enclosure),
            ("li.6", RuleKind::Area),
            ("m2.1", RuleKind::Width),
        ],
    );
    // The enclosing swap: engine `layer` is the enclosed mcon, `other_layer` the met1.
    let m1_4 = rules.iter().find(|r| r.name == "m1.4").unwrap();
    assert_eq!(m1_4.layer, MCON);
    assert_eq!(m1_4.other_layer, Some(MET1));
    assert_eq!(m1_4.value, 30, "0.03 um -> 30 dbu");
}

#[test]
fn subset_deck_produces_the_expected_verdicts() {
    let doc = subset_layout();
    let counts = counts_by_rule(DECK, &doc);

    // Exact per-rule violation counts (the authoritative reticle verdict; must
    // match reticle_count in fixtures/expected-verdicts.json).
    assert_eq!(counts.get("m1.1").copied().unwrap_or(0), 1, "met1 width");
    assert_eq!(counts.get("m1.2").copied().unwrap_or(0), 1, "met1 spacing");
    assert_eq!(
        counts.get("m1.4").copied().unwrap_or(0),
        1,
        "mcon enclosure"
    );
    assert_eq!(counts.get("li.6").copied().unwrap_or(0), 1, "li1 area");
    assert_eq!(counts.get("m2.1").copied().unwrap_or(0), 0, "met2 clean");

    // The layout-level fired verdict is what the KLayout comparison checks.
    let fired = |name: &str| counts.get(name).copied().unwrap_or(0) > 0;
    assert!(fired("m1.1") && fired("m1.2") && fired("m1.4") && fired("li.6"));
    assert!(!fired("m2.1"));
}

#[test]
fn a_clean_layout_fires_nothing() {
    // Same deck, but a layout with a single wide, isolated, large met1 pad.
    let mut cell = Cell::new("SUBSET");
    cell.shapes = vec![rect(MET1, 0, 0, 1000, 1000)];
    let mut doc = Document::new();
    doc.insert_cell(cell);
    let counts = counts_by_rule(DECK, &doc);
    assert!(counts.values().all(|&c| c == 0), "clean layout: {counts:?}");
}
