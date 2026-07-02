//! The hierarchical document: cells, instances, arrays, layers, and technology.

use reticle_geometry::{Dbu, LayerId, Path, Point, Polygon, Rect, Shape, Transform};
use std::collections::{HashMap, HashSet};

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
    /// Text labels owned by this cell (GDSII TEXT records).
    pub labels: Vec<crate::Label>,
    /// Named terminals (pins) this cell exposes.
    pub pins: Vec<crate::Pin>,
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

/// Physical layer-stack data for one layer: where the layer sits in z and how
/// thick it is, both in integer nanometers.
///
/// This is a separate table on [`Technology`] rather than extra fields on
/// [`LayerInfo`]: stack data is optional and orthogonal to the display layer
/// table (a `stack` directive may describe a layer that has no `layer` entry
/// and vice versa), and a separate list keeps every existing [`Technology`]
/// constructor and every technology file without stack lines working
/// unchanged.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StackEntry {
    /// The layer/datatype this entry describes.
    pub layer: LayerId,
    /// Bottom of the layer slab, in nanometers. May be negative (below the
    /// substrate origin).
    pub z_bottom_nm: i64,
    /// Slab thickness in nanometers; always positive.
    pub thickness_nm: i64,
}

impl StackEntry {
    /// Top of the layer slab, in nanometers (`z_bottom_nm + thickness_nm`),
    /// saturating at the numeric range instead of wrapping.
    #[must_use]
    pub fn z_top_nm(&self) -> i64 {
        self.z_bottom_nm.saturating_add(self.thickness_nm)
    }
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
    /// Physical layer-stack entries from `stack` directives, in declaration
    /// order. Empty when the technology declares no stack data.
    pub stack: Vec<StackEntry>,
}

impl Technology {
    /// The stack entry for `layer`, if the technology declares one.
    ///
    /// When a layer is declared more than once the first declaration wins,
    /// matching how the renderer's palette resolves duplicate layer entries.
    #[must_use]
    pub fn stack_for(&self, layer: LayerId) -> Option<&StackEntry> {
        self.stack.iter().find(|entry| entry.layer == layer)
    }
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

    /// Removes and returns the named cell, if present.
    pub fn remove_cell(&mut self, name: &str) -> Option<Cell> {
        self.cells.remove(name)
    }

    /// The full bounding box of a cell in its own coordinate system, including the
    /// transformed bounding boxes of every instance and array, computed
    /// recursively. Returns `None` for a missing or empty cell.
    #[must_use]
    pub fn cell_bbox(&self, name: &str) -> Option<Rect> {
        let mut visiting = HashSet::new();
        self.cell_bbox_visiting(name, &mut visiting)
    }

    fn cell_bbox_visiting(&self, name: &str, visiting: &mut HashSet<String>) -> Option<Rect> {
        if !visiting.insert(name.to_owned()) {
            return None; // cycle guard
        }
        let result = self.cell(name).and_then(|cell| {
            let mut bbox = cell.shapes_bbox();
            for inst in &cell.instances {
                if let Some(child) = self.cell_bbox_visiting(&inst.cell, visiting) {
                    let placed = transform_rect(&inst.transform, child);
                    bbox = Some(bbox.map_or(placed, |acc| acc.union(&placed)));
                }
            }
            for array in &cell.arrays {
                if let Some(child) = self.cell_bbox_visiting(&array.cell, visiting) {
                    let base = transform_rect(&array.transform, child);
                    let dx = array.column_pitch.saturating_mul(span(array.columns));
                    let dy = array.row_pitch.saturating_mul(span(array.rows));
                    let far = Rect::new(base.min.translate(dx, dy), base.max.translate(dx, dy));
                    let spanned = base.union(&far);
                    bbox = Some(bbox.map_or(spanned, |acc| acc.union(&spanned)));
                }
            }
            bbox
        });
        visiting.remove(name);
        result
    }

