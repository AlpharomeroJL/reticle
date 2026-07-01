//! The hierarchical document: cells, instances, arrays, layers, and technology.

use reticle_geometry::{Dbu, LayerId, Path, Polygon, Rect, Shape, Transform};
use std::collections::HashMap;

/// A concrete piece of drawn geometry on a layer.
#[derive(Clone, PartialEq, Debug)]
pub enum ShapeKind {
    /// An axis-aligned rectangle.
    Rect(Rect),
    /// A polygon.
    Polygon(Polygon),
    /// A path (wire).
    Path(Path),
}

/// A drawable shape: geometry tagged with the layer it lives on. Implements the
/// geometry [`Shape`] trait so it can be indexed, rendered, and checked.
#[derive(Clone, PartialEq, Debug)]
pub struct DrawShape {
    /// The layer/datatype this shape is drawn on.
    pub layer: LayerId,
    /// The geometry.
    pub kind: ShapeKind,
}

impl DrawShape {
    /// Creates a drawable shape from a layer and geometry.
    #[must_use]
    pub fn new(layer: LayerId, kind: ShapeKind) -> Self {
        Self { layer, kind }
    }
}

impl Shape for DrawShape {
    fn bounding_box(&self) -> Rect {
        match &self.kind {
            ShapeKind::Rect(r) => *r,
            ShapeKind::Polygon(p) => p.bounding_box(),
            ShapeKind::Path(p) => p.bounding_box(),
        }
    }

    fn layer(&self) -> LayerId {
        self.layer
    }
}

/// A single placement of another cell, with a transform.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Instance {
    /// Name of the referenced cell.
    pub cell: String,
    /// Placement transform applied to the referenced cell's geometry.
    pub transform: Transform,
}

/// A regular array of placements of another cell (rows × columns with pitch).
///
/// This is the primary source of scale: a modest cell arrayed thousands of times
/// yields effectively billions of leaf shapes that still browse interactively via
/// cell-level culling and LOD.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ArrayInstance {
    /// Name of the referenced cell.
    pub cell: String,
    /// Transform of the array's origin element.
    pub transform: Transform,
    /// Number of columns (x repetitions).
    pub columns: u32,
    /// Number of rows (y repetitions).
    pub rows: u32,
    /// Column pitch in DBU.
    pub column_pitch: Dbu,
    /// Row pitch in DBU.
    pub row_pitch: Dbu,
}

impl ArrayInstance {
    /// Total number of placements (`columns * rows`).
    #[must_use]
    pub fn count(&self) -> u64 {
        u64::from(self.columns) * u64::from(self.rows)
    }
}

/// A cell: a named collection of geometry plus placements of other cells.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Cell {
    /// The cell's unique name within the document.
    pub name: String,
    /// Flat geometry owned directly by this cell.
    pub shapes: Vec<DrawShape>,
    /// Single placements of other cells.
    pub instances: Vec<Instance>,
    /// Arrayed placements of other cells.
    pub arrays: Vec<ArrayInstance>,
    /// Cached bounding box of this cell's own geometry (excluding children),
    /// filled by the Wave 2 bbox-cache pass. `None` until computed.
    pub(crate) cached_bbox: Option<Rect>,
}

impl Cell {
    /// Creates an empty cell with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// The bounding box of the cell's own (non-instanced) geometry, or `None` if
    /// the cell has no direct shapes.
    #[must_use]
    pub fn shapes_bbox(&self) -> Option<Rect> {
        self.shapes
            .iter()
            .map(Shape::bounding_box)
            .reduce(|a, b| a.union(&b))
    }
}

/// Display and identification metadata for one layer.
#[derive(Clone, PartialEq, Debug)]
pub struct LayerInfo {
    /// The layer/datatype identifier.
    pub id: LayerId,
    /// Human-readable layer name.
    pub name: String,
    /// Packed `0xRRGGBBAA` display color.
    pub color_rgba: u32,
    /// Whether the layer is currently visible.
    pub visible: bool,
}

/// The technology description: database resolution, layers, and DRC rules.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Technology {
    /// Technology name.
    pub name: String,
    /// Database units per micron (the coordinate resolution).
    pub dbu_per_micron: i64,
    /// Layer table.
    pub layers: Vec<LayerInfo>,
    /// Declarative DRC rules (see [`crate::Rule`]).
    pub rules: Vec<crate::Rule>,
}

/// A hierarchical layout document: a set of named cells plus technology data.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Document {
    cells: HashMap<String, Cell>,
    top_cells: Vec<String>,
    technology: Technology,
}

impl Document {
    /// Creates an empty document.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or replaces a cell, returning the previous cell with that name.
    pub fn insert_cell(&mut self, cell: Cell) -> Option<Cell> {
        self.cells.insert(cell.name.clone(), cell)
    }

    /// Returns the cell with the given name, if present.
    #[must_use]
    pub fn cell(&self, name: &str) -> Option<&Cell> {
        self.cells.get(name)
    }

    /// Returns a mutable reference to the named cell, if present.
    pub fn cell_mut(&mut self, name: &str) -> Option<&mut Cell> {
        self.cells.get_mut(name)
    }

    /// Iterates over all cells in unspecified order.
    pub fn cells(&self) -> impl Iterator<Item = &Cell> {
        self.cells.values()
    }

    /// The number of cells in the document.
    #[must_use]
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// The names of the document's top (root) cells.
    #[must_use]
    pub fn top_cells(&self) -> &[String] {
        &self.top_cells
    }

    /// Sets the top-cell list.
    pub fn set_top_cells(&mut self, tops: Vec<String>) {
        self.top_cells = tops;
    }

    /// The technology data.
    #[must_use]
    pub fn technology(&self) -> &Technology {
        &self.technology
    }

    /// Replaces the technology data.
    pub fn set_technology(&mut self, tech: Technology) {
        self.technology = tech;
    }
}
