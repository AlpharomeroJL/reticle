//! A conformant **OASIS** (SEMI P39) writer for a practical subset — the real thing
//! `KLayout` can read, distinct from the in-house [`Oasis`](crate::Oasis) container.
//!
//! # Scope and honesty
//!
//! [`Oasis`](crate::Oasis) is an in-house, OASIS-*inspired* binary container (ADR 0004)
//! that `KLayout` cannot read. This module is the opposite: a genuine SEMI P39 OASIS
//! *writer* whose output `KLayout` reads as OASIS. A reader is **out of scope** (this is a
//! one-directional export). The subset is:
//!
//! * uncompressed only — no `CBLOCK` (zlib) blocks;
//! * `RECTANGLE`, `POLYGON`, `PATH`, `PLACEMENT`, and `TEXT` records;
//! * explicit modal state — every element carries its own layer, datatype, and
//!   coordinates (and dimensions/point-list/extension where applicable), so the writer
//!   never depends on an inherited modal variable, the most common OASIS-conformance
//!   pitfall;
//! * `CELLNAME` records (implicit numbering) plus `CELL` records referencing them by
//!   number; `PLACEMENT` type 18 references cells by number and carries magnification
//!   and angle so any Reticle [`Transform`] round-trips;
//! * arrays are **expanded** into individual placements (no OASIS repetition), so a
//!   large array inflates the file; this is a documented simplification of the subset.
//!
//! Reticle features OASIS has no room for in this subset are dropped honestly: a
//! [`Label`]'s anchor (OASIS `TEXT` is a point), and a path's *round* end cap (OASIS
//! path extensions are flush / half-width / explicit only, so a round cap is written
//! flush and noted here).
//!
//! # Encoding reference
//!
//! All integers are OASIS variable-length: unsigned integers are 7 bits/byte,
//! little-endian, high bit = continuation; signed integers put the sign in the low bit
//! of the magnitude shifted left one (0 = positive). Strings are a length (unsigned
//! integer) followed by the bytes. The file is
//! `magic · START · (CELLNAME)* · (CELL · elements)* · END`, and the `END` record is
//! padded so the whole record is exactly 256 bytes with no trailing bytes, per §14.2 of
//! the standard.

use reticle_geometry::{Dbu, Endcap, Orientation, Point, Transform};
use reticle_model::{ArrayInstance, Cell, Document, Exporter, Label, Result, ShapeKind};

/// The conformant-OASIS exporter (a practical writer subset; see the [module docs](self)).
///
/// Implements [`Exporter`], so it plugs into the same trait the CLI and app use for
/// every format. It never reads OASIS — export only.
#[derive(Debug, Default, Clone, Copy)]
pub struct OasisStd;

// Record ids (each is a single-byte unsigned integer here).
const REC_START: u8 = 1;
const REC_END: u8 = 2;
const REC_CELLNAME: u8 = 3; // implicit numbering
const REC_CELL_REF: u8 = 13; // cell named by reference number
const REC_PLACEMENT_TRANSFORM: u8 = 18; // placement with magnification + angle
const REC_TEXT: u8 = 19;
const REC_RECTANGLE: u8 = 20;
const REC_POLYGON: u8 = 21;
const REC_PATH: u8 = 22;

impl Exporter for OasisStd {
    fn export(&self, doc: &Document) -> Result<Vec<u8>> {
        Ok(write_document(doc))
    }
}

