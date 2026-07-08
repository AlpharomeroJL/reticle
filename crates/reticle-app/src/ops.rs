//! Boolean and transform operations on the current selection.
//!
//! This module turns a shape selection into document [`Edit`]s: boolean combines
//! (union / intersection / difference / XOR), an offset (grow / shrink), rotate and
//! mirror, and align / distribute. The heavy geometry (robust polygon booleans and
//! offsetting) is delegated to [`reticle_geometry`]; everything here is the glue that
//! maps selected scene shapes back to editable cell shapes, applies the operation,
//! and packages the result as one undo step.
//!
//! # Layout of the module
//!
//! * The pure logic ([`boolean_edits`], [`offset_edits`], [`rotate_edits`],
//!   [`mirror_edits`], [`align_edits`], [`distribute_edits`]) takes the flattened
//!   scene shapes, the selected indices, and how many of those shapes are directly
//!   editable, and returns a `Vec<Edit>` (or nothing when the operation does not
//!   apply). These are window-free and unit-tested.
//! * [`OpsState`] holds the panel's numeric inputs, and the `ops_panel` method on
//!   the app root renders it.
//!
//! # What can be operated on
//!
//! The selection indexes the *flattened* top-cell scene. Only the top cell's own
//! shapes are editable by index, and [`Document::flatten`](reticle_model::Document)
//! emits those first, so a scene index below the editable count maps one-to-one to a
//! top-cell shape index. Selected indices at or above that count come from placed
//! instances and are skipped: there is no single shape in the top cell to rewrite.
//!
//! Booleans and offset run on *filled* geometry, so they consider rectangles and
//! polygons; a [`Path`](reticle_geometry::Path) is a stroked wire, not a fill region,
//! and is skipped from boolean and offset input. Transforms (rotate, mirror, align,
//! distribute) are coordinate maps and apply to every shape kind.

use eframe::egui;
use reticle_geometry::{BooleanOp, Dbu, Point, Polygon, Rect, Shape as _, offset, polygon_boolean};
use reticle_model::{DrawShape, Edit, ShapeKind};
use std::collections::BTreeMap;

/// Which planar boolean to run on the selection.
///
/// A thin echo of [`reticle_geometry::BooleanOp`], kept separate so the panel can
/// list the operations without importing the geometry enum directly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BoolKind {
    /// Merge the selected shapes into their union.
    Union,
    /// Keep only the region common to all selected shapes.
    Intersection,
    /// Subtract the later shapes from the first (by scene order).
    Difference,
    /// Keep the region covered by an odd number of shapes.
    Xor,
}

impl BoolKind {
    /// The geometry engine's operation for this kind.
    fn engine(self) -> BooleanOp {
        match self {
            Self::Union => BooleanOp::Union,
            Self::Intersection => BooleanOp::Intersection,
            Self::Difference => BooleanOp::Difference,
            Self::Xor => BooleanOp::Xor,
        }
    }

    /// A short human label for the panel and status line.
    fn label(self) -> &'static str {
        match self {
            Self::Union => "Union",
            Self::Intersection => "Intersection",
            Self::Difference => "Difference",
            Self::Xor => "XOR",
        }
    }
}

/// Which way to align a selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AlignKind {
    /// Line up the shapes' left edges to the leftmost edge.
    Left,
    /// Line up the shapes' right edges to the rightmost edge.
    Right,
    /// Center the shapes horizontally on the selection's center.
    CenterX,
    /// Line up the shapes' top edges to the topmost edge.
    Top,
    /// Line up the shapes' bottom edges to the bottommost edge.
    Bottom,
    /// Center the shapes vertically on the selection's center.
    CenterY,
}

/// The axis a mirror reflects across.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MirrorAxis {
    /// Reflect left-to-right about the selection's vertical center line.
    Vertical,
    /// Reflect top-to-bottom about the selection's horizontal center line.
    Horizontal,
}

/// The panel's numeric inputs and last-run status.
///
/// Pure UI state: it holds what the user typed and the message to echo, and never
/// borrows the document. [`crate::app::App`] owns one of these and passes the
/// numbers into the pure edit builders.
#[derive(Clone, Debug)]
pub struct OpsState {
    /// The boolean chosen in the panel's dropdown.
    pub bool_kind: BoolKind,
    /// Offset amount in DBU (positive grows, negative shrinks).
    pub offset_dbu: i32,
    /// Rotation angle in degrees, applied counter-clockwise about the selection
    /// center.
    pub rotate_deg: f64,
    /// The axis the mirror control reflects across.
    pub mirror_axis: MirrorAxis,
    /// The last operation's status message, shown under the controls.
    pub status: String,
}

impl Default for OpsState {
    fn default() -> Self {
        Self {
            bool_kind: BoolKind::Union,
            offset_dbu: 100,
            rotate_deg: 90.0,
            mirror_axis: MirrorAxis::Vertical,
            status: String::new(),
        }
    }
}

