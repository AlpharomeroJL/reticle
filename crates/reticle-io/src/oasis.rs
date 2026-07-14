//! An in-house, OASIS-inspired binary subset (Wave 1, ADR 0004).
//!
//! # Honest scope
//!
//! This is **not** a conformant OASIS (SEMI P39) reader/writer. Full OASIS is a
//! large modal binary grammar with `CBLOCK` (zlib) compression, modal state
//! variables, `PLACEMENT`/`REPETITION` records, `TRAPEZOID`/`CTRAPEZOID`/`CIRCLE`
//! shapes, `PROPERTY` and `XNAME` tables, and strict start/end byte markers.
//! Implementing all of that is out of scope for Wave 1.
//!
//! Instead this module defines a small, self-describing binary container that
//! carries the geometry and hierarchy Reticle most needs to round-trip today:
//! rectangles, polygons, and paths tagged by `(layer, datatype)`, text labels
//! (with their anchor), plus cell instances (placements) and arrays, grouped
//! into named cells, for a document with any number of cells (its top cells are
//! preserved). It borrows OASIS's spirit, a magic string, a `START`/`END`
//! frame, and `CELL` / `RECTANGLE` / `POLYGON` / `PATH` / `TEXT` / `PLACEMENT` /
//! `ARRAY` records with explicit layer and datatype, without claiming its wire
//! format.
//!
//! The reader and writer are exact inverses, so
//! `import(export(doc)) == doc`-equivalent geometry is guaranteed for the
//! supported subset (see the round-trip test and the property test).
//!
//! # Container layout
//!
//! ```text
//! MAGIC        14 bytes  b"RETICLE-OASIS\0"
//! version       u8       format version (currently 3)
//! START  0x01   { dbu_per_micron: u64, top_count: u32, [top_name]*, cell_count: u32 }
//! CELL   0x02   { name: str, shape_count: u32, [shape]*,
//!                 instance_count: u32, [PLACEMENT]*,
//!                 array_count: u32, [ARRAY]*,
//!                 label_count: u32, [TEXT]* }              (repeated cell_count times)
//! END    0xFF
//!
//! shape :=
//!   RECTANGLE 0x10 { layer: u16, datatype: u16, min_x,min_y,max_x,max_y: i32 }
//!   POLYGON   0x11 { layer: u16, datatype: u16, n: u32, [x: i32, y: i32]{n} }
//!   PATH      0x12 { layer: u16, datatype: u16, width: i32, endcap,
//!                    n: u32, [x: i32, y: i32]{n} }
//!
//! endcap :=                       (a 1-byte kind, then a 4-byte i32 extension)
//!   0x00 Flat        + ext i32 (0)
//!   0x01 Square      + ext i32 (0)
//!   0x02 Round       + ext i32 (0)
//!   0x03 Custom(ext) + ext i32
//!
//! transform :=                    (translation, orientation, magnification)
//!   { dx: i32, dy: i32, orientation: u8, magnification: u64 (f64 bits) }
//!
//! PLACEMENT 0x20 { cell_ref: str, transform }
//! ARRAY     0x21 { cell_ref: str, transform,
//!                  columns: u32, rows: u32, column_pitch: i32, row_pitch: i32 }
//! TEXT      0x30 { layer: u16, datatype: u16, anchor: u8, x: i32, y: i32,
//!                  text: str }
//!
//! anchor :=                       (which point of the label box x,y denotes)
//!   0x00 Center   0x01 SouthWest   0x02 SouthEast   0x03 NorthWest
//!   0x04 NorthEast
//!
//! str := { len: u16, bytes: [u8; len] }   (UTF-8)
//! ```
//!
//! All multi-byte integers are little-endian. The magnification is written as the
//! IEEE-754 bit pattern of its `f64` value (`f64::to_bits`), then reconstructed on
//! read as an exact rational, matching the GDSII importer's magnification handling.
//!
//! # TODO (documented coverage gaps, acceptable per the project's operating rules)
//!
//! - No compression, properties, or non-rectangular trapezoids. Text labels are
//!   carried (with their anchor, which GDSII drops), but pins are not.
//! - Path end caps are preserved (including a custom extension), but the round /
//!   square distinction carries no separate geometry beyond the recorded kind.
//! - The technology's layer table and rules are not serialized (only
//!   `dbu_per_micron`); the layer set is reconstructed from the geometry on
//!   import, mirroring the GDSII importer.

