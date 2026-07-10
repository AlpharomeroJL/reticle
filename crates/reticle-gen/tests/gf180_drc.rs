//! Per-rule fixture tests for the GF180MCU DRC subset, plus the tech/toml provenance
//! check.
//!
//! Mirrors `crates/reticle-drc/tests/sky130.rs`'s two-way seeded-violation shape (a
//! fixture built to violate exactly one rule, and a clean fixture that satisfies the
//! whole subset), but the rules come from parsing the committed `tech/gf180.tech`
//! (mirroring `second_pdk.rs`'s `sg13g2_pdk()`), not a hand-coded Rust rule table. That
//! matters mechanically: `reticle_io::parse_technology` derives each `Rule::name` as
//! `"{kind}_{layer}_{datatype}"` (see `crates/reticle-io/src/technology.rs`), not the
//! PDK's human rule id ("M1.1", "V1.3a", ...), so these tests assert on the structured
//! `Violation` fields (`kind`, `layer`, `other_layer`, `measured`, `required`) rather
//! than a rule-name string. Each test's doc comment names the real GF180MCU rule id
//! (see `tech/gf180-drc-subset.toml` for the full citation) for traceability.
//!
//! Coordinates are database units, 1 dbu = 1 nm, matching the committed
//! `tech/gf180.tech` (`dbu_per_micron 1000`). Every value here is the GF180MCU 3.3V
//! variant; see the gf180 lane RESULT.md for the citation table.

use reticle_drc::DrcEngine;
use reticle_geometry::{LayerId, Point, Rect};
use reticle_io::parse_technology;
use reticle_model::{Cell, Document, DrawShape, RuleKind, RuleSet, ShapeKind, Violation};

/// The committed GF180MCU technology file (layers, stack, and the inline DRC subset).
const GF180_TECH: &str = include_str!("../../../tech/gf180.tech");
/// The committed GF180MCU DRC subset, the cited source of record for the rules.
const GF180_DRC_SUBSET: &str = include_str!("../../../tech/gf180-drc-subset.toml");

// GF180MCU GDS layers used by the fixtures (see tech/gf180.tech).
const POLY2: LayerId = LayerId::new(30, 0);
const COMP: LayerId = LayerId::new(22, 0);
const CONTACT: LayerId = LayerId::new(33, 0);
const METAL1: LayerId = LayerId::new(34, 0);
const VIA1: LayerId = LayerId::new(35, 0);
const METAL2: LayerId = LayerId::new(36, 0);

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

/// Runs the whole loaded GF180MCU subset, parsed fresh from the committed `.tech`
/// file, over `shapes`.
fn gf180_check(shapes: Vec<DrawShape>) -> Vec<Violation> {
    let tech = parse_technology(GF180_TECH).expect("committed tech/gf180.tech must parse");
    let doc = doc_with(shapes);
    DrcEngine::new(tech.rules).check_cell(&doc, "top")
}

/// Asserts the layout broke exactly one rule and returns it.
fn assert_single(mut violations: Vec<Violation>) -> Violation {
    assert_eq!(
        violations.len(),
        1,
        "expected exactly one violation, got {violations:?}"
    );
    violations.remove(0)
}

/// Asserts the layout is clean against the entire subset.
fn assert_clean(violations: &[Violation]) {
    assert!(
        violations.is_empty(),
        "expected a clean layout, got {violations:?}"
    );
}

// --- PL.1_3.3V: min Poly2 interconnect width (180) ------------------------------

#[test]
fn gf180_pl_1_narrow_poly2_flagged() {
    // 170 wide: 10 dbu under the 180 minimum.
    let v = assert_single(gf180_check(vec![rect_shape(POLY2, 0, 0, 170, 700)]));
    assert_eq!(v.kind, RuleKind::Width);
    assert_eq!(v.layer, POLY2);
    assert_eq!(v.measured, 170);
    assert_eq!(v.required, 180);
}

#[test]
fn gf180_pl_1_wide_poly2_clean() {
    assert_clean(&gf180_check(vec![rect_shape(POLY2, 0, 0, 300, 300)]));
}

// --- DF.1a_3.3V: min COMP width (220) --------------------------------------------

