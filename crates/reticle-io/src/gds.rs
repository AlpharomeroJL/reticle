//! GDSII import and export via the [`gds21`] crate.
//!
//! This module maps between [`gds21`]'s faithful-to-GDSII data model and
//! Reticle's [`Document`]. The mapping is:
//!
//! | GDSII (`gds21`)      | Reticle model                                    |
//! |----------------------|--------------------------------------------------|
//! | [`GdsLibrary`]       | [`Document`] (one library per document)          |
//! | [`GdsStruct`]        | [`Cell`] (keyed by struct name)                  |
//! | [`GdsBoundary`]      | [`DrawShape`], [`ShapeKind::Rect`] when the ring |
//! |                      | is an axis-aligned box, else [`ShapeKind::Polygon`] |
//! | [`GdsPath`]          | [`DrawShape`], [`ShapeKind::Path`]              |
//! | [`GdsTextElem`]      | [`Label`] (`layer`/`texttype` as [`LayerId`])   |
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
//! # Labels
//!
//! GDSII TEXT elements carry net and port names. Import surfaces each one as a
//! [`Label`] on [`Cell::labels`]: the TEXT `layer`/`texttype` pair becomes the
//! label's [`LayerId`] and the insertion point its position. `gds21` does not
//! expose the PRESENTATION justification bits (its `GdsPresentation` fields are
//! private), so every imported label is anchored [`Anchor::Center`], the model
//! default; export likewise writes no PRESENTATION record, letting readers apply
//! their own default. A non-`Center` anchor therefore does not survive a GDSII
//! round-trip; text, position, and layer always do.
//!
//! # Round-trip fidelity
//!
//! Because layer/datatype, integer coordinates, cell names, instances, arrays,
//! and text labels all have direct GDSII equivalents, an export/import cycle
//! preserves geometry exactly. Rectangles are recovered from axis-aligned
//! boundaries so a `Rect` survives as a `Rect`.

use crate::IoError;
use crate::error::{ImportWarning, WarningKind};
use chrono::NaiveDate;
use gds21::{
    GdsArrayRef, GdsBoundary, GdsDateTimes, GdsElement, GdsLibrary, GdsPath, GdsPoint, GdsStrans,
    GdsStruct, GdsStructRef, GdsTextElem, GdsUnits,
};
use reticle_geometry::{
    Dbu, LayerId, Magnification, Orientation, Path, Point, Polygon, Rect, Transform,
};
use reticle_model::{
    Anchor, ArrayInstance, Cell, Document, DrawShape, Exporter, Importer, Instance, Label, Result,
    ShapeKind, Technology,
};

/// GDSII import/export (Wave 1: `gds21`).
///
/// Implements [`Importer`] (bytes to [`Document`]) and [`Exporter`]
/// ([`Document`] to bytes) using the [`gds21`] library. See the [module
/// docs](self) for the full mapping.
///
/// # Robustness
///
/// [`Gds::import`] never panics and never hangs, whatever bytes it is handed.
/// `gds21` reads from an in-memory cursor and every record consumes at least four
/// bytes, so parsing a finite slice terminates and allocates O(input) memory; the
/// few inputs that make `gds21` itself panic (a zero-length string record, an
/// out-of-range date field) are contained with [`std::panic::catch_unwind`] and
/// returned as [`IoError`]. Before parsing, an input larger than
/// [`MAX_INPUT_BYTES`] is rejected outright so a hostile length can never force a
/// huge allocation. After parsing, geometry that survived `gds21` but is
/// degenerate or oversized is *not* fatal: [`Gds::import_with_warnings`] drops or
/// clamps it and records an [`ImportWarning`], so a real-world file with a stray
/// bad element still opens. See [`import_with_warnings`](Gds::import_with_warnings).
#[derive(Debug, Default, Clone, Copy)]
pub struct Gds;

