//! An editable document with an exact transactional undo/redo history.
//!
//! [`EditableDocument`] wraps a [`Document`] and implements [`DocumentStore`].
//! Each applied [`Edit`] records the data needed to reverse it, so undo and redo
//! are exact and unbounded. The same edit log is what the collaboration layer
//! replicates.

use crate::{Document, DocumentStore, Edit, ModelError, Result};
use reticle_geometry::Rect;
use std::cell::RefCell;
use std::collections::HashMap;

/// Captures the data needed to reverse one applied [`Edit`].
#[derive(Debug)]
enum Reverse {
    /// Undo an added cell by removing it.
    RemoveCell(String),
    /// Undo a removed cell by restoring it.
    RestoreCell(Box<crate::Cell>),
    /// Undo an appended shape by popping it.
    PopShape(String),
    /// Undo a removed shape by re-inserting it at its original index.
    InsertShape(String, usize, Box<crate::DrawShape>),
    /// Undo an appended instance by popping it.
    PopInstance(String),
    /// Undo an appended array by popping it.
    PopArray(String),
    /// Undo an appended label by popping it.
    PopLabel(String),
    /// Undo a removed label by re-inserting it at its original index.
    InsertLabel(String, usize, Box<crate::Label>),
}

/// A [`Document`] paired with a transactional undo/redo history.
///
/// It also memoizes [`Document::cell_bbox`] in a per-cell cache (see
/// [`EditableDocument::cell_bbox`]). The cache is cleared on every mutation
/// (`apply`/`undo`/`redo`), so a bounding box read after an edit always reflects
/// that edit. The cache lives here rather than on [`Document`] to keep `Document`
/// free of interior mutability.
#[derive(Debug, Default)]
pub struct EditableDocument {
    doc: Document,
    undo_stack: Vec<(Edit, Reverse)>,
    redo_stack: Vec<Edit>,
    /// Memoized `cell_bbox` results, keyed by cell name. `None` is a cached
    /// "missing or empty cell" answer, distinct from an absent (uncached) entry.
    /// Cleared wholesale on every edit.
    bbox_cache: RefCell<HashMap<String, Option<Rect>>>,
    /// Monotonic document revision. See [`EditableDocument::revision`].
    revision: u64,
}

impl EditableDocument {
    /// Wraps an existing document with an empty history.
    #[must_use]
    pub fn new(doc: Document) -> Self {
        Self {
            doc,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            bbox_cache: RefCell::new(HashMap::new()),
            revision: 0,
        }
    }

    /// The document revision: a monotonic counter of applied mutations.
    ///
    /// Starts at 0 for a fresh editor and increments by exactly 1 every time the
    /// document actually changes: on each successful [`DocumentStore::apply`], on
    /// each successful [`DocumentStore::undo`], and on each successful
    /// [`DocumentStore::redo`] (undo and redo *apply* a mutation, so they count).
    /// Failed or no-op calls (an `apply` that errors, an `undo`/`redo` with an
    /// empty stack) leave it unchanged. It never resets or goes backwards, so two
    /// equal revisions observed on the same editor mean the document is unchanged
    /// between them; renderers and caches can key invalidation on it.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Borrows the underlying document.
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// Consumes the editor and returns the document.
    #[must_use]
    pub fn into_document(self) -> Document {
        self.doc
    }

    /// Replaces the document's [`Technology`](crate::Technology) in place, bumping
    /// the revision so renderers and caches re-read.
    ///
    /// Setting technology is deliberately outside the [`Edit`] vocabulary: the
    /// undo/redo log records geometry mutations, and a `Reverse` only ever touches
    /// cells, so swapping the technology neither corrupts the existing history nor
    /// participates in it. The undo and redo stacks are left untouched (a later
    /// undo of a shape edit still works), but a technology change is itself *not*
    /// undoable. The bounding-box cache keys on cells only, yet is cleared here too
    /// so this stays a single, uniform "the document changed" point. This mirrors
    /// the agent API's out-of-band `set_technology` while preserving history rather
    /// than discarding it.
    pub fn set_technology(&mut self, tech: crate::Technology) {
        self.doc.set_technology(tech);
        self.invalidate_bbox_cache();
        self.revision += 1;
    }

    /// The full recursive bounding box of a cell, memoized.
    ///
    /// On a cache hit this returns the stored answer; on a miss it calls
    /// [`Document::cell_bbox`] (the uncached source of truth) exactly once and
    /// stores the result. The value is byte-for-byte identical to calling
    /// `self.document().cell_bbox(name)` directly. The cache is cleared by every
    /// edit, so the returned box always reflects the current document state.
    #[must_use]
    pub fn cell_bbox(&self, name: &str) -> Option<Rect> {
        if let Some(cached) = self.bbox_cache.borrow().get(name) {
            return *cached;
        }
        let computed = self.doc.cell_bbox(name);
        self.bbox_cache
            .borrow_mut()
            .insert(name.to_owned(), computed);
        computed
    }

    /// Drops every memoized bounding box. Called after any mutation so the next
    /// [`EditableDocument::cell_bbox`] recomputes against the edited document.
    fn invalidate_bbox_cache(&mut self) {
        self.bbox_cache.get_mut().clear();
    }

