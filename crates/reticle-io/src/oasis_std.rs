//! A conformant **OASIS** (SEMI P39) writer for a practical subset - the real thing
//! `KLayout` can read, distinct from the in-house [`Oasis`](crate::Oasis) container.
//!
//! # Scope and honesty
//!
//! [`Oasis`](crate::Oasis) is an in-house, OASIS-*inspired* binary container (ADR 0004)
//! that `KLayout` cannot read. This module is the opposite: a genuine SEMI P39 OASIS
//! *writer* whose output `KLayout` reads as OASIS, paired with a matching *reader*
//! ([`OasisStd`]'s [`Importer`] impl) that parses the writer's own record subset back
//! into a [`Document`]. The writer subset is:
//!
//! * uncompressed only - no `CBLOCK` (zlib) blocks;
//! * `RECTANGLE`, `POLYGON`, `PATH`, `PLACEMENT`, and `TEXT` records;
//! * explicit modal state - every element carries its own layer, datatype, and
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
//!
//! # Reading it back
//!
//! [`OasisStd`] also implements [`Importer`]: it reads the exact record subset the
//! writer emits (`START`, `CELLNAME`, `CELL`, `RECTANGLE`, `POLYGON`, `PATH`,
//! `PLACEMENT` types 17 and 18, `TEXT`, `END`) back into a [`Document`], so a document
//! survives a write-then-read round trip. The reader is hardened for untrusted input in
//! the same spirit as the GDSII importer: the input size, every string length, and every
//! point-list vertex count are capped, and malformed bytes (a bad magic string, a
//! truncated record, an unknown record id, an over-cap count) return a
//! [`reticle_model::ModelError`] rather than panicking or allocating without bound.
//! Third-party OASIS features outside the writer's subset are not decoded: `CBLOCK`
//! compression, point-list types other than 4, and repetition records return an
//! unsupported-format error. See the reader source for the honest coverage ledger.

use crate::error::IoError;
use reticle_geometry::{
    Dbu, Endcap, LayerId, Magnification, Orientation, Path, Point, Polygon, Rect, Transform,
};
use reticle_model::{
    Anchor, ArrayInstance, Cell, Document, DrawShape, Exporter, Importer, Instance, Label, Result,
    ShapeKind, Technology,
};

/// The conformant-OASIS exporter (a practical writer subset; see the [module docs](self)).
///
/// Implements [`Exporter`], so it plugs into the same trait the CLI and app use for
/// every format. It never reads OASIS - export only.
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
/// x, y - never relying on inherited modal state.
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

// ======================================================================================
// Reader: parse the writer's own OASIS record subset back into a `Document`.
//
// Coverage ledger (honest gaps for third-party OASIS files):
// * CBLOCK (id 34, zlib-compressed blocks): not decoded. Adding it needs a DEFLATE
//   dependency, which is outside this reader's boundary; a CBLOCK record errors.
// * Repetition records (the `R` info bit on any element): not expanded. The writer
//   never sets it; an element that does returns an unsupported-format error.
// * Point-list types other than 4 (Manhattan/octangular short forms): not decoded.
// * Modal inheritance IS honored for the common fields (layer, datatype, geometry and
//   text coordinates, path half-width, placement cell/coordinates): an element that
//   omits a field inherits the last value. The single-integer octangular g-delta form
//   is decoded even though the writer only emits the general two-integer form.
// ======================================================================================

/// Additional cell record: a cell named inline by an n-string (the writer instead
/// references the cellname table by number with [`REC_CELL_REF`]).
const REC_CELL_NAME: u8 = 14;
/// Additional placement record: a placement whose angle is a 2-bit `0/90/180/270`
/// field and whose magnification is always unity (the writer emits the richer
/// [`REC_PLACEMENT_TRANSFORM`] form).
const REC_PLACEMENT: u8 = 17;

