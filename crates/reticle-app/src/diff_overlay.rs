//! The layout-diff overlay state and its window-free logic.
//!
//! The overlay compares two [`Document`] snapshots with the pure
//! [`reticle_diff`] crate and keeps the resulting [`LayoutDiff`] so the canvas can
//! paint each added/removed/changed shape as a colored rectangle. Following the
//! same split as [`crate::drc_panel`], all the state and logic live here as plain
//! functions tested without an egui context; the app module owns only the thin
//! painting and button wiring.
//!
//! # The two-snapshot flow
//!
//! There is no separate file-open path for a comparison document in this build, so
//! the overlay obtains its "other version" from the live document itself:
//! [`snapshot`](DiffOverlay::snapshot) captures the current document as the
//! baseline (the *before*), the user then edits, and
//! [`compute`](DiffOverlay::compute) diffs the baseline against the now-current
//! document (the *after*). Shapes drawn since the snapshot show as added (green),
//! shapes deleted since show as removed (red). `changed` is deferred in the diff
//! engine (see the `reticle_diff` crate docs), so nothing paints amber in v1.

use reticle_diff::{DiffShape, LayoutDiff, diff};
use reticle_model::Document;

/// The diff overlay's stored state: the baseline snapshot, the computed diff, and
/// whether the overlay is currently shown.
///
/// The diff is empty until [`compute`](DiffOverlay::compute) runs against a
/// baseline captured by [`snapshot`](DiffOverlay::snapshot), and again after
/// [`clear`](DiffOverlay::clear). `visible` gates painting so the user can hide the
/// overlay without discarding the computed diff.
#[derive(Clone, Debug)]
pub struct DiffOverlay {
    /// The baseline document (the *before*) captured by the last snapshot, if any.
    baseline: Option<Document>,
    /// The most recently computed diff (baseline vs current).
    diff: LayoutDiff,
    /// Whether a diff has been computed at least once since the last clear (so the
    /// panel can distinguish "not computed" from "computed, no differences").
    has_run: bool,
    /// Whether the overlay is painted. Toggling this hides the overlay without
    /// dropping the computed diff or the baseline.
    visible: bool,
}

impl Default for DiffOverlay {
    fn default() -> Self {
        Self {
            baseline: None,
            diff: LayoutDiff::default(),
            has_run: false,
            visible: true,
        }
    }
}

impl DiffOverlay {
    /// Creates an empty overlay: no baseline, no diff, shown by default.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Captures `doc` as the baseline (the *before*) for the next diff.
    ///
    /// A fresh snapshot invalidates any previously computed diff, since it now
    /// refers to an older baseline. The overlay's visibility is left unchanged.
    pub fn snapshot(&mut self, doc: &Document) {
        self.baseline = Some(doc.clone());
        self.diff = LayoutDiff::default();
        self.has_run = false;
    }

    /// Diffs the captured baseline against `current`, storing the result.
    ///
    /// The baseline is the *before* and `current` is the *after*, so a shape that
    /// exists now but not in the baseline is added, and one that existed in the
    /// baseline but not now is removed. Returns the number of reported shapes
    /// (added + removed + changed). Does nothing and returns `None` when no
    /// baseline has been captured.
    pub fn compute(&mut self, current: &Document) -> Option<usize> {
        let baseline = self.baseline.as_ref()?;
        self.diff = diff(baseline, current);
        self.has_run = true;
        Some(self.count())
    }

    /// Drops the baseline and the computed diff. Visibility is left unchanged.
    pub fn clear(&mut self) {
        self.baseline = None;
        self.diff = LayoutDiff::default();
        self.has_run = false;
    }

    /// The most recently computed diff.
    #[must_use]
    pub fn diff(&self) -> &LayoutDiff {
        &self.diff
    }

    /// Whether a baseline snapshot has been captured.
    #[must_use]
    pub fn has_baseline(&self) -> bool {
        self.baseline.is_some()
    }

    /// Whether a diff has been computed since the last clear.
    #[must_use]
    pub fn has_run(&self) -> bool {
        self.has_run
    }

    /// The total number of reported shapes (added + removed + changed).
    #[must_use]
    pub fn count(&self) -> usize {
        self.diff.added_count() + self.diff.removed_count() + self.diff.changed_count()
    }

    /// The number of added shapes in the current diff.
    #[must_use]
    pub fn added_count(&self) -> usize {
        self.diff.added_count()
    }

    /// The number of removed shapes in the current diff.
    #[must_use]
    pub fn removed_count(&self) -> usize {
        self.diff.removed_count()
    }

