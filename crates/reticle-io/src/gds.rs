//! GDSII import and export via the [`gds21`] crate.
//!
//! This module maps between [`gds21`]'s faithful-to-GDSII data model and
//! Reticle's [`Document`]. The mapping is:
//!
//! | GDSII (`gds21`)      | Reticle model                                    |
//! |----------------------|--------------------------------------------------|
//! | [`GdsLibrary`]       | [`Document`] (one library per document)          |
//! | [`GdsStruct`]        | [`Cell`] (keyed by struct name)                  |
//! | [`GdsBoundary`]      | [`DrawShape`] — [`ShapeKind::Rect`] when the ring |
//! |                      | is an axis-aligned box, else [`ShapeKind::Polygon`] |
//! | [`GdsPath`]          | [`DrawShape`] — [`ShapeKind::Path`]              |
//! | [`GdsStructRef`]     | [`Instance`]                                     |
//! | [`GdsArrayRef`]      | [`ArrayInstance`]                               |
//! | `layer` / `datatype` | [`LayerId`]                                     |
//! | [`GdsUnits`]         | [`Technology::dbu_per_micron`]                  |
//! | [`GdsStrans`]        | [`Transform`] (`reflected`/`angle`/`mag`)       |
//!
//! # Coordinates and units
//!
//! GDSII stores geometry in integer *database units* (DBU), exactly like Reticle.
//! The `UNITS` record records the physical size of a DBU. A DBU is
//! `units.1` metres and `units.0` user units (microns), so
//! `dbu_per_micron = round(1e-6 / units.1)`. Export writes the reciprocal.
//!
//! # Round-trip fidelity
//!
//! Because layer/datatype, integer coordinates, cell names, instances and arrays
//! all have direct GDSII equivalents, an export/import cycle preserves geometry
//! exactly. Rectangles are recovered from axis-aligned boundaries so a `Rect`
//! survives as a `Rect`.

use crate::IoError;
use gds21::{
    GdsArrayRef, GdsBoundary, GdsElement, GdsLibrary, GdsPath, GdsPoint, GdsStrans, GdsStruct,
    GdsStructRef, GdsUnits,
};
use reticle_geometry::{
    Dbu, LayerId, Magnification, Orientation, Path, Point, Polygon, Rect, Transform,
};
use reticle_model::{
    ArrayInstance, Cell, Document, DrawShape, Exporter, Importer, Instance, Result, ShapeKind,
    Technology,
};

/// GDSII import/export (Wave 1: `gds21`).
///
/// Implements [`Importer`] (bytes to [`Document`]) and [`Exporter`]
/// ([`Document`] to bytes) using the [`gds21`] library. See the [module
/// docs](self) for the full mapping.
#[derive(Debug, Default, Clone, Copy)]
pub struct Gds;

/// Default GDSII layout version written on export (matches `gds21`'s own default).
const GDS_VERSION: i16 = 3;

/// The user unit assumed for GDSII, in metres. One micron.
const USER_UNIT_METERS: f64 = 1e-6;

/// Fallback database resolution when a document carries no technology, or a
/// technology with a non-positive `dbu_per_micron`. 1000 DBU/µm (1 nm grid) is
/// the ubiquitous GDSII default.
const DEFAULT_DBU_PER_MICRON: i64 = 1000;

impl Importer for Gds {
    fn import(&self, bytes: &[u8]) -> Result<Document> {
        // `gds21` panics on a few malformed inputs (for example a zero-length
        // string record indexes `data[len - 1]`). Contain any such panic so
        // `import` upholds its contract of returning `Err` rather than
        // unwinding across the FFI-like boundary. `catch_unwind` is safe Rust;
        // no `unsafe` is required.
        let owned = bytes.to_vec();
        let parsed = std::panic::catch_unwind(move || GdsLibrary::from_bytes(owned));
        let lib = match parsed {
            Ok(Ok(lib)) => lib,
            Ok(Err(e)) => return Err(IoError::gds(&e).into()),
            Err(_) => {
                return Err(IoError::Malformed("gds21 panicked while parsing GDSII bytes").into());
            }
        };
        Ok(library_to_document(&lib))
    }
}