impl OpsState {
    /// Creates the default panel state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// One selected, directly-editable shape: its index in the top cell plus a clone of
/// its geometry. Produced by [`editable_selection`] so the edit builders work from a
/// stable snapshot rather than re-reading the scene.
#[derive(Clone, Debug)]
struct Selected {
    /// Index of the shape in the top cell's `shapes` list.
    cell_index: usize,
    /// The shape itself.
    shape: DrawShape,
}

/// Resolves the selected scene indices to the top cell's own editable shapes.
///
/// Scene index `i` is an editable top-cell shape exactly when `i < editable_count`
/// (the count of the top cell's own shapes); higher indices are flattened instance
/// geometry and are dropped. The returned list is sorted by `cell_index` ascending.
fn editable_selection(
    scene_shapes: &[DrawShape],
    selection: &[usize],
    editable_count: usize,
) -> Vec<Selected> {
    let mut out: Vec<Selected> = selection
        .iter()
        .copied()
        .filter(|&i| i < editable_count && i < scene_shapes.len())
        .map(|i| Selected {
            cell_index: i,
            shape: scene_shapes[i].clone(),
        })
        .collect();
    out.sort_by_key(|s| s.cell_index);
    out
}

/// Converts a fillable shape (rectangle or polygon) into a boolean-input polygon.
/// Returns `None` for a path, which is a stroke rather than a fill region.
fn shape_as_polygon(shape: &DrawShape) -> Option<Polygon> {
    match &shape.kind {
        ShapeKind::Rect(r) => Some(Polygon::from_rect(*r)),
        ShapeKind::Polygon(p) => Some(p.clone()),
        ShapeKind::Path(_) => None,
    }
}

/// Removals of a set of cell indices, ordered so earlier removals do not shift the
/// indices of later ones (highest index first). Every produced [`Edit::RemoveShape`]
/// targets `cell`.
fn removals(cell: &str, mut cell_indices: Vec<usize>) -> Vec<Edit> {
    cell_indices.sort_unstable();
    cell_indices.dedup();
    cell_indices
        .into_iter()
        .rev()
        .map(|index| Edit::RemoveShape {
            cell: cell.to_owned(),
            index,
        })
        .collect()
}

/// Builds the edits for a boolean over the selection, grouped as one undo step.
///
/// Shapes are grouped by layer; only shapes on the *same* layer combine, and each
/// group's result stays on that layer (per-layer rules). Within a layer the fillable
/// shapes are combined with `kind`, their inputs removed, and the resulting polygons
/// added back. Returns an empty vector when nothing applies (fewer than two fillable
/// shapes share any layer, or the result is empty).
///
/// The returned edits remove every consumed input (across all layers) before adding
/// any result, and removals are ordered highest-index-first so the batch applies
/// cleanly as a group.
#[must_use]
pub fn boolean_edits(
    kind: BoolKind,
    scene_shapes: &[DrawShape],
    selection: &[usize],
    top_cell: &str,
    editable_count: usize,
) -> Vec<Edit> {
    let selected = editable_selection(scene_shapes, selection, editable_count);

    // Group the fillable selected shapes by layer, preserving scene order within a
    // layer (a BTreeMap keyed on the layer's packed bits keeps this deterministic).
    let mut by_layer: BTreeMap<u32, Vec<&Selected>> = BTreeMap::new();
    for s in &selected {
        if shape_as_polygon(&s.shape).is_some() {
            let key = u32::from(s.shape.layer.layer) << 16 | u32::from(s.shape.layer.datatype);
            by_layer.entry(key).or_default().push(s);
        }
    }

    let mut removed_indices: Vec<usize> = Vec::new();
    let mut additions: Vec<Edit> = Vec::new();

    for group in by_layer.values() {
        // A boolean needs at least two shapes on the layer to be meaningful.
        if group.len() < 2 {
            continue;
        }
        let layer = group[0].shape.layer;
        let polys: Vec<Polygon> = group
            .iter()
            .filter_map(|s| shape_as_polygon(&s.shape))
            .collect();

        let result = fold_boolean(kind.engine(), &polys);

        // Only commit the group if the engine produced geometry; otherwise leave the
        // inputs untouched (e.g. an empty intersection would just delete shapes).
        if result.is_empty() {
            continue;
        }
        for s in group {
            removed_indices.push(s.cell_index);
        }
        for poly in result {
            additions.push(Edit::AddShape {
                cell: top_cell.to_owned(),
                shape: DrawShape::new(layer, ShapeKind::Polygon(poly)),
            });
        }
    }

    if removed_indices.is_empty() {
        return Vec::new();
    }
    let mut edits = removals(top_cell, removed_indices);
    edits.extend(additions);
    edits
}

/// Folds a boolean across a list of polygons on one layer.
///
/// Union / intersection / XOR are associative, so we accumulate pairwise. Difference
/// subtracts every later shape from the first, matching the usual "A minus the rest"
/// editor behavior. Skips degenerate (empty) inputs.
fn fold_boolean(op: BooleanOp, polys: &[Polygon]) -> Vec<Polygon> {
    let mut it = polys.iter().filter(|p| p.len() >= 3);
    let Some(first) = it.next() else {
        return Vec::new();
    };
    let mut acc = vec![first.clone()];
    for poly in it {
        let rhs = vec![poly.clone()];
        acc = polygon_boolean(op, &acc, &rhs).unwrap_or_default();
        if acc.is_empty() {
            break;
        }
    }
    acc
}

/// Builds the edits to offset (grow for positive `delta`, shrink for negative) each
/// fillable selected shape by `delta` DBU, grouped as one undo step. Each shape's
/// offset result replaces it on the same layer. Paths and non-editable selections are
/// skipped; a zero delta or empty selection yields no edits.
#[must_use]
pub fn offset_edits(
    delta: Dbu,
    scene_shapes: &[DrawShape],
    selection: &[usize],
    top_cell: &str,
    editable_count: usize,
) -> Vec<Edit> {
    if delta == 0 {
        return Vec::new();
    }
    let selected = editable_selection(scene_shapes, selection, editable_count);
    let mut removed_indices: Vec<usize> = Vec::new();
    let mut additions: Vec<Edit> = Vec::new();

    for s in &selected {
        let Some(poly) = shape_as_polygon(&s.shape) else {
            continue;
        };
        let result = offset(&[poly], delta).unwrap_or_default();
        // Drop shapes that a shrink collapsed to nothing.
        let result: Vec<Polygon> = result.into_iter().filter(|p| p.len() >= 3).collect();
        if result.is_empty() && delta > 0 {
            continue;
        }
        removed_indices.push(s.cell_index);
        for poly in result {
            additions.push(Edit::AddShape {
                cell: top_cell.to_owned(),
                shape: DrawShape::new(s.shape.layer, ShapeKind::Polygon(poly)),
            });
        }
    }

    if removed_indices.is_empty() {
        return Vec::new();
    }
    let mut edits = removals(top_cell, removed_indices);
    edits.extend(additions);
    edits
}

/// A point-to-point map applied to every vertex of a shape. Used to implement rotate
/// and mirror without special-casing each shape kind at the call site.
fn map_shape(shape: &DrawShape, f: impl Fn(Point) -> Point) -> DrawShape {
    let kind = match &shape.kind {
        ShapeKind::Rect(r) => {
            // A rotation or reflection can tilt a rectangle, so promote it to a
            // polygon and map the corners; an axis mirror keeps it a box but the
            // polygon form is still correct and simpler.
            let poly = Polygon::from_rect(*r);
            let mapped: Vec<Point> = poly.vertices().iter().map(|p| f(*p)).collect();
            // If the map kept the shape axis-aligned we could re-detect a rect, but a
            // polygon renders and checks identically, so keep it simple.
            ShapeKind::Polygon(Polygon::new(mapped))
        }
        ShapeKind::Polygon(p) => {
            ShapeKind::Polygon(Polygon::new(p.vertices().iter().map(|v| f(*v)).collect()))
        }
        ShapeKind::Path(p) => ShapeKind::Path(reticle_geometry::Path::new(
            p.points().iter().map(|v| f(*v)).collect(),
            p.width(),
            p.endcap(),
        )),
    };
    DrawShape::new(shape.layer, kind)
}

/// Rounds a floating coordinate to the nearest DBU, clamped to the coordinate range.
fn round_dbu(v: f64) -> Dbu {
    let r = v.round();
    if r >= f64::from(Dbu::MAX) {
        Dbu::MAX
    } else if r <= f64::from(Dbu::MIN) {
        Dbu::MIN
    } else {
        r as Dbu
    }
}

/// The center of the bounding box enclosing `shapes`, or `None` if the list is empty.
fn selection_center(shapes: &[&DrawShape]) -> Option<Point> {
    let bbox = combined_bbox(shapes)?;
    Some(Point::new(
        round_dbu(f64::midpoint(f64::from(bbox.min.x), f64::from(bbox.max.x))),
        round_dbu(f64::midpoint(f64::from(bbox.min.y), f64::from(bbox.max.y))),
    ))
}

/// The union of every shape's bounding box, or `None` for an empty list.
fn combined_bbox(shapes: &[&DrawShape]) -> Option<Rect> {
    shapes
        .iter()
        .map(|s| s.bounding_box())
        .reduce(|a, b| a.union(&b))
}

/// Builds the edits to rotate the selection by `degrees` counter-clockwise about its
/// center, grouped as one undo step. Each selected editable shape is replaced by its
/// rotated form.
///
/// Rotation runs on a floating basis and rounds each vertex to the nearest DBU, so a
/// non-orthogonal angle is on-grid but not exactly reversible; multiples of 90 degrees
/// are exact.
#[must_use]
pub fn rotate_edits(
    degrees: f64,
    scene_shapes: &[DrawShape],
    selection: &[usize],
    top_cell: &str,
    editable_count: usize,
) -> Vec<Edit> {
    let selected = editable_selection(scene_shapes, selection, editable_count);
    let refs: Vec<&DrawShape> = selected.iter().map(|s| &s.shape).collect();
    let Some(center) = selection_center(&refs) else {
        return Vec::new();
    };
    let radians = degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    let cx = f64::from(center.x);
    let cy = f64::from(center.y);
    let rotate = |p: Point| {
        let dx = f64::from(p.x) - cx;
        let dy = f64::from(p.y) - cy;
        Point::new(
            round_dbu(cx + dx * cos - dy * sin),
            round_dbu(cy + dx * sin + dy * cos),
        )
    };
    replace_each(&selected, top_cell, |s| map_shape(&s.shape, rotate))
}

/// Builds the edits to mirror the selection across `axis` through its center, grouped
/// as one undo step. An axis mirror is exact on the integer grid.
#[must_use]
pub fn mirror_edits(
    axis: MirrorAxis,
    scene_shapes: &[DrawShape],
    selection: &[usize],
    top_cell: &str,
    editable_count: usize,
) -> Vec<Edit> {
    let selected = editable_selection(scene_shapes, selection, editable_count);
    let refs: Vec<&DrawShape> = selected.iter().map(|s| &s.shape).collect();
    let Some(center) = selection_center(&refs) else {
        return Vec::new();
    };
    let reflect = move |p: Point| match axis {
        // Reflect x about the center's x (2*cx - x), keep y.
        MirrorAxis::Vertical => Point::new(2 * center.x - p.x, p.y),
        // Reflect y about the center's y, keep x.
        MirrorAxis::Horizontal => Point::new(p.x, 2 * center.y - p.y),
    };
    replace_each(&selected, top_cell, |s| map_shape(&s.shape, reflect))
}

/// The shared "replace every selected shape with a mapped version" edit builder used
/// by rotate and mirror. Removes each input (highest index first) and adds its
/// replacement, as one group.
fn replace_each(
    selected: &[Selected],
    top_cell: &str,
    make: impl Fn(&Selected) -> DrawShape,
) -> Vec<Edit> {
    if selected.is_empty() {
        return Vec::new();
    }
    let removed: Vec<usize> = selected.iter().map(|s| s.cell_index).collect();
    let mut edits = removals(top_cell, removed);
    for s in selected {
        edits.push(Edit::AddShape {
            cell: top_cell.to_owned(),
            shape: make(s),
        });
    }
    edits
}

/// The per-shape translation `(dx, dy)` to align each selected shape as `kind` asks.
///
/// Returned in `selected` order. A shape already in place gets `(0, 0)`. Exposed for
/// tests; [`align_edits`] wraps it into edits.
#[must_use]
fn align_offsets(selected: &[Selected], kind: AlignKind) -> Vec<(Dbu, Dbu)> {
    let refs: Vec<&DrawShape> = selected.iter().map(|s| &s.shape).collect();
    let Some(bounds) = combined_bbox(&refs) else {
        return Vec::new();
    };
    // Widen to i64 before subtracting so a large coordinate span cannot overflow.
    let diff = |a: Dbu, b: Dbu| i64::from(a) - i64::from(b);
    selected
        .iter()
        .map(|s| {
            let b = s.shape.bounding_box();
            match kind {
                AlignKind::Left => (dbu(diff(bounds.min.x, b.min.x)), 0),
                AlignKind::Right => (dbu(diff(bounds.max.x, b.max.x)), 0),
                AlignKind::Top => (0, dbu(diff(bounds.max.y, b.max.y))),
                AlignKind::Bottom => (0, dbu(diff(bounds.min.y, b.min.y))),
                AlignKind::CenterX => {
                    let target = midpoint(bounds.min.x, bounds.max.x);
                    let cur = midpoint(b.min.x, b.max.x);
                    (dbu(diff(target, cur)), 0)
                }
                AlignKind::CenterY => {
                    let target = midpoint(bounds.min.y, bounds.max.y);
                    let cur = midpoint(b.min.y, b.max.y);
                    (0, dbu(diff(target, cur)))
                }
            }
        })
        .collect()
}

/// Midpoint of two coordinates as a rounded DBU.
fn midpoint(a: Dbu, b: Dbu) -> Dbu {
    round_dbu(f64::midpoint(f64::from(a), f64::from(b)))
}

/// Narrows a widened DBU delta back to [`Dbu`], saturating (deltas from bounding-box
/// arithmetic are always in range in practice).
fn dbu(v: i64) -> Dbu {
    Dbu::try_from(v).unwrap_or(if v < 0 { Dbu::MIN } else { Dbu::MAX })
}

/// Builds the edits to align the selection, grouped as one undo step. Each shape that
/// moves is replaced by a translated copy; shapes already in place are left alone.
#[must_use]
pub fn align_edits(
    kind: AlignKind,
    scene_shapes: &[DrawShape],
    selection: &[usize],
    top_cell: &str,
    editable_count: usize,
) -> Vec<Edit> {
    let selected = editable_selection(scene_shapes, selection, editable_count);
    let offsets = align_offsets(&selected, kind);
    translate_edits(&selected, &offsets, top_cell)
}

/// The per-shape translation to distribute the selection so the gaps between adjacent
/// shapes are equal along the given axis. Returned in `selected` order.
///
/// With fewer than three shapes there is nothing to distribute (the extremes are
/// fixed and everything between is what moves), so the result is all-zero. Shapes are
/// ordered by their center along the axis; the two extreme shapes stay put and the
/// inner shapes are respaced to equalize edge-to-edge gaps.
#[must_use]
fn distribute_offsets(selected: &[Selected], horizontal: bool) -> Vec<(Dbu, Dbu)> {
    let n = selected.len();
    let mut offsets = vec![(0, 0); n];
    if n < 3 {
        return offsets;
    }
    // Order shapes by center along the axis, remembering their original slot.
    let mut order: Vec<usize> = (0..n).collect();
    let center = |idx: usize| -> f64 {
        let b = selected[idx].shape.bounding_box();
        if horizontal {
            f64::midpoint(f64::from(b.min.x), f64::from(b.max.x))
        } else {
            f64::midpoint(f64::from(b.min.y), f64::from(b.max.y))
        }
    };
    order.sort_by(|&a, &b| {
        center(a)
            .partial_cmp(&center(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let extent = |idx: usize| -> f64 {
        let b = selected[idx].shape.bounding_box();
        if horizontal {
            f64::from(b.max.x) - f64::from(b.min.x)
        } else {
            f64::from(b.max.y) - f64::from(b.min.y)
        }
    };
    let lo = |idx: usize| -> f64 {
        let b = selected[idx].shape.bounding_box();
        if horizontal {
            f64::from(b.min.x)
        } else {
            f64::from(b.min.y)
        }
    };

    let first = order[0];
    let last = order[n - 1];
    // The span the shapes occupy, minus the total shape thickness, is the whitespace
    // to share as equal gaps between the n-1 adjacent pairs.
    let span = (lo(last) + extent(last)) - lo(first);
    let total_extent: f64 = order.iter().map(|&i| extent(i)).sum();
    let gap = (span - total_extent) / (n as f64 - 1.0);

    // Walk the ordered shapes, placing each new low edge after the running cursor.
    let mut cursor = lo(first) + extent(first) + gap;
    for &idx in &order[1..n - 1] {
        let target_lo = cursor;
        let delta = round_dbu(target_lo - lo(idx));
        offsets[idx] = if horizontal { (delta, 0) } else { (0, delta) };
        cursor += extent(idx) + gap;
    }
    offsets
}

/// Builds the edits to distribute the selection with equal spacing, grouped as one
/// undo step.
#[must_use]
pub fn distribute_edits(
    horizontal: bool,
    scene_shapes: &[DrawShape],
    selection: &[usize],
    top_cell: &str,
    editable_count: usize,
) -> Vec<Edit> {
    let selected = editable_selection(scene_shapes, selection, editable_count);
    let offsets = distribute_offsets(&selected, horizontal);
    translate_edits(&selected, &offsets, top_cell)
}

/// Turns a per-shape `(dx, dy)` list (aligned with `selected`) into remove+add edits,
/// skipping shapes whose offset is zero. Removals are highest-index-first so the batch
/// applies as a group.
fn translate_edits(selected: &[Selected], offsets: &[(Dbu, Dbu)], top_cell: &str) -> Vec<Edit> {
    let moved: Vec<(&Selected, (Dbu, Dbu))> = selected
        .iter()
        .zip(offsets.iter().copied())
        .filter(|(_, (dx, dy))| *dx != 0 || *dy != 0)
        .collect();
    if moved.is_empty() {
        return Vec::new();
    }
    let removed: Vec<usize> = moved.iter().map(|(s, _)| s.cell_index).collect();
    let mut edits = removals(top_cell, removed);
    for (s, (dx, dy)) in moved {
        edits.push(Edit::AddShape {
            cell: top_cell.to_owned(),
            shape: translate_shape(&s.shape, dx, dy),
        });
    }
    edits
}

/// Translates a shape by `(dx, dy)` DBU, preserving its kind.
fn translate_shape(shape: &DrawShape, dx: Dbu, dy: Dbu) -> DrawShape {
    map_shape(shape, |p| p.translate(dx, dy))
}

impl crate::app::App {
    /// Draws the boolean-and-transform operations panel.
    ///
    /// Every button reads the current selection and, when the operation produces
    /// edits, applies them as one undo step and rebuilds the scene. Buttons are
    /// disabled when the selection is too small for the operation.
    pub(crate) fn ops_panel(&mut self, ui: &mut egui::Ui) {
        let selected = self.selection.len();
        ui.label(format!("Selected: {selected} shape(s)"));
        self.ops_boolean_section(ui, selected);
        self.ops_offset_section(ui, selected);
        self.ops_rotate_mirror_section(ui, selected);
        self.ops_align_section(ui, selected);
        if !self.ops.status.is_empty() {
            ui.separator();
            ui.label(&self.ops.status);
        }
    }

    /// The boolean row: an operation dropdown and an apply button (needs 2+ shapes).
    fn ops_boolean_section(&mut self, ui: &mut egui::Ui, selected: usize) {
        ui.separator();
        ui.label("Boolean (same-layer)");
        egui::ComboBox::from_id_salt("ops_bool_kind")
            .selected_text(self.ops.bool_kind.label())
            .show_ui(ui, |ui| {
                for kind in [
                    BoolKind::Union,
                    BoolKind::Intersection,
                    BoolKind::Difference,
                    BoolKind::Xor,
                ] {
                    ui.selectable_value(&mut self.ops.bool_kind, kind, kind.label());
                }
            });
        if ui
            .add_enabled(selected >= 2, egui::Button::new("Apply boolean"))
            .clicked()
        {
            let kind = self.ops.bool_kind;
            self.run_ops(kind.label(), |scene, sel, cell, editable| {
                boolean_edits(kind, scene, sel, cell, editable)
            });
        }
    }

    /// The offset row: a signed DBU amount and an apply button (needs 1+ shape).
    fn ops_offset_section(&mut self, ui: &mut egui::Ui, selected: usize) {
        ui.separator();
        ui.label("Offset / grow / shrink");
        ui.horizontal(|ui| {
            ui.label("Amount (DBU)");
            ui.add(egui::DragValue::new(&mut self.ops.offset_dbu).speed(10));
        });
        if ui
            .add_enabled(selected >= 1, egui::Button::new("Apply offset"))
            .clicked()
        {
            let delta = self.ops.offset_dbu;
            self.run_ops("Offset", |scene, sel, cell, editable| {
                offset_edits(delta, scene, sel, cell, editable)
            });
        }
    }

    /// The rotate/mirror rows: a numeric angle and a mirror-axis dropdown, each with
    /// an apply button (needs 1+ shape).
    fn ops_rotate_mirror_section(&mut self, ui: &mut egui::Ui, selected: usize) {
        ui.separator();
        ui.label("Rotate / mirror");
        ui.horizontal(|ui| {
            ui.label("Angle (deg)");
            ui.add(egui::DragValue::new(&mut self.ops.rotate_deg).speed(1.0));
        });
        if ui
            .add_enabled(selected >= 1, egui::Button::new("Rotate"))
            .clicked()
        {
            let deg = self.ops.rotate_deg;
            self.run_ops("Rotate", |scene, sel, cell, editable| {
                rotate_edits(deg, scene, sel, cell, editable)
            });
        }
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("ops_mirror_axis")
                .selected_text(match self.ops.mirror_axis {
                    MirrorAxis::Vertical => "Vertical axis",
                    MirrorAxis::Horizontal => "Horizontal axis",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.ops.mirror_axis,
                        MirrorAxis::Vertical,
                        "Vertical axis",
                    );
                    ui.selectable_value(
                        &mut self.ops.mirror_axis,
                        MirrorAxis::Horizontal,
                        "Horizontal axis",
                    );
                });
            if ui
                .add_enabled(selected >= 1, egui::Button::new("Mirror"))
                .clicked()
            {
                let axis = self.ops.mirror_axis;
                self.run_ops("Mirror", |scene, sel, cell, editable| {
                    mirror_edits(axis, scene, sel, cell, editable)
                });
            }
        });
    }

    /// The align rows (needs 2+ shapes) and the distribute row (needs 3+).
    fn ops_align_section(&mut self, ui: &mut egui::Ui, selected: usize) {
        ui.separator();
        ui.label("Align");
        let can_align = selected >= 2;
        ui.horizontal(|ui| {
            for (text, kind) in [
                ("L", AlignKind::Left),
                ("C", AlignKind::CenterX),
                ("R", AlignKind::Right),
            ] {
                if ui.add_enabled(can_align, egui::Button::new(text)).clicked() {
                    self.run_ops("Align", |scene, sel, cell, editable| {
                        align_edits(kind, scene, sel, cell, editable)
                    });
                }
            }
        });
        ui.horizontal(|ui| {
            for (text, kind) in [
                ("T", AlignKind::Top),
                ("M", AlignKind::CenterY),
                ("B", AlignKind::Bottom),
            ] {
                if ui.add_enabled(can_align, egui::Button::new(text)).clicked() {
                    self.run_ops("Align", |scene, sel, cell, editable| {
                        align_edits(kind, scene, sel, cell, editable)
                    });
                }
            }
        });
        ui.label("Distribute (equal gaps)");
        let can_distribute = selected >= 3;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(can_distribute, egui::Button::new("Horizontal"))
                .clicked()
            {
                self.run_ops("Distribute", |scene, sel, cell, editable| {
                    distribute_edits(true, scene, sel, cell, editable)
                });
            }
            if ui
                .add_enabled(can_distribute, egui::Button::new("Vertical"))
                .clicked()
            {
                self.run_ops("Distribute", |scene, sel, cell, editable| {
                    distribute_edits(false, scene, sel, cell, editable)
                });
            }
        });
    }
}

