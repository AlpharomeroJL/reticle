//! Property tests: the fast [`DrcEngine`] must find *exactly* the same width,
//! spacing, and area violations as a naive `O(n²)` reference checker computed the
//! obvious slow way over randomized rectangle sets.
//!
//! Violations are compared as *sets of locations* per rule, never by order or by
//! object identity: the engine visits shapes in index order while the oracle visits
//! them in insertion order, so only the reported regions must agree.

use std::collections::BTreeSet;

use proptest::prelude::*;
use reticle_drc::DrcEngine;
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, Document, DrawShape, Rule, RuleKind, RuleSet, ShapeKind};

const METAL1: LayerId = LayerId::new(1, 0);
const METAL2: LayerId = LayerId::new(2, 0);

/// Coordinate bound: small so random boxes cluster and genuinely trigger spacing
/// and width violations rather than sitting comfortably far apart.
const BOUND: i32 = 200;

/// A rectangle location as a comparable, ordered tuple.
type LocKey = (i32, i32, i32, i32);

fn loc_key(r: &Rect) -> LocKey {
    (r.min.x, r.min.y, r.max.x, r.max.y)
}

/// The set of violation locations reported for a specific rule name.
fn located(v: &[reticle_model::Violation], rule: &str) -> BTreeSet<LocKey> {
    v.iter()
        .filter(|x| x.rule == rule)
        .map(|x| loc_key(&x.location))
        .collect()
}

// --- Naive O(n^2) reference checkers -----------------------------------------

/// Naive minimum-width: flag every rectangle on `layer` whose smaller side is
/// below `value`. Location is the shape's own box.
fn naive_width(shapes: &[(LayerId, Rect)], layer: LayerId, value: i64) -> BTreeSet<LocKey> {
    shapes
        .iter()
        .filter(|(l, _)| *l == layer)
        .filter(|(_, r)| r.width().min(r.height()) < value)
        .map(|(_, r)| loc_key(r))
        .collect()
}

/// Naive minimum-area: flag every rectangle on `layer` whose area is below `value`.
fn naive_area(shapes: &[(LayerId, Rect)], layer: LayerId, value: i64) -> BTreeSet<LocKey> {
    shapes
        .iter()
        .filter(|(l, _)| *l == layer)
        .filter(|(_, r)| r.area() < value)
        .map(|(_, r)| loc_key(r))
        .collect()
}

/// Exact edge-to-edge gap of two rectangles, floored, mirroring the engine's own
/// helper but written independently as the oracle.
fn gap(a: &Rect, b: &Rect) -> i64 {
    let axis = |a0: i64, a1: i64, b0: i64, b1: i64| -> i64 {
        if b0 > a1 {
            b0 - a1
        } else if a0 > b1 {
            a0 - b1
        } else {
            0
        }
    };
    let dx = axis(
        i64::from(a.min.x),
        i64::from(a.max.x),
        i64::from(b.min.x),
        i64::from(b.max.x),
    );
    let dy = axis(
        i64::from(a.min.y),
        i64::from(a.max.y),
        i64::from(b.min.y),
        i64::from(b.max.y),
    );
    // Floor of the Euclidean corner distance (integer sqrt via f64 is exact for the
    // small BOUND used here).
    let d2 = dx * dx + dy * dy;
    (d2 as f64).sqrt().floor() as i64
}

/// Naive minimum-spacing over all pairs. `cross` selects a two-layer rule; when
/// `None`, both shapes must be on `layer`. Overlapping/touching pairs (gap 0) are
/// never flagged. Location is the union of the two boxes.
fn naive_spacing(
    shapes: &[(LayerId, Rect)],
    layer: LayerId,
    cross: Option<LayerId>,
    value: i64,
) -> BTreeSet<LocKey> {
    let mut out = BTreeSet::new();
    for i in 0..shapes.len() {
        for j in 0..shapes.len() {
            if i >= j {
                continue;
            }
            let (la, ra) = &shapes[i];
            let (lb, rb) = &shapes[j];
            // Select the pair according to the rule's layer configuration.
            let matches = match cross {
                None => *la == layer && *lb == layer,
                Some(other) => (*la == layer && *lb == other) || (*la == other && *lb == layer),
            };
            if !matches {
                continue;
            }
            if ra.intersects(rb) {
                continue; // positive-area overlap is not a spacing case
            }
            let g = gap(ra, rb);
            if g > 0 && g < value {
                out.insert(loc_key(&ra.union(rb)));
            }
        }
    }
    out
}