/// Serializes `doc` to a conformant OASIS byte stream.
fn write_document(doc: &Document) -> Vec<u8> {
    let mut w = Writer::new();
    w.magic();

    // START: version "1.0", unit = database units per micron (grid steps per micron,
    // §13.3), offset-flag 0 with all table-offsets zero (non-strict, tables inline).
    let dbu_per_micron = doc.technology().dbu_per_micron;
    let unit = if dbu_per_micron > 0 {
        dbu_per_micron as u64
    } else {
        1000 // a sane 1 nm grid when the document carries no resolution
    };
    w.byte(REC_START);
    w.a_string("1.0");
    w.real_whole(unit);
    w.uint(0); // offset-flag = 0: table-offsets follow, here, all zero
    for _ in 0..12 {
        w.byte(0); // six (flag, offset) pairs, all zero
    }

    // The cell name table: every defined cell plus any cell merely referenced by a
    // placement, sorted for deterministic output and assigned reference numbers 0,1,2…
    let names = cell_name_table(doc);
    let index_of = |name: &str| names.iter().position(|n| n == name);
    for name in &names {
        w.byte(REC_CELLNAME);
        w.n_string(name);
    }

    // Cell bodies, in the same deterministic order. Only defined cells get a body.
    let mut cells: Vec<&Cell> = doc.cells().collect();
    cells.sort_by(|a, b| a.name.cmp(&b.name));
    for cell in cells {
        let Some(idx) = index_of(&cell.name) else {
            continue;
        };
        w.byte(REC_CELL_REF);
        w.uint(idx as u64);
        write_cell_body(&mut w, cell, &index_of);
    }

    w.end_record();
    w.into_bytes()
}

/// The sorted, de-duplicated list of every cell name that must appear in the cellname
/// table: all defined cells plus any name a placement or array references.
fn cell_name_table(doc: &Document) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let collect = |n: &str, names: &mut Vec<String>| {
        if !names.iter().any(|x| x == n) {
            names.push(n.to_string());
        }
    };
    for cell in doc.cells() {
        collect(&cell.name, &mut names);
        for inst in &cell.instances {
            collect(&inst.cell, &mut names);
        }
        for arr in &cell.arrays {
            collect(&arr.cell, &mut names);
        }
    }
    names.sort();
    names
}

/// Writes one cell's geometry, placements, arrays, and labels. `index_of` maps a
/// referenced cell name to its cellname reference number.
fn write_cell_body(w: &mut Writer, cell: &Cell, index_of: &impl Fn(&str) -> Option<usize>) {
    for shape in &cell.shapes {
        let (layer, datatype) = (shape.layer.layer, shape.layer.datatype);
        match &shape.kind {
            ShapeKind::Rect(r) => write_rectangle(w, layer, datatype, r),
            ShapeKind::Polygon(p) => write_polygon(w, layer, datatype, p.vertices()),
            ShapeKind::Path(p) => write_path(w, layer, datatype, p),
        }
    }
    for label in &cell.labels {
        write_text(w, label);
    }
    for inst in &cell.instances {
        if let Some(idx) = index_of(&inst.cell) {
            write_placement(w, idx, &inst.transform);
        }
    }
    for arr in &cell.arrays {
        if let Some(idx) = index_of(&arr.cell) {
            write_array_expanded(w, idx, arr);
        }
    }
}

/// RECTANGLE (id 20): info `SWHXYRDL`. Always emits layer, datatype, width, height,
/// x, y — never relying on inherited modal state.
fn write_rectangle(w: &mut Writer, layer: u16, datatype: u16, r: &reticle_geometry::Rect) {
    // Info: S0 W1 H1 X1 Y1 R0 D1 L1 = 0b0111_1011.
    w.byte(REC_RECTANGLE);
    w.byte(0b0111_1011);
    w.uint(u64::from(layer));
    w.uint(u64::from(datatype));
    w.uint(dim(i64::from(r.max.x) - i64::from(r.min.x)));
    w.uint(dim(i64::from(r.max.y) - i64::from(r.min.y)));
    w.sint(i64::from(r.min.x));
    w.sint(i64::from(r.min.y));
}

/// POLYGON (id 21): info `00PXYRDL`, point-list type 4 (explicit all-angle g-deltas),
/// with the closing delta implicit. Degenerate polygons (< 3 vertices) are skipped.
fn write_polygon(w: &mut Writer, layer: u16, datatype: u16, verts: &[Point]) {
    // Drop a trailing vertex that repeats the first (Reticle rings are implicitly
    // closed, but tolerate an explicit closure).
    let mut vs = verts;
    if vs.len() >= 2 && vs[0] == vs[vs.len() - 1] {
        vs = &vs[..vs.len() - 1];
    }
    if vs.len() < 3 {
        return;
    }
    // Info: 0 0 P1 X1 Y1 R0 D1 L1 = 0b0011_1011.
    w.byte(REC_POLYGON);
    w.byte(0b0011_1011);
    w.uint(u64::from(layer));
    w.uint(u64::from(datatype));
    // point-list type 4, count = vertices - 1 (closing delta implied for polygons).
    w.uint(4);
    w.uint((vs.len() - 1) as u64);
    for pair in vs.windows(2) {
        w.g_delta(pair[1].x, pair[1].y, pair[0].x, pair[0].y);
    }
    w.sint(i64::from(vs[0].x));
    w.sint(i64::from(vs[0].y));
}

