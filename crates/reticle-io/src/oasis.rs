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
//! carries the geometry Reticle most needs to round-trip today: rectangles and
//! polygons, tagged by `(layer, datatype)`, grouped into named cells, for a
//! document with any number of cells (its top cells are preserved). It borrows
//! OASIS's spirit — a magic string, a `START`/`END` frame, and `CELL` /
//! `RECTANGLE` / `POLYGON` records with explicit layer and datatype — without
//! claiming its wire format.
//!
//! The reader and writer are exact inverses, so
//! `import(export(doc)) == doc`-equivalent geometry is guaranteed for the
//! supported subset (see the round-trip test).
//!
//! # Container layout
//!
//! ```text
//! MAGIC        14 bytes  b"RETICLE-OASIS\0"
//! version       u8       format version (currently 1)
//! START  0x01   { dbu_per_micron: u64, top_count: u32, [top_name]*, cell_count: u32 }
//! CELL   0x02   { name: str, shape_count: u32, [shape]* }   (repeated cell_count times)
//! END    0xFF
//!
//! shape :=
//!   RECTANGLE 0x10 { layer: u16, datatype: u16, min_x,min_y,max_x,max_y: i32 }
//!   POLYGON   0x11 { layer: u16, datatype: u16, n: u32, [x: i32, y: i32]{n} }
//!
//! str := { len: u16, bytes: [u8; len] }   (UTF-8)
//! ```
//!
//! All multi-byte integers are little-endian.
//!
//! # TODO (documented coverage gaps, acceptable per the project's operating rules)
//!
//! - Paths ([`ShapeKind::Path`]) are not encoded. [`Oasis::export`] returns
//!   [`ModelError::Unsupported`] if a cell contains a path, rather than silently
//!   dropping it.
//! - Cell instances and arrays ([`Instance`](reticle_model::Instance),
//!   [`ArrayInstance`](reticle_model::ArrayInstance)) are not encoded (no
//!   `PLACEMENT`/`REPETITION` analog yet). Rather than silently flatten or drop
//!   the hierarchy, [`Oasis::export`] returns [`ModelError::Unsupported`] when a
//!   cell carries instances or arrays.
//! - No compression, properties, text, or non-rectangular trapezoids.
//! - The technology's layer table and rules are not serialized (only
//!   `dbu_per_micron`); the layer set is reconstructed from the geometry on
//!   import, mirroring the GDSII importer.

use crate::IoError;
use reticle_geometry::{LayerId, Point, Polygon, Rect};
use reticle_model::{
    Cell, Document, DrawShape, Exporter, Importer, LayerInfo, ModelError, Result, ShapeKind,
    Technology,
};

/// OASIS import/export (Wave 1: in-house subset, ADR 0004).
///
/// Implements [`Importer`] and [`Exporter`] for the container described in the
/// [module docs](self). Supports rectangles and polygons on `(layer, datatype)`;
/// paths, instances, and arrays are reported as unsupported rather than dropped.
#[derive(Debug, Default, Clone, Copy)]
pub struct Oasis;

/// Magic bytes at the head of every container.
const MAGIC: &[u8; 14] = b"RETICLE-OASIS\0";
/// Current container format version.
const VERSION: u8 = 1;

// Record tags.
const TAG_START: u8 = 0x01;
const TAG_CELL: u8 = 0x02;
const TAG_END: u8 = 0xFF;
const TAG_RECTANGLE: u8 = 0x10;
const TAG_POLYGON: u8 = 0x11;

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
        let mut tops = Vec::with_capacity(top_count as usize);
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

/// Writes one cell's `CELL` record, erroring on shapes outside the subset.
fn write_cell(w: &mut Writer, cell: &Cell) -> Result<()> {
    if !cell.instances.is_empty() || !cell.arrays.is_empty() {
        return Err(ModelError::Unsupported(
            "Reticle-OASIS subset does not encode instances or arrays",
        ));
    }
    w.bytes.push(TAG_CELL);
    w.string(&cell.name)?;
    w.u32(u32::try_from(cell.shapes.len()).unwrap_or(u32::MAX));
    for shape in &cell.shapes {
        write_shape(w, shape)?;
    }
    Ok(())
}

/// Writes one shape record, erroring on paths (unsupported in the subset).
fn write_shape(w: &mut Writer, shape: &DrawShape) -> Result<()> {
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
        ShapeKind::Path(_) => {
            return Err(ModelError::Unsupported(
                "Reticle-OASIS subset does not encode paths",
            ));
        }
    }
    Ok(())
}

/// Reads one `CELL` record.
fn read_cell(r: &mut Reader) -> Result<Cell> {
    expect_tag(r, TAG_CELL, "CELL")?;
    let name = r.string()?;
    let mut cell = Cell::new(name);
    let shape_count = r.u32()?;
    for _ in 0..shape_count {
        cell.shapes.push(read_shape(r)?);
    }
    Ok(cell)
}

/// Reads one shape record (rectangle or polygon).
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
            let mut verts = Vec::with_capacity(n as usize);
            for _ in 0..n {
                verts.push(Point::new(r.i32()?, r.i32()?));
            }
            Ok(DrawShape::new(
                layer,
                ShapeKind::Polygon(Polygon::new(verts)),
            ))
        }
        _ => Err(IoError::Malformed("unknown Reticle-OASIS shape tag").into()),
    }
}

/// Derives a sorted layer table from the geometry present in `doc`.
fn derive_layers(doc: &Document) -> Vec<LayerInfo> {
    let mut layers: Vec<LayerId> = Vec::new();
    for cell in doc.cells() {
        for shape in &cell.shapes {
            if !layers.contains(&shape.layer) {
                layers.push(shape.layer);
            }
        }
    }
    layers.sort_unstable();
    layers
        .into_iter()
        .map(|id| LayerInfo {
            id,
            name: format!("L{}D{}", id.layer, id.datatype),
            color_rgba: 0xFFFF_FFFF,
            visible: true,
        })
        .collect()
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
            _ => IoError::Malformed("expected CELL record"),
        }
        .into())
    }
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
}