use crate::IoError;
use reticle_geometry::{
    Endcap, LayerId, Magnification, Orientation, Path, Point, Polygon, Rect, Transform,
};
use reticle_model::{
    Anchor, ArrayInstance, Cell, Document, DrawShape, Exporter, Importer, Instance, Label,
    LayerInfo, Result, ShapeKind, Technology,
};

/// OASIS import/export (Wave 1: in-house subset, ADR 0004).
///
/// Implements [`Importer`] and [`Exporter`] for the container described in the
/// [module docs](self). Supports rectangles, polygons, and paths on
/// `(layer, datatype)`, text labels (anchor included), plus cell instances
/// (placements) and arrays, so a document's flat geometry, annotations, and
/// hierarchy all round-trip.
#[derive(Debug, Default, Clone, Copy)]
pub struct Oasis;

/// Magic bytes at the head of every container.
const MAGIC: &[u8; 14] = b"RETICLE-OASIS\0";
/// Current container format version.
const VERSION: u8 = 3;

// Record tags.
const TAG_START: u8 = 0x01;
const TAG_CELL: u8 = 0x02;
const TAG_END: u8 = 0xFF;
const TAG_RECTANGLE: u8 = 0x10;
const TAG_POLYGON: u8 = 0x11;
const TAG_PATH: u8 = 0x12;
const TAG_PLACEMENT: u8 = 0x20;
const TAG_ARRAY: u8 = 0x21;
const TAG_TEXT: u8 = 0x30;

// Anchor discriminants (a single byte), matching the model enum order.
const ANCH_CENTER: u8 = 0x00;
const ANCH_SOUTH_WEST: u8 = 0x01;
const ANCH_SOUTH_EAST: u8 = 0x02;
const ANCH_NORTH_WEST: u8 = 0x03;
const ANCH_NORTH_EAST: u8 = 0x04;

// Endcap kind discriminants (a 1-byte kind followed by an i32 extension).
const CAP_FLAT: u8 = 0x00;
const CAP_SQUARE: u8 = 0x01;
const CAP_ROUND: u8 = 0x02;
const CAP_CUSTOM: u8 = 0x03;

// Orientation discriminants (a single byte), matching the geometry enum order.
const ORI_R0: u8 = 0x00;
const ORI_R90: u8 = 0x01;
const ORI_R180: u8 = 0x02;
const ORI_R270: u8 = 0x03;
const ORI_MIRROR_X: u8 = 0x04;
const ORI_MIRROR_X90: u8 = 0x05;
const ORI_MIRROR_X180: u8 = 0x06;
const ORI_MIRROR_X270: u8 = 0x07;

impl Exporter for Oasis {
    fn export(&self, doc: &Document) -> Result<Vec<u8>> {
        let mut w = Writer::new();
        w.bytes.extend_from_slice(MAGIC);
        w.bytes.push(VERSION);

        // START record.
        w.bytes.push(TAG_START);
        w.u64(doc.technology().dbu_per_micron as u64);
        let tops = doc.top_cells();
        w.u32(u32::try_from(tops.len()).unwrap_or(u32::MAX));
        for name in tops {
            w.string(name)?;
        }

        // Emit cells in a deterministic order (sorted by name) for stable output.
        let mut cells: Vec<&Cell> = doc.cells().collect();
        cells.sort_by(|a, b| a.name.cmp(&b.name));
        w.u32(u32::try_from(cells.len()).unwrap_or(u32::MAX));
        for cell in cells {
            write_cell(&mut w, cell)?;
        }

        // END record.
        w.bytes.push(TAG_END);
        Ok(w.bytes)
    }
}

impl Importer for Oasis {
    fn import(&self, bytes: &[u8]) -> Result<Document> {
        let mut r = Reader::new(bytes);
        // Header.
        let magic = r.take(MAGIC.len())?;
        if magic != MAGIC {
            return Err(IoError::Malformed("not a Reticle-OASIS container").into());
        }
        let version = r.u8()?;
        if version != VERSION {
            return Err(IoError::Unsupported("unsupported Reticle-OASIS version").into());
        }

        // START.
        expect_tag(&mut r, TAG_START, "START")?;
        let dbu_per_micron = r.u64()? as i64;
        let top_count = r.u32()?;
        // A string costs at least its 2-byte length prefix, so cap the reserve.
        let mut tops = Vec::with_capacity(r.prealloc(top_count, 2));
        for _ in 0..top_count {
            tops.push(r.string()?);
        }

        let mut doc = Document::new();
        let cell_count = r.u32()?;
        for _ in 0..cell_count {
            let cell = read_cell(&mut r)?;
            doc.insert_cell(cell);
        }

        // END.
        expect_tag(&mut r, TAG_END, "END")?;

        // Rebuild a minimal technology: the resolution plus a layer table derived
        // from the geometry, matching the GDSII importer's behaviour.
        let mut tech = Technology {
            dbu_per_micron,
            ..Technology::default()
        };
        tech.layers = derive_layers(&doc);
        doc.set_technology(tech);
        doc.set_top_cells(tops);
        Ok(doc)
    }
}

