//! Runs the SKY130 DRC subset over real, flattened `sky130_fd_sc_hd` cells from
//! the committed corpus (`tests/corpus/sky130/`, see `NOTICE.md`).
//!
//! These are correct, silicon-proven layouts, yet two of the three flag. That is
//! expected and pinned here, honestly: the engine reduces polygons and paths to
//! bounding boxes (a documented conservative approximation that can over-report
//! but never miss), and the deck encodes li.5 and poly.8 as uniform all-sides
//! constraints while the real `SkyWater` rules are directional (li.5 requires
//! 0.08 um on two adjacent sides only; poly.8 is the endcap past the gate edge).
//! The exact counts below pin that conservatism so any engine or deck change
//! that adds or removes findings shows up in review. A clean run on `fill_1`
//! says nothing about tape-out; the deck is a subset.

use std::collections::BTreeMap;

use reticle_drc::{DrcEngine, sky130_drc_rules};
use reticle_geometry::LayerId;
use reticle_io::Gds;
use reticle_model::{Cell, Document, Importer, RuleKind, RuleSet, Violation};

/// Imports a committed corpus cell (see `sky130_cells.rs` for structure tests).
fn import_corpus_cell(cell: &str) -> Document {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus/sky130")
        .join(format!("sky130_fd_sc_hd__{cell}.gds"));
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("corpus cell {} should exist: {e}", path.display()));
    Gds.import(&bytes)
        .unwrap_or_else(|e| panic!("corpus cell {cell} should import: {e}"))
}

/// Flattens the document's top cell into a fresh single-cell document, the shape
/// a hierarchical design takes before a full-cell DRC pass.
fn flatten_top(doc: &Document) -> Document {
    let top = doc.top_cells().first().expect("a top cell").clone();
    let mut flat = Cell::new("flat");
    flat.shapes = doc.flatten(&top);
    let mut out = Document::new();
    out.insert_cell(flat);
    out
}

/// Imports, flattens, and checks one corpus cell against the SKY130 subset.
fn check(cell: &str) -> Vec<Violation> {
    let doc = flatten_top(&import_corpus_cell(cell));
    DrcEngine::new(sky130_drc_rules()).check_cell(&doc, "flat")
}

/// Violation counts grouped by rule id.
fn by_rule(violations: &[Violation]) -> BTreeMap<&str, usize> {
    let mut map = BTreeMap::new();
    for v in violations {
        *map.entry(v.rule.as_str()).or_insert(0) += 1;
    }
    map
}

/// The filler cell is clean even under the conservative engine: nothing but
/// straight supply rails and well fill, so no rule in the subset trips.
#[test]
fn fill_cell_is_clean() {
    let violations = check("fill_1");
    assert!(
        violations.is_empty(),
        "fill_1 should pass the subset cleanly, got {violations:?}"
    );
}

/// The well tap flags exactly three li.5 enclosures, all conservative.
///
/// The deck states li.5 as "licon enclosed by li1 by 80 dbu on every side"; the
/// real `SkyWater` li.5 requires 0.08 um on two adjacent sides only. The tap's
/// three well-tap licons sit in a li1 strap with a 60 dbu side margin, which is
/// legal upstream but 20 dbu short of the uniform reading. Silicon says the
/// cell is right; the pinned count documents the deck's over-approximation.
#[test]
fn tap_cell_flags_only_conservative_li_enclosure() {
    let violations = check("tap_1");
    assert_eq!(by_rule(&violations), BTreeMap::from([("li.5", 3)]));
    for v in &violations {
        assert!(
            matches!(v.kind, RuleKind::Enclosure),
            "li.5 is an enclosure"
        );
        assert_eq!(v.layer, LayerId::new(66, 44), "licon is the enclosed layer");
        assert_eq!(v.other_layer, Some(LayerId::new(67, 20)), "li1 encloses");
        assert_eq!(
            (v.measured, v.required),
            (60, 80),
            "tap licons keep 60 dbu of side margin, short of the uniform 80"
        );
    }
}

/// The inverter flags exactly 11 violations across three rules, each a known
/// conservative artifact rather than a broken cell:
///
/// - li.3 (x1): the two li1 connection polygons are L-shaped; their *bounding
///   boxes* come within 70 dbu although the true polygon edges keep the
///   required 170. Bounding-box spacing over-reports on concave geometry.
/// - li.5 (x8): six licons sit in minimum-width (170 dbu) li1 straps with zero
///   side enclosure, and two more keep only 40 and 60 dbu. Legal under the
///   directional upstream rule, short of the deck's uniform 80 on all sides.
/// - poly.8 (x2, one per diffusion region the gate crosses): the gate poly does
///   extend 130 dbu past diff in y (the real endcap), but the bounding-box
///   extension check also demands it in x, where diff is wider than the gate
///   finger, so the measured minimum extension goes negative.
#[test]
fn inverter_flags_pinned_bbox_conservative_set() {
    let violations = check("inv_1");
    assert_eq!(
        by_rule(&violations),
        BTreeMap::from([("li.3", 1), ("li.5", 8), ("poly.8", 2)]),
        "the inverter's conservative findings changed; re-derive the analysis \
         in this test's comment before repinning"
    );

    let li3 = violations
        .iter()
        .find(|v| v.rule == "li.3")
        .expect("li.3 violation");
    assert_eq!(
        (li3.measured, li3.required),
        (70, 170),
        "bbox gap of the two L-shaped li1 polygons"
    );
    let mut li5_measured: Vec<i64> = violations
        .iter()
        .filter(|v| v.rule == "li.5")
        .map(|v| v.measured)
        .collect();
    li5_measured.sort_unstable();
    assert_eq!(
        li5_measured,
        [0, 0, 0, 0, 0, 0, 40, 60],
        "six licons in min-width li1 straps (zero side margin), two with \
         partial 40 and 60 dbu margins, all under the uniform 80"
    );
    assert!(
        violations
            .iter()
            .filter(|v| v.rule == "poly.8")
            .all(|v| v.measured < 0 && v.required == 130),
        "gate poly never extends past diff in x, so bbox extension is negative"
    );
}