    /// Flattens `top` into a flat list of shapes in `top`'s coordinate system,
    /// recursively expanding instances and arrays and applying their transforms.
    ///
    /// This materializes every leaf shape, so for a design with large arrays the
    /// output can be enormous; use it only when a tool genuinely needs the expanded
    /// geometry.
    #[must_use]
    pub fn flatten(&self, top: &str) -> Vec<DrawShape> {
        let mut visiting = HashSet::new();
        self.flatten_local(top, &mut visiting)
    }

    fn flatten_local(&self, name: &str, visiting: &mut HashSet<String>) -> Vec<DrawShape> {
        if !visiting.insert(name.to_owned()) {
            return Vec::new(); // cycle guard
        }
        let mut out = Vec::new();
        if let Some(cell) = self.cell(name) {
            out.extend(cell.shapes.iter().cloned());
            for inst in &cell.instances {
                for shape in self.flatten_local(&inst.cell, visiting) {
                    out.push(transform_shape(&inst.transform, &shape));
                }
            }
            for array in &cell.arrays {
                let child = self.flatten_local(&array.cell, visiting);
                for col in 0..array.columns {
                    for row in 0..array.rows {
                        let dx = array.column_pitch.saturating_mul(span_from(col));
                        let dy = array.row_pitch.saturating_mul(span_from(row));
                        for shape in &child {
                            let placed = transform_shape(&array.transform, shape);
                            out.push(translate_shape(&placed, dx, dy));
                        }
                    }
                }
            }
        }
        visiting.remove(name);
        out
    }
}

/// The span multiplier for an array dimension of `count` repetitions: `count - 1`,
/// clamped into the coordinate range.
fn span(count: u32) -> Dbu {
    Dbu::try_from(count.saturating_sub(1)).unwrap_or(Dbu::MAX)
}

/// The offset multiplier for the `index`-th repetition in an array.
fn span_from(index: u32) -> Dbu {
    Dbu::try_from(index).unwrap_or(Dbu::MAX)
}

/// Transforms an axis-aligned rectangle by `transform` and returns the bounding box
/// of the result (exact for the dihedral orientations and integer magnifications
/// used by placements).
fn transform_rect(transform: &Transform, rect: Rect) -> Rect {
    let corners = [
        rect.min,
        Point::new(rect.max.x, rect.min.y),
        rect.max,
        Point::new(rect.min.x, rect.max.y),
    ];
    Rect::from_points(corners.into_iter().map(|corner| transform.apply(corner))).unwrap_or_default()
}

/// Transforms a drawable shape by an orientation/magnification/translation.
fn transform_shape(transform: &Transform, shape: &DrawShape) -> DrawShape {
    let kind = match &shape.kind {
        ShapeKind::Rect(rect) => ShapeKind::Rect(transform_rect(transform, *rect)),
        ShapeKind::Polygon(poly) => ShapeKind::Polygon(Polygon::new(
            poly.vertices()
                .iter()
                .map(|pt| transform.apply(*pt))
                .collect(),
        )),
        ShapeKind::Path(path) => ShapeKind::Path(Path::new(
            path.points()
                .iter()
                .map(|pt| transform.apply(*pt))
                .collect(),
            transform.magnification.scale(path.width()),
            path.endcap(),
        )),
    };
    DrawShape::new(shape.layer, kind)
}

/// Translates a drawable shape by `(dx, dy)` DBU.
fn translate_shape(shape: &DrawShape, dx: Dbu, dy: Dbu) -> DrawShape {
    let kind = match &shape.kind {
        ShapeKind::Rect(rect) => ShapeKind::Rect(Rect::new(
            rect.min.translate(dx, dy),
            rect.max.translate(dx, dy),
        )),
        ShapeKind::Polygon(poly) => ShapeKind::Polygon(Polygon::new(
            poly.vertices()
                .iter()
                .map(|pt| pt.translate(dx, dy))
                .collect(),
        )),
        ShapeKind::Path(path) => ShapeKind::Path(Path::new(
            path.points()
                .iter()
                .map(|pt| pt.translate(dx, dy))
                .collect(),
            path.width(),
            path.endcap(),
        )),
    };
    DrawShape::new(shape.layer, kind)
}