impl Exporter for Gds {
    fn export(&self, doc: &Document) -> Result<Vec<u8>> {
        let lib = document_to_library(doc);
        let mut bytes = Vec::new();
        lib.write(&mut bytes).map_err(|e| IoError::gds(&e))?;
        Ok(bytes)
    }
}

/// Converts a parsed [`GdsLibrary`] into a Reticle [`Document`].
fn library_to_document(lib: &GdsLibrary) -> Document {
    let mut doc = Document::new();

    // Recover the database resolution from the UNITS record.
    let dbu_per_micron = dbu_per_micron_from_units(&lib.units);
    let mut tech = Technology {
        name: lib.name.clone(),
        dbu_per_micron,
        ..Technology::default()
    };

    // Track which cells are referenced by others; the remainder are tops.
    let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();

    for strukt in &lib.structs {
        let cell = struct_to_cell(strukt, &mut referenced);
        doc.insert_cell(cell);
    }

    // Collect the layer/datatype pairs actually used, so downstream tooling sees
    // a layer table even when importing from a bare GDS with no technology file.
    let mut layers: Vec<LayerId> = Vec::new();
    for cell in doc.cells() {
        for shape in &cell.shapes {
            if !layers.contains(&shape.layer) {
                layers.push(shape.layer);
            }
        }
    }
    layers.sort_unstable();
    tech.layers = layers
        .into_iter()
        .map(|id| reticle_model::LayerInfo {
            id,
            name: format!("L{}D{}", id.layer, id.datatype),
            color_rgba: 0xFFFF_FFFF,
            visible: true,
        })
        .collect();
    doc.set_technology(tech);

    // Any struct not referenced by a SREF/AREF is a top cell. Preserve library
    // order for determinism.
    let tops: Vec<String> = lib
        .structs
        .iter()
        .filter(|s| !referenced.contains(&s.name))
        .map(|s| s.name.clone())
        .collect();
    doc.set_top_cells(tops);

    doc
}

/// Converts one [`GdsStruct`] to a [`Cell`], recording referenced cell names.
fn struct_to_cell(strukt: &GdsStruct, referenced: &mut std::collections::HashSet<String>) -> Cell {
    let mut cell = Cell::new(strukt.name.clone());
    for elem in &strukt.elems {
        match elem {
            GdsElement::GdsBoundary(b) => cell.shapes.push(boundary_to_shape(b)),
            GdsElement::GdsPath(p) => cell.shapes.push(path_to_shape(p)),
            GdsElement::GdsStructRef(sref) => {
                referenced.insert(sref.name.clone());
                cell.instances.push(struct_ref_to_instance(sref));
            }
            GdsElement::GdsArrayRef(aref) => {
                referenced.insert(aref.name.clone());
                cell.arrays.push(array_ref_to_array(aref));
            }
            // Text, Node, and Box carry no drawn fill geometry we model in Wave 1.
            // They are skipped rather than erroring so real-world GDS still imports.
            GdsElement::GdsTextElem(_) | GdsElement::GdsNode(_) | GdsElement::GdsBox(_) => {}
        }
    }
    cell
}

/// Maps a GDSII boundary to a rectangle when its ring is an axis-aligned box,
/// otherwise to a polygon.
fn boundary_to_shape(b: &GdsBoundary) -> DrawShape {
    let layer = layer_id(b.layer, b.datatype);
    let mut pts: Vec<Point> = b.xy.iter().map(gds_point_to_point).collect();
    // GDSII repeats the first vertex as the last to close the ring. Drop the
    // duplicate closing vertex so our (implicitly closed) polygon is canonical.
    if pts.len() >= 2 && pts.first() == pts.last() {
        pts.pop();
    }
    if let Some(rect) = axis_aligned_rect(&pts) {
        DrawShape::new(layer, ShapeKind::Rect(rect))
    } else {
        DrawShape::new(layer, ShapeKind::Polygon(Polygon::new(pts)))
    }
}