#[test]
fn gf180_df_1a_narrow_comp_flagged() {
    // 200 wide: 20 dbu under the 220 minimum.
    let v = assert_single(gf180_check(vec![rect_shape(COMP, 0, 0, 200, 700)]));
    assert_eq!(v.kind, RuleKind::Width);
    assert_eq!(v.layer, COMP);
    assert_eq!(v.measured, 200);
    assert_eq!(v.required, 220);
}

#[test]
fn gf180_df_1a_wide_comp_clean() {
    assert_clean(&gf180_check(vec![rect_shape(COMP, 0, 0, 300, 300)]));
}

// --- M1.1: min Metal1 width (230) -------------------------------------------------

#[test]
fn gf180_m1_1_narrow_metal1_flagged() {
    // 200 wide: 30 dbu under the 230 minimum.
    let v = assert_single(gf180_check(vec![rect_shape(METAL1, 0, 0, 200, 700)]));
    assert_eq!(v.kind, RuleKind::Width);
    assert_eq!(v.layer, METAL1);
    assert_eq!(v.measured, 200);
    assert_eq!(v.required, 230);
}

#[test]
fn gf180_m1_1_wide_metal1_clean() {
    assert_clean(&gf180_check(vec![rect_shape(METAL1, 0, 0, 300, 300)]));
}

// --- M1.2a: min Metal1 spacing to Metal1 (230) ------------------------------------

#[test]
fn gf180_m1_2a_close_metal1_pair_flagged() {
    // Two DRC-clean metal1 blocks 150 apart: 80 dbu under the 230 minimum.
    let v = assert_single(gf180_check(vec![
        rect_shape(METAL1, 0, 0, 300, 300),
        rect_shape(METAL1, 450, 0, 750, 300),
    ]));
    assert_eq!(v.kind, RuleKind::Spacing);
    assert_eq!(v.layer, METAL1);
    assert_eq!(v.measured, 150);
    assert_eq!(v.required, 230);
}

#[test]
fn gf180_m1_2a_spaced_metal1_pair_clean() {
    // Gap of exactly 230: the minimum itself must pass.
    assert_clean(&gf180_check(vec![
        rect_shape(METAL1, 0, 0, 300, 300),
        rect_shape(METAL1, 530, 0, 830, 300),
    ]));
}

// --- M2.1: min Metal2 width (280) -------------------------------------------------

#[test]
fn gf180_m2_1_narrow_metal2_flagged() {
    // 250 wide: 30 dbu under the 280 minimum.
    let v = assert_single(gf180_check(vec![rect_shape(METAL2, 0, 0, 250, 700)]));
    assert_eq!(v.kind, RuleKind::Width);
    assert_eq!(v.layer, METAL2);
    assert_eq!(v.measured, 250);
    assert_eq!(v.required, 280);
}

#[test]
fn gf180_m2_1_wide_metal2_clean() {
    assert_clean(&gf180_check(vec![rect_shape(METAL2, 0, 0, 350, 350)]));
}

// --- M2.2a: min Metal2 spacing to Metal2 (280) ------------------------------------

#[test]
fn gf180_m2_2a_close_metal2_pair_flagged() {
    // Two DRC-clean metal2 blocks 180 apart: 100 dbu under the 280 minimum.
    let v = assert_single(gf180_check(vec![
        rect_shape(METAL2, 0, 0, 350, 350),
        rect_shape(METAL2, 530, 0, 880, 350),
    ]));
    assert_eq!(v.kind, RuleKind::Spacing);
    assert_eq!(v.layer, METAL2);
    assert_eq!(v.measured, 180);
    assert_eq!(v.required, 280);
}

#[test]
fn gf180_m2_2a_spaced_metal2_pair_clean() {
    // Gap of exactly 280.
    assert_clean(&gf180_check(vec![
        rect_shape(METAL2, 0, 0, 350, 350),
        rect_shape(METAL2, 630, 0, 980, 350),
    ]));
}

// --- CO.1: exact Contact size, min encoded as a width floor (220) ----------------

#[test]
fn gf180_co_1_undersized_contact_flagged() {
    // 200 x 200: 20 dbu under the 220 minimum.
    let v = assert_single(gf180_check(vec![rect_shape(CONTACT, 0, 0, 200, 200)]));
    assert_eq!(v.kind, RuleKind::Width);
    assert_eq!(v.layer, CONTACT);
    assert_eq!(v.measured, 200);
    assert_eq!(v.required, 220);
}