/// Writes one cell's `CELL` record: its shapes, then its instances, arrays, and
/// labels.
fn write_cell(w: &mut Writer, cell: &Cell) -> Result<()> {
    w.bytes.push(TAG_CELL);
    w.string(&cell.name)?;
    w.u32(u32::try_from(cell.shapes.len()).unwrap_or(u32::MAX));
    for shape in &cell.shapes {
        write_shape(w, shape);
    }
    w.u32(u32::try_from(cell.instances.len()).unwrap_or(u32::MAX));
    for inst in &cell.instances {
        write_placement(w, inst)?;
    }
    w.u32(u32::try_from(cell.arrays.len()).unwrap_or(u32::MAX));
    for arr in &cell.arrays {
        write_array(w, arr)?;
    }
    w.u32(u32::try_from(cell.labels.len()).unwrap_or(u32::MAX));
    for label in &cell.labels {
        write_text(w, label)?;
    }
    Ok(())
}

/// Writes one shape record (rectangle, polygon, or path).
fn write_shape(w: &mut Writer, shape: &DrawShape) {
    let layer = shape.layer;
    match &shape.kind {
        ShapeKind::Rect(r) => {
            w.bytes.push(TAG_RECTANGLE);
            w.layer(layer);
            w.i32(r.min.x);
            w.i32(r.min.y);
            w.i32(r.max.x);
            w.i32(r.max.y);
        }
        ShapeKind::Polygon(p) => {
            w.bytes.push(TAG_POLYGON);
            w.layer(layer);
            w.u32(u32::try_from(p.vertices().len()).unwrap_or(u32::MAX));
            for v in p.vertices() {
                w.i32(v.x);
                w.i32(v.y);
            }
        }
        ShapeKind::Path(p) => {
            w.bytes.push(TAG_PATH);
            w.layer(layer);
            w.i32(p.width());
            w.endcap(p.endcap());
            w.u32(u32::try_from(p.points().len()).unwrap_or(u32::MAX));
            for v in p.points() {
                w.i32(v.x);
                w.i32(v.y);
            }
        }
    }
}

/// Writes one `PLACEMENT` record for a single [`Instance`].
fn write_placement(w: &mut Writer, inst: &Instance) -> Result<()> {
    w.bytes.push(TAG_PLACEMENT);
    w.string(&inst.cell)?;
    w.transform(&inst.transform);
    Ok(())
}

/// Writes one `ARRAY` record for an [`ArrayInstance`].
fn write_array(w: &mut Writer, arr: &ArrayInstance) -> Result<()> {
    w.bytes.push(TAG_ARRAY);
    w.string(&arr.cell)?;
    w.transform(&arr.transform);
    w.u32(arr.columns);
    w.u32(arr.rows);
    w.i32(arr.column_pitch);
    w.i32(arr.row_pitch);
    Ok(())
}

/// Writes one `TEXT` record for a [`Label`], anchor included (which the GDSII
/// path cannot carry; see `gds.rs`).
fn write_text(w: &mut Writer, label: &Label) -> Result<()> {
    w.bytes.push(TAG_TEXT);
    w.layer(label.layer);
    w.bytes.push(anchor_to_u8(label.anchor));
    w.i32(label.position.x);
    w.i32(label.position.y);
    w.string(&label.text)?;
    Ok(())
}

/// Reads one `CELL` record: its shapes, then its instances, arrays, and labels.
fn read_cell(r: &mut Reader) -> Result<Cell> {
    expect_tag(r, TAG_CELL, "CELL")?;
    let name = r.string()?;
    let mut cell = Cell::new(name);
    let shape_count = r.u32()?;
    for _ in 0..shape_count {
        cell.shapes.push(read_shape(r)?);
    }
    let instance_count = r.u32()?;
    for _ in 0..instance_count {
        cell.instances.push(read_placement(r)?);
    }
    let array_count = r.u32()?;
    for _ in 0..array_count {
        cell.arrays.push(read_array(r)?);
    }
    let label_count = r.u32()?;
    for _ in 0..label_count {
        cell.labels.push(read_text(r)?);
    }
    Ok(cell)
}