// --- Strategies --------------------------------------------------------------

/// A rectangle with positive area within `[-BOUND, BOUND]`, on one of two layers.
fn shape_strategy() -> impl Strategy<Value = (LayerId, Rect)> {
    (
        -BOUND..BOUND,
        -BOUND..BOUND,
        1..=40i32,
        1..=40i32,
        prop::bool::ANY,
    )
        .prop_map(|(x, y, w, h, second)| {
            let layer = if second { METAL2 } else { METAL1 };
            (layer, Rect::new(Point::new(x, y), Point::new(x + w, y + h)))
        })
}

/// A small axis-aligned query region within the layout bounds, the sort of
/// rectangle a single local edit dirties.
fn region_strategy() -> impl Strategy<Value = Rect> {
    (-BOUND..BOUND, -BOUND..BOUND, 1..=60i32, 1..=60i32)
        .prop_map(|(x, y, w, h)| Rect::new(Point::new(x, y), Point::new(x + w, y + h)))
}

/// The engine's region filter, replicated as the oracle predicate: a violation is
/// reported for `region` exactly when its location's edge-to-edge gap to `region`
/// is zero (overlapping or merely touching), matching `CellContext::in_region`.
fn touches(location: &Rect, region: &Rect) -> bool {
    gap(location, region) == 0
}

