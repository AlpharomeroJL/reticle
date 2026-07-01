//! Transactional edit operations and the undo/redo history.
//!
//! The [`Edit`] enum is the frozen vocabulary of document mutations. Their
//! transactional application and inverse computation live in
//! [`EditableDocument`](crate::EditableDocument); the enum here is the contract.

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

// The transactional application of these edits lives in `editable::EditableDocument`.
