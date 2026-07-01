//! An editable document with an exact transactional undo/redo history.
//!
//! [`EditableDocument`] wraps a [`Document`] and implements [`DocumentStore`].
//! Each applied [`Edit`] records the data needed to reverse it, so undo and redo
//! are exact and unbounded. The same edit log is what the collaboration layer
//! replicates.

use crate::{Document, DocumentStore, Edit, ModelError, Result};

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
}

/// A [`Document`] paired with a transactional undo/redo history.
#[derive(Debug, Default)]
pub struct EditableDocument {
    doc: Document,
    undo_stack: Vec<(Edit, Reverse)>,
    redo_stack: Vec<Edit>,
}

impl EditableDocument {
    /// Wraps an existing document with an empty history.
    #[must_use]
    pub fn new(doc: Document) -> Self {
        Self {
            doc,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
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
        let reverse = Self::execute(&mut self.doc, &edit)?;
        self.undo_stack.push((edit, reverse));
        self.redo_stack.clear();
        Ok(())
    }

    fn undo(&mut self) -> bool {
        if let Some((edit, reverse)) = self.undo_stack.pop() {
            Self::apply_reverse(&mut self.doc, reverse);
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
