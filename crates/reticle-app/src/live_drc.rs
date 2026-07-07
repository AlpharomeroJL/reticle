//! DRC-as-you-type: the incremental checker that underlines violations live.
//!
//! [`DrcResults`](crate::drc_panel::DrcResults) runs a *full* DRC pass on demand and
//! lists every violation. This module is its live counterpart: it owns a
//! [`PreparedDrc`] index over the flattened top cell and, on every edit, re-checks
//! only the edited region so a fresh violation is underlined the moment the geometry
//! is drawn, the way a text editor squiggles a misspelling as you type.
//!
//! # Two costs, two cadences
//!
//! The engine splits the work into a cheap step and an expensive one, and this
//! wrapper keeps them on different cadences:
//!
//! * [`recheck`](LiveDrc::recheck) calls [`PreparedDrc::check_region`] over the
//!   edit's dirty rectangle. It is proportional to the shapes near the edit (measured
//!   at us-scale even on a million-shape cell) so it runs **synchronously on every
//!   edit**.
//! * [`reprepare`](LiveDrc::reprepare) rebuilds the whole [`PreparedDrc`] index from
//!   the current document. It is the expensive part (tens to hundreds of
//!   milliseconds on a large cell), so the app runs it on a **throttle**: a stale
//!   index is swapped for a fresh one off the hot path, not on every keystroke. See
//!   [`is_stale`](LiveDrc::is_stale).
//!
//! A [`PreparedDrc`] is an immutable snapshot, so *any* edit (add, move, delete)
//! shows up only once the index is rebuilt. The underlines therefore reflect the last
//! prepared snapshot until the throttle fires, at which point the region dirtied since
//! is re-checked against the fresh index: a brief, bounded lag, exactly like a
//! spell-checker catching up after a burst of typing. [`apply_dirty`](LiveDrc::apply_dirty)
//! is the per-frame step the app runs; its `reprepare` flag is that throttle decision.
//!
//! The live violation set is kept minimal: [`recheck`](LiveDrc::recheck) replaces
//! only the violations touching the region it re-checked, so the underlines converge
//! to the checked neighbourhoods without a whole-cell sweep.

use reticle_drc::{DrcEngine, PreparedDrc};
use reticle_geometry::Rect;
use reticle_model::{Document, Violation};

use crate::drc_panel::{flatten_top_cell, resolve_rules};
use crate::history::Dirty;

/// The live incremental DRC state: a prepared index plus the violations underlined
/// near recent edits.
#[derive(Debug, Default)]
pub struct LiveDrc {
    /// The incremental checker over the last-prepared flattened top cell, or `None`
    /// before the first [`reprepare`](Self::reprepare).
    prepared: Option<PreparedDrc>,
    /// The document revision the prepared index was built at, so the app can tell a
    /// stale index (an edit landed since) from a current one.
    prepared_revision: u64,
    /// The bounding box of the geometry the current index was prepared over, so a
    /// [`Dirty::Full`] re-check (undo/redo or a structural edit) can sweep the whole
    /// indexed area. `None` before the first [`reprepare`](Self::reprepare).
    bounds: Option<Rect>,
    /// The violations underlined live, one per offending region near a recent edit.
    violations: Vec<Violation>,
}

impl LiveDrc {
    /// Creates an empty live checker with no prepared index yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuilds the prepared index from the current document (the expensive step).
    ///
    /// Flattens `top` and resolves the rule set exactly as the full DRC panel does
    /// (see [`flatten_top_cell`] and [`resolve_rules`]), then bulk-loads a fresh
    /// [`PreparedDrc`]. `revision` is the document revision this snapshot was built
    /// at; [`is_stale`](Self::is_stale) compares against it. Callers throttle how
    /// often this runs because it is proportional to the whole cell, not the edit.
    pub fn reprepare(&mut self, doc: &Document, top: &str, revision: u64) {
        let flat = flatten_top_cell(doc, top);
        let rules = resolve_rules(doc);
        self.bounds = flat.cell_bbox(top);
        self.prepared = Some(DrcEngine::new(rules).prepare(&flat, top));
        self.prepared_revision = revision;
    }

