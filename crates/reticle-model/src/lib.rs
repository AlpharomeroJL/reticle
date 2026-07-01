//! The hierarchical layout document model for Reticle.
//!
//! This crate owns the native document types, [`Document`], [`Cell`],
//! [`Instance`], [`ArrayInstance`], [`DrawShape`], and the cross-crate trait
//! surface ([`DocumentStore`], [`RuleSet`], [`Router`], [`Importer`], [`Exporter`],
//! [`Renderer`]) that Wave 2+ crates implement. It builds on `reticle-geometry`
//! and, like it, contains no GPU, async, or UI code.
//!
//! Scale comes from hierarchy: a modest [`Cell`] placed by an [`ArrayInstance`]
//! thousands of times yields effectively billions of leaf shapes that still browse
//! interactively through cell-level culling and LOD (see the rendering crate).

use core::fmt;

mod document;
mod edit;
mod editable;
mod rules;
mod traits;

pub use document::{
    ArrayInstance, Cell, Document, DrawShape, Instance, LayerInfo, ShapeKind, Technology,
};
pub use edit::Edit;
pub use editable::EditableDocument;
pub use rules::{Rule, RuleKind, RuleSet, Violation};
pub use traits::{
    Camera, DocumentStore, Exporter, Importer, NetSpec, Renderer, RouteReport, RouteRequest, Router,
};

/// Result type for fallible model operations.
pub type Result<T> = core::result::Result<T, ModelError>;

/// Errors produced by document operations.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModelError {
    /// A referenced cell does not exist.
    CellNotFound(String),
    /// A cell with this name already exists.
    DuplicateCell(String),
    /// An index was out of bounds for the target collection.
    IndexOutOfBounds(usize),
    /// A wrapped geometry error.
    Geometry(reticle_geometry::GeometryError),
    /// The operation is not supported, with a reason.
    Unsupported(&'static str),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CellNotFound(n) => write!(f, "cell not found: {n}"),
            Self::DuplicateCell(n) => write!(f, "duplicate cell: {n}"),
            Self::IndexOutOfBounds(i) => write!(f, "index out of bounds: {i}"),
            Self::Geometry(e) => write!(f, "geometry error: {e}"),
            Self::Unsupported(why) => write!(f, "unsupported model operation: {why}"),
        }
    }
}

impl core::error::Error for ModelError {}

impl From<reticle_geometry::GeometryError> for ModelError {
    fn from(e: reticle_geometry::GeometryError) -> Self {
        Self::Geometry(e)
    }
}