/// Recognises a 4-vertex axis-aligned rectangle (in any winding/rotation of the
/// vertex list) and returns it as a [`Rect`]; otherwise `None`.
fn axis_aligned_rect(pts: &[Point]) -> Option<Rect> {
    if pts.len() != 4 {
        return None;
    }
    let rect = Rect::from_points(pts.iter().copied())?;
    if rect.is_empty() {
        return None;
    }
    // Every vertex must sit on a corner of the bounding box, and all four
    // corners must be present, for the ring to be exactly that box.
    let corners = [
        rect.min,
        Point::new(rect.max.x, rect.min.y),
        rect.max,
        Point::new(rect.min.x, rect.max.y),
    ];
    if pts.iter().all(|p| corners.contains(p)) && corners.iter().all(|c| pts.contains(c)) {
        Some(rect)
    } else {
        None
    }
}

/// Maps a GDSII path to a [`ShapeKind::Path`].
fn path_to_shape(p: &GdsPath) -> DrawShape {
    let layer = layer_id(p.layer, p.datatype);
    let points: Vec<Point> = p.xy.iter().map(gds_point_to_point).collect();
    let width = p.width.unwrap_or(0);
    // Reticle's default endcap (`Flat`) corresponds to GDSII path-type 0. Path
    // types 1/2 (round/square) are not distinguished in Wave 1; width and the
    // centre-line are preserved, which is what round-trips through our writer.
    let path = Path::new(points, width, reticle_geometry::Endcap::Flat);
    DrawShape::new(layer, ShapeKind::Path(path))
}

/// Maps a GDSII struct reference to an [`Instance`].
fn struct_ref_to_instance(sref: &GdsStructRef) -> Instance {
    let transform = strans_to_transform(sref.strans.as_ref(), gds_point_to_point(&sref.xy));
    Instance {
        cell: sref.name.clone(),
        transform,
    }
}

/// Maps a GDSII array reference to an [`ArrayInstance`].
///
/// The three `xy` points are, per the GDSII spec: the array origin, a point
/// `columns` column-pitches away, and a point `rows` row-pitches away. We derive
/// the axis-aligned pitches from those deltas.
fn array_ref_to_array(aref: &GdsArrayRef) -> ArrayInstance {
    let origin = gds_point_to_point(&aref.xy[0]);
    let col_end = gds_point_to_point(&aref.xy[1]);
    let row_end = gds_point_to_point(&aref.xy[2]);

    let cols = u32::try_from(aref.cols.max(0)).unwrap_or(0);
    let rows = u32::try_from(aref.rows.max(0)).unwrap_or(0);

    // Pitch = total span / repetition count, guarding against a zero count.
    let column_pitch = if cols > 0 {
        (col_end.x - origin.x) / Dbu::try_from(cols).unwrap_or(1).max(1)
    } else {
        0
    };
    let row_pitch = if rows > 0 {
        (row_end.y - origin.y) / Dbu::try_from(rows).unwrap_or(1).max(1)
    } else {
        0
    };

    let transform = strans_to_transform(aref.strans.as_ref(), origin);
    ArrayInstance {
        cell: aref.name.clone(),
        transform,
        columns: cols,
        rows,
        column_pitch,
        row_pitch,
    }
}

/// Builds a Reticle [`Transform`] from an optional GDSII [`GdsStrans`] plus the
/// instance's translation.
fn strans_to_transform(strans: Option<&GdsStrans>, translation: Point) -> Transform {
    let mut transform = Transform {
        translation,
        ..Transform::IDENTITY
    };
    if let Some(s) = strans {
        transform.orientation = orientation_from_strans(s);
        if let Some(mag) = s.mag {
            transform.magnification = magnification_from_f64(mag);
        }
    }
    transform
}