    /// Runs one live-DRC frame step: optionally rebuild the index, then re-check the
    /// region this frame's edits dirtied. This is the whole per-frame update the app
    /// runs, and the two-way "draw into a violation, move apart to clear it" behaviour
    /// is exactly this method called twice.
    ///
    /// `reprepare` is the app's throttle decision: when set, the index is rebuilt from
    /// `doc` at `revision` *before* the re-check, so freshly *added* geometry is now
    /// indexed and its violation is found. When clear, only the re-check runs, against
    /// the existing index; that is exact for moving or deleting already-indexed
    /// geometry, and a just-added shape simply waits for the next throttle tick.
    ///
    /// The `dirty` region selects the neighbourhood re-checked:
    /// * [`Dirty::None`] re-checks nothing.
    /// * [`Dirty::Region`] re-checks that rectangle (a local shape add or remove).
    /// * [`Dirty::Full`] re-checks the whole indexed area (undo/redo or a structural
    ///   edit whose region is not cheaply bounded); a no-op until the first index.
    ///
    /// Returns how many violations the re-checked region now contributes.
    pub fn apply_dirty(
        &mut self,
        dirty: Dirty,
        doc: &Document,
        top: &str,
        revision: u64,
        reprepare: bool,
    ) -> usize {
        if reprepare {
            self.reprepare(doc, top, revision);
        }
        match dirty {
            Dirty::None => 0,
            Dirty::Region(r) => self.recheck(r),
            Dirty::Full => match self.bounds {
                Some(b) => self.recheck(b),
                None => 0,
            },
        }
    }

    /// Whether the prepared index predates `revision` (or there is none yet), i.e. an
    /// edit has landed since the last [`reprepare`](Self::reprepare) so the index no
    /// longer contains the current geometry.
    #[must_use]
    pub fn is_stale(&self, revision: u64) -> bool {
        self.prepared.is_none() || self.prepared_revision != revision
    }

    /// Whether an index has been prepared at least once.
    #[must_use]
    pub fn has_index(&self) -> bool {
        self.prepared.is_some()
    }

    /// Re-checks the dirty `region` against the prepared index and folds the result
    /// into the live violation set, returning how many violations that region now
    /// contributes.
    ///
    /// This is the per-edit step: it calls [`PreparedDrc::check_region`] (bounded by
    /// the edit's neighbourhood) and then replaces the previously underlined
    /// violations touching `region` with the freshly found ones. So drawing a shape
    /// into a spacing violation adds an underline, and moving the shape back out
    /// removes it, with no whole-cell rescan.
    ///
    /// A no-op returning `0` when no index has been prepared yet.
    pub fn recheck(&mut self, region: Rect) -> usize {
        let Some(prepared) = self.prepared.as_ref() else {
            return 0;
        };
        let fresh = prepared.check_region(region);
        // Drop the stale underlines overlapping the re-checked region (expanded by one
        // DBU so a violation merely touching the region's edge is replaced, not
        // duplicated), then install the fresh ones.
        let touched = region.expanded(1);
        self.violations.retain(|v| !v.location.intersects(&touched));
        let n = fresh.len();
        self.violations.extend(fresh);
        n
    }

    /// The violations currently underlined live.
    #[must_use]
    pub fn violations(&self) -> &[Violation] {
        &self.violations
    }

