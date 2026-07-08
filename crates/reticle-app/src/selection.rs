//! The shape-selection model and the layer query/filter.
//!
//! Selection is a set of indices into the flattened scene shape list (the same
//! indices [`crate::culling::SceneIndex`] returns). The model here is pure set
//! bookkeeping plus two selection *builders* that the canvas and query bar drive:
//!
//! * [`shapes_in_rect`], the shapes fully enclosed by a rubber-band rectangle.
//! * [`shapes_on_layer`], every shape on a given layer, for the "select by layer"
//!   query bar.
//!
//! Keeping this window-free means selection behavior is unit-testable and the egui
//! layer only has to translate pixels to world rectangles.

use reticle_geometry::{LayerId, Rect, Shape};
use reticle_model::DrawShape;
use std::collections::BTreeSet;

/// A set of selected shape indices into the flattened scene list.
///
/// A [`BTreeSet`] keeps the selection ordered and deduplicated, so iteration and
/// equality are deterministic for tests and stable highlight drawing.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Selection {
    indices: BTreeSet<usize>,
}

impl Selection {
    /// Creates an empty selection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether nothing is selected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// The number of selected shapes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.indices.len()
    }

    /// Whether shape `index` is selected.
    #[must_use]
    pub fn contains(&self, index: usize) -> bool {
        self.indices.contains(&index)
    }

    /// The selected indices in ascending order.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.indices.iter().copied()
    }

    /// Clears the selection.
    pub fn clear(&mut self) {
        self.indices.clear();
    }

    /// Replaces the selection with exactly `indices`.
    pub fn set<I: IntoIterator<Item = usize>>(&mut self, indices: I) {
        self.indices = indices.into_iter().collect();
    }

    /// Selects a single shape, replacing the current selection.
    pub fn select_one(&mut self, index: usize) {
        self.indices.clear();
        self.indices.insert(index);
    }

    /// Adds `index` to the selection (a shift/ctrl-click style additive pick).
    pub fn add(&mut self, index: usize) {
        self.indices.insert(index);
    }

    /// Toggles `index` in the selection, returning `true` if it is now selected.
    pub fn toggle(&mut self, index: usize) -> bool {
        if self.indices.contains(&index) {
            self.indices.remove(&index);
            false
        } else {
            self.indices.insert(index);
            true
        }
    }

    /// Unions `indices` into the selection (used by the query bar to add matches).
    pub fn extend<I: IntoIterator<Item = usize>>(&mut self, indices: I) {
        self.indices.extend(indices);
    }

    /// Removes `indices` from the selection (an alt-drag subtractive marquee, item 45).
    pub fn subtract<I: IntoIterator<Item = usize>>(&mut self, indices: I) {
        for index in indices {
            self.indices.remove(&index);
        }
    }
}

/// Returns the indices of every shape whose bounding box is fully contained in
/// `rect`, the shapes captured by a rubber-band selection.
///
/// Full containment (rather than mere intersection) matches the usual layout-editor
/// convention that a rubber band grabs only shapes it completely encloses.
#[must_use]
pub fn shapes_in_rect(shapes: &[DrawShape], rect: Rect) -> Vec<usize> {
    shapes
        .iter()
        .enumerate()
        .filter(|(_, s)| contains_rect(&rect, &s.bounding_box()))
        .map(|(i, _)| i)
        .collect()
}

/// Like [`shapes_in_rect`], but when `layer` is `Some` only shapes on that layer are
/// captured, the "same-layer filter" for a marquee begun over a shape (item 45).
///
/// A marquee started over empty canvas passes `None` and grabs every enclosed shape; one
/// started over a shape passes that shape's layer, so the box selects only within that
/// layer - the usual "rubber-band this one layer" gesture without a modifier key.
#[must_use]
pub fn shapes_in_rect_on_layer(
    shapes: &[DrawShape],
    rect: Rect,
    layer: Option<LayerId>,
) -> Vec<usize> {
    shapes
        .iter()
        .enumerate()
        .filter(|(_, s)| contains_rect(&rect, &s.bounding_box()))
        .filter(|(_, s)| layer.is_none_or(|l| s.layer() == l))
        .map(|(i, _)| i)
        .collect()
}

