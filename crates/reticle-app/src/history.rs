//! The editing document with undo/redo, wrapping [`EditableDocument`].
//!
//! Every model mutation the app makes goes through [`History`] so it lands on the
//! [`EditableDocument`] undo stack and can be stepped from the undo-history panel.
//! The wrapper adds two things over the raw store: it re-seeds itself from the demo
//! document, and it exposes the undo/redo *depths* the panel lists. The
//! [`reticle_model::DocumentStore`] trait supplies `apply`/`undo`/`redo`, so those
//! are imported here.

use reticle_geometry::{Rect, Shape};
use reticle_model::{Document, DocumentStore, Edit, EditableDocument, Result};

/// The region the most recent edits dirtied, for incremental live DRC re-checks.
///
/// A local shape add or remove dirties a known rectangle; a structural change
/// (instance, array, cell) or an undo/redo dirties an unknown region and asks the
/// live checker to fall back to the whole cell. Accumulated across every apply since
/// the app last drained it with [`History::take_dirty`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dirty {
    /// No geometry changed since the last drain.
    #[default]
    None,
    /// Geometry changed within this rectangle (a local shape add or remove).
    Region(Rect),
    /// A structural or unknown-region change: the whole current cell should be
    /// re-checked (instance/array/cell edits, undo, redo).
    Full,
}

impl Dirty {
    /// Combines two dirty regions: `Full` dominates, `None` is the identity, and two
    /// regions union into the rectangle that contains both.
    #[must_use]
    pub fn merge(self, other: Dirty) -> Dirty {
        match (self, other) {
            (Dirty::Full, _) | (_, Dirty::Full) => Dirty::Full,
            (Dirty::None, x) | (x, Dirty::None) => x,
            (Dirty::Region(a), Dirty::Region(b)) => Dirty::Region(a.union(&b)),
        }
    }
}

/// The editable document plus its undo/redo history.
///
/// This is the single source of truth for the layout the user edits (as opposed to
/// the read-only demo the canvas starts from). It owns an [`EditableDocument`] and
/// forwards edits to it.
///
/// # Grouped edits
///
/// The underlying [`EditableDocument`] records one undo step per [`Edit`]. A single
/// user action can be several edits (a boolean removes its inputs and adds one
/// result), and those must undo together. [`History::apply_group`] applies a batch
/// as one logical step by remembering how many underlying steps it added; `undo`
/// and `redo` then move over the whole group at once. Grouping lives here in the
/// app layer, not in the frozen model `Edit` vocabulary.
#[derive(Debug)]
pub struct History {
    doc: EditableDocument,
    /// Size (in underlying edits) of each logical undo step still on the undo
    /// stack. A plain [`History::apply`] pushes `1`; [`History::apply_group`] pushes
    /// the batch length. `undo` pops one entry and steps the store that many times.
    undo_groups: Vec<usize>,
    /// The mirror of [`History::undo_groups`] for the redo stack.
    redo_groups: Vec<usize>,
    /// The region dirtied by edits since the app last drained it (see
    /// [`take_dirty`](History::take_dirty)), so the live DRC checker re-checks only
    /// the edited neighbourhood rather than the whole cell.
    pending_dirty: Dirty,
}

impl Default for History {
    fn default() -> Self {
        Self::new(Document::new())
    }
}

impl History {
    /// Wraps `document` with a fresh (empty) undo history.
    #[must_use]
    pub fn new(document: Document) -> Self {
        Self {
            doc: EditableDocument::new(document),
            undo_groups: Vec::new(),
            redo_groups: Vec::new(),
            pending_dirty: Dirty::None,
        }
    }

    /// The dirty region of a single edit, before it is applied.
    ///
    /// A shape add dirties the shape's own bounding box; a shape remove dirties the
    /// bounding box of the shape still at that index in the current document (so it
    /// must be read *before* the remove lands). Structural edits (cells, instances,
    /// arrays) have no cheap bounding box and dirty [`Dirty::Full`]; labels are not
    /// DRC geometry and dirty nothing.
    fn dirty_of(&self, edit: &Edit) -> Dirty {
        match edit {
            Edit::AddShape { shape, .. } => Dirty::Region(shape.bounding_box()),
            Edit::RemoveShape { cell, index } => self
                .doc
                .document()
                .cell(cell)
                .and_then(|c| c.shapes.get(*index))
                .map_or(Dirty::None, |s| Dirty::Region(s.bounding_box())),
            Edit::AddLabel { .. } | Edit::RemoveLabel { .. } => Dirty::None,
            // Cells, instances, arrays, and any future edit kind: a region we cannot
            // cheaply bound, so re-check the whole cell on the next throttle tick.
            _ => Dirty::Full,
        }
    }