    /// Whether there are no live violations.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.violations.is_empty()
    }

    /// Clears the prepared index and every live violation (e.g. on a document load).
    pub fn clear(&mut self) {
        self.prepared = None;
        self.prepared_revision = 0;
        self.bounds = None;
        self.violations.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::{LayerId, Point};
    use reticle_model::{Cell, DrawShape, Rule, RuleKind, ShapeKind, Technology};

    const LAYER: LayerId = LayerId {
        layer: 4,
        datatype: 0,
    };

    /// A document whose technology declares one min-spacing rule on `LAYER`, plus a
    /// top cell holding `rects` on that layer.
    fn doc_with_spacing(min_spacing: i64, rects: &[Rect]) -> Document {
        let mut cell = Cell::new("TOP");
        for r in rects {
            cell.shapes.push(DrawShape::new(LAYER, ShapeKind::Rect(*r)));
        }
        let mut doc = Document::new();
        let tech = Technology {
            rules: vec![Rule {
                name: "met1_spacing".to_owned(),
                kind: RuleKind::Spacing,
                layer: LAYER,
                other_layer: None,
                value: min_spacing,
            }],
            ..Technology::default()
        };
        doc.set_technology(tech);
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["TOP".to_owned()]);
        doc
    }

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
        Rect::new(Point::new(x0, y0), Point::new(x1, y1))
    }

    #[test]
    fn recheck_without_an_index_is_a_noop() {
        let mut live = LiveDrc::new();
        assert!(!live.has_index());
        assert_eq!(live.recheck(rect(0, 0, 100, 100)), 0);
        assert!(live.is_empty());
    }

    #[test]
    fn is_stale_tracks_revision() {
        let mut live = LiveDrc::new();
        assert!(live.is_stale(7), "no index is always stale");
        let doc = doc_with_spacing(100, &[rect(0, 0, 50, 50)]);
        live.reprepare(&doc, "TOP", 7);
        assert!(!live.is_stale(7), "fresh at the prepared revision");
        assert!(live.is_stale(8), "a later revision is stale");
    }

    #[test]
    fn recheck_reports_a_local_spacing_violation() {
        // Two rects 40 DBU apart with a 100-DBU min-spacing rule: a violation.
        let a = rect(0, 0, 50, 50);
        let b = rect(90, 0, 140, 50); // 40 DBU gap to `a`
        let doc = doc_with_spacing(100, &[a, b]);
        let mut live = LiveDrc::new();
        live.reprepare(&doc, "TOP", 1);
        let n = live.recheck(b);
        assert_eq!(n, 1, "the spacing violation is found near the edit");
        assert!(!live.is_empty());
        assert_eq!(live.violations()[0].kind, RuleKind::Spacing);
    }

    #[test]
    fn recheck_clears_a_violation_once_the_shape_moves_apart() {
        let a = rect(0, 0, 50, 50);
        let b_close = rect(90, 0, 140, 50); // 40 DBU gap: violates
        let doc = doc_with_spacing(100, &[a, b_close]);
        let mut live = LiveDrc::new();
        live.reprepare(&doc, "TOP", 1);
        assert_eq!(live.recheck(b_close), 1);
        assert!(!live.is_empty());

        // Move `b` far away (well past the 100-DBU threshold) and re-prepare.
        let b_far = rect(900, 0, 950, 50);
        let doc2 = doc_with_spacing(100, &[a, b_far]);
        live.reprepare(&doc2, "TOP", 2);
        // The dirty region spans the old and new positions of `b`.
        let dirty = b_close.union(&b_far);
        assert_eq!(live.recheck(dirty), 0, "no violation after moving apart");
        assert!(live.is_empty(), "the stale underline is cleared");
    }

    #[test]
    fn apply_dirty_reprepares_then_rechecks_a_region() {
        // A throttle tick (reprepare = true) over a freshly drawn, too-close pair
        // rebuilds the index and underlines the spacing violation in one call.
        let a = rect(0, 0, 50, 50);
        let b = rect(90, 0, 140, 50); // 40 DBU gap: violates the 100-DBU rule
        let doc = doc_with_spacing(100, &[a, b]);
        let mut live = LiveDrc::new();
        let n = live.apply_dirty(Dirty::Region(b), &doc, "TOP", 1, true);
        assert_eq!(n, 1);
        assert!(live.has_index(), "the throttle tick built the index");
        assert!(!live.is_empty());
    }

    #[test]
    fn apply_dirty_full_sweeps_the_indexed_bounds() {
        // A structural/undo edit dirties Full: after an index exists, it re-checks the
        // whole indexed area rather than a single region.
        let a = rect(0, 0, 50, 50);
        let b = rect(90, 0, 140, 50);
        let doc = doc_with_spacing(100, &[a, b]);
        let mut live = LiveDrc::new();
        // No index yet: a Full dirty is a no-op (nothing to sweep).
        assert_eq!(live.apply_dirty(Dirty::Full, &doc, "TOP", 1, false), 0);
        // Throttle tick with no dirtied region still builds the index (reprepare set).
        live.apply_dirty(Dirty::None, &doc, "TOP", 1, true);
        assert!(live.is_empty(), "None re-checks nothing");
        // Now a Full sweep finds the violation across the whole indexed bounds.
        assert_eq!(live.apply_dirty(Dirty::Full, &doc, "TOP", 1, false), 1);
        assert!(!live.is_empty());
    }
}