/// Returns the indices of every shape drawn on `layer`, the "select by layer"
/// query.
#[must_use]
pub fn shapes_on_layer(shapes: &[DrawShape], layer: LayerId) -> Vec<usize> {
    shapes
        .iter()
        .enumerate()
        .filter(|(_, s)| s.layer() == layer)
        .map(|(i, _)| i)
        .collect()
}

/// Returns `true` if `outer` fully contains `inner` (inclusive on all edges).
fn contains_rect(outer: &Rect, inner: &Rect) -> bool {
    inner.min.x >= outer.min.x
        && inner.min.y >= outer.min.y
        && inner.max.x <= outer.max.x
        && inner.max.y <= outer.max.y
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::Point;
    use reticle_model::ShapeKind;

    fn rect_on(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    }

    #[test]
    fn select_one_replaces() {
        let mut sel = Selection::new();
        sel.select_one(3);
        sel.select_one(5);
        assert_eq!(sel.len(), 1);
        assert!(sel.contains(5));
        assert!(!sel.contains(3));
    }

    #[test]
    fn toggle_adds_then_removes() {
        let mut sel = Selection::new();
        assert!(sel.toggle(7));
        assert!(sel.contains(7));
        assert!(!sel.toggle(7));
        assert!(!sel.contains(7));
    }

    #[test]
    fn add_and_extend_union() {
        let mut sel = Selection::new();
        sel.add(1);
        sel.extend([2, 3, 3]);
        assert_eq!(sel.iter().collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    #[test]
    fn clear_empties() {
        let mut sel = Selection::new();
        sel.set([1, 2, 3]);
        assert_eq!(sel.len(), 3);
        sel.clear();
        assert!(sel.is_empty());
    }

    #[test]
    fn rubber_band_selects_enclosed_only() {
        let l = LayerId::new(1, 0);
        let shapes = vec![
            rect_on(l, 0, 0, 100, 100),         // fully inside band
            rect_on(l, 50, 50, 400, 400),       // straddles band edge
            rect_on(l, 1000, 1000, 1100, 1100), // outside
        ];
        let band = Rect::new(Point::new(-10, -10), Point::new(300, 300));
        let hits = shapes_in_rect(&shapes, band);
        assert_eq!(hits, vec![0]);
    }

    #[test]
    fn subtract_removes_indices() {
        let mut sel = Selection::new();
        sel.set([1, 2, 3, 4]);
        sel.subtract([2, 4, 9]);
        assert_eq!(sel.iter().collect::<Vec<_>>(), vec![1, 3]);
    }

    #[test]
    fn same_layer_marquee_filters_to_one_layer() {
        let m1 = LayerId::new(4, 0);
        let m2 = LayerId::new(5, 0);
        let shapes = vec![
            rect_on(m1, 0, 0, 100, 100),
            rect_on(m2, 120, 120, 200, 200),
            rect_on(m1, 210, 210, 260, 260),
        ];
        let band = Rect::new(Point::new(-10, -10), Point::new(300, 300));
        // No filter: every enclosed shape.
        assert_eq!(shapes_in_rect_on_layer(&shapes, band, None), vec![0, 1, 2]);
        // Filter to m1: only the two m1 shapes inside the band.
        assert_eq!(shapes_in_rect_on_layer(&shapes, band, Some(m1)), vec![0, 2]);
    }

    #[test]
    fn select_by_layer_matches_layer() {
        let m1 = LayerId::new(4, 0);
        let m2 = LayerId::new(5, 0);
        let shapes = vec![
            rect_on(m1, 0, 0, 10, 10),
            rect_on(m2, 0, 0, 10, 10),
            rect_on(m1, 20, 20, 30, 30),
        ];
        assert_eq!(shapes_on_layer(&shapes, m1), vec![0, 2]);
        assert_eq!(shapes_on_layer(&shapes, m2), vec![1]);
        assert!(shapes_on_layer(&shapes, LayerId::new(9, 9)).is_empty());
    }
}