/// Reads one shape record (rectangle, polygon, or path).
fn read_shape(r: &mut Reader) -> Result<DrawShape> {
    let tag = r.u8()?;
    match tag {
        TAG_RECTANGLE => {
            let layer = r.layer()?;
            let min = Point::new(r.i32()?, r.i32()?);
            let max = Point::new(r.i32()?, r.i32()?);
            Ok(DrawShape::new(layer, ShapeKind::Rect(Rect::new(min, max))))
        }
        TAG_POLYGON => {
            let layer = r.layer()?;
            let n = r.u32()?;
            // Each vertex is two i32s (8 bytes); cap the reserve to the input.
            let mut verts = Vec::with_capacity(r.prealloc(n, 8));
            for _ in 0..n {
                verts.push(Point::new(r.i32()?, r.i32()?));
            }
            Ok(DrawShape::new(
                layer,
                ShapeKind::Polygon(Polygon::new(verts)),
            ))
        }
        TAG_PATH => {
            let layer = r.layer()?;
            let width = r.i32()?;
            let endcap = r.endcap()?;
            let n = r.u32()?;
            // Each point is two i32s (8 bytes); cap the reserve to the input.
            let mut points = Vec::with_capacity(r.prealloc(n, 8));
            for _ in 0..n {
                points.push(Point::new(r.i32()?, r.i32()?));
            }
            Ok(DrawShape::new(
                layer,
                ShapeKind::Path(Path::new(points, width, endcap)),
            ))
        }
        _ => Err(IoError::Malformed("unknown Reticle-OASIS shape tag").into()),
    }
}

/// Reads one `PLACEMENT` record into an [`Instance`].
fn read_placement(r: &mut Reader) -> Result<Instance> {
    expect_tag(r, TAG_PLACEMENT, "PLACEMENT")?;
    let cell = r.string()?;
    let transform = r.transform()?;
    Ok(Instance { cell, transform })
}

/// Reads one `ARRAY` record into an [`ArrayInstance`].
fn read_array(r: &mut Reader) -> Result<ArrayInstance> {
    expect_tag(r, TAG_ARRAY, "ARRAY")?;
    let cell = r.string()?;
    let transform = r.transform()?;
    let columns = r.u32()?;
    let rows = r.u32()?;
    let column_pitch = r.i32()?;
    let row_pitch = r.i32()?;
    Ok(ArrayInstance {
        cell,
        transform,
        columns,
        rows,
        column_pitch,
        row_pitch,
    })
}

/// Reads one `TEXT` record into a [`Label`].
fn read_text(r: &mut Reader) -> Result<Label> {
    expect_tag(r, TAG_TEXT, "TEXT")?;
    let layer = r.layer()?;
    let anchor = anchor_from_u8(r.u8()?)?;
    let position = Point::new(r.i32()?, r.i32()?);
    let text = r.string()?;
    Ok(Label {
        text,
        position,
        layer,
        anchor,
    })
}

/// Derives a sorted layer table from the geometry and labels present in `doc`,
/// matching the GDSII importer's behaviour.
fn derive_layers(doc: &Document) -> Vec<LayerInfo> {
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
    // Distinct fallback colors, not a uniform white (see gds.rs): a bare OASIS renders as
    // distinct-colored layers rather than a single white blob.
    layers.into_iter().map(LayerInfo::placeholder).collect()
}

/// Reads a record tag and checks it matches, naming the record in any error.
fn expect_tag(r: &mut Reader, expected: u8, what: &'static str) -> Result<()> {
    let tag = r.u8()?;
    if tag == expected {
        Ok(())
    } else {
        Err(match what {
            "START" => IoError::Malformed("expected START record"),
            "END" => IoError::Malformed("expected END record"),
            "PLACEMENT" => IoError::Malformed("expected PLACEMENT record"),
            "ARRAY" => IoError::Malformed("expected ARRAY record"),
            "TEXT" => IoError::Malformed("expected TEXT record"),
            _ => IoError::Malformed("expected CELL record"),
        }
        .into())
    }
}