    /// The number of changed shapes in the current diff (always `0` in v1).
    #[must_use]
    pub fn changed_count(&self) -> usize {
        self.diff.changed_count()
    }

    /// Whether the computed diff reports no differences.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.diff.is_empty()
    }

    /// Whether the overlay is currently painted.
    #[must_use]
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Sets whether the overlay is painted.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// Whether the overlay should paint this frame: shown, and a diff has run.
    #[must_use]
    pub fn should_paint(&self) -> bool {
        self.visible && self.has_run
    }

    /// The added shapes to paint (green).
    #[must_use]
    pub fn added(&self) -> &[DiffShape] {
        &self.diff.added
    }

    /// The removed shapes to paint (red).
    #[must_use]
    pub fn removed(&self) -> &[DiffShape] {
        &self.diff.removed
    }

    /// The changed shapes to paint (amber). Always empty in v1.
    #[must_use]
    pub fn changed(&self) -> &[DiffShape] {
        &self.diff.changed
    }
}

#[cfg(test)]
mod tests {
    use super::DiffOverlay;
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
    fn compute_without_baseline_is_a_noop() {
        let mut overlay = DiffOverlay::new();
        assert!(!overlay.has_baseline());
        assert_eq!(overlay.compute(&doc_from_rects(&[rect(0, 0, 4, 4)])), None);
        assert!(!overlay.has_run());
        assert!(!overlay.should_paint());
    }

    #[test]
    fn snapshot_then_compute_reports_added_and_removed() {
        let base = doc_from_rects(&[rect(0, 0, 4, 4), rect(10, 0, 4, 4)]);
        // Drop the second rect, add a new one: one removed, one added.
        let current = doc_from_rects(&[rect(0, 0, 4, 4), rect(20, 0, 4, 4)]);
        let mut overlay = DiffOverlay::new();
        overlay.snapshot(&base);
        assert!(overlay.has_baseline());
        let n = overlay.compute(&current).expect("baseline captured");
        assert_eq!(n, 2);
        assert_eq!(overlay.added_count(), 1);
        assert_eq!(overlay.removed_count(), 1);
        assert_eq!(overlay.changed_count(), 0);
        assert_eq!(overlay.added()[0].rect, rect(20, 0, 4, 4));
        assert_eq!(overlay.removed()[0].rect, rect(10, 0, 4, 4));
        assert!(overlay.should_paint());
    }

    #[test]
    fn identical_snapshots_report_empty() {
        let doc = doc_from_rects(&[rect(0, 0, 4, 4)]);
        let mut overlay = DiffOverlay::new();
        overlay.snapshot(&doc);
        overlay.compute(&doc).expect("baseline captured");
        assert!(overlay.has_run());
        assert!(overlay.is_empty());
        assert_eq!(overlay.count(), 0);
    }

    #[test]
    fn visibility_toggles_without_dropping_the_diff() {
        let base = doc_from_rects(&[]);
        let current = doc_from_rects(&[rect(0, 0, 4, 4)]);
        let mut overlay = DiffOverlay::new();
        overlay.snapshot(&base);
        overlay.compute(&current).expect("baseline captured");
        assert!(overlay.should_paint());
        overlay.set_visible(false);
        assert!(!overlay.visible());
        assert!(!overlay.should_paint());
        // The diff survives hiding.
        assert_eq!(overlay.added_count(), 1);
        overlay.set_visible(true);
        assert!(overlay.should_paint());
    }

    #[test]
    fn clear_drops_baseline_and_diff() {
        let base = doc_from_rects(&[]);
        let current = doc_from_rects(&[rect(0, 0, 4, 4)]);
        let mut overlay = DiffOverlay::new();
        overlay.snapshot(&base);
        overlay.compute(&current).expect("baseline captured");
        overlay.clear();
        assert!(!overlay.has_baseline());
        assert!(!overlay.has_run());
        assert!(overlay.is_empty());
    }

    #[test]
    fn a_fresh_snapshot_invalidates_the_previous_diff() {
        let base = doc_from_rects(&[]);
        let current = doc_from_rects(&[rect(0, 0, 4, 4)]);
        let mut overlay = DiffOverlay::new();
        overlay.snapshot(&base);
        overlay.compute(&current).expect("baseline captured");
        assert!(overlay.has_run());
        // Re-snapshotting the current doc clears the stale diff until recomputed.
        overlay.snapshot(&current);
        assert!(!overlay.has_run());
        assert!(overlay.is_empty());
    }
}
