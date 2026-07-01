//! Transactional edit operations and the undo/redo history.
//!
//! The [`Edit`] enum is the frozen vocabulary of document mutations. The Wave 2
//! `reticle-model` lane implements the transactional application and inverse
//! computation that drive [`History`]; the types here are the contract.

use crate::{ArrayInstance, Cell, DrawShape, Instance};

/// A single, reversible mutation of a document.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Edit {
    /// Insert a new (empty or populated) cell.
    AddCell {
        /// The cell to add.
        cell: Cell,
    },
    /// Remove a cell by name.
    RemoveCell {
        /// Name of the cell to remove.
        name: String,
    },
    /// Append a shape to a cell.
    AddShape {
        /// Target cell name.
        cell: String,
        /// The shape to add.
        shape: DrawShape,
    },
    /// Remove the shape at `index` from a cell.
    RemoveShape {
        /// Target cell name.
        cell: String,
        /// Index of the shape to remove.
        index: usize,
    },
    /// Add a single instance placement to a cell.
    AddInstance {
        /// Target cell name.
        cell: String,
        /// The instance to add.
        instance: Instance,
    },
    /// Add an array placement to a cell.
    AddArray {
        /// Target cell name.
        cell: String,
        /// The array to add.
        array: ArrayInstance,
    },
}

/// An undo/redo history of applied edits.
///
/// Wave 2 fills in the transactional application and inverse-edit computation;
/// this holds the frozen shape of the stacks.
#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

impl History {
    /// Creates an empty history.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of edits that can be undone.
    #[must_use]
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    /// Number of edits that can be redone.
    #[must_use]
    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    /// Records an applied edit, clearing the redo stack.
    pub fn record(&mut self, edit: Edit) {
        self.undo.push(edit);
        self.redo.clear();
    }
}