/// Maps a GDSII reflect flag + rotation angle to the nearest [`Orientation`].
///
/// GDSII applies reflection about the x-axis first, then a counter-clockwise
/// rotation — exactly Reticle's [`Orientation`] convention. Angles are snapped to
/// the nearest 90°; non-Manhattan placement angles are not represented by the
/// eight-element orientation group and collapse to the closest quadrant.
fn orientation_from_strans(s: &GdsStrans) -> Orientation {
    let angle = s.angle.unwrap_or(0.0);
    // Normalise to [0, 360) and snap to the nearest multiple of 90°.
    let normalized = angle.rem_euclid(360.0);
    let quadrant = (normalized / 90.0).round() as i64 % 4;
    match (s.reflected, quadrant) {
        (false, 0) => Orientation::R0,
        (false, 1) => Orientation::R90,
        (false, 2) => Orientation::R180,
        (false, _) => Orientation::R270,
        (true, 0) => Orientation::MirrorX,
        (true, 1) => Orientation::MirrorX90,
        (true, 2) => Orientation::MirrorX180,
        (true, _) => Orientation::MirrorX270,
    }
}

/// Approximates a floating-point magnification as an exact rational.
///
/// Unit magnification is by far the common case and is represented exactly.
/// Other values are scaled by `1_000_000` and stored as `n / 1_000_000`, which is
/// well within DBU precision for the placement magnitudes GDSII permits.
fn magnification_from_f64(mag: f64) -> Magnification {
    const SCALE: f64 = 1_000_000.0;
    if (mag - 1.0).abs() < f64::EPSILON || mag <= 0.0 {
        return Magnification::UNITY;
    }
    let num = (mag * SCALE).round();
    if num <= 0.0 || num > f64::from(u32::MAX) {
        return Magnification::UNITY;
    }
    Magnification::new(num as u32, SCALE as u32).unwrap_or(Magnification::UNITY)
}

// ---------------------------------------------------------------------------
// Export direction: Document -> GdsLibrary
// ---------------------------------------------------------------------------

/// Converts a Reticle [`Document`] into a [`GdsLibrary`] ready to serialize.
fn document_to_library(doc: &Document) -> GdsLibrary {
    let dbu_per_micron = {
        let d = doc.technology().dbu_per_micron;
        if d > 0 { d } else { DEFAULT_DBU_PER_MICRON }
    };
    let name = if doc.technology().name.is_empty() {
        "reticle".to_string()
    } else {
        doc.technology().name.clone()
    };

    let mut lib = GdsLibrary::new(name);
    lib.version = GDS_VERSION;
    lib.units = units_from_dbu_per_micron(dbu_per_micron);

    // Emit cells in a deterministic order: top cells first (in their declared
    // order), then the remaining cells sorted by name. This keeps exports stable.
    let mut ordered: Vec<&Cell> = Vec::with_capacity(doc.cell_count());
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for name in doc.top_cells() {
        if let Some(cell) = doc.cell(name)
            && seen.insert(cell.name.as_str())
        {
            ordered.push(cell);
        }
    }
    let mut rest: Vec<&Cell> = doc
        .cells()
        .filter(|c| !seen.contains(c.name.as_str()))
        .collect();
    rest.sort_by(|a, b| a.name.cmp(&b.name));
    ordered.extend(rest);

    for cell in ordered {
        lib.structs.push(cell_to_struct(cell));
    }
    lib
}

/// Converts a [`Cell`] into a [`GdsStruct`].
fn cell_to_struct(cell: &Cell) -> GdsStruct {
    let mut strukt = GdsStruct::new(cell.name.clone());
    for shape in &cell.shapes {
        strukt.elems.push(shape_to_element(shape));
    }
    for inst in &cell.instances {
        strukt.elems.push(instance_to_element(inst));
    }
    for arr in &cell.arrays {
        strukt.elems.push(array_to_element(arr));
    }
    strukt
}