/// Converts a [`Magnification`] to the `f64` written on the wire.
///
/// [`Magnification`] keeps its numerator and denominator private, exposing only
/// unity detection and [`Magnification::scale`]. Unity is represented exactly;
/// other ratios are recovered by scaling a high-precision probe, matching the
/// GDSII importer (`magnification_to_f64`).
fn magnification_to_f64(mag: Magnification) -> f64 {
    const PROBE: i32 = 1_000_000;
    if mag.is_unity() {
        return 1.0;
    }
    f64::from(mag.scale(PROBE)) / f64::from(PROBE)
}

/// Reconstructs a [`Magnification`] from a wire `f64`, as an exact rational.
///
/// Mirrors the GDSII importer (`magnification_from_f64`): unity and non-positive
/// or out-of-range values collapse to [`Magnification::UNITY`]; other values are
/// stored as `round(mag * 1_000_000) / 1_000_000`, well within DBU precision.
fn magnification_from_f64(mag: f64) -> Magnification {
    const SCALE: f64 = 1_000_000.0;
    if !mag.is_finite() || (mag - 1.0).abs() < f64::EPSILON || mag <= 0.0 {
        return Magnification::UNITY;
    }
    let num = (mag * SCALE).round();
    if num <= 0.0 || num > f64::from(u32::MAX) {
        return Magnification::UNITY;
    }
    Magnification::new(num as u32, SCALE as u32).unwrap_or(Magnification::UNITY)
}

/// A minimal little-endian byte writer.
struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }
    fn u32(&mut self, v: u32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    fn layer(&mut self, id: LayerId) {
        self.bytes.extend_from_slice(&id.layer.to_le_bytes());
        self.bytes.extend_from_slice(&id.datatype.to_le_bytes());
    }
    /// Writes a length-prefixed UTF-8 string, erroring if it exceeds `u16::MAX`.
    fn string(&mut self, s: &str) -> Result<()> {
        let len = u16::try_from(s.len())
            .map_err(|_| IoError::Unsupported("Reticle-OASIS name exceeds 65535 bytes"))?;
        self.bytes.extend_from_slice(&len.to_le_bytes());
        self.bytes.extend_from_slice(s.as_bytes());
        Ok(())
    }
    /// Writes a path end cap: a 1-byte kind followed by an i32 extension (0 for the
    /// non-custom kinds, the custom amount otherwise).
    fn endcap(&mut self, cap: Endcap) {
        let (kind, ext) = match cap {
            Endcap::Flat => (CAP_FLAT, 0),
            Endcap::Square => (CAP_SQUARE, 0),
            Endcap::Round => (CAP_ROUND, 0),
            Endcap::Custom(e) => (CAP_CUSTOM, e),
        };
        self.bytes.push(kind);
        self.i32(ext);
    }
    /// Writes a placement transform: translation, orientation, magnification.
    fn transform(&mut self, t: &Transform) {
        self.i32(t.translation.x);
        self.i32(t.translation.y);
        self.bytes.push(orientation_to_u8(t.orientation));
        self.u64(magnification_to_f64(t.magnification).to_bits());
    }
}

/// Maps an [`Anchor`] to its 1-byte wire discriminant.
///
/// [`Anchor`] is `#[non_exhaustive]`; any variant this version does not know
/// (there are none today) would write as `Center`, the model default, rather
/// than failing the export.
fn anchor_to_u8(a: Anchor) -> u8 {
    match a {
        Anchor::SouthWest => ANCH_SOUTH_WEST,
        Anchor::SouthEast => ANCH_SOUTH_EAST,
        Anchor::NorthWest => ANCH_NORTH_WEST,
        Anchor::NorthEast => ANCH_NORTH_EAST,
        _ => ANCH_CENTER,
    }
}

/// Maps a 1-byte wire discriminant back to an [`Anchor`].
fn anchor_from_u8(b: u8) -> Result<Anchor> {
    match b {
        ANCH_CENTER => Ok(Anchor::Center),
        ANCH_SOUTH_WEST => Ok(Anchor::SouthWest),
        ANCH_SOUTH_EAST => Ok(Anchor::SouthEast),
        ANCH_NORTH_WEST => Ok(Anchor::NorthWest),
        ANCH_NORTH_EAST => Ok(Anchor::NorthEast),
        _ => Err(IoError::Malformed("unknown Reticle-OASIS anchor").into()),
    }
}

