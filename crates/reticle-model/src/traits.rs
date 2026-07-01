//! The cross-crate trait surface: document access, routing, import/export, and
//! rendering. These are the stable interfaces that Wave 2+ crates implement, kept
//! abstract here so the core model never depends on the GPU, IO, or async stacks.

use crate::{Document, Edit, Result};
use reticle_geometry::{LayerId, Point, Rect};

/// Read/write access to a hierarchical document with undo/redo.
///
/// The concrete implementation is the Wave 2 editable document wrapping
/// [`Document`] with a [`crate::History`].
pub trait DocumentStore {
    /// The names of all cells.
    fn cell_names(&self) -> Vec<String>;

    /// The names of the top (root) cells.
    fn top_cells(&self) -> &[String];

    /// Applies an edit transactionally.
    ///
    /// # Errors
    ///
    /// Returns a [`crate::ModelError`] if the edit references a missing cell or is
    /// otherwise invalid.
    fn apply(&mut self, edit: Edit) -> Result<()>;

    /// Undoes the most recent edit; returns `false` if there was nothing to undo.
    fn undo(&mut self) -> bool;

    /// Redoes the most recently undone edit; returns `false` if none.
    fn redo(&mut self) -> bool;
}

/// A single net to be routed: named terminals to connect on a layer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NetSpec {
    /// Net name.
    pub name: String,
    /// Terminal points to connect, in DBU.
    pub terminals: Vec<Point>,
    /// The routing layer.
    pub layer: LayerId,
}

/// A routing request: the nets to route into a cell.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct RouteRequest {
    /// Target cell name.
    pub cell: String,
    /// Nets to route.
    pub nets: Vec<NetSpec>,
}

/// The outcome of a routing run.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct RouteReport {
    /// Number of nets successfully routed.
    pub routed: usize,
    /// Number of nets that could not be routed.
    pub failed: usize,
    /// Total routed wire length in DBU.
    pub total_length_dbu: i64,
}

/// A router that adds routing geometry to a document. Implemented by `reticle-route`.
pub trait Router {
    /// Routes `request` into `doc`, returning a summary report.
    fn route(&mut self, doc: &mut Document, request: &RouteRequest) -> RouteReport;
}

/// Imports a document from encoded bytes (GDSII, OASIS, VLSIR).
/// Implemented by `reticle-io`.
pub trait Importer {
    /// Parses `bytes` into a document.
    ///
    /// # Errors
    ///
    /// Returns a [`crate::ModelError`] on malformed or unsupported input.
    fn import(&self, bytes: &[u8]) -> Result<Document>;
}

/// Exports a document to encoded bytes. Implemented by `reticle-io`.
pub trait Exporter {
    /// Serializes `doc` to the target format.
    ///
    /// # Errors
    ///
    /// Returns a [`crate::ModelError`] if the document cannot be represented.
    fn export(&self, doc: &Document) -> Result<Vec<u8>>;
}

/// A 2D camera: the view onto the layout used by a [`Renderer`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Camera {
    /// World-space center of the view, in DBU.
    pub center: Point,
    /// Zoom in device pixels per DBU.
    pub pixels_per_dbu: f32,
    /// The visible world rectangle, in DBU.
    pub viewport: Rect,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: Point::ORIGIN,
            pixels_per_dbu: 1.0,
            viewport: Rect::default(),
        }
    }
}

/// Renders a document. Implemented by `reticle-render` on top of `wgpu`, kept
/// abstract here so core crates never depend on the GPU stack.
pub trait Renderer {
    /// Renders `doc` as seen through `camera`.
    fn render(&mut self, doc: &Document, camera: &Camera);
}