/// PATH (id 22): info `EWPXYRDL`, explicit extension scheme, point-list type 4. Paths
/// list every delta (no implicit closure). Paths with < 2 points are skipped.
fn write_path(w: &mut Writer, layer: u16, datatype: u16, p: &reticle_geometry::Path) {
    let pts = p.points();
    if pts.len() < 2 {
        return;
    }
    // Info: E1 W1 P1 X1 Y1 R0 D1 L1 = 0b1111_1011.
    w.byte(REC_PATH);
    w.byte(0b1111_1011);
    w.uint(u64::from(layer));
    w.uint(u64::from(datatype));
    w.uint(dim(i64::from(p.width()) / 2)); // half-width
    // Extension scheme byte 0000SSEE, plus explicit extensions for Custom.
    match p.endcap() {
        Endcap::Flat | Endcap::Round => w.byte(0b0000_0101), // flush/flush (round -> flush)
        Endcap::Square => w.byte(0b0000_1010),               // half-width/half-width
        Endcap::Custom(ext) => {
            w.byte(0b0000_1111); // explicit/explicit
            w.sint(i64::from(ext));
            w.sint(i64::from(ext));
        }
    }
    w.uint(4); // point-list type 4
    w.uint((pts.len() - 1) as u64);
    for pair in pts.windows(2) {
        w.g_delta(pair[1].x, pair[1].y, pair[0].x, pair[0].y);
    }
    w.sint(i64::from(pts[0].x));
    w.sint(i64::from(pts[0].y));
}

/// TEXT (id 19): info `0CNXYRTL`, inline text string (N = 0), textlayer/texttype from
/// the label's layer. The label's anchor has no OASIS counterpart and is dropped.
fn write_text(w: &mut Writer, label: &Label) {
    // Info: 0 C1 N0 X1 Y1 R0 T1 L1 = 0b0101_1011.
    w.byte(REC_TEXT);
    w.byte(0b0101_1011);
    w.a_string(&label.text);
    w.uint(u64::from(label.layer.layer)); // textlayer
    w.uint(u64::from(label.layer.datatype)); // texttype
    w.sint(i64::from(label.position.x));
    w.sint(i64::from(label.position.y));
}

/// PLACEMENT (id 18): info `CNXYRMAF`, cell by reference number, magnification and
/// angle present so any [`Transform`] round-trips.
fn write_placement(w: &mut Writer, cell_index: usize, t: &Transform) {
    let mirror = t.orientation.is_mirrored();
    // Info: C1 N1 X1 Y1 R0 M1 A1 F = 0b1111_0110 | mirror.
    w.byte(REC_PLACEMENT_TRANSFORM);
    w.byte(0b1111_0110 | u8::from(mirror));
    w.uint(cell_index as u64);
    let mag = f64::from(t.magnification.numerator()) / f64::from(t.magnification.denominator());
    w.real_double(mag);
    w.real_double(orientation_angle_degrees(t.orientation));
    w.sint(i64::from(t.translation.x));
    w.sint(i64::from(t.translation.y));
}

/// Expands an array into `columns * rows` individual placements at the array pitch,
/// each sharing the array's orientation and magnification.
fn write_array_expanded(w: &mut Writer, cell_index: usize, arr: &ArrayInstance) {
    let base = arr.transform.translation;
    for row in 0..arr.rows {
        for col in 0..arr.columns {
            let dx = i64::from(base.x) + i64::from(col) * i64::from(arr.column_pitch);
            let dy = i64::from(base.y) + i64::from(row) * i64::from(arr.row_pitch);
            let t = Transform {
                translation: Point::new(clamp_dbu(dx), clamp_dbu(dy)),
                orientation: arr.transform.orientation,
                magnification: arr.transform.magnification,
            };
            write_placement(w, cell_index, &t);
        }
    }
}