/// The largest GDSII input this importer will attempt to parse, in bytes
/// (256 MiB). A stream at or under this bound parses within a bounded allocation;
/// a larger one is refused with a clear [`IoError`] rather than risking an
/// out-of-memory abort on a hostile or truncated-huge input. Real published tiles
/// (the Tiny Tapeout and Sky130 corpora) are a few megabytes, far under this
/// ceiling.
pub const MAX_INPUT_BYTES: usize = 256 * 1024 * 1024;

/// The largest number of vertices a single boundary or path is allowed to carry
/// into the model. A conformant GDSII XY record is length-limited by its 16-bit
/// header (at most a few thousand points), but a re-encoded or crafted stream can
/// present more; past this bound the shape is skipped with a
/// [`WarningKind::LimitExceeded`] warning rather than materialized. This is a
/// defense-in-depth ceiling, not a live limit: a single conformant record can never
/// reach it.
pub const MAX_SHAPE_VERTICES: usize = 200_000;

/// Default GDSII layout version written on export (matches `gds21`'s own default).
const GDS_VERSION: i16 = 3;

/// The user unit assumed for GDSII, in metres. One micron.
const USER_UNIT_METERS: f64 = 1e-6;

/// Fallback database resolution when a document carries no technology, or a
/// technology with a non-positive `dbu_per_micron`. 1000 DBU/µm (1 nm grid) is
/// the ubiquitous GDSII default.
const DEFAULT_DBU_PER_MICRON: i64 = 1000;

/// The result of a GDSII import that kept its non-fatal warnings.
///
/// Returned by [`Gds::import_with_warnings`]. The [`document`](GdsImport::document)
/// is always well-formed and safe to use; [`warnings`](GdsImport::warnings) lists
/// every recoverable problem that was skipped, clamped, or defaulted during the
/// import (empty for a clean file). The frozen [`Importer::import`] path discards
/// the warnings and returns only the document, so callers that want to surface
/// what was dropped call this method instead.
#[derive(Debug, Clone)]
pub struct GdsImport {
    /// The imported document. Always valid, even when warnings are present.
    pub document: Document,
    /// Recoverable problems found during import, in encounter order.
    pub warnings: Vec<ImportWarning>,
}

impl Importer for Gds {
    fn import(&self, bytes: &[u8]) -> Result<Document> {
        // The frozen trait method returns only the document; warnings are dropped
        // here (callers wanting them use `import_with_warnings`).
        Ok(self.import_with_warnings(bytes)?.document)
    }
}

impl Gds {
    /// Imports GDSII `bytes` into a [`Document`], keeping every non-fatal warning.
    ///
    /// This is the hardened import entry point. It never panics and never hangs on
    /// any input (see the [type docs](Gds#robustness)):
    ///
    /// * An input larger than [`MAX_INPUT_BYTES`] is refused up front with an
    ///   [`IoError`], so a hostile length cannot force a huge allocation.
    /// * `gds21` parses from an in-memory cursor where every record consumes at
    ///   least four bytes, so parsing a finite slice terminates. The handful of
    ///   inputs that panic `gds21` internally are contained with
    ///   [`std::panic::catch_unwind`] (safe Rust, no `unsafe`) and returned as an
    ///   `Err`.
    /// * Geometry that parsed but is degenerate (too few vertices, zero area) or
    ///   oversized (more than [`MAX_SHAPE_VERTICES`] points) is not fatal: it is
    ///   skipped or clamped and an [`ImportWarning`] is recorded, so one bad
    ///   element does not sink an otherwise-good file.
    ///
    /// # Errors
    ///
    /// Returns [`reticle_model::ModelError`] (via [`IoError`]) when the input is
    /// too large, is not GDSII, or is malformed past recovery.
    pub fn import_with_warnings(&self, bytes: &[u8]) -> Result<GdsImport> {
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(IoError::Malformed(
                "GDSII input exceeds the maximum accepted size (256 MiB)",
            )
            .into());
        }