/// Converts a [`DrawShape`] into the matching [`GdsElement`].
fn shape_to_element(shape: &DrawShape) -> GdsElement {
    let (layer, datatype) = layer_parts(shape.layer);
    match &shape.kind {
        ShapeKind::Rect(r) => GdsElement::GdsBoundary(GdsBoundary {
            layer,
            datatype,
            xy: rect_to_closed_xy(r),
            ..GdsBoundary::default()
        }),
        ShapeKind::Polygon(p) => GdsElement::GdsBoundary(GdsBoundary {
            layer,
            datatype,
            xy: polygon_to_closed_xy(p),
            ..GdsBoundary::default()
        }),
        ShapeKind::Path(p) => GdsElement::GdsPath(GdsPath {
            layer,
            datatype,
            xy: points_to_xy(p.points()),
            width: if p.width() != 0 {
                Some(p.width())
            } else {
                None
            },
            ..GdsPath::default()
        }),
    }
}

/// Converts an [`Instance`] into a GDSII struct reference element.
fn instance_to_element(inst: &Instance) -> GdsElement {
    GdsElement::GdsStructRef(GdsStructRef {
        name: inst.cell.clone(),
        xy: point_to_gds_point(inst.transform.translation),
        strans: transform_to_strans(&inst.transform),
        ..GdsStructRef::default()
    })
}

/// Converts an [`ArrayInstance`] into a GDSII array reference element.
fn array_to_element(arr: &ArrayInstance) -> GdsElement {
    let origin = arr.transform.translation;
    // Reconstruct the two span points from origin + count * pitch (saturating to
    // stay within `Dbu`), matching the GDSII AREF three-point convention.
    let col_span = i64::from(arr.columns) * i64::from(arr.column_pitch);
    let row_span = i64::from(arr.rows) * i64::from(arr.row_pitch);
    let col_end = Point::new(saturating_add(origin.x, col_span), origin.y);
    let row_end = Point::new(origin.x, saturating_add(origin.y, row_span));
    GdsElement::GdsArrayRef(GdsArrayRef {
        name: arr.cell.clone(),
        xy: [
            point_to_gds_point(origin),
            point_to_gds_point(col_end),
            point_to_gds_point(row_end),
        ],
        cols: clamp_i16(arr.columns),
        rows: clamp_i16(arr.rows),
        strans: transform_to_strans(&arr.transform),
        ..GdsArrayRef::default()
    })
}

/// Builds a [`GdsStrans`] from a [`Transform`], returning `None` when the
/// transform is a plain (unrotated, unmirrored, unit-magnification) placement so
/// the emitted GDS stays minimal.
fn transform_to_strans(transform: &Transform) -> Option<GdsStrans> {
    let orientation = transform.orientation;
    let mag = transform.magnification;
    if orientation == Orientation::R0 && mag.is_unity() {
        return None;
    }
    let (reflected, angle) = strans_parts(orientation);
    Some(GdsStrans {
        reflected,
        abs_mag: false,
        abs_angle: false,
        mag: if mag.is_unity() {
            None
        } else {
            Some(magnification_to_f64(mag))
        },
        angle: if angle == 0.0 { None } else { Some(angle) },
    })
}

/// Decomposes an [`Orientation`] into a GDSII (reflected, angle-in-degrees) pair.
fn strans_parts(orientation: Orientation) -> (bool, f64) {
    match orientation {
        Orientation::R0 => (false, 0.0),
        Orientation::R90 => (false, 90.0),
        Orientation::R180 => (false, 180.0),
        Orientation::R270 => (false, 270.0),
        Orientation::MirrorX => (true, 0.0),
        Orientation::MirrorX90 => (true, 90.0),
        Orientation::MirrorX180 => (true, 180.0),
        Orientation::MirrorX270 => (true, 270.0),
    }
}

/// Recovers an `f64` magnification from a rational [`Magnification`].
fn magnification_to_f64(mag: Magnification) -> f64 {
    // Represent via scaling a unit distance; avoids exposing private fields.
    // `scale` rounds, so use a large probe value for precision.
    const PROBE: i32 = 1_000_000;
    f64::from(mag.scale(PROBE)) / f64::from(PROBE)
}