    /// The number of edits that can be undone.
    #[must_use]
    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }

    /// The number of edits that can be redone.
    #[must_use]
    pub fn redo_depth(&self) -> usize {
        self.redo_stack.len()
    }

    /// Applies `edit` to `doc`, returning the reverse operation to record. Used by
    /// both `apply` (first time) and `redo` (re-apply); both start from the same
    /// pre-edit state, so the reverse is identical.
    fn execute(doc: &mut Document, edit: &Edit) -> Result<Reverse> {
        match edit {
            Edit::AddCell { cell } => {
                if doc.cell(&cell.name).is_some() {
                    return Err(ModelError::DuplicateCell(cell.name.clone()));
                }
                let name = cell.name.clone();
                doc.insert_cell(cell.clone());
                Ok(Reverse::RemoveCell(name))
            }
            Edit::RemoveCell { name } => {
                let cell = doc
                    .remove_cell(name)
                    .ok_or_else(|| ModelError::CellNotFound(name.clone()))?;
                Ok(Reverse::RestoreCell(Box::new(cell)))
            }
            Edit::AddShape { cell, shape } => {
                let target = doc
                    .cell_mut(cell)
                    .ok_or_else(|| ModelError::CellNotFound(cell.clone()))?;
                target.shapes.push(shape.clone());
                Ok(Reverse::PopShape(cell.clone()))
            }
            Edit::RemoveShape { cell, index } => {
                let target = doc
                    .cell_mut(cell)
                    .ok_or_else(|| ModelError::CellNotFound(cell.clone()))?;
                if *index >= target.shapes.len() {
                    return Err(ModelError::IndexOutOfBounds(*index));
                }
                let shape = target.shapes.remove(*index);
                Ok(Reverse::InsertShape(cell.clone(), *index, Box::new(shape)))
            }
            Edit::AddInstance { cell, instance } => {
                let target = doc
                    .cell_mut(cell)
                    .ok_or_else(|| ModelError::CellNotFound(cell.clone()))?;
                target.instances.push(instance.clone());
                Ok(Reverse::PopInstance(cell.clone()))
            }
            Edit::AddArray { cell, array } => {
                let target = doc
                    .cell_mut(cell)
                    .ok_or_else(|| ModelError::CellNotFound(cell.clone()))?;
                target.arrays.push(array.clone());
                Ok(Reverse::PopArray(cell.clone()))
            }
            Edit::AddLabel { cell, label } => {
                let target = doc
                    .cell_mut(cell)
                    .ok_or_else(|| ModelError::CellNotFound(cell.clone()))?;
                target.labels.push(label.clone());
                Ok(Reverse::PopLabel(cell.clone()))
            }
            Edit::RemoveLabel { cell, index } => {
                let target = doc
                    .cell_mut(cell)
                    .ok_or_else(|| ModelError::CellNotFound(cell.clone()))?;
                if *index >= target.labels.len() {
                    return Err(ModelError::IndexOutOfBounds(*index));
                }
                let label = target.labels.remove(*index);
                Ok(Reverse::InsertLabel(cell.clone(), *index, Box::new(label)))
            }
        }
    }

    /// Applies a recorded reverse operation, mutating `doc` back one step.
    fn apply_reverse(doc: &mut Document, reverse: Reverse) {
        match reverse {
            Reverse::RemoveCell(name) => {
                doc.remove_cell(&name);
            }
            Reverse::RestoreCell(cell) => {
                doc.insert_cell(*cell);
            }
            Reverse::PopShape(cell) => {
                if let Some(target) = doc.cell_mut(&cell) {
                    target.shapes.pop();
                }
            }
            Reverse::InsertShape(cell, index, shape) => {
                if let Some(target) = doc.cell_mut(&cell) {
                    target.shapes.insert(index, *shape);
                }
            }
            Reverse::PopInstance(cell) => {
                if let Some(target) = doc.cell_mut(&cell) {
                    target.instances.pop();
                }
            }
            Reverse::PopArray(cell) => {
                if let Some(target) = doc.cell_mut(&cell) {
                    target.arrays.pop();
                }
            }
            Reverse::PopLabel(cell) => {
                if let Some(target) = doc.cell_mut(&cell) {
                    target.labels.pop();
                }
            }
            Reverse::InsertLabel(cell, index, label) => {
                if let Some(target) = doc.cell_mut(&cell) {
                    target.labels.insert(index, *label);
                }
            }
        }
    }
}

impl DocumentStore for EditableDocument {
    fn cell_names(&self) -> Vec<String> {
        self.doc.cells().map(|cell| cell.name.clone()).collect()
    }

    fn top_cells(&self) -> &[String] {
        self.doc.top_cells()
    }

    fn apply(&mut self, edit: Edit) -> Result<()> {
        // Execute first: on error the document is unchanged, so the cache stays
        // valid. On success the document changed, so drop the memoized boxes.
        let reverse = Self::execute(&mut self.doc, &edit)?;
        self.invalidate_bbox_cache();
        self.revision += 1;
        self.undo_stack.push((edit, reverse));
        self.redo_stack.clear();
        Ok(())
    }

    fn undo(&mut self) -> bool {
        if let Some((edit, reverse)) = self.undo_stack.pop() {
            Self::apply_reverse(&mut self.doc, reverse);
            self.invalidate_bbox_cache();
            self.revision += 1;
            self.redo_stack.push(edit);
            true
        } else {
            false
        }
    }

    fn redo(&mut self) -> bool {
        if let Some(edit) = self.redo_stack.pop() {
            // Re-applying from the restored pre-edit state reproduces the edit.
            match Self::execute(&mut self.doc, &edit) {
                Ok(reverse) => {
                    self.invalidate_bbox_cache();
                    self.revision += 1;
                    self.undo_stack.push((edit, reverse));
                    true
                }
                Err(_) => false,
            }
        } else {
            false
        }
    }
}