/// Maps an [`Orientation`] to its 1-byte wire discriminant.
fn orientation_to_u8(o: Orientation) -> u8 {
    match o {
        Orientation::R0 => ORI_R0,
        Orientation::R90 => ORI_R90,
        Orientation::R180 => ORI_R180,
        Orientation::R270 => ORI_R270,
        Orientation::MirrorX => ORI_MIRROR_X,
        Orientation::MirrorX90 => ORI_MIRROR_X90,
        Orientation::MirrorX180 => ORI_MIRROR_X180,
        Orientation::MirrorX270 => ORI_MIRROR_X270,
    }
}

/// A minimal, bounds-checked little-endian byte reader.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
    /// Bytes not yet consumed.
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }
    /// A safe pre-allocation size for a collection the input claims has `count`
    /// elements, each costing at least `min_elem_bytes` in the stream.
    ///
    /// A hostile or truncated container can name a `count` of billions in a few
    /// bytes; pre-allocating `count` directly is an out-of-memory vector (found
    /// by the v8 fuzz campaign). Since each element consumes at least
    /// `min_elem_bytes` of not-yet-read input, the stream can hold at most
    /// `remaining / min_elem_bytes` of them, so capping the reservation there
    /// bounds the allocation to the input size while never over-reserving for an
    /// honest file. The loop that fills the vector still reads element by element
    /// and errors cleanly at end of input if the count was a lie.
    fn prealloc(&self, count: u32, min_elem_bytes: usize) -> usize {
        (count as usize).min(self.remaining() / min_elem_bytes.max(1))
    }
    /// Returns the next `n` bytes, or a malformed error if the input is short.
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(IoError::Malformed("Reticle-OASIS length overflow"))?;
        if end > self.bytes.len() {
            return Err(IoError::Malformed("Reticle-OASIS truncated").into());
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
    fn i32(&mut self) -> Result<i32> {
        let b = self.take(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn layer(&mut self) -> Result<LayerId> {
        let l = self.take(2)?;
        let layer = u16::from_le_bytes([l[0], l[1]]);
        let d = self.take(2)?;
        let datatype = u16::from_le_bytes([d[0], d[1]]);
        Ok(LayerId::new(layer, datatype))
    }
    /// Reads a length-prefixed UTF-8 string.
    fn string(&mut self) -> Result<String> {
        let len = {
            let b = self.take(2)?;
            u16::from_le_bytes([b[0], b[1]]) as usize
        };
        let raw = self.take(len)?;
        String::from_utf8(raw.to_vec())
            .map_err(|_| IoError::Malformed("Reticle-OASIS name is not valid UTF-8").into())
    }
    /// Reads a path end cap: a 1-byte kind followed by an i32 extension.
    fn endcap(&mut self) -> Result<Endcap> {
        let kind = self.u8()?;
        let ext = self.i32()?;
        match kind {
            CAP_FLAT => Ok(Endcap::Flat),
            CAP_SQUARE => Ok(Endcap::Square),
            CAP_ROUND => Ok(Endcap::Round),
            CAP_CUSTOM => Ok(Endcap::Custom(ext)),
            _ => Err(IoError::Malformed("unknown Reticle-OASIS endcap kind").into()),
        }
    }
    /// Reads a placement transform: translation, orientation, magnification.
    fn transform(&mut self) -> Result<Transform> {
        let translation = Point::new(self.i32()?, self.i32()?);
        let orientation = orientation_from_u8(self.u8()?)?;
        let magnification = magnification_from_f64(f64::from_bits(self.u64()?));
        Ok(Transform {
            translation,
            orientation,
            magnification,
        })
    }
}

/// Maps a 1-byte wire discriminant back to an [`Orientation`].
fn orientation_from_u8(b: u8) -> Result<Orientation> {
    match b {
        ORI_R0 => Ok(Orientation::R0),
        ORI_R90 => Ok(Orientation::R90),
        ORI_R180 => Ok(Orientation::R180),
        ORI_R270 => Ok(Orientation::R270),
        ORI_MIRROR_X => Ok(Orientation::MirrorX),
        ORI_MIRROR_X90 => Ok(Orientation::MirrorX90),
        ORI_MIRROR_X180 => Ok(Orientation::MirrorX180),
        ORI_MIRROR_X270 => Ok(Orientation::MirrorX270),
        _ => Err(IoError::Malformed("unknown Reticle-OASIS orientation").into()),
    }
}
