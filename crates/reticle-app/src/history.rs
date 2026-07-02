//! The editing document with undo/redo, wrapping [`EditableDocument`].
//!
//! Every model mutation the app makes goes through [`History`] so it lands on the
//! [`EditableDocument`] undo stack and can be stepped from the undo-history panel.
//! The wrapper adds two things over the raw store: it re-seeds itself from the demo
//! document, and it exposes the undo/redo *depths* the panel lists. The
//! [`reticle_model::DocumentStore`] trait supplies `apply`/`undo`/`redo`, so those
//! are imported here.

use reticle_model::{Document, DocumentStore, Edit, EditableDocument, Result};

/// The editable document plus its undo/redo history.
///
/// This is the single source of truth for the layout the user edits (as opposed to
/// the read-only demo the canvas starts from). It owns an [`EditableDocument`] and
/// forwards edits to it.
#[derive(Debug)]
pub struct History {
    doc: EditableDocument,
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
        }
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

    /// Applies `edit`, pushing it onto the undo stack.
    ///
    /// # Errors
    ///
    /// Propagates any [`reticle_model::ModelError`] from the underlying store (for
    /// example an edit that references a missing cell).
    pub fn apply(&mut self, edit: Edit) -> Result<()> {
        self.doc.apply(edit)
    }

    /// Undoes the most recent edit, returning `true` if there was one to undo.
    pub fn undo(&mut self) -> bool {
        self.doc.undo()
    }

    /// Redoes the most recently undone edit, returning `true` if there was one.
    pub fn redo(&mut self) -> bool {
        self.doc.redo()
    }

    /// The number of edits currently on the undo stack.
    #[must_use]
    pub fn undo_depth(&self) -> usize {
        self.doc.undo_depth()
    }

    /// The number of edits currently on the redo stack.
    #[must_use]
    pub fn redo_depth(&self) -> usize {
        self.doc.redo_depth()
    }

    /// Whether there is at least one edit to undo.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.doc.undo_depth() > 0
    }

    /// Whether there is at least one edit to redo.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        self.doc.redo_depth() > 0
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