        // `gds21` panics on a few malformed inputs (for example a zero-length
        // string record indexes `data[len - 1]`, and an out-of-range date field
        // panics chrono). Contain any such panic so import upholds its contract of
        // returning `Err` rather than unwinding across this parser boundary.
        // `catch_unwind` is safe Rust; no `unsafe` is required.
        let owned = bytes.to_vec();
        let parsed = std::panic::catch_unwind(move || GdsLibrary::from_bytes(owned));
        let lib = match parsed {
            Ok(Ok(lib)) => lib,
            Ok(Err(e)) => return Err(IoError::gds(&e).into()),
            Err(_) => {
                return Err(IoError::Malformed("gds21 panicked while parsing GDSII bytes").into());
            }
        };

        let mut warnings = Warnings::new();
        let document = library_to_document(&lib, &mut warnings);
        Ok(GdsImport {
            document,
            warnings: warnings.into_vec(),
        })
    }
}

/// A bounded accumulator for [`ImportWarning`]s raised during an import.
///
/// Deduplicates by category so a pathological file (say, ten thousand degenerate
/// boundaries) yields one representative warning per category with a running
/// count, not ten thousand warnings that would themselves be a memory hazard.
struct Warnings {
    /// One entry per [`WarningKind`] seen: the first warning of that kind and how
    /// many total were folded into it.
    seen: Vec<(WarningKind, ImportWarning, usize)>,
}

impl Warnings {
    fn new() -> Self {
        Self { seen: Vec::new() }
    }

    /// Records one warning, folding repeats of the same kind into a single entry
    /// with a count so the list stays small and bounded.
    fn push(&mut self, w: ImportWarning) {
        if let Some(entry) = self.seen.iter_mut().find(|(k, ..)| *k == w.kind) {
            entry.2 += 1;
        } else {
            let kind = w.kind;
            self.seen.push((kind, w, 1));
        }
    }