    /// Drains and returns the region dirtied since the last drain, resetting it to
    /// [`Dirty::None`]. The app calls this each frame to feed the live DRC checker.
    pub fn take_dirty(&mut self) -> Dirty {
        std::mem::take(&mut self.pending_dirty)
    }

    /// Borrows the current document snapshot for reading (culling, drawing, export).
    #[must_use]
    pub fn document(&self) -> &Document {
        self.doc.document()
    }

    /// The document revision: a monotonic counter bumped on every apply/undo/redo.
    ///
    /// The retained GPU renderer keys its cache invalidation on this, so it re-uploads
    /// geometry only when the document actually changed, not on every frame.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.doc.revision()
    }

    /// Replaces the document's [`reticle_model::Technology`] in place and bumps the
    /// revision so the canvas re-reads.
    ///
    /// Technology is not part of the [`Edit`] vocabulary, so this is applied
    /// directly to the document rather than pushed onto the undo stack: a
    /// technology change is not undoable, and it leaves the existing shape-edit
    /// history intact (see [`reticle_model::EditableDocument::set_technology`]).
    /// The tech editor panel calls this to commit a validated technology back to
    /// the live document.
    pub fn set_technology(&mut self, tech: reticle_model::Technology) {
        self.doc.set_technology(tech);
    }

    /// Applies `edit` as one undo step, pushing it onto the undo stack.
    ///
    /// # Errors
    ///
    /// Propagates any [`reticle_model::ModelError`] from the underlying store (for
    /// example an edit that references a missing cell).
    pub fn apply(&mut self, edit: Edit) -> Result<()> {
        // Bound the dirty region before the edit lands (a remove needs the shape that
        // is still present), then accumulate it only once the edit actually applies.
        let dirty = self.dirty_of(&edit);
        self.doc.apply(edit)?;
        self.pending_dirty = self.pending_dirty.merge(dirty);
        self.undo_groups.push(1);
        self.redo_groups.clear();
        Ok(())
    }

    /// Applies `edits` in order as a single logical undo step, so one `undo` reverses
    /// the whole batch (and one `redo` re-applies it). This is how a boolean lands as
    /// one step: the removals of its inputs plus the addition of its result.
    ///
    /// Applies greedily. If some edit in the middle fails, the edits already applied
    /// stay applied but are demoted to individual undo steps (so the document is
    /// never left in a half-grouped state), and the error is returned. Callers should
    /// build batches that cannot partially fail (see the index ordering note on
    /// [`Edit::RemoveShape`]).
    ///
    /// An empty batch is a no-op and records nothing.
    ///
    /// # Errors
    ///
    /// Propagates the first [`reticle_model::ModelError`] from the underlying store.
    pub fn apply_group(&mut self, edits: Vec<Edit>) -> Result<()> {
        if edits.is_empty() {
            return Ok(());
        }
        let mut applied = 0usize;
        for edit in edits {
            let dirty = self.dirty_of(&edit);
            match self.doc.apply(edit) {
                Ok(()) => {
                    applied += 1;
                    self.pending_dirty = self.pending_dirty.merge(dirty);
                }
                Err(e) => {
                    // Demote the partial batch to individual steps and surface the
                    // error; the document reflects exactly the edits that landed.
                    for _ in 0..applied {
                        self.undo_groups.push(1);
                    }
                    self.redo_groups.clear();
                    return Err(e);
                }
            }
        }
        self.undo_groups.push(applied);
        self.redo_groups.clear();
        Ok(())
    }

    /// Undoes the most recent logical step (a whole group), returning `true` if there
    /// was one to undo.
    pub fn undo(&mut self) -> bool {
        let Some(steps) = self.undo_groups.pop() else {
            return false;
        };
        for _ in 0..steps {
            self.doc.undo();
        }
        // An undo can move geometry anywhere; its region is not cheaply known, so ask
        // the live checker to re-check the whole cell on the next throttle tick.
        self.pending_dirty = Dirty::Full;
        self.redo_groups.push(steps);
        true
    }

    /// Redoes the most recently undone logical step (a whole group), returning `true`
    /// if there was one.
    pub fn redo(&mut self) -> bool {
        let Some(steps) = self.redo_groups.pop() else {
            return false;
        };
        for _ in 0..steps {
            self.doc.redo();
        }
        self.pending_dirty = Dirty::Full;
        self.undo_groups.push(steps);
        true
    }

    /// The number of logical steps currently on the undo stack.
    #[must_use]
    pub fn undo_depth(&self) -> usize {
        self.undo_groups.len()
    }

    /// The number of logical steps currently on the redo stack.
    #[must_use]
    pub fn redo_depth(&self) -> usize {
        self.redo_groups.len()
    }

    /// Whether there is at least one step to undo.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo_groups.is_empty()
    }

    /// Whether there is at least one step to redo.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo_groups.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{Cell, DrawShape, ShapeKind};

    fn shape() -> DrawShape {
        DrawShape::new(
            LayerId::new(4, 0),
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(10, 10))),
        )
    }

    #[test]
    fn apply_then_undo_restores_state() {
        let mut h = History::new(demo::demo_document());
        let before = h.document().cell(demo::TOP_CELL).unwrap().shapes.len();
        h.apply(Edit::AddShape {
            cell: demo::TOP_CELL.to_owned(),
            shape: shape(),
        })
        .unwrap();
        assert_eq!(
            h.document().cell(demo::TOP_CELL).unwrap().shapes.len(),
            before + 1
        );
        assert_eq!(h.undo_depth(), 1);
        assert!(h.can_undo());

        assert!(h.undo());
        assert_eq!(
            h.document().cell(demo::TOP_CELL).unwrap().shapes.len(),
            before
        );
        assert_eq!(h.undo_depth(), 0);
        assert_eq!(h.redo_depth(), 1);
    }

    #[test]
    fn redo_reapplies() {
        let mut h = History::new(demo::demo_document());
        let before = h.document().cell(demo::TOP_CELL).unwrap().shapes.len();
        h.apply(Edit::AddShape {
            cell: demo::TOP_CELL.to_owned(),
            shape: shape(),
        })
        .unwrap();
        h.undo();
        assert!(h.can_redo());
        assert!(h.redo());
        assert_eq!(
            h.document().cell(demo::TOP_CELL).unwrap().shapes.len(),
            before + 1
        );
        assert!(!h.can_redo());
    }

    #[test]
    fn empty_history_has_nothing_to_undo() {
        let mut h = History::default();
        assert!(!h.can_undo());
        assert!(!h.can_redo());
        assert!(!h.undo());
        assert!(!h.redo());
    }

    #[test]
    fn add_cell_edit_round_trips() {
        let mut h = History::new(Document::new());
        h.apply(Edit::AddCell {
            cell: Cell::new("A"),
        })
        .unwrap();
        assert!(h.document().cell("A").is_some());
        h.undo();
        assert!(h.document().cell("A").is_none());
    }

    #[test]
    fn apply_group_is_one_undo_step() {
        let mut h = History::new(demo::demo_document());
        let before = h.document().cell(demo::TOP_CELL).unwrap().shapes.len();
        // Two adds applied as one logical group.
        h.apply_group(vec![
            Edit::AddShape {
                cell: demo::TOP_CELL.to_owned(),
                shape: shape(),
            },
            Edit::AddShape {
                cell: demo::TOP_CELL.to_owned(),
                shape: shape(),
            },
        ])
        .unwrap();
        assert_eq!(
            h.document().cell(demo::TOP_CELL).unwrap().shapes.len(),
            before + 2
        );
        // One logical step, not two.
        assert_eq!(h.undo_depth(), 1);
        // A single undo reverses the whole group.
        assert!(h.undo());
        assert_eq!(
            h.document().cell(demo::TOP_CELL).unwrap().shapes.len(),
            before
        );
        assert_eq!(h.undo_depth(), 0);
        assert_eq!(h.redo_depth(), 1);
        // And a single redo re-applies the whole group.
        assert!(h.redo());
        assert_eq!(
            h.document().cell(demo::TOP_CELL).unwrap().shapes.len(),
            before + 2
        );
    }

    #[test]
    fn empty_group_is_noop() {
        let mut h = History::new(demo::demo_document());
        h.apply_group(Vec::new()).unwrap();
        assert_eq!(h.undo_depth(), 0);
        assert!(!h.can_undo());
    }

    #[test]
    fn add_shape_dirties_its_bounding_box_then_drains() {
        let mut h = History::new(Document::new());
        h.apply(Edit::AddCell {
            cell: Cell::new("A"),
        })
        .unwrap();
        h.take_dirty(); // drop the structural dirty from the cell add
        h.apply(Edit::AddShape {
            cell: "A".to_owned(),
            shape: DrawShape::new(
                LayerId::new(4, 0),
                ShapeKind::Rect(Rect::new(Point::new(10, 20), Point::new(40, 60))),
            ),
        })
        .unwrap();
        assert_eq!(
            h.take_dirty(),
            Dirty::Region(Rect::new(Point::new(10, 20), Point::new(40, 60)))
        );
        // Draining resets to None.
        assert_eq!(h.take_dirty(), Dirty::None);
    }

    #[test]
    fn remove_shape_dirties_the_removed_bounding_box() {
        let mut h = History::new(Document::new());
        h.apply(Edit::AddCell {
            cell: Cell::new("A"),
        })
        .unwrap();
        let bbox = Rect::new(Point::new(0, 0), Point::new(30, 30));
        h.apply(Edit::AddShape {
            cell: "A".to_owned(),
            shape: DrawShape::new(LayerId::new(4, 0), ShapeKind::Rect(bbox)),
        })
        .unwrap();
        h.take_dirty();
        h.apply(Edit::RemoveShape {
            cell: "A".to_owned(),
            index: 0,
        })
        .unwrap();
        assert_eq!(h.take_dirty(), Dirty::Region(bbox));
    }

    #[test]
    fn apply_group_unions_the_dirty_regions() {
        let mut h = History::new(Document::new());
        h.apply(Edit::AddCell {
            cell: Cell::new("A"),
        })
        .unwrap();
        h.take_dirty();
        let a = Rect::new(Point::new(0, 0), Point::new(10, 10));
        let b = Rect::new(Point::new(100, 100), Point::new(120, 120));
        h.apply_group(vec![
            Edit::AddShape {
                cell: "A".to_owned(),
                shape: DrawShape::new(LayerId::new(4, 0), ShapeKind::Rect(a)),
            },
            Edit::AddShape {
                cell: "A".to_owned(),
                shape: DrawShape::new(LayerId::new(4, 0), ShapeKind::Rect(b)),
            },
        ])
        .unwrap();
        assert_eq!(h.take_dirty(), Dirty::Region(a.union(&b)));
    }

    #[test]
    fn structural_and_undo_edits_dirty_full() {
        let mut h = History::new(Document::new());
        // A cell add has no cheap bounding box: full.
        h.apply(Edit::AddCell {
            cell: Cell::new("A"),
        })
        .unwrap();
        assert_eq!(h.take_dirty(), Dirty::Full);
        // Undo dirties the whole cell too.
        h.apply(Edit::AddShape {
            cell: "A".to_owned(),
            shape: shape(),
        })
        .unwrap();
        h.take_dirty();
        assert!(h.undo());
        assert_eq!(h.take_dirty(), Dirty::Full);
    }

    #[test]
    fn dirty_merge_rules() {
        let r = Rect::new(Point::new(0, 0), Point::new(1, 1));
        assert_eq!(Dirty::None.merge(Dirty::Region(r)), Dirty::Region(r));
        assert_eq!(Dirty::Region(r).merge(Dirty::None), Dirty::Region(r));
        assert_eq!(Dirty::Full.merge(Dirty::Region(r)), Dirty::Full);
        assert_eq!(Dirty::Region(r).merge(Dirty::Full), Dirty::Full);
        let s = Rect::new(Point::new(5, 5), Point::new(6, 6));
        assert_eq!(
            Dirty::Region(r).merge(Dirty::Region(s)),
            Dirty::Region(r.union(&s))
        );
    }

    #[test]
    fn applying_clears_redo_stack() {
        let mut h = History::new(demo::demo_document());
        h.apply(Edit::AddShape {
            cell: demo::TOP_CELL.to_owned(),
            shape: shape(),
        })
        .unwrap();
        h.undo();
        assert_eq!(h.redo_depth(), 1);
        // A fresh edit invalidates the redo stack.
        h.apply(Edit::AddShape {
            cell: demo::TOP_CELL.to_owned(),
            shape: shape(),
        })
        .unwrap();
        assert_eq!(h.redo_depth(), 0);
    }
}