/// The largest OASIS input this reader will attempt, in bytes (256 MiB). A hostile
/// length past this is refused before any allocation, matching the GDSII importer.
const MAX_INPUT_BYTES: usize = 256 * 1024 * 1024;
/// The largest vertex count a single polygon or path point-list may carry into the
/// model. A conformant point-list is far smaller; this is a defense-in-depth ceiling so
/// a crafted count can never force an unbounded allocation.
const MAX_SHAPE_VERTICES: usize = 200_000;
/// The largest byte length accepted for one name or label string (1 MiB). Also bounded
/// by the remaining input, so a truncated length can never over-allocate.
const MAX_STRING_LEN: usize = 1 << 20;
/// A defense-in-depth ceiling on the number of `CELLNAME` records (the stream length
/// already bounds the table).
const MAX_CELL_NAMES: usize = 16 * 1024 * 1024;

impl Importer for OasisStd {
    fn import(&self, bytes: &[u8]) -> Result<Document> {
        read_document(bytes)
    }
}

/// Parses a conformant-OASIS byte stream produced by [`OasisStd`]'s writer back into a
/// [`Document`].
///
/// Returns a [`reticle_model::ModelError`] (lowered from an [`IoError`]) for any
/// malformed or out-of-subset input; it never panics and never allocates past the caps
/// above.
fn read_document(bytes: &[u8]) -> Result<Document> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(
            IoError::Malformed("OASIS input exceeds the maximum accepted size (256 MiB)").into(),
        );
    }
    let mut r = Reader::new(bytes);
    r.expect_magic()?;
    let dbu_per_micron = r.read_start()?;

    let mut cellnames: Vec<String> = Vec::new();
    let mut cells: Vec<Cell> = Vec::new();
    let mut current: Option<Cell> = None;
    let mut modal = Modal::default();

    while r.remaining() > 0 {
        let id = r.read_byte()?;
        match id {
            REC_END => break,
            REC_CELLNAME => {
                if cellnames.len() >= MAX_CELL_NAMES {
                    return Err(
                        IoError::Malformed("OASIS cellname table exceeds the reader cap").into(),
                    );
                }
                cellnames.push(r.read_string()?);
            }
            REC_CELL_REF => {
                flush_cell(&mut cells, &mut current);
                let idx = r.read_uint()?;
                current = Some(Cell::new(cellname_at(&cellnames, idx)?));
                modal = Modal::default();
            }
            REC_CELL_NAME => {
                flush_cell(&mut cells, &mut current);
                current = Some(Cell::new(r.read_string()?));
                modal = Modal::default();
            }
            REC_RECTANGLE => {
                let shape = r.read_rectangle(&mut modal)?;
                cell_mut(&mut current)?.shapes.push(shape);
            }
            REC_POLYGON => {
                let shape = r.read_polygon(&mut modal)?;
                cell_mut(&mut current)?.shapes.push(shape);
            }
            REC_PATH => {
                let shape = r.read_path(&mut modal)?;
                cell_mut(&mut current)?.shapes.push(shape);
            }
            REC_TEXT => {
                let label = r.read_text(&mut modal)?;
                cell_mut(&mut current)?.labels.push(label);
            }
            REC_PLACEMENT => {
                let inst = r.read_placement(&mut modal, &cellnames, false)?;
                cell_mut(&mut current)?.instances.push(inst);
            }
            REC_PLACEMENT_TRANSFORM => {
                let inst = r.read_placement(&mut modal, &cellnames, true)?;
                cell_mut(&mut current)?.instances.push(inst);
            }
            REC_START => {
                return Err(IoError::Malformed("unexpected second START record").into());
            }
            _ => {
                return Err(
                    IoError::Unsupported("OASIS record id not in the reader subset").into(),
                );
            }
        }
    }
    flush_cell(&mut cells, &mut current);

    let mut doc = Document::new();
    doc.set_technology(Technology {
        dbu_per_micron,
        ..Technology::default()
    });
    for cell in cells {
        doc.insert_cell(cell);
    }
    Ok(doc)
}

/// Moves the in-progress cell, if any, into the finished list.
fn flush_cell(cells: &mut Vec<Cell>, current: &mut Option<Cell>) {
    if let Some(cell) = current.take() {
        cells.push(cell);
    }
}

