//! Pure geometric diff between two Reticle [`Document`] snapshots.
//!
//! [`diff`] compares two documents over their flattened top cells and reports, per
//! layer, the shapes that were ADDED (present in `after`, absent in `before`),
//! REMOVED (present in `before`, absent in `after`), and CHANGED (see below). The
//! result is a plain data structure ([`LayoutDiff`]) that a viewer can paint as a
//! colored overlay: added green, removed red, changed amber.
//!
//! # Semantics
//!
//! The comparison is a multiset difference keyed by `(layer, exact geometry)`.
//! Two shapes match only if they share a layer and are geometrically identical
//! (same rectangle, polygon ring, or path). Duplicates are counted: if `after`
//! carries three identical rectangles where `before` carried one, two are
//! reported as added. Because the comparison runs on the *flattened* top cell,
//! hierarchy is expanded first, so a diff sees leaf geometry regardless of how the
//! two documents structured their cells.
//!
//! `changed` (a shape whose anchor is stable but whose extent moved) is deferred
//! in v1: distinguishing a moved/resized shape from an independent add plus remove
//! is a fuzzy match, and emitting it wrongly is worse than not emitting it. v1
//! therefore reports every geometric difference as an add or a remove and always
//! leaves [`LayoutDiff::changed`] empty. The field exists so the overlay and any
//! future revision keep a stable shape.
//!
//! # Example
//!
//! ```
//! use reticle_geometry::{LayerId, Point, Rect};
//! use reticle_model::{Cell, Document, DrawShape, ShapeKind};
//!
//! let metal = LayerId::new(68, 20);
//! fn doc_with(rects: &[Rect], layer: LayerId) -> Document {
//!     let mut cell = Cell::new("top");
//!     for r in rects {
//!         cell.shapes.push(DrawShape::new(layer, ShapeKind::Rect(*r)));
//!     }
//!     let mut doc = Document::new();
//!     doc.insert_cell(cell);
//!     doc.set_top_cells(vec!["top".into()]);
//!     doc
//! }
//!
//! let a = Rect::new(Point::new(0, 0), Point::new(10, 10));
//! let b = Rect::new(Point::new(20, 0), Point::new(30, 10));
//! let before = doc_with(&[a], metal);
//! let after = doc_with(&[a, b], metal);
//!
//! let d = reticle_diff::diff(&before, &after);
//! assert_eq!(d.added.len(), 1); // b is new
//! assert!(d.removed.is_empty()); // a is unchanged
//! assert_eq!(d.added[0].rect, b);
//! ```

use std::collections::HashMap;

use reticle_geometry::{LayerId, Path, Polygon, Rect, Shape};
use reticle_model::{Document, DrawShape, ShapeKind};

/// A single shape reported by a [`LayoutDiff`].
///
/// Carries just what an overlay needs to paint the shape and name where it lives:
/// the layer, the shape's axis-aligned bounding box (in DBU), and a label naming
/// the flattened top cell the shape belongs to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DiffShape {
    /// The layer/datatype the shape is drawn on.
    pub layer: LayerId,
    /// The shape's axis-aligned bounding box, in DBU. This is what the overlay
    /// paints; the exact geometry is what the diff matched on.
    pub rect: Rect,
    /// A label naming where the shape lives: the flattened top cell's name.
    pub label: String,
}

/// The geometric difference between two documents, partitioned by change kind.
///
/// Each vector is sorted for a stable, reproducible order: by layer, then by the
/// bounding box corners. See the [crate-level docs](crate) for the matching
/// semantics and why `changed` is deferred.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct LayoutDiff {
    /// Shapes present in `after` but not `before` (paint green).
    pub added: Vec<DiffShape>,
    /// Shapes present in `before` but not `after` (paint red).
    pub removed: Vec<DiffShape>,
    /// Shapes whose extent changed in place (paint amber). Always empty in v1;
    /// see the [crate-level docs](crate).
    pub changed: Vec<DiffShape>,
}

impl LayoutDiff {
    /// Returns `true` when the two documents are geometrically identical: nothing
    /// added, removed, or changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    /// The number of added shapes.
    #[must_use]
    pub fn added_count(&self) -> usize {
        self.added.len()
    }

    /// The number of removed shapes.
    #[must_use]
    pub fn removed_count(&self) -> usize {
        self.removed.len()
    }

    /// The number of changed shapes (always `0` in v1).
    #[must_use]
    pub fn changed_count(&self) -> usize {
        self.changed.len()
    }
}