// ---------------------------------------------------------------------------
// Small conversion helpers
// ---------------------------------------------------------------------------

/// Builds a [`LayerId`] from GDSII's signed 16-bit layer/datatype, reinterpreting
/// the bits as unsigned (GDSII layer numbers are conventionally 0..=255 but the
/// field is `i16`; a bit-preserving cast round-trips any value).
fn layer_id(layer: i16, datatype: i16) -> LayerId {
    LayerId::new(layer as u16, datatype as u16)
}

/// Splits a [`LayerId`] back into GDSII's signed 16-bit fields (bit-preserving).
fn layer_parts(id: LayerId) -> (i16, i16) {
    (id.layer as i16, id.datatype as i16)
}

/// Converts a [`GdsPoint`] to a geometry [`Point`].
fn gds_point_to_point(p: &GdsPoint) -> Point {
    Point::new(p.x, p.y)
}

/// Converts a geometry [`Point`] to a [`GdsPoint`].
fn point_to_gds_point(p: Point) -> GdsPoint {
    GdsPoint::new(p.x, p.y)
}

/// Flattens a slice of geometry points to GDSII `xy` form.
fn points_to_xy(points: &[Point]) -> Vec<GdsPoint> {
    points.iter().copied().map(point_to_gds_point).collect()
}

/// Emits a rectangle as a closed 5-point GDSII boundary ring (CCW, first vertex
/// repeated last).
fn rect_to_closed_xy(r: &Rect) -> Vec<GdsPoint> {
    vec![
        GdsPoint::new(r.min.x, r.min.y),
        GdsPoint::new(r.max.x, r.min.y),
        GdsPoint::new(r.max.x, r.max.y),
        GdsPoint::new(r.min.x, r.max.y),
        GdsPoint::new(r.min.x, r.min.y),
    ]
}

/// Emits a polygon as a closed GDSII boundary ring (first vertex repeated last).
fn polygon_to_closed_xy(p: &Polygon) -> Vec<GdsPoint> {
    let mut xy: Vec<GdsPoint> = p
        .vertices()
        .iter()
        .copied()
        .map(point_to_gds_point)
        .collect();
    if let Some(first) = xy.first().cloned()
        && xy.last() != Some(&first)
    {
        xy.push(first);
    }
    xy
}

/// Recovers `dbu_per_micron` from a GDSII [`GdsUnits`] record.
fn dbu_per_micron_from_units(units: &GdsUnits) -> i64 {
    let db_unit_meters = units.db_unit();
    if db_unit_meters > 0.0 {
        let per_micron = (USER_UNIT_METERS / db_unit_meters).round();
        if per_micron >= 1.0 && per_micron <= f64::from(u32::MAX) {
            return per_micron as i64;
        }
    }
    DEFAULT_DBU_PER_MICRON
}

/// Builds a GDSII [`GdsUnits`] record from `dbu_per_micron`.
///
/// `units.0` is the size of a DBU in user units (microns) = `1 / dbu_per_micron`;
/// `units.1` is the size of a DBU in metres = `1e-6 / dbu_per_micron`.
fn units_from_dbu_per_micron(dbu_per_micron: i64) -> GdsUnits {
    let d = dbu_per_micron as f64;
    GdsUnits::new(1.0 / d, USER_UNIT_METERS / d)
}

/// Saturating add of an `i64` offset onto a `Dbu`, clamped to the `Dbu` range.
fn saturating_add(base: Dbu, offset: i64) -> Dbu {
    let sum = i64::from(base).saturating_add(offset);
    sum.clamp(i64::from(Dbu::MIN), i64::from(Dbu::MAX)) as Dbu
}

/// Clamps a `u32` repetition count into GDSII's `i16` column/row field.
fn clamp_i16(v: u32) -> i16 {
    i16::try_from(v).unwrap_or(i16::MAX)
}