/// The current cell, or a malformed error when an element appears before any `CELL`.
fn cell_mut(current: &mut Option<Cell>) -> Result<&mut Cell> {
    current
        .as_mut()
        .ok_or_else(|| IoError::Malformed("OASIS element before any CELL record").into())
}

/// The cell name for reference number `idx`, or a malformed error if it is out of range.
fn cellname_at(cellnames: &[String], idx: u64) -> Result<String> {
    usize::try_from(idx)
        .ok()
        .and_then(|i| cellnames.get(i))
        .cloned()
        .ok_or_else(|| {
            IoError::Malformed("OASIS cell reference number past the cellname table").into()
        })
}

/// The error returned when an element carries a repetition (the `R` info bit), which the
/// reader does not expand. The writer never sets it.
fn repetition_unsupported() -> reticle_model::ModelError {
    IoError::Unsupported("OASIS repetition records are not decoded by the reader").into()
}

/// Whether bit `n` (0 = least significant) of an info byte is set.
fn bit(info: u8, n: u8) -> bool {
    (info >> n) & 1 == 1
}

/// A non-negative dimension widened to `i64`, saturating an out-of-range value.
fn dim_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// Builds a [`LayerId`] from modal layer/datatype numbers, narrowing to the model's
/// 16-bit fields (matching the GDSII importer's behavior).
fn layer_id(layer: u64, datatype: u64) -> LayerId {
    LayerId::new(layer as u16, datatype as u16)
}

/// Accumulates a first vertex and a sequence of deltas into an absolute point ring,
/// clamping each coordinate into the DBU range so a hostile delta cannot overflow.
fn accumulate_ring(x0: i64, y0: i64, deltas: &[(i64, i64)]) -> Vec<Point> {
    let mut points = Vec::with_capacity(deltas.len() + 1);
    let (mut x, mut y) = (x0, y0);
    points.push(Point::new(clamp_dbu(x), clamp_dbu(y)));
    for &(dx, dy) in deltas {
        x = x.saturating_add(dx);
        y = y.saturating_add(dy);
        points.push(Point::new(clamp_dbu(x), clamp_dbu(y)));
    }
    points
}

/// The extension length (in DBU) implied by a two-bit `PATH` extension scheme.
fn extension_value(scheme: u8, half_width: i64, explicit: Option<i64>) -> i64 {
    match scheme {
        2 => half_width,            // half-width extension
        3 => explicit.unwrap_or(0), // explicit signed extension
        _ => 0,                     // flush (1) or reserved (0)
    }
}

/// Classifies a pair of start/end path extensions into a Reticle [`Endcap`]. A round cap
/// has no OASIS counterpart, so the writer flushes it and the reader reads flush back as
/// [`Endcap::Flat`].
fn endcap_from_extensions(start: i64, end: i64, half_width: i64) -> Endcap {
    if start == 0 && end == 0 {
        Endcap::Flat
    } else if start == half_width && end == half_width {
        Endcap::Square
    } else {
        Endcap::Custom(clamp_dbu(end))
    }
}

/// A signed value from a magnitude and a sign flag, erroring if the magnitude overflows
/// `i64`.
fn signed_from_magnitude(magnitude: u64, negative: bool) -> Result<i64> {
    let value = i64::try_from(magnitude)
        .map_err(|_| IoError::Malformed("OASIS delta magnitude overflows i64"))?;
    Ok(if negative { -value } else { value })
}

/// The `(dx, dy)` of a single-integer octangular g-delta for one of the eight compass
/// directions scaled by `magnitude`.
fn octangular_delta(direction: u64, magnitude: i64) -> (i64, i64) {
    match direction & 0b111 {
        0 => (magnitude, 0),
        1 => (0, magnitude),
        2 => (-magnitude, 0),
        3 => (0, -magnitude),
        4 => (magnitude, magnitude),
        5 => (-magnitude, magnitude),
        6 => (-magnitude, -magnitude),
        _ => (magnitude, -magnitude),
    }
}