#[test]
fn gf180_co_1_sized_contact_clean() {
    // Exactly 220 x 220, the min=max legal size.
    assert_clean(&gf180_check(vec![rect_shape(CONTACT, 0, 0, 220, 220)]));
}

// --- CO.2a: min Contact spacing to Contact (250) ----------------------------------

#[test]
fn gf180_co_2a_close_contact_pair_flagged() {
    // Two legally-sized (220 x 220) contacts 150 apart: 100 dbu under the 250 minimum.
    let v = assert_single(gf180_check(vec![
        rect_shape(CONTACT, 0, 0, 220, 220),
        rect_shape(CONTACT, 370, 0, 590, 220),
    ]));
    assert_eq!(v.kind, RuleKind::Spacing);
    assert_eq!(v.layer, CONTACT);
    assert_eq!(v.measured, 150);
    assert_eq!(v.required, 250);
}

#[test]
fn gf180_co_2a_spaced_contact_pair_clean() {
    // Gap of exactly 250.
    assert_clean(&gf180_check(vec![
        rect_shape(CONTACT, 0, 0, 220, 220),
        rect_shape(CONTACT, 470, 0, 690, 220),
    ]));
}

// --- V1.1: exact Via1 size, min encoded as a width floor (260) -------------------

#[test]
fn gf180_v1_1_undersized_via_flagged() {
    // 240 x 240 via, correctly enclosed by metal1 so only the size rule fires.
    let v = assert_single(gf180_check(vec![
        rect_shape(VIA1, 0, 0, 240, 240),
        rect_shape(METAL1, -50, -50, 290, 290),
    ]));
    assert_eq!(v.kind, RuleKind::Width);
    assert_eq!(v.layer, VIA1);
    assert_eq!(v.measured, 240);
    assert_eq!(v.required, 260);
}

#[test]
fn gf180_v1_1_sized_via_clean() {
    // Exactly 260 x 260, fully enclosed by metal1.
    assert_clean(&gf180_check(vec![
        rect_shape(VIA1, 0, 0, 260, 260),
        rect_shape(METAL1, -50, -50, 310, 310),
    ]));
}

// --- V1.2a: min Via1 spacing to Via1 (260) ----------------------------------------

#[test]
fn gf180_v1_2a_close_via_pair_flagged() {
    // Two legally-sized (260 x 260) vias 140 apart, both enclosed by one metal1 block
    // (so metal1-to-metal1 spacing never engages): 120 dbu under the 260 minimum.
    let v = assert_single(gf180_check(vec![
        rect_shape(VIA1, 0, 0, 260, 260),
        rect_shape(VIA1, 400, 0, 660, 260),
        rect_shape(METAL1, -50, -50, 710, 310),
    ]));
    assert_eq!(v.kind, RuleKind::Spacing);
    assert_eq!(v.layer, VIA1);
    assert_eq!(v.measured, 140);
    assert_eq!(v.required, 260);
}

#[test]
fn gf180_v1_2a_spaced_via_pair_clean() {
    // Gap of exactly 260, both vias enclosed by one metal1 block.
    assert_clean(&gf180_check(vec![
        rect_shape(VIA1, 0, 0, 260, 260),
        rect_shape(VIA1, 520, 0, 780, 260),
        rect_shape(METAL1, -50, -50, 830, 310),
    ]));
}

// --- V1.3a: Via1 enclosed by Metal1, zero-margin containment (0) -----------------
//
// Source: via1.drc `v13a_l1 = via1.not(metal1)` -- any via1 area not covered by
// metal1 is flagged. There is no numeric margin in the source beyond exact
// containment, so the rule's `value_dbu` is 0. reticle-drc's enclosure check still
// makes this a meaningful two-way test: a via with no metal1 candidate that fully
// contains it is *always* flagged (regardless of the threshold, via the "not
// enclosed at all" sentinel), while a fully-contained via always passes (its
// margin is never negative once contained), so 0 correctly distinguishes "some
// via1 pokes outside metal1" from "via1 sits entirely inside metal1".