/// The counter-clockwise rotation of an [`Orientation`], in degrees; the reflection is
/// carried separately by the placement's flip flag.
fn orientation_angle_degrees(o: Orientation) -> f64 {
    match o {
        Orientation::R0 | Orientation::MirrorX => 0.0,
        Orientation::R90 | Orientation::MirrorX90 => 90.0,
        Orientation::R180 | Orientation::MirrorX180 => 180.0,
        Orientation::R270 | Orientation::MirrorX270 => 270.0,
    }
}

/// A non-negative dimension as an unsigned value, saturating a negative (degenerate)
/// input to zero so the writer never panics on malformed geometry.
fn dim(v: i64) -> u64 {
    v.max(0) as u64
}

/// Clamps a widened coordinate back into the DBU range.
fn clamp_dbu(v: i64) -> Dbu {
    v.clamp(i64::from(Dbu::MIN), i64::from(Dbu::MAX)) as Dbu
}

/// The OASIS byte writer: variable-length integers, strings, reals, deltas, and the
/// framing records.
struct Writer {
    out: Vec<u8>,
}

impl Writer {
    fn new() -> Self {
        Self { out: Vec::new() }
    }

    fn into_bytes(self) -> Vec<u8> {
        self.out
    }

    fn byte(&mut self, b: u8) {
        self.out.push(b);
    }

    /// The 13-byte OASIS magic string `%SEMI-OASIS\r\n`.
    fn magic(&mut self) {
        self.out.extend_from_slice(b"%SEMI-OASIS\r\n");
    }