    /// Flattens the accumulator into the final warning list, appending the folded
    /// repeat count to each entry's detail so nothing is hidden.
    fn into_vec(self) -> Vec<ImportWarning> {
        self.seen
            .into_iter()
            .map(|(_, mut w, count)| {
                if count > 1 {
                    w.detail = format!(
                        "{} ({} occurrences of this kind; first shown)",
                        w.detail, count
                    );
                }
                w
            })
            .collect()
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

/// Converts a parsed [`GdsLibrary`] into a Reticle [`Document`], recording any
/// non-fatal problems into `warnings`.
fn library_to_document(lib: &GdsLibrary, warnings: &mut Warnings) -> Document {
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
        let cell = struct_to_cell(strukt, &mut referenced, warnings);
        doc.insert_cell(cell);
    }

    // Collect the layer/datatype pairs actually used (by shapes and labels), so
    // downstream tooling sees a layer table even when importing from a bare GDS
    // with no technology file.
    let mut layers: Vec<LayerId> = Vec::new();
    for cell in doc.cells() {
        for layer in cell
            .shapes
            .iter()
            .map(|s| s.layer)
            .chain(cell.labels.iter().map(|l| l.layer))
        {
            if !layers.contains(&layer) {
                layers.push(layer);
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

/// Converts one [`GdsStruct`] to a [`Cell`], recording referenced cell names and
/// folding any degenerate/oversized geometry into `warnings` (skipping it) rather
/// than materializing it.
fn struct_to_cell(
    strukt: &GdsStruct,
    referenced: &mut std::collections::HashSet<String>,
    warnings: &mut Warnings,
) -> Cell {
    let mut cell = Cell::new(strukt.name.clone());
    for elem in &strukt.elems {
        match elem {
            GdsElement::GdsBoundary(b) => {
                if let Some(shape) = boundary_to_shape(b, &strukt.name, warnings) {
                    cell.shapes.push(shape);
                }
            }
            GdsElement::GdsPath(p) => {
                if let Some(shape) = path_to_shape(p, &strukt.name, warnings) {
                    cell.shapes.push(shape);
                }
            }
            GdsElement::GdsStructRef(sref) => {
                referenced.insert(sref.name.clone());
                cell.instances
                    .push(struct_ref_to_instance(sref, &strukt.name, warnings));
            }
            GdsElement::GdsArrayRef(aref) => {
                referenced.insert(aref.name.clone());
                cell.arrays
                    .push(array_ref_to_array(aref, &strukt.name, warnings));
            }
            GdsElement::GdsTextElem(text) => cell.labels.push(text_elem_to_label(text)),
            // Node and Box carry no drawn fill geometry we model in Wave 1. They
            // are skipped rather than erroring so real-world GDS still imports.
            GdsElement::GdsNode(_) | GdsElement::GdsBox(_) => {}
        }
    }
    cell
}

/// Maps a GDSII TEXT element to a [`Label`].
///
/// The TEXT `layer`/`texttype` pair plays the same role datatype plays for
/// boundaries, so it becomes the label's [`LayerId`] (bit-preserving, like
/// [`layer_id`]). The insertion point becomes [`Label::position`]. PRESENTATION
/// justification is not exposed by `gds21`, so the anchor is always
/// [`Anchor::Center`] (see the [module docs](self)).
fn text_elem_to_label(text: &GdsTextElem) -> Label {
    Label {
        text: text.string.clone(),
        position: gds_point_to_point(&text.xy),
        layer: layer_id(text.layer, text.texttype),
        anchor: Anchor::Center,
    }
}

/// Maps a GDSII boundary to a rectangle when its ring is an axis-aligned box,
/// otherwise to a polygon. Returns `None` (with a recorded warning) for a boundary
/// too degenerate to draw or one carrying an implausible number of vertices.
fn boundary_to_shape(b: &GdsBoundary, cell: &str, warnings: &mut Warnings) -> Option<DrawShape> {
    let layer = layer_id(b.layer, b.datatype);

    // Guard the vertex count before allocating the point vector: a crafted or
    // re-encoded XY record can present far more points than a real polygon needs.
    if b.xy.len() > MAX_SHAPE_VERTICES {
        warnings.push(ImportWarning::new(
            WarningKind::LimitExceeded,
            "boundary skipped: too many vertices",
            format!(
                "cell '{cell}', layer {}/{}: boundary has {} vertices (limit {MAX_SHAPE_VERTICES}); skipped",
                b.layer,
                b.datatype,
                b.xy.len()
            ),
        ));
        return None;
    }

    let mut pts: Vec<Point> = b.xy.iter().map(gds_point_to_point).collect();
    // GDSII repeats the first vertex as the last to close the ring. Drop the
    // duplicate closing vertex so our (implicitly closed) polygon is canonical.
    if pts.len() >= 2 && pts.first() == pts.last() {
        pts.pop();
    }

    // A ring needs at least three distinct vertices to bound any area. Fewer than
    // that is a degenerate boundary (a point or a sliver); skip it with a warning
    // rather than pushing a zero-area shape that no tool can use.
    if pts.len() < 3 {
        warnings.push(ImportWarning::new(
            WarningKind::DegenerateGeometry,
            "boundary skipped: fewer than 3 vertices",
            format!(
                "cell '{cell}', layer {}/{}: boundary ring has {} distinct vertices; skipped",
                b.layer,
                b.datatype,
                pts.len()
            ),
        ));
        return None;
    }

    if let Some(rect) = axis_aligned_rect(&pts) {
        Some(DrawShape::new(layer, ShapeKind::Rect(rect)))
    } else {
        Some(DrawShape::new(layer, ShapeKind::Polygon(Polygon::new(pts))))
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

/// Maps a GDSII path to a [`ShapeKind::Path`]. Returns `None` (with a recorded
/// warning) for a path with too few points to draw a centre-line or one carrying
/// an implausible number of vertices.
fn path_to_shape(p: &GdsPath, cell: &str, warnings: &mut Warnings) -> Option<DrawShape> {
    let layer = layer_id(p.layer, p.datatype);

    if p.xy.len() > MAX_SHAPE_VERTICES {
        warnings.push(ImportWarning::new(
            WarningKind::LimitExceeded,
            "path skipped: too many vertices",
            format!(
                "cell '{cell}', layer {}/{}: path has {} points (limit {MAX_SHAPE_VERTICES}); skipped",
                p.layer,
                p.datatype,
                p.xy.len()
            ),
        ));
        return None;
    }

    // A path centre-line needs at least two points to have any length.
    if p.xy.len() < 2 {
        warnings.push(ImportWarning::new(
            WarningKind::DegenerateGeometry,
            "path skipped: fewer than 2 points",
            format!(
                "cell '{cell}', layer {}/{}: path has {} points; skipped",
                p.layer,
                p.datatype,
                p.xy.len()
            ),
        ));
        return None;
    }

    let points: Vec<Point> = p.xy.iter().map(gds_point_to_point).collect();
    let width = p.width.unwrap_or(0);
    // Reticle's default endcap (`Flat`) corresponds to GDSII path-type 0. Path
    // types 1/2 (round/square) are not distinguished in Wave 1; width and the
    // centre-line are preserved, which is what round-trips through our writer.
    let path = Path::new(points, width, reticle_geometry::Endcap::Flat);
    Some(DrawShape::new(layer, ShapeKind::Path(path)))
}

/// Maps a GDSII struct reference to an [`Instance`].
fn struct_ref_to_instance(sref: &GdsStructRef, cell: &str, warnings: &mut Warnings) -> Instance {
    let transform = strans_to_transform(
        sref.strans.as_ref(),
        gds_point_to_point(&sref.xy),
        cell,
        &sref.name,
        warnings,
    );
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
fn array_ref_to_array(aref: &GdsArrayRef, cell: &str, warnings: &mut Warnings) -> ArrayInstance {
    let origin = gds_point_to_point(&aref.xy[0]);
    let col_end = gds_point_to_point(&aref.xy[1]);
    let row_end = gds_point_to_point(&aref.xy[2]);

    // GDSII stores counts as i16; a negative count is meaningless. Clamp to zero
    // and note it rather than trusting a wrapped value.
    if aref.cols < 0 || aref.rows < 0 {
        warnings.push(ImportWarning::new(
            WarningKind::ValueClamped,
            "array counts clamped: negative repetition",
            format!(
                "cell '{cell}', array of '{}': cols={} rows={} contained a negative count, clamped to 0",
                aref.name, aref.cols, aref.rows
            ),
        ));
    }
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

    let transform = strans_to_transform(aref.strans.as_ref(), origin, cell, &aref.name, warnings);
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
/// instance's translation. `cell`/`target` name the placement for any warning.
fn strans_to_transform(
    strans: Option<&GdsStrans>,
    translation: Point,
    cell: &str,
    target: &str,
    warnings: &mut Warnings,
) -> Transform {
    let mut transform = Transform {
        translation,
        ..Transform::IDENTITY
    };
    if let Some(s) = strans {
        transform.orientation = orientation_from_strans(s);
        if let Some(mag) = s.mag {
            transform.magnification = magnification_from_f64(mag, cell, target, warnings);
        }
    }
    transform
}

/// Maps a GDSII reflect flag + rotation angle to the nearest [`Orientation`].
///
/// GDSII applies reflection about the x-axis first, then a counter-clockwise
/// rotation, exactly Reticle's [`Orientation`] convention. Angles are snapped to
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
/// well within DBU precision for the placement magnitudes GDSII permits. A value
/// that is non-finite, non-positive, or too large to scale into the rational is
/// not representable; it falls back to unity and records a [`WarningKind::ValueClamped`]
/// warning rather than silently distorting the placement.
fn magnification_from_f64(
    mag: f64,
    cell: &str,
    target: &str,
    warnings: &mut Warnings,
) -> Magnification {
    const SCALE: f64 = 1_000_000.0;
    if (mag - 1.0).abs() < f64::EPSILON {
        return Magnification::UNITY;
    }
    if !mag.is_finite() || mag <= 0.0 {
        warnings.push(ImportWarning::new(
            WarningKind::ValueClamped,
            "magnification defaulted to 1.0",
            format!(
                "cell '{cell}', placement of '{target}': magnification {mag} is not a usable positive value; using 1.0"
            ),
        ));
        return Magnification::UNITY;
    }
    let num = (mag * SCALE).round();
    if num <= 0.0 || num > f64::from(u32::MAX) {
        warnings.push(ImportWarning::new(
            WarningKind::ValueClamped,
            "magnification defaulted to 1.0",
            format!(
                "cell '{cell}', placement of '{target}': magnification {mag} is outside the representable range; using 1.0"
            ),
        ));
        return Magnification::UNITY;
    }
    if let Some(m) = Magnification::new(num as u32, SCALE as u32) {
        m
    } else {
        warnings.push(ImportWarning::new(
            WarningKind::ValueClamped,
            "magnification defaulted to 1.0",
            format!(
                "cell '{cell}', placement of '{target}': magnification {mag} could not be represented; using 1.0"
            ),
        ));
        Magnification::UNITY
    }
}

// ---------------------------------------------------------------------------
// Export direction: Document -> GdsLibrary
// ---------------------------------------------------------------------------

/// The fixed modification/access timestamp stamped into every BGNLIB and BGNSTR
/// record on export.
///
/// `gds21` defaults these date fields to `Utc::now`, so an otherwise fully
/// deterministic document exports to different bytes on every run (the seconds
/// field ticks between two exports, e.g. two `xtask gen-layout` invocations).
/// Writing a fixed, valid date instead makes GDSII export byte-reproducible and
/// free of build time, upholding the generator's determinism contract. The
/// constant matches the corpus generator's `valid_dates`, so every reproducible
/// GDSII in the tree carries the same stamp.
fn reproducible_dates() -> GdsDateTimes {
    let stamp = NaiveDate::from_ymd_opt(2023, 1, 1)
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .expect("2023-01-01T00:00:00 is a valid timestamp");
    GdsDateTimes {
        modified: stamp,
        accessed: stamp,
    }
}

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
    lib.dates = reproducible_dates();
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
///
/// Elements are emitted in a fixed order (shapes, labels, instances, arrays) so
/// exports stay deterministic and a re-imported document exports to identical
/// bytes.
fn cell_to_struct(cell: &Cell) -> GdsStruct {
    let mut strukt = GdsStruct::new(cell.name.clone());
    strukt.dates = reproducible_dates();
    for shape in &cell.shapes {
        strukt.elems.push(shape_to_element(shape));
    }
    for label in &cell.labels {
        strukt.elems.push(label_to_element(label));
    }
    for inst in &cell.instances {
        strukt.elems.push(instance_to_element(inst));
    }
    for arr in &cell.arrays {
        strukt.elems.push(array_to_element(arr));
    }
    strukt
}

/// Converts a [`Label`] into a GDSII TEXT element on the label's layer/texttype.
///
/// The label's anchor is not encoded: `gds21` offers no public constructor for
/// its PRESENTATION type, so the record is omitted and readers fall back to
/// their default justification. Import mirrors this by anchoring every label
/// [`Anchor::Center`] (see the [module docs](self)).
fn label_to_element(label: &Label) -> GdsElement {
    let (layer, texttype) = layer_parts(label.layer);
    GdsElement::GdsTextElem(GdsTextElem {
        string: label.text.clone(),
        layer,
        texttype,
        xy: point_to_gds_point(label.position),
        ..GdsTextElem::default()
    })
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