#[test]
fn gf180_v1_3a_unenclosed_via_flagged() {
    // metal1 covers the via on three sides but falls 20 short of the top edge
    // (240 tall under a 260-tall via), so the via is not fully contained.
    let v = assert_single(gf180_check(vec![
        rect_shape(VIA1, 0, 0, 260, 260),
        rect_shape(METAL1, 0, 0, 260, 240),
    ]));
    assert_eq!(v.kind, RuleKind::Enclosure);
    assert_eq!(v.layer, VIA1);
    assert_eq!(v.other_layer, Some(METAL1));
    assert_eq!(v.measured, i64::MIN);
    assert_eq!(v.required, 0);
}

#[test]
fn gf180_v1_3a_enclosed_via_clean() {
    // metal1 fully contains the via with margin to spare on every side.
    assert_clean(&gf180_check(vec![
        rect_shape(VIA1, 0, 0, 260, 260),
        rect_shape(METAL1, -50, -50, 310, 310),
    ]));
}

// --- Whole-subset sanity ----------------------------------------------------------

#[test]
fn gf180_multi_layer_clean_snippet_passes_the_whole_subset() {
    // A little legal stack, each item far enough from every other that no spacing
    // rule engages: a poly2 strip, a comp region, a legally-spaced contact pair, a
    // standalone metal1 block, a standalone metal2 block, and a via properly
    // enclosed by its own metal1 block.
    assert_clean(&gf180_check(vec![
        rect_shape(POLY2, 0, 0, 300, 300),
        rect_shape(COMP, 1000, 0, 1300, 300),
        rect_shape(CONTACT, 2000, 0, 2220, 220),
        rect_shape(CONTACT, 2500, 0, 2720, 220),
        rect_shape(METAL1, 3000, 0, 3400, 400),
        rect_shape(METAL2, 4000, 0, 4400, 400),
        rect_shape(VIA1, 5000, 0, 5260, 260),
        rect_shape(METAL1, 4950, -50, 5310, 310),
    ]));
}

// --- Provenance: parses, and the inline .tech rules match the cited .toml --------

/// The committed `gf180.tech` must parse and carry the expected layer/rule counts, so
/// a future edit that breaks the grammar or silently drops a layer/rule fails loudly
/// here instead of surfacing only as missing DRC coverage.
#[test]
fn gf180_tech_parses() {
    let tech = parse_technology(GF180_TECH).expect("committed tech/gf180.tech must parse");
    assert_eq!(tech.name, "gf180");
    assert_eq!(tech.dbu_per_micron, 1000);
    assert_eq!(tech.layers.len(), 9, "expected the 9-layer GF180MCU subset");
    assert_eq!(
        tech.rules.len(),
        11,
        "expected the 11-rule GF180MCU DRC subset"
    );
}

/// The inline DRC rules in `gf180.tech` must match the cited `gf180-drc-subset.toml`
/// exactly (as sets of kind/layer/other/value), so the two committed representations
/// of the subset cannot silently diverge. Copied from `second_pdk.rs`'s
/// `sg13g2_tech_rules_match_drc_subset` matcher.
#[test]
fn gf180_tech_rules_match_drc_subset() {
    use std::collections::BTreeSet;

    #[derive(serde::Deserialize)]
    struct RuleFile {
        rule: Vec<RawRule>,
    }
    #[derive(serde::Deserialize)]
    struct RawRule {
        kind: String,
        layer: [u16; 2],
        other_layer: Option<[u16; 2]>,
        value_dbu: i64,
    }

    // Canonical tuple: (kind, layer, datatype, other?, value).
    type Key = (String, u16, u16, Option<(u16, u16)>, i64);

    let tech = parse_technology(GF180_TECH).expect("parse gf180.tech");
    let from_tech: BTreeSet<Key> = tech
        .rules
        .iter()
        .map(|r| {
            (
                format!("{:?}", r.kind).to_lowercase(),
                r.layer.layer,
                r.layer.datatype,
                r.other_layer.map(|l| (l.layer, l.datatype)),
                r.value,
            )
        })
        .collect();

    let subset: RuleFile = toml::from_str(GF180_DRC_SUBSET).expect("parse gf180 drc subset");
    let from_toml: BTreeSet<Key> = subset
        .rule
        .iter()
        .map(|r| {
            (
                r.kind.to_lowercase(),
                r.layer[0],
                r.layer[1],
                r.other_layer.map(|l| (l[0], l[1])),
                r.value_dbu,
            )
        })
        .collect();

    assert_eq!(
        from_tech, from_toml,
        "gf180.tech rules and gf180-drc-subset.toml diverge"
    );
}