    /// An unsigned integer: 7 bits per byte, little-endian, high bit = continuation.
    fn uint(&mut self, mut v: u64) {
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            self.out.push(b);
            if v == 0 {
                break;
            }
        }
    }

    /// A signed integer: magnitude shifted left one, sign in the low bit (1 = negative).
    fn sint(&mut self, v: i64) {
        let mag = v.unsigned_abs();
        let sign = u64::from(v < 0);
        self.uint((mag << 1) | sign);
    }

    /// A length-prefixed string (used for a-strings and n-strings alike; the two differ
    /// only in the character set, which Reticle's names and labels already satisfy).
    fn a_string(&mut self, s: &str) {
        self.uint(s.len() as u64);
        self.out.extend_from_slice(s.as_bytes());
    }

    /// An n-string (a name). Same wire form as [`a_string`](Self::a_string).
    fn n_string(&mut self, s: &str) {
        self.a_string(s);
    }

    /// A real that is a positive whole number: real type 0 followed by the value.
    fn real_whole(&mut self, v: u64) {
        self.uint(0); // real type 0 = positive whole number
        self.uint(v);
    }

    /// A real as an IEEE-754 double: real type 7 followed by 8 little-endian bytes.
    fn real_double(&mut self, v: f64) {
        self.uint(7);
        self.out.extend_from_slice(&v.to_bits().to_le_bytes());
    }

    /// A g-delta (point-list type 4 element) from `prev` to `cur`, general form 2:
    /// two unsigned integers carrying the x and y components with direction bits.
    fn g_delta(&mut self, cur_x: Dbu, cur_y: Dbu, prev_x: Dbu, prev_y: Dbu) {
        let dx = i64::from(cur_x) - i64::from(prev_x);
        let dy = i64::from(cur_y) - i64::from(prev_y);
        // First integer: bit0 = 1 (general form), bit1 = x sign (0=east/+), rest = |dx|.
        let x_word = (dx.unsigned_abs() << 2) | (u64::from(dx < 0) << 1) | 1;
        // Second integer: bit0 = y sign (0=north/+), rest = |dy|.
        let y_word = (dy.unsigned_abs() << 1) | u64::from(dy < 0);
        self.uint(x_word);
        self.uint(y_word);
    }

    /// The END record, padded so the whole record is exactly 256 bytes, with no
    /// validation (scheme 0). Layout: `02 · b-string(252 NULs) · 00`.
    fn end_record(&mut self) {
        self.byte(REC_END);
        self.uint(252); // padding b-string length
        self.out.extend(std::iter::repeat_n(0u8, 252));
        self.byte(0); // validation-scheme = 0 (no validation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::{LayerId, Path, Point, Polygon, Rect};
    use reticle_model::{DrawShape, Instance, Technology};

    fn doc_with(cell: Cell) -> Document {
        let mut doc = Document::new();
        doc.set_technology(Technology {
            dbu_per_micron: 1000,
            ..Technology::default()
        });
        doc.insert_cell(cell);
        doc
    }

    #[test]
    fn starts_with_magic_and_ends_with_256_byte_end() {
        let mut cell = Cell::new("TOP");
        cell.shapes.push(DrawShape::new(
            LayerId::new(1, 0),
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(100, 200))),
        ));
        let bytes = OasisStd.export(&doc_with(cell)).unwrap();
        assert_eq!(&bytes[..13], b"%SEMI-OASIS\r\n");
        // The END record is the last 256 bytes: id 2, length-252 b-string, 252 NULs, 0.
        let end = &bytes[bytes.len() - 256..];
        assert_eq!(end[0], REC_END);
        assert_eq!(end[255], 0);
        assert!(end[3..255].iter().all(|&b| b == 0), "padding is NUL");
    }

    #[test]
    fn rectangle_encodes_like_the_spec_example() {
        // Spec §25 example: layer 1, datatype 0, LL (0,0), width 100, height 200.
        let mut cell = Cell::new("R");
        cell.shapes.push(DrawShape::new(
            LayerId::new(1, 0),
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(100, 200))),
        ));
        let bytes = OasisStd.export(&doc_with(cell)).unwrap();
        // Find the RECTANGLE record and check the info byte + fields.
        let pos = bytes.iter().position(|&b| b == REC_RECTANGLE).unwrap();
        assert_eq!(bytes[pos + 1], 0b0111_1011, "info SWHXYRDL");
        assert_eq!(bytes[pos + 2], 1, "layer");
        assert_eq!(bytes[pos + 3], 0, "datatype");
        assert_eq!(bytes[pos + 4], 100, "width");
        assert_eq!(&bytes[pos + 5..pos + 7], &[0xC8, 0x01], "height 200 uint");
        assert_eq!(bytes[pos + 7], 0, "x = +0 signed");
        assert_eq!(bytes[pos + 8], 0, "y = +0 signed");
    }

    #[test]
    fn uint_and_sint_match_spec_vectors() {
        let mut w = Writer::new();
        w.uint(128);
        w.uint(16384);
        assert_eq!(w.out, vec![0x80, 0x01, 0x80, 0x80, 0x01]);
        let mut w = Writer::new();
        w.sint(1);
        w.sint(-1);
        w.sint(-64);
        assert_eq!(w.out, vec![0x02, 0x03, 0x81, 0x01]);
    }

    #[test]
    fn polygon_path_text_placement_emit_records() {
        let mut sub = Cell::new("SUB");
        sub.shapes.push(DrawShape::new(
            LayerId::new(2, 0),
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(10, 10))),
        ));
        let mut top = Cell::new("TOP");
        top.shapes.push(DrawShape::new(
            LayerId::new(3, 0),
            ShapeKind::Polygon(Polygon::new(vec![
                Point::new(0, 0),
                Point::new(10, 0),
                Point::new(10, 10),
            ])),
        ));
        top.shapes.push(DrawShape::new(
            LayerId::new(4, 0),
            ShapeKind::Path(Path::new(
                vec![Point::new(0, 0), Point::new(100, 0)],
                20,
                Endcap::Flat,
            )),
        ));
        top.labels.push(Label {
            text: "PIN".to_string(),
            position: Point::new(5, 5),
            layer: LayerId::new(5, 0),
            anchor: reticle_model::Anchor::Center,
        });
        top.instances.push(Instance {
            cell: "SUB".to_string(),
            transform: Transform::IDENTITY,
        });

        let mut doc = Document::new();
        doc.set_technology(Technology {
            dbu_per_micron: 1000,
            ..Technology::default()
        });
        doc.insert_cell(sub);
        doc.insert_cell(top);
        let bytes = OasisStd.export(&doc).unwrap();

        for id in [
            REC_POLYGON,
            REC_PATH,
            REC_TEXT,
            REC_PLACEMENT_TRANSFORM,
            REC_CELLNAME,
        ] {
            assert!(bytes.contains(&id), "record {id} present");
        }
    }
}