/// Builds a one-cell document from `(layer, rect)` pairs.
fn build_doc(shapes: &[(LayerId, Rect)]) -> Document {
    let mut cell = Cell::new("top");
    cell.shapes = shapes
        .iter()
        .map(|(l, r)| DrawShape::new(*l, ShapeKind::Rect(*r)))
        .collect();
    let mut doc = Document::new();
    doc.insert_cell(cell);
    doc
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    /// Width: engine equals the naive reference, exactly.
    #[test]
    fn width_matches_naive(
        shapes in prop::collection::vec(shape_strategy(), 0..40),
        value in 1..=50i64,
    ) {
        let doc = build_doc(&shapes);
        let engine = DrcEngine::new(vec![Rule {
            name: "w".into(),
            kind: RuleKind::Width,
            layer: METAL1,
            other_layer: None,
            value,
        }]);
        let got = located(&engine.check_cell(&doc, "top"), "w");
        let expected = naive_width(&shapes, METAL1, value);
        prop_assert_eq!(got, expected);
    }

    /// Area: engine equals the naive reference, exactly.
    #[test]
    fn area_matches_naive(
        shapes in prop::collection::vec(shape_strategy(), 0..40),
        value in 1..=800i64,
    ) {
        let doc = build_doc(&shapes);
        let engine = DrcEngine::new(vec![Rule {
            name: "a".into(),
            kind: RuleKind::Area,
            layer: METAL1,
            other_layer: None,
            value,
        }]);
        let got = located(&engine.check_cell(&doc, "top"), "a");
        let expected = naive_area(&shapes, METAL1, value);
        prop_assert_eq!(got, expected);
    }

    /// Single-layer spacing: engine equals the naive all-pairs reference, exactly.
    #[test]
    fn spacing_matches_naive(
        shapes in prop::collection::vec(shape_strategy(), 0..40),
        value in 1..=30i64,
    ) {
        let doc = build_doc(&shapes);
        let engine = DrcEngine::new(vec![Rule {
            name: "s".into(),
            kind: RuleKind::Spacing,
            layer: METAL1,
            other_layer: None,
            value,
        }]);
        let got = located(&engine.check_cell(&doc, "top"), "s");
        let expected = naive_spacing(&shapes, METAL1, None, value);
        prop_assert_eq!(got, expected);
    }

    /// Cross-layer spacing: engine equals the naive all-pairs reference, exactly.
    #[test]
    fn cross_spacing_matches_naive(
        shapes in prop::collection::vec(shape_strategy(), 0..40),
        value in 1..=30i64,
    ) {
        let doc = build_doc(&shapes);
        let engine = DrcEngine::new(vec![Rule {
            name: "s2".into(),
            kind: RuleKind::Spacing,
            layer: METAL1,
            other_layer: Some(METAL2),
            value,
        }]);
        let got = located(&engine.check_cell(&doc, "top"), "s2");
        let expected = naive_spacing(&shapes, METAL1, Some(METAL2), value);
        prop_assert_eq!(got, expected);
    }

    /// A region covering the whole layout reproduces the full-cell result for all
    /// three rule kinds at once.
    #[test]
    fn region_covering_all_equals_full(
        shapes in prop::collection::vec(shape_strategy(), 0..40),
    ) {
        let doc = build_doc(&shapes);
        let engine = DrcEngine::new(vec![
            Rule { name: "w".into(), kind: RuleKind::Width, layer: METAL1, other_layer: None, value: 12 },
            Rule { name: "s".into(), kind: RuleKind::Spacing, layer: METAL1, other_layer: None, value: 15 },
            Rule { name: "a".into(), kind: RuleKind::Area, layer: METAL1, other_layer: None, value: 200 },
        ]);
        let full = engine.check_cell(&doc, "top");
        let big = Rect::new(Point::new(-10_000, -10_000), Point::new(10_000, 10_000));
        let region = engine.check_region(&doc, "top", big);

        for name in ["w", "s", "a"] {
            prop_assert_eq!(
                located(&region, name),
                located(&full, name),
                "rule {} under covering region",
                name
            );
        }
    }

    /// The prepared incremental path over a random small region equals, per rule,
    /// the full-cell pass filtered to violations whose location touches that region.
    ///
    /// This pins the local optimization (query the prepared index around the region,
    /// grown by the max threshold; check only local candidates; dedup pairs within
    /// the local set) against the naive full pass as the oracle, across every rule
    /// kind at once, including the same-layer pairwise dedup and cross-region
    /// partners that the region expansion is designed to catch.
    #[test]
    fn prepared_region_equals_filtered_full(
        shapes in prop::collection::vec(shape_strategy(), 0..40),
        region in region_strategy(),
    ) {
        let doc = build_doc(&shapes);
        // A rule set spanning single-shape, same-layer pairwise, cross-layer, and
        // two-layer rules so every code path is under test at once. Thresholds are
        // varied so the max-threshold region expansion is genuinely exercised.
        let engine = DrcEngine::new(vec![
            Rule { name: "w".into(), kind: RuleKind::Width, layer: METAL1, other_layer: None, value: 12 },
            Rule { name: "a".into(), kind: RuleKind::Area, layer: METAL1, other_layer: None, value: 200 },
            Rule { name: "s".into(), kind: RuleKind::Spacing, layer: METAL1, other_layer: None, value: 15 },
            Rule { name: "n".into(), kind: RuleKind::Notch, layer: METAL2, other_layer: None, value: 20 },
            Rule { name: "x".into(), kind: RuleKind::Spacing, layer: METAL1, other_layer: Some(METAL2), value: 25 },
            Rule { name: "enc".into(), kind: RuleKind::Enclosure, layer: METAL1, other_layer: Some(METAL2), value: 8 },
            Rule { name: "ext".into(), kind: RuleKind::Extension, layer: METAL1, other_layer: Some(METAL2), value: 8 },
        ]);

        let full = engine.check_cell(&doc, "top");
        let prepared = engine.prepare(&doc, "top");
        let local = prepared.check_region(region);

        // The oracle: full-pass violations whose location touches the region, keyed
        // per rule to keep the failure message specific.
        for name in ["w", "a", "s", "n", "x", "enc", "ext"] {
            let expected: BTreeSet<LocKey> = full
                .iter()
                .filter(|v| v.rule == name && touches(&v.location, &region))
                .map(|v| loc_key(&v.location))
                .collect();
            prop_assert_eq!(
                located(&local, name),
                expected,
                "rule {} under local region {:?}",
                name,
                region
            );
        }
    }
}