/// An exact, hashable identity for a shape's geometry.
///
/// `reticle_model::ShapeKind` is only `PartialEq`, so it cannot key a hash map
/// directly. Every variant it wraps (`Rect`, `Polygon`, `Path`) is `Eq + Hash`,
/// so this mirror carries the same data and derives the traits a multiset needs.
#[derive(Clone, PartialEq, Eq, Hash)]
enum GeometryKey {
    Rect(Rect),
    Polygon(Polygon),
    Path(Path),
}

/// The full match key for a flattened shape: its layer plus its exact geometry.
type ShapeKey = (LayerId, GeometryKey);

/// Builds the exact match key for a flattened shape.
fn key_of(shape: &DrawShape) -> ShapeKey {
    let geom = match &shape.kind {
        ShapeKind::Rect(r) => GeometryKey::Rect(*r),
        ShapeKind::Polygon(p) => GeometryKey::Polygon(p.clone()),
        ShapeKind::Path(p) => GeometryKey::Path(p.clone()),
    };
    (shape.layer, geom)
}

/// The name of the first declared top cell, if any.
fn top_cell(doc: &Document) -> Option<&str> {
    doc.top_cells().first().map(String::as_str)
}

/// A multiset of a document's flattened shapes, keyed by `(layer, geometry)`.
///
/// Each entry keeps a running count and one representative [`DiffShape`] (the
/// bounding box and top-cell label) so the diff can emit the right number of
/// reported shapes without re-deriving geometry.
fn multiset(doc: &Document) -> HashMap<ShapeKey, (usize, DiffShape)> {
    let mut set: HashMap<ShapeKey, (usize, DiffShape)> = HashMap::new();
    let Some(top) = top_cell(doc) else {
        return set;
    };
    let label = top.to_owned();
    for shape in doc.flatten(top) {
        let key = key_of(&shape);
        let entry = set.entry(key).or_insert_with(|| {
            (
                0,
                DiffShape {
                    layer: shape.layer,
                    rect: shape.bounding_box(),
                    label: label.clone(),
                },
            )
        });
        entry.0 += 1;
    }
    set
}

/// Sort key giving a stable, reproducible order for reported shapes.
fn sort_key(s: &DiffShape) -> (u16, u16, i32, i32, i32, i32) {
    (
        s.layer.layer,
        s.layer.datatype,
        s.rect.min.x,
        s.rect.min.y,
        s.rect.max.x,
        s.rect.max.y,
    )
}