/// Reconstructs a [`Magnification`] from the writer's floating-point factor. Unity and
/// small exact ratios round-trip; a non-finite or non-positive factor falls back to
/// unity.
fn magnification_from(factor: f64) -> Magnification {
    const SCALE: i64 = 1_000_000;
    if !factor.is_finite() || factor <= 0.0 || (factor - 1.0).abs() < f64::EPSILON {
        return Magnification::UNITY;
    }
    let numerator = (factor * SCALE as f64).round();
    if !(1.0..=i64::MAX as f64).contains(&numerator) {
        return Magnification::UNITY;
    }
    let numerator = numerator as i64;
    let divisor = gcd(numerator, SCALE);
    match (
        u32::try_from(numerator / divisor),
        u32::try_from(SCALE / divisor),
    ) {
        (Ok(num), Ok(den)) => Magnification::new(num, den).unwrap_or(Magnification::UNITY),
        _ => Magnification::UNITY,
    }
}

/// The greatest common divisor of two positive integers (Euclid), never returning zero.
fn gcd(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

/// Reconstructs an [`Orientation`] from a rotation angle in degrees and a mirror flag,
/// inverting the writer's angle/flip split.
fn orientation_from(angle: f64, mirror: bool) -> Orientation {
    let quarter = if angle.is_finite() {
        (angle / 90.0).round().rem_euclid(4.0) as u32
    } else {
        0
    };
    match (quarter, mirror) {
        (1, false) => Orientation::R90,
        (2, false) => Orientation::R180,
        (3, false) => Orientation::R270,
        (0, true) => Orientation::MirrorX,
        (1, true) => Orientation::MirrorX90,
        (2, true) => Orientation::MirrorX180,
        (3, true) => Orientation::MirrorX270,
        _ => Orientation::R0,
    }
}

/// The OASIS modal variables the reader tracks. Every field defaults to zero (or an
/// empty text / absent cell), and each element updates the fields its info byte marks
/// present, so an element that omits a field inherits the last value. The writer always
/// marks every field present, so a round trip never depends on inheritance; the state
/// exists so a third-party file that does use modal inheritance for these common fields
/// still reads.
#[derive(Debug, Default)]
struct Modal {
    layer: u64,
    datatype: u64,
    geom_w: u64,
    geom_h: u64,
    geom_x: i64,
    geom_y: i64,
    text: String,
    text_layer: u64,
    text_type: u64,
    text_x: i64,
    text_y: i64,
    path_half_width: u64,
    place_x: i64,
    place_y: i64,
    place_cell: Option<u64>,
}

/// A bounds-checked cursor over an OASIS byte stream. Every read is guarded against the
/// end of input, so a truncated or hostile stream yields an [`IoError`] instead of a
/// panic.
#[derive(Debug)]
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Bytes not yet consumed.
    fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// Consumes `n` bytes, or errors if the stream is too short.
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(
                IoError::Malformed("truncated OASIS record (read past end of input)").into(),
            );
        }
        let out = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    /// Reads one byte.
    fn read_byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    /// Verifies and consumes the 13-byte OASIS magic string.
    fn expect_magic(&mut self) -> Result<()> {
        const MAGIC: &[u8] = b"%SEMI-OASIS\r\n";
        if self.take(MAGIC.len())? == MAGIC {
            Ok(())
        } else {
            Err(IoError::Malformed("not an OASIS stream (bad magic string)").into())
        }
    }

    /// Reads the `START` record and returns the database resolution (DBU per micron),
    /// clamped to at least one.
    fn read_start(&mut self) -> Result<i64> {
        if self.read_byte()? != REC_START {
            return Err(IoError::Malformed("OASIS stream does not begin with START").into());
        }
        let _version = self.read_string()?;
        let unit = self.read_real()?;
        let offset_flag = self.read_uint()?;
        if offset_flag == 0 {
            // Six (flag, offset) table pairs are stored inline; the writer zeroes them.
            for _ in 0..12 {
                let _ = self.read_uint()?;
            }
        }
        let dbu = if unit.is_finite() && unit >= 1.0 {
            unit.round() as i64
        } else {
            1000
        };
        Ok(dbu)
    }

    /// Reads an OASIS unsigned integer (7 bits/byte, little-endian, high bit = continue).
    fn read_uint(&mut self) -> Result<u64> {
        let mut value: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = self.read_byte()?;
            let payload = u64::from(byte & 0x7f);
            if shift >= 64 || (shift == 63 && payload > 1) {
                return Err(IoError::Malformed("OASIS unsigned integer overflows 64 bits").into());
            }
            value |= payload << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
        }
    }

    /// Reads an OASIS signed integer (magnitude shifted left one, sign in the low bit).
    fn read_sint(&mut self) -> Result<i64> {
        let raw = self.read_uint()?;
        signed_from_magnitude(raw >> 1, raw & 1 == 1)
    }

    /// Reads an unsigned integer that must be non-zero (a real-number denominator).
    fn read_nonzero_uint(&mut self) -> Result<u64> {
        match self.read_uint()? {
            0 => Err(IoError::Malformed("OASIS rational real has a zero denominator").into()),
            value => Ok(value),
        }
    }

    /// Reads a length-prefixed OASIS string (a-string or n-string) as UTF-8.
    fn read_string(&mut self) -> Result<String> {
        let len = usize::try_from(self.read_uint()?)
            .ok()
            .filter(|&n| n <= MAX_STRING_LEN)
            .ok_or(IoError::Malformed(
                "OASIS string length exceeds the reader cap",
            ))?;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| IoError::Malformed("OASIS string is not valid UTF-8").into())
    }

    /// Reads an OASIS real number (types 0-7) as an `f64`.
    fn read_real(&mut self) -> Result<f64> {
        let real = match self.read_uint()? {
            0 => self.read_uint()? as f64,
            1 => -(self.read_uint()? as f64),
            2 => 1.0 / self.read_nonzero_uint()? as f64,
            3 => -1.0 / self.read_nonzero_uint()? as f64,
            4 => self.read_uint()? as f64 / self.read_nonzero_uint()? as f64,
            5 => -(self.read_uint()? as f64) / self.read_nonzero_uint()? as f64,
            6 => f64::from(f32::from_le_bytes(
                self.take(4)?.try_into().unwrap_or([0; 4]),
            )),
            7 => f64::from_le_bytes(self.take(8)?.try_into().unwrap_or([0; 8])),
            _ => return Err(IoError::Malformed("OASIS real number has an unknown type").into()),
        };
        Ok(real)
    }

    /// Reads a count-valued unsigned integer, capping it at `ceiling` and at the number
    /// of bytes left (each counted item consumes at least one byte), so a hostile count
    /// can never drive an unbounded allocation.
    fn read_count(&mut self, ceiling: usize) -> Result<usize> {
        let count = usize::try_from(self.read_uint()?).unwrap_or(usize::MAX);
        if count > ceiling || count > self.remaining() {
            return Err(IoError::Malformed(
                "OASIS count exceeds the reader cap or remaining input",
            )
            .into());
        }
        Ok(count)
    }

    /// Reads one point-list g-delta (type-4 element): a general two-integer delta or a
    /// single-integer octangular delta. Returns the `(dx, dy)` displacement.
    fn read_g_delta(&mut self) -> Result<(i64, i64)> {
        let first = self.read_uint()?;
        if first & 1 == 1 {
            let dx = signed_from_magnitude(first >> 2, (first >> 1) & 1 == 1)?;
            let second = self.read_uint()?;
            let dy = signed_from_magnitude(second >> 1, second & 1 == 1)?;
            Ok((dx, dy))
        } else {
            let magnitude = signed_from_magnitude(first >> 4, false)?;
            Ok(octangular_delta(first >> 1, magnitude))
        }
    }

    /// Reads a point-list: a type byte, a vertex count, then that many g-deltas. Only
    /// type 4 (explicit all-angle g-deltas) is decoded; other types are unsupported.
    fn read_point_list(&mut self) -> Result<Vec<(i64, i64)>> {
        if self.read_uint()? != 4 {
            return Err(
                IoError::Unsupported("OASIS point-list type other than 4 is not decoded").into(),
            );
        }
        let count = self.read_count(MAX_SHAPE_VERTICES)?;
        let mut deltas = Vec::with_capacity(count);
        for _ in 0..count {
            deltas.push(self.read_g_delta()?);
        }
        Ok(deltas)
    }

    /// Reads a `RECTANGLE` (info `SWHXYRDL`) into a [`DrawShape`], updating modal state.
    fn read_rectangle(&mut self, m: &mut Modal) -> Result<DrawShape> {
        let info = self.read_byte()?;
        if bit(info, 0) {
            m.layer = self.read_uint()?;
        }
        if bit(info, 1) {
            m.datatype = self.read_uint()?;
        }
        if bit(info, 6) {
            m.geom_w = self.read_uint()?;
        }
        if bit(info, 7) {
            m.geom_h = m.geom_w; // square: one dimension serves for both
        } else if bit(info, 5) {
            m.geom_h = self.read_uint()?;
        }
        if bit(info, 4) {
            m.geom_x = self.read_sint()?;
        }
        if bit(info, 3) {
            m.geom_y = self.read_sint()?;
        }
        if bit(info, 2) {
            return Err(repetition_unsupported());
        }
        let min = Point::new(clamp_dbu(m.geom_x), clamp_dbu(m.geom_y));
        let max = Point::new(
            clamp_dbu(m.geom_x.saturating_add(dim_i64(m.geom_w))),
            clamp_dbu(m.geom_y.saturating_add(dim_i64(m.geom_h))),
        );
        Ok(DrawShape::new(
            layer_id(m.layer, m.datatype),
            ShapeKind::Rect(Rect::new(min, max)),
        ))
    }

    /// Reads a `POLYGON` (info `00PXYRDL`) into a [`DrawShape`].
    fn read_polygon(&mut self, m: &mut Modal) -> Result<DrawShape> {
        let info = self.read_byte()?;
        if bit(info, 0) {
            m.layer = self.read_uint()?;
        }
        if bit(info, 1) {
            m.datatype = self.read_uint()?;
        }
        let deltas = if bit(info, 5) {
            self.read_point_list()?
        } else {
            return Err(IoError::Unsupported(
                "OASIS polygon without an explicit point-list is not decoded",
            )
            .into());
        };
        if bit(info, 4) {
            m.geom_x = self.read_sint()?;
        }
        if bit(info, 3) {
            m.geom_y = self.read_sint()?;
        }
        if bit(info, 2) {
            return Err(repetition_unsupported());
        }
        let vertices = accumulate_ring(m.geom_x, m.geom_y, &deltas);
        Ok(DrawShape::new(
            layer_id(m.layer, m.datatype),
            ShapeKind::Polygon(Polygon::new(vertices)),
        ))
    }

    /// Reads a `PATH` (info `EWPXYRDL`) into a [`DrawShape`].
    fn read_path(&mut self, m: &mut Modal) -> Result<DrawShape> {
        let info = self.read_byte()?;
        if bit(info, 0) {
            m.layer = self.read_uint()?;
        }
        if bit(info, 1) {
            m.datatype = self.read_uint()?;
        }
        if bit(info, 6) {
            m.path_half_width = self.read_uint()?;
        }
        let endcap = if bit(info, 7) {
            self.read_path_extension(m.path_half_width)?
        } else {
            Endcap::Flat
        };
        let deltas = if bit(info, 5) {
            self.read_point_list()?
        } else {
            return Err(IoError::Unsupported(
                "OASIS path without an explicit point-list is not decoded",
            )
            .into());
        };
        if bit(info, 4) {
            m.geom_x = self.read_sint()?;
        }
        if bit(info, 3) {
            m.geom_y = self.read_sint()?;
        }
        if bit(info, 2) {
            return Err(repetition_unsupported());
        }
        let points = accumulate_ring(m.geom_x, m.geom_y, &deltas);
        let width = clamp_dbu(dim_i64(m.path_half_width).saturating_mul(2));
        Ok(DrawShape::new(
            layer_id(m.layer, m.datatype),
            ShapeKind::Path(Path::new(points, width, endcap)),
        ))
    }

    /// Reads a `PATH` extension scheme byte (`0000SSEE`) plus any explicit extensions,
    /// mapping the start/end extensions back to a Reticle [`Endcap`].
    fn read_path_extension(&mut self, half_width: u64) -> Result<Endcap> {
        let scheme = self.read_byte()?;
        let start_scheme = (scheme >> 2) & 0b11;
        let end_scheme = scheme & 0b11;
        let start_explicit = if start_scheme == 3 {
            Some(self.read_sint()?)
        } else {
            None
        };
        let end_explicit = if end_scheme == 3 {
            Some(self.read_sint()?)
        } else {
            None
        };
        let half = dim_i64(half_width);
        let start = extension_value(start_scheme, half, start_explicit);
        let end = extension_value(end_scheme, half, end_explicit);
        Ok(endcap_from_extensions(start, end, half))
    }

    /// Reads a `TEXT` (info `0CNXYRTL`) into a [`Label`].
    fn read_text(&mut self, m: &mut Modal) -> Result<Label> {
        let info = self.read_byte()?;
        if bit(info, 6) {
            if bit(info, 5) {
                // Referenced textstring by number: the writer emits text inline, so the
                // textstring table is absent here; record an empty label text.
                let _reference = self.read_uint()?;
                m.text = String::new();
            } else {
                m.text = self.read_string()?;
            }
        }
        if bit(info, 0) {
            m.text_layer = self.read_uint()?;
        }
        if bit(info, 1) {
            m.text_type = self.read_uint()?;
        }
        if bit(info, 4) {
            m.text_x = self.read_sint()?;
        }
        if bit(info, 3) {
            m.text_y = self.read_sint()?;
        }
        if bit(info, 2) {
            return Err(repetition_unsupported());
        }
        Ok(Label {
            text: m.text.clone(),
            position: Point::new(clamp_dbu(m.text_x), clamp_dbu(m.text_y)),
            layer: layer_id(m.text_layer, m.text_type),
            anchor: Anchor::Center,
        })
    }

    /// Reads a `PLACEMENT`: type 17 (`has_magnification` false, angle is a 2-bit field)
    /// or type 18 (`has_magnification` true, magnification and angle are reals). Both
    /// share the info layout `CNXYR..F`.
    fn read_placement(
        &mut self,
        m: &mut Modal,
        cellnames: &[String],
        has_magnification: bool,
    ) -> Result<Instance> {
        let info = self.read_byte()?;
        let mirror = bit(info, 0);
        let name = self.read_placement_cell(m, cellnames, info)?;
        let (magnification, angle) = if has_magnification {
            let mag = if bit(info, 2) { self.read_real()? } else { 1.0 };
            let angle = if bit(info, 1) { self.read_real()? } else { 0.0 };
            (magnification_from(mag), angle)
        } else {
            // Type 17: bits [2:1] hold the angle as a multiple of 90 degrees.
            (Magnification::UNITY, f64::from((info >> 1) & 0b11) * 90.0)
        };
        if bit(info, 5) {
            m.place_x = self.read_sint()?;
        }
        if bit(info, 4) {
            m.place_y = self.read_sint()?;
        }
        if bit(info, 3) {
            return Err(repetition_unsupported());
        }
        Ok(Instance {
            cell: name,
            transform: Transform {
                translation: Point::new(clamp_dbu(m.place_x), clamp_dbu(m.place_y)),
                orientation: orientation_from(angle, mirror),
                magnification,
            },
        })
    }

    /// Resolves the placed cell name from a `PLACEMENT` info byte: by reference number
    /// (`N` set), by inline name (`C` set, `N` clear), or from the modal cell (`C` clear).
    fn read_placement_cell(
        &mut self,
        m: &mut Modal,
        cellnames: &[String],
        info: u8,
    ) -> Result<String> {
        if bit(info, 7) {
            if bit(info, 6) {
                let idx = self.read_uint()?;
                m.place_cell = Some(idx);
                cellname_at(cellnames, idx)
            } else {
                self.read_string()
            }
        } else {
            match m.place_cell {
                Some(idx) => cellname_at(cellnames, idx),
                None => Err(IoError::Malformed("OASIS placement has no cell reference").into()),
            }
        }
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