/// The count of directly-editable shapes for the current top cell: the number of the
/// top cell's own shapes. Scene indices below this map one-to-one to those shapes.
///
/// Free function (rather than an `App` method) so the pure builders and the panel
/// share one definition; `App::editable_shape_count` forwards to it.
#[must_use]
pub fn editable_shape_count(doc: &reticle_model::Document, top_cell: &str) -> usize {
    doc.cell(top_cell).map_or(0, |cell| cell.shapes.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::{LayerId, Path as GPath, Point, Rect};
    use reticle_model::ShapeKind;

    const M1: LayerId = LayerId {
        layer: 4,
        datatype: 0,
    };
    const M2: LayerId = LayerId {
        layer: 5,
        datatype: 0,
    };

    fn rect(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    }

    /// The total area of every polygon/rect an edit list adds, for area-based checks.
    fn added_area(edits: &[Edit]) -> f64 {
        edits
            .iter()
            .filter_map(|e| match e {
                Edit::AddShape { shape, .. } => Some(shape),
                _ => None,
            })
            .map(|s| match &s.kind {
                ShapeKind::Rect(r) => r.area() as f64,
                ShapeKind::Polygon(p) => p.area(),
                ShapeKind::Path(_) => 0.0,
            })
            .sum()
    }

    fn add_count(edits: &[Edit]) -> usize {
        edits
            .iter()
            .filter(|e| matches!(e, Edit::AddShape { .. }))
            .count()
    }

    fn remove_indices(edits: &[Edit]) -> Vec<usize> {
        edits
            .iter()
            .filter_map(|e| match e {
                Edit::RemoveShape { index, .. } => Some(*index),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn union_of_two_overlapping_rects_has_merged_area() {
        // Two 100x100 rects overlapping in a 50x50 corner: union area = 2*10000 - 2500.
        let shapes = vec![rect(M1, 0, 0, 100, 100), rect(M1, 50, 50, 150, 150)];
        let edits = boolean_edits(BoolKind::Union, &shapes, &[0, 1], "TOP", shapes.len());
        assert_eq!(add_count(&edits), 1, "union yields one polygon");
        assert!((added_area(&edits) - 17_500.0).abs() < 1.0);
        // Both inputs removed, highest index first.
        assert_eq!(remove_indices(&edits), vec![1, 0]);
    }

    #[test]
    fn intersection_of_two_overlapping_rects_is_the_overlap() {
        let shapes = vec![rect(M1, 0, 0, 100, 100), rect(M1, 50, 50, 150, 150)];
        let edits = boolean_edits(
            BoolKind::Intersection,
            &shapes,
            &[0, 1],
            "TOP",
            shapes.len(),
        );
        // The overlap is the 50x50 corner.
        assert!((added_area(&edits) - 2_500.0).abs() < 1.0);
    }

    #[test]
    fn difference_subtracts_second_from_first() {
        let shapes = vec![rect(M1, 0, 0, 100, 100), rect(M1, 50, 50, 150, 150)];
        let edits = boolean_edits(BoolKind::Difference, &shapes, &[0, 1], "TOP", shapes.len());
        // First minus the overlap = 10000 - 2500.
        assert!((added_area(&edits) - 7_500.0).abs() < 1.0);
    }

    #[test]
    fn xor_is_symmetric_difference() {
        let shapes = vec![rect(M1, 0, 0, 100, 100), rect(M1, 50, 50, 150, 150)];
        let edits = boolean_edits(BoolKind::Xor, &shapes, &[0, 1], "TOP", shapes.len());
        // Both areas minus twice the overlap: 20000 - 5000.
        assert!((added_area(&edits) - 15_000.0).abs() < 1.0);
    }

    #[test]
    fn boolean_only_combines_same_layer() {
        // One rect on M1, one on M2: no pair shares a layer, so nothing combines.
        let shapes = vec![rect(M1, 0, 0, 100, 100), rect(M2, 50, 50, 150, 150)];
        let edits = boolean_edits(BoolKind::Union, &shapes, &[0, 1], "TOP", shapes.len());
        assert!(edits.is_empty());
    }

    #[test]
    fn boolean_groups_per_layer_and_keeps_layer() {
        // Two overlapping on M1, two overlapping on M2: two independent unions.
        let shapes = vec![
            rect(M1, 0, 0, 100, 100),
            rect(M1, 50, 50, 150, 150),
            rect(M2, 0, 0, 100, 100),
            rect(M2, 50, 50, 150, 150),
        ];
        let edits = boolean_edits(BoolKind::Union, &shapes, &[0, 1, 2, 3], "TOP", shapes.len());
        assert_eq!(add_count(&edits), 2, "one union per layer");
        // Every added shape stays on its input layer.
        let layers: Vec<LayerId> = edits
            .iter()
            .filter_map(|e| match e {
                Edit::AddShape { shape, .. } => Some(shape.layer),
                _ => None,
            })
            .collect();
        assert!(layers.contains(&M1) && layers.contains(&M2));
        // All four inputs removed, highest first.
        assert_eq!(remove_indices(&edits), vec![3, 2, 1, 0]);
    }

    #[test]
    fn boolean_skips_paths() {
        let path = DrawShape::new(
            M1,
            ShapeKind::Path(GPath::new(
                vec![Point::new(0, 0), Point::new(100, 0)],
                20,
                reticle_geometry::Endcap::Flat,
            )),
        );
        let shapes = vec![rect(M1, 0, 0, 100, 100), path];
        // Only one fillable shape on the layer, so no boolean.
        let edits = boolean_edits(BoolKind::Union, &shapes, &[0, 1], "TOP", shapes.len());
        assert!(edits.is_empty());
    }

    #[test]
    fn boolean_ignores_non_editable_indices() {
        // Two overlapping rects but editable_count is 1: index 1 is instance geometry.
        let shapes = vec![rect(M1, 0, 0, 100, 100), rect(M1, 50, 50, 150, 150)];
        let edits = boolean_edits(BoolKind::Union, &shapes, &[0, 1], "TOP", 1);
        assert!(edits.is_empty());
    }

    #[test]
    fn grow_offset_increases_area() {
        let shapes = vec![rect(M1, 0, 0, 100, 100)];
        let edits = offset_edits(10, &shapes, &[0], "TOP", shapes.len());
        // Grow by 10 on every side -> 120x120 = 14400 (miter corners, so ~exact here).
        assert!(added_area(&edits) > 10_000.0);
        assert!((added_area(&edits) - 14_400.0).abs() < 200.0);
        assert_eq!(remove_indices(&edits), vec![0]);
    }

    #[test]
    fn shrink_offset_decreases_area() {
        let shapes = vec![rect(M1, 0, 0, 100, 100)];
        let edits = offset_edits(-10, &shapes, &[0], "TOP", shapes.len());
        // Shrink by 10 -> 80x80 = 6400.
        assert!((added_area(&edits) - 6_400.0).abs() < 200.0);
    }

    #[test]
    fn zero_offset_is_noop() {
        let shapes = vec![rect(M1, 0, 0, 100, 100)];
        assert!(offset_edits(0, &shapes, &[0], "TOP", shapes.len()).is_empty());
    }

    #[test]
    fn align_left_moves_to_leftmost_edge() {
        let shapes = vec![
            rect(M1, 0, 0, 100, 100),   // left edge x=0 (leftmost)
            rect(M1, 200, 0, 260, 100), // left edge x=200
        ];
        let selected = editable_selection(&shapes, &[0, 1], shapes.len());
        let offsets = align_offsets(&selected, AlignKind::Left);
        assert_eq!(offsets[0], (0, 0)); // already leftmost
        assert_eq!(offsets[1], (-200, 0)); // moves left to x=0
    }

    #[test]
    fn align_right_moves_to_rightmost_edge() {
        let shapes = vec![
            rect(M1, 0, 0, 100, 100),   // right edge x=100
            rect(M1, 200, 0, 260, 100), // right edge x=260 (rightmost)
        ];
        let selected = editable_selection(&shapes, &[0, 1], shapes.len());
        let offsets = align_offsets(&selected, AlignKind::Right);
        assert_eq!(offsets[0], (160, 0)); // 260 - 100
        assert_eq!(offsets[1], (0, 0));
    }

    #[test]
    fn align_center_x_centers_on_selection() {
        let shapes = vec![
            rect(M1, 0, 0, 100, 100),   // center x=50
            rect(M1, 200, 0, 260, 100), // center x=230
        ];
        // Selection bounds x: [0, 260], center = 130.
        let selected = editable_selection(&shapes, &[0, 1], shapes.len());
        let offsets = align_offsets(&selected, AlignKind::CenterX);
        assert_eq!(offsets[0], (80, 0)); // 130 - 50
        assert_eq!(offsets[1], (-100, 0)); // 130 - 230
    }

    #[test]
    fn align_top_moves_to_topmost_edge() {
        let shapes = vec![
            rect(M1, 0, 0, 100, 100),   // top y=100
            rect(M1, 0, 200, 100, 260), // top y=260 (topmost)
        ];
        let selected = editable_selection(&shapes, &[0, 1], shapes.len());
        let offsets = align_offsets(&selected, AlignKind::Top);
        assert_eq!(offsets[0], (0, 160)); // 260 - 100
        assert_eq!(offsets[1], (0, 0));
    }

    #[test]
    fn distribute_equalizes_horizontal_gaps() {
        // Three 100-wide shapes; extremes at x=0 and x=500. Total width 300, span from
        // 0 to 600 = 600, whitespace 300 over 2 gaps = 150 each. So shapes at low
        // edges 0, 250, 500; the middle one must move from 200 to 250 (+50).
        let shapes = vec![
            rect(M1, 0, 0, 100, 100),
            rect(M1, 200, 0, 300, 100),
            rect(M1, 500, 0, 600, 100),
        ];
        let selected = editable_selection(&shapes, &[0, 1, 2], shapes.len());
        let offsets = distribute_offsets(&selected, true);
        assert_eq!(offsets[0], (0, 0)); // extreme, fixed
        assert_eq!(offsets[2], (0, 0)); // extreme, fixed
        assert_eq!(offsets[1], (50, 0)); // respaced to equal gaps
    }

    #[test]
    fn distribute_needs_three_shapes() {
        let shapes = vec![rect(M1, 0, 0, 100, 100), rect(M1, 200, 0, 300, 100)];
        let selected = editable_selection(&shapes, &[0, 1], shapes.len());
        let offsets = distribute_offsets(&selected, true);
        assert!(offsets.iter().all(|&o| o == (0, 0)));
    }

    #[test]
    fn rotate_90_about_center_maps_corners_exactly() {
        // A single 0..100 square, center (50,50). Rotating 90 CCW keeps the square
        // (rounding-free for a right angle), so the bounding box is unchanged.
        let shapes = vec![rect(M1, 0, 0, 100, 100)];
        let edits = rotate_edits(90.0, &shapes, &[0], "TOP", shapes.len());
        let added: Vec<&DrawShape> = edits
            .iter()
            .filter_map(|e| match e {
                Edit::AddShape { shape, .. } => Some(shape),
                _ => None,
            })
            .collect();
        assert_eq!(added.len(), 1);
        let bbox = added[0].bounding_box();
        assert_eq!(bbox.min, Point::new(0, 0));
        assert_eq!(bbox.max, Point::new(100, 100));
    }

    #[test]
    fn mirror_vertical_reflects_x_about_center() {
        // Square at x in [0,100] plus one at [200,300]; selection center x = 150.
        // Vertical mirror sends x -> 300 - x, so the first square lands at [200,300]
        // and vice versa; the combined bounding box is unchanged.
        let shapes = vec![rect(M1, 0, 0, 100, 100), rect(M1, 200, 0, 300, 100)];
        let edits = mirror_edits(MirrorAxis::Vertical, &shapes, &[0, 1], "TOP", shapes.len());
        let added_bbox = edits
            .iter()
            .filter_map(|e| match e {
                Edit::AddShape { shape, .. } => Some(shape.bounding_box()),
                _ => None,
            })
            .reduce(|a, b| a.union(&b))
            .unwrap();
        assert_eq!(added_bbox.min, Point::new(0, 0));
        assert_eq!(added_bbox.max, Point::new(300, 100));
    }

    #[test]
    fn mirror_is_its_own_inverse() {
        let shapes = vec![rect(M1, 10, 20, 40, 60)];
        let selected = editable_selection(&shapes, &[0], shapes.len());
        let center = selection_center(&[&shapes[0]]).unwrap();
        let once = map_shape(&shapes[0], |p| Point::new(2 * center.x - p.x, p.y));
        let twice = map_shape(&once, |p| Point::new(2 * center.x - p.x, p.y));
        // Reflecting twice about the same axis returns the original geometry.
        assert_eq!(twice.bounding_box(), shapes[0].bounding_box());
        assert_eq!(selected.len(), 1);
    }

    #[test]
    fn empty_selection_yields_no_edits() {
        let shapes = vec![rect(M1, 0, 0, 100, 100)];
        assert!(boolean_edits(BoolKind::Union, &shapes, &[], "TOP", shapes.len()).is_empty());
        assert!(offset_edits(10, &shapes, &[], "TOP", shapes.len()).is_empty());
        assert!(rotate_edits(45.0, &shapes, &[], "TOP", shapes.len()).is_empty());
        assert!(align_edits(AlignKind::Left, &shapes, &[], "TOP", shapes.len()).is_empty());
    }
}