/// Computes the geometric diff between two document snapshots.
///
/// Compares the flattened top cells of `before` and `after` as multisets keyed by
/// `(layer, exact geometry)`. A shape in `after` with no match in `before` is
/// added; a shape in `before` with no match in `after` is removed; matched shapes
/// (including matched duplicates) are reported as neither. See the
/// [crate-level docs](crate) for the full semantics, including why `changed` is
/// always empty in v1.
///
/// A document that declares no top cell contributes no shapes, so
/// `diff(&Document::new(), after)` reports every shape in `after` as added.
#[must_use]
pub fn diff(before: &Document, after: &Document) -> LayoutDiff {
    let before_ms = multiset(before);
    let after_ms = multiset(after);

    let mut added = Vec::new();
    for (key, (after_n, rep)) in &after_ms {
        let before_n = before_ms.get(key).map_or(0, |(n, _)| *n);
        for _ in 0..after_n.saturating_sub(before_n) {
            added.push(rep.clone());
        }
    }

    let mut removed = Vec::new();
    for (key, (before_n, rep)) in &before_ms {
        let after_n = after_ms.get(key).map_or(0, |(n, _)| *n);
        for _ in 0..before_n.saturating_sub(after_n) {
            removed.push(rep.clone());
        }
    }

    added.sort_by_key(sort_key);
    removed.sort_by_key(sort_key);

    LayoutDiff {
        added,
        removed,
        changed: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{DiffShape, diff};
    use proptest::prelude::*;
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{Cell, Document, DrawShape, ShapeKind};

    const L: LayerId = LayerId::new(68, 20);

    fn doc_from_rects(rects: &[Rect]) -> Document {
        let mut cell = Cell::new("top");
        for r in rects {
            cell.shapes.push(DrawShape::new(L, ShapeKind::Rect(*r)));
        }
        let mut doc = Document::new();
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["top".into()]);
        doc
    }

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect::new(Point::new(x, y), Point::new(x + w, y + h))
    }

    #[test]
    fn identical_docs_diff_empty() {
        let doc = doc_from_rects(&[rect(0, 0, 4, 6), rect(10, 10, 2, 2)]);
        assert!(diff(&doc, &doc).is_empty());
    }

    #[test]
    fn added_shape_is_reported_once() {
        let a = rect(0, 0, 4, 6);
        let b = rect(20, 0, 4, 6);
        let before = doc_from_rects(&[a]);
        let after = doc_from_rects(&[a, b]);
        let d = diff(&before, &after);
        assert_eq!(d.added_count(), 1);
        assert_eq!(d.removed_count(), 0);
        assert_eq!(d.added[0].rect, b);
        assert_eq!(d.added[0].layer, L);
        assert_eq!(d.added[0].label, "top");
    }

    #[test]
    fn removed_shape_is_reported_once() {
        let a = rect(0, 0, 4, 6);
        let b = rect(20, 0, 4, 6);
        let before = doc_from_rects(&[a, b]);
        let after = doc_from_rects(&[a]);
        let d = diff(&before, &after);
        assert_eq!(d.removed_count(), 1);
        assert_eq!(d.added_count(), 0);
        assert_eq!(d.removed[0].rect, b);
    }

    #[test]
    fn duplicates_are_counted() {
        let a = rect(0, 0, 4, 6);
        // before has one copy, after has three: exactly two added.
        let before = doc_from_rects(&[a]);
        let after = doc_from_rects(&[a, a, a]);
        let d = diff(&before, &after);
        assert_eq!(d.added_count(), 2);
        assert_eq!(d.removed_count(), 0);
    }

    #[test]
    fn changed_is_always_empty_in_v1() {
        // A same-anchor resize is reported as remove + add, not changed.
        let before = doc_from_rects(&[rect(0, 0, 4, 4)]);
        let after = doc_from_rects(&[rect(0, 0, 8, 8)]);
        let d = diff(&before, &after);
        assert_eq!(d.changed_count(), 0);
        assert_eq!(d.added_count(), 1);
        assert_eq!(d.removed_count(), 1);
    }

    /// Flattened shape count of a document (the oracle's notion of "size").
    fn shape_count(doc: &Document) -> usize {
        doc.top_cells()
            .first()
            .map_or(0, |top| doc.flatten(top).len())
    }

    prop_compose! {
        fn arb_rect()(x in 0i32..40, y in 0i32..40, w in 1i32..20, h in 1i32..20) -> Rect {
            rect(x, y, w, h)
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        /// Reflexivity: a document diffed against itself is empty.
        #[test]
        fn diff_self_is_empty(rects in prop::collection::vec(arb_rect(), 0..8)) {
            let doc = doc_from_rects(&rects);
            prop_assert!(diff(&doc, &doc).is_empty());
        }

        /// From nothing, every shape in `after` is added and nothing is removed.
        #[test]
        fn diff_empty_to_doc_is_all_added(rects in prop::collection::vec(arb_rect(), 0..8)) {
            let after = doc_from_rects(&rects);
            let d = diff(&Document::new(), &after);
            prop_assert_eq!(d.added_count(), shape_count(&after));
            prop_assert_eq!(d.removed_count(), 0);
            prop_assert_eq!(d.changed_count(), 0);
        }

        /// To nothing, every shape in `before` is removed and nothing is added.
        #[test]
        fn diff_doc_to_empty_is_all_removed(rects in prop::collection::vec(arb_rect(), 0..8)) {
            let before = doc_from_rects(&rects);
            let d = diff(&before, &Document::new());
            prop_assert_eq!(d.removed_count(), shape_count(&before));
            prop_assert_eq!(d.added_count(), 0);
            prop_assert_eq!(d.changed_count(), 0);
        }

        /// The single-insertion oracle: appending one rectangle to a document
        /// yields exactly one added shape and zero removed, whatever the base
        /// document and whatever the inserted rectangle (even a duplicate: the
        /// multiset count still rises by exactly one).
        #[test]
        fn single_insertion_yields_one_added(
            base in prop::collection::vec(arb_rect(), 0..8),
            inserted in arb_rect(),
        ) {
            let before = doc_from_rects(&base);
            let mut after_rects = base.clone();
            after_rects.push(inserted);
            let after = doc_from_rects(&after_rects);

            let d = diff(&before, &after);
            prop_assert_eq!(d.added_count(), 1);
            prop_assert_eq!(d.removed_count(), 0);
            let want = DiffShape { layer: L, rect: inserted, label: "top".into() };
            prop_assert_eq!(&d.added[0], &want);
        }
    }
}
