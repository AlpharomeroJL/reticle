//! Forward-only, allocation-bounded GDSII record reader (Wave 2 contract, ADR 0062).
//!
//! The DOM importer in [`crate::gds`] (via `gds21`) reads a whole library into memory
//! and is capped at [`crate::gds::MAX_INPUT_BYTES`] (256 MiB). A `.rtla` archive is
//! built from full shuttle dies that are multiple gigabytes, and lane 6C converts a
//! dropped file to an OPFS archive inside a browser worker, so both need a reader that
//! pulls one record at a time over any [`std::io::Read`] without ever holding the
//! whole file. That reader is this module.
//!
//! **Contract state.** The [`GdsEvent`] vocabulary below is frozen and complete: it is
//! the shared surface lane 2A (the reader), lane 2D (the converter), and lane 6C (the
//! in-browser worker) all agree on. The [`GdsRecordReader`] that produces the events
//! is implemented by lane 2A; until then [`GdsRecordReader::next_event`] returns a
//! clear error rather than a partial parse.
//!
//! # Untrusted lengths
//!
//! Every count or length field in a GDSII record is attacker-controlled. The reader
//! must never reserve capacity from a length field beyond what the remaining input can
//! hold (the OASIS OOM lesson, commit 1b1b56b), and must never index past a record's
//! bounds (the gds21 `read_str` zero-length lesson, commit e8752f7). Its fuzz target
//! seeds from the committed GDS crash fixtures so the streaming path cannot
//! reintroduce a fixed panic class.

use std::io::Read;

use crate::IoError;
use reticle_model::Result;

// ---------------------------------------------------------------------------
// GDSII record framing constants (hand-rolled; no `gds21` dependency so this
// path is wasm-clean). A record is `[len: u16 BE][rtype: u8][dtype: u8][payload]`
// where `len` counts the 4-byte header, so the payload is `len - 4` bytes. See
// the mirrored framing walk in `crate::gds::guard_gds21_records`.
// ---------------------------------------------------------------------------

// HEADER (0x00), BGNLIB (0x01), and BGNSTR (0x05) are consumed by the catch-all skip
// arm (the reader never needs to name a record it uniformly ignores), so their codes
// live only in the test module that synthesizes streams.

/// Record type: units (user unit + database unit, two GDSII reals).
const RT_UNITS: u8 = 0x03;
/// Record type: end of library.
const RT_ENDLIB: u8 = 0x04;
/// Record type: structure name.
const RT_STRNAME: u8 = 0x06;
/// Record type: end of structure.
const RT_ENDSTR: u8 = 0x07;
/// Record type: boundary (filled polygon) element start.
const RT_BOUNDARY: u8 = 0x08;
/// Record type: path (wire) element start.
const RT_PATH: u8 = 0x09;
/// Record type: structure reference element start.
const RT_SREF: u8 = 0x0A;
/// Record type: array reference element start.
const RT_AREF: u8 = 0x0B;
/// Record type: text element start.
const RT_TEXT: u8 = 0x0C;
/// Record type: layer number.
const RT_LAYER: u8 = 0x0D;
/// Record type: datatype number (boundary/path).
const RT_DATATYPE: u8 = 0x0E;
/// Record type: path width.
const RT_WIDTH: u8 = 0x0F;
/// Record type: coordinate list.
const RT_XY: u8 = 0x10;
/// Record type: end of element.
const RT_ENDEL: u8 = 0x11;
/// Record type: referenced structure name (SREF/AREF).
const RT_SNAME: u8 = 0x12;
/// Record type: array column/row counts.
const RT_COLROW: u8 = 0x13;
/// Record type: node element start (no drawn fill we model; skipped).
const RT_NODE: u8 = 0x15;
/// Record type: texttype number.
const RT_TEXTTYPE: u8 = 0x16;
/// Record type: text string payload.
const RT_STRING: u8 = 0x19;
/// Record type: box element start (no drawn fill we model; skipped).
const RT_BOX: u8 = 0x2D;

/// GDSII data-type code for an ASCII string payload. A string record with a
/// zero-length payload is the panic class `gds21`'s `read_str` hits by indexing
/// `data[len - 1]`; this reader rejects it up front, exactly as
/// `crate::gds::guard_gds21_records` does, so the streaming path cannot
/// reintroduce it (commit e8752f7).
const DT_STRING: u8 = 0x06;

/// The user unit assumed for GDSII, in metres (one micron). Mirrors
/// [`crate::gds`]'s `USER_UNIT_METERS`.
const USER_UNIT_METERS: f64 = 1e-6;

/// Fallback database resolution when a UNITS record is missing or unusable,
/// matching [`crate::gds`]'s `DEFAULT_DBU_PER_MICRON`.
const DEFAULT_DBU_PER_MICRON: i64 = 1000;

/// One pulled record event from a GDSII stream, in document order. A structure is a
/// [`GdsEvent::BeginStruct`] ... [`GdsEvent::EndStruct`] span; the library is a
/// [`GdsEvent::BeginLibrary`] ... [`GdsEvent::EndLibrary`] span. Geometry, references,
/// and text arrive as their own events between struct boundaries.
///
/// This is a deliberately small, flat vocabulary: it carries exactly what the tiled
/// archive and the model need, not the full GDSII record zoo. Records outside this set
/// (properties, node types, formatting) are consumed and skipped by the reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GdsEvent {
    /// Start of the library. `dbu_per_micron` is recovered from the UNITS record.
    BeginLibrary {
        /// Database units per micron.
        dbu_per_micron: i64,
    },
    /// Start of a structure (cell) with this name.
    BeginStruct {
        /// The structure name.
        name: String,
    },
    /// A boundary (filled polygon or rectangle) on `layer`/`datatype` with these
    /// vertices, in database units. A closing vertex equal to the first may be
    /// present; consumers should not assume it.
    Boundary {
        /// GDSII layer number.
        layer: u16,
        /// GDSII datatype number.
        datatype: u16,
        /// The boundary vertices, in database units.
        xy: Vec<(i32, i32)>,
    },
    /// A path (wire) on `layer`/`datatype` with this width and centreline.
    Path {
        /// GDSII layer number.
        layer: u16,
        /// GDSII datatype number.
        datatype: u16,
        /// Path width in database units (0 for a zero-width path).
        width: i32,
        /// The path centreline vertices, in database units.
        xy: Vec<(i32, i32)>,
    },
    /// A single structure reference (instance placement).
    StructRef {
        /// The referenced structure name.
        name: String,
        /// Placement origin x, in database units.
        x: i32,
        /// Placement origin y, in database units.
        y: i32,
    },
    /// An array reference (tiled placement).
    ArrayRef {
        /// The referenced structure name.
        name: String,
        /// Number of columns.
        cols: u16,
        /// Number of rows.
        rows: u16,
        /// Array anchor origin x, in database units.
        x: i32,
        /// Array anchor origin y, in database units.
        y: i32,
    },
    /// A text/label element on `layer`/`texttype`.
    Text {
        /// GDSII layer number.
        layer: u16,
        /// GDSII texttype number.
        texttype: u16,
        /// Insertion point x, in database units.
        x: i32,
        /// Insertion point y, in database units.
        y: i32,
        /// The label string.
        text: String,
    },
    /// End of the current structure.
    EndStruct,
    /// End of the library; no further events follow.
    EndLibrary,
}

/// A forward-only GDSII record reader over any [`std::io::Read`].
///
/// **Contract stub** (ADR 0062): lane 2A implements [`Self::next_event`] and the
/// internal record decoding. The public shape (construct from a reader, pull events to
/// end of stream) is frozen so lane 2D's converter and lane 6C's worker can be written
/// against it now.
pub struct GdsRecordReader<R: Read> {
    inner: R,
    done: bool,
}

impl<R: Read> std::fmt::Debug for GdsRecordReader<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GdsRecordReader")
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

impl<R: Read> GdsRecordReader<R> {
    /// Wraps a byte source. No input is read until [`Self::next_event`] is called.
    pub fn new(inner: R) -> Self {
        Self { inner, done: false }
    }

    /// Consumes the reader and returns the underlying source, so a caller can reclaim
    /// it after reaching [`GdsEvent::EndLibrary`].
    pub fn into_inner(self) -> R {
        self.inner
    }

    /// Pulls the next [`GdsEvent`], or `Ok(None)` at end of stream.
    ///
    /// The reader loops over raw records, skipping everything outside the frozen
    /// [`GdsEvent`] vocabulary (dates, properties, formatting), and returns as soon
    /// as it has one complete event. An element (`BOUNDARY`/`PATH`/`SREF`/`AREF`/
    /// `TEXT` through its `ENDEL`) is fully consumed within a single call, so no
    /// element state is carried across calls; only end-of-stream is.
    ///
    /// # Errors
    ///
    /// Returns [`IoError`] (as a [`reticle_model::ModelError`]) on a malformed record:
    /// a length under four, an odd length, a zero-length string record (the
    /// `gds21` `read_str` panic class), or a payload truncated by the underlying
    /// reader. A record length is a `u16`, so a payload buffer is at most ~64 KiB and
    /// is filled only from bytes the reader actually returns: no count field ever
    /// drives an allocation past the remaining input (the OASIS OOM lesson).
    pub fn next_event(&mut self) -> Result<Option<GdsEvent>> {
        if self.done {
            return Ok(None);
        }

        // Element-accumulation state, local because an element never spans calls.
        let mut pending = Pending::None;

        loop {
            let Some(rec) = self.read_record()? else {
                // Clean or truncated end of stream: stop. Any half-read element is
                // dropped (the DOM importer rejects such a file; we simply end).
                self.done = true;
                return Ok(None);
            };

            match rec.rtype {
                RT_UNITS => {
                    // The library is considered begun at UNITS, the record that
                    // carries the database resolution the event needs.
                    return Ok(Some(GdsEvent::BeginLibrary {
                        dbu_per_micron: dbu_per_micron_from_units(&rec.payload),
                    }));
                }
                RT_STRNAME => {
                    return Ok(Some(GdsEvent::BeginStruct {
                        name: parse_gds_string(&rec.payload),
                    }));
                }
                RT_ENDSTR => return Ok(Some(GdsEvent::EndStruct)),
                RT_ENDLIB => {
                    self.done = true;
                    return Ok(Some(GdsEvent::EndLibrary));
                }

                // Element starts: begin accumulating until ENDEL.
                RT_BOUNDARY => {
                    pending = Pending::Boundary {
                        layer: 0,
                        datatype: 0,
                        xy: Vec::new(),
                    }
                }
                RT_PATH => {
                    pending = Pending::Path {
                        layer: 0,
                        datatype: 0,
                        width: 0,
                        xy: Vec::new(),
                    }
                }
                RT_SREF => {
                    pending = Pending::StructRef {
                        name: String::new(),
                        x: 0,
                        y: 0,
                    }
                }
                RT_AREF => {
                    pending = Pending::ArrayRef {
                        name: String::new(),
                        cols: 0,
                        rows: 0,
                        x: 0,
                        y: 0,
                    }
                }
                RT_TEXT => {
                    pending = Pending::Text {
                        layer: 0,
                        texttype: 0,
                        x: 0,
                        y: 0,
                        text: String::new(),
                    }
                }
                // Elements we do not model carry their own ENDEL; swallow to it.
                RT_NODE | RT_BOX => pending = Pending::Skip,

                // Property records: fill the current element in progress.
                RT_LAYER => pending.set_layer(read_i16(&rec.payload)),
                RT_DATATYPE => pending.set_datatype(read_i16(&rec.payload)),
                RT_TEXTTYPE => pending.set_texttype(read_i16(&rec.payload)),
                RT_WIDTH => pending.set_width(read_i32(&rec.payload)),
                RT_SNAME => pending.set_name(parse_gds_string(&rec.payload)),
                RT_COLROW => pending.set_colrow(&rec.payload),
                RT_STRING => pending.set_string(parse_gds_string(&rec.payload)),
                RT_XY => pending.set_xy(parse_xy(&rec.payload)),

                RT_ENDEL => {
                    if let Some(event) = std::mem::replace(&mut pending, Pending::None).finish() {
                        return Ok(Some(event));
                    }
                    // A skipped or incomplete element yields nothing; keep reading.
                }

                // HEADER, BGNLIB/BGNSTR (dates), LIBNAME, PRESENTATION, STRANS, MAG,
                // ANGLE, PATHTYPE, properties, and any unknown record are consumed
                // and skipped. Dates are never parsed here, so the out-of-range-date
                // panic class cannot fire on this path.
                _ => {}
            }
        }
    }

    /// Reads one raw GDSII record, or `Ok(None)` at end of stream.
    ///
    /// A `u16` length bounds every payload to under 64 KiB, and the payload buffer is
    /// filled only from bytes the reader returns, so a hostile length can never force
    /// an allocation beyond the input. A length under four or odd would leave the
    /// stream unadvanceable (a hang risk) and is rejected as malformed.
    fn read_record(&mut self) -> Result<Option<Record>> {
        let mut header = [0u8; 4];
        // A full 4-byte header advances; anything less is a clean record-boundary EOF
        // (0 bytes) or a header truncated at EOF, both of which simply end the stream.
        if fill(&mut self.inner, &mut header)? != 4 {
            return Ok(None);
        }
        let reclen = u16::from_be_bytes([header[0], header[1]]) as usize;
        if reclen < 4 || !reclen.is_multiple_of(2) {
            return Err(IoError::Malformed("GDSII record length under four or odd").into());
        }
        let rtype = header[2];
        let dtype = header[3];
        let payload_len = reclen - 4;

        // Reject a zero-length string record before anyone could index `data[-1]`,
        // mirroring the DOM importer's guard so this path shares its no-panic bound.
        if dtype == DT_STRING && payload_len == 0 {
            return Err(IoError::Malformed(
                "GDSII zero-length string record (would index data[-1])",
            )
            .into());
        }

        // `payload_len` is at most 65_531 (a `u16` length less the header), so this is
        // a bounded allocation; `fill` then reads only real bytes into it.
        let mut payload = vec![0u8; payload_len];
        if fill(&mut self.inner, &mut payload)? != payload_len {
            return Err(IoError::Malformed("GDSII record payload truncated").into());
        }
        Ok(Some(Record { rtype, payload }))
    }
}

/// One raw GDSII record after framing: its type and payload bytes. The data-type
/// code is only needed for the zero-length-string guard in [`GdsRecordReader::read_record`]
/// and is not retained.
struct Record {
    rtype: u8,
    payload: Vec<u8>,
}

/// The element being accumulated between an element-start record and its `ENDEL`.
enum Pending {
    /// No element in progress.
    None,
    /// An element type this reader does not surface (`NODE`/`BOX`).
    Skip,
    Boundary {
        layer: u16,
        datatype: u16,
        xy: Vec<(i32, i32)>,
    },
    Path {
        layer: u16,
        datatype: u16,
        width: i32,
        xy: Vec<(i32, i32)>,
    },
    StructRef {
        name: String,
        x: i32,
        y: i32,
    },
    ArrayRef {
        name: String,
        cols: u16,
        rows: u16,
        x: i32,
        y: i32,
    },
    Text {
        layer: u16,
        texttype: u16,
        x: i32,
        y: i32,
        text: String,
    },
}

impl Pending {
    fn set_layer(&mut self, v: i16) {
        match self {
            Self::Boundary { layer, .. } | Self::Path { layer, .. } | Self::Text { layer, .. } => {
                *layer = v as u16;
            }
            _ => {}
        }
    }

    fn set_datatype(&mut self, v: i16) {
        match self {
            Self::Boundary { datatype, .. } | Self::Path { datatype, .. } => *datatype = v as u16,
            _ => {}
        }
    }

    fn set_texttype(&mut self, v: i16) {
        if let Self::Text { texttype, .. } = self {
            *texttype = v as u16;
        }
    }

    fn set_width(&mut self, v: i32) {
        if let Self::Path { width, .. } = self {
            *width = v;
        }
    }

    fn set_name(&mut self, v: String) {
        match self {
            Self::StructRef { name, .. } | Self::ArrayRef { name, .. } => *name = v,
            _ => {}
        }
    }

    fn set_string(&mut self, v: String) {
        if let Self::Text { text, .. } = self {
            *text = v;
        }
    }

    fn set_colrow(&mut self, payload: &[u8]) {
        if let Self::ArrayRef { cols, rows, .. } = self {
            *cols = read_i16(payload) as u16;
            *rows = read_i16_at(payload, 2) as u16;
        }
    }

    /// Assigns a coordinate list to the element. The first point anchors the
    /// single-origin elements (`SREF`/`AREF`/`TEXT`); boundaries and paths keep the
    /// whole list.
    fn set_xy(&mut self, points: Vec<(i32, i32)>) {
        match self {
            Self::Boundary { xy, .. } | Self::Path { xy, .. } => *xy = points,
            Self::StructRef { x, y, .. }
            | Self::ArrayRef { x, y, .. }
            | Self::Text { x, y, .. } => {
                if let Some(&(px, py)) = points.first() {
                    *x = px;
                    *y = py;
                }
            }
            Self::None | Self::Skip => {}
        }
    }

    /// Converts a completed element into its event, or `None` if there was no
    /// element (a stray `ENDEL`) or it is one we do not surface.
    fn finish(self) -> Option<GdsEvent> {
        match self {
            Self::None | Self::Skip => None,
            Self::Boundary {
                layer,
                datatype,
                xy,
            } => Some(GdsEvent::Boundary {
                layer,
                datatype,
                xy,
            }),
            Self::Path {
                layer,
                datatype,
                width,
                xy,
            } => Some(GdsEvent::Path {
                layer,
                datatype,
                width,
                xy,
            }),
            Self::StructRef { name, x, y } => Some(GdsEvent::StructRef { name, x, y }),
            Self::ArrayRef {
                name,
                cols,
                rows,
                x,
                y,
            } => Some(GdsEvent::ArrayRef {
                name,
                cols,
                rows,
                x,
                y,
            }),
            Self::Text {
                layer,
                texttype,
                x,
                y,
                text,
            } => Some(GdsEvent::Text {
                layer,
                texttype,
                x,
                y,
                text,
            }),
        }
    }
}

/// Reads into `buf` until it is full or the reader signals EOF, returning how many
/// bytes were read. Retries [`std::io::ErrorKind::Interrupted`]; any other transport
/// error is surfaced as a malformed-input error (the frozen [`reticle_model::ModelError`]
/// has no I/O variant).
fn fill<R: Read>(reader: &mut R, buf: &mut [u8]) -> Result<usize> {
    let mut read = 0;
    while read < buf.len() {
        match reader.read(&mut buf[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                return Err(IoError::Malformed("transport error reading GDSII record").into());
            }
        }
    }
    Ok(read)
}

/// Reads a big-endian `i16` from the front of `payload`, or `0` if it is too short.
fn read_i16(payload: &[u8]) -> i16 {
    read_i16_at(payload, 0)
}

/// Reads a big-endian `i16` at byte offset `at`, or `0` if `payload` is too short.
fn read_i16_at(payload: &[u8], at: usize) -> i16 {
    match payload.get(at..at + 2) {
        Some(b) => i16::from_be_bytes([b[0], b[1]]),
        None => 0,
    }
}

/// Reads a big-endian `i32` from the front of `payload`, or `0` if it is too short.
fn read_i32(payload: &[u8]) -> i32 {
    match payload.get(0..4) {
        Some(b) => i32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        None => 0,
    }
}

/// Parses an XY record payload into `(x, y)` pairs of big-endian `i32`s. A trailing
/// partial pair (a payload whose length is not a multiple of eight) is ignored. The
/// vector's capacity comes from the record length, already bounded to under 64 KiB.
fn parse_xy(payload: &[u8]) -> Vec<(i32, i32)> {
    payload
        .chunks_exact(8)
        .map(|c| {
            let x = i32::from_be_bytes([c[0], c[1], c[2], c[3]]);
            let y = i32::from_be_bytes([c[4], c[5], c[6], c[7]]);
            (x, y)
        })
        .collect()
}

/// Decodes a GDSII string payload: trailing NUL padding (GDSII strings are padded to
/// an even length) is stripped and the remaining bytes are read as UTF-8, lossily so
/// a stray non-ASCII byte cannot fail the parse. An all-NUL or empty payload yields
/// an empty string (a zero-length string *record* is rejected earlier, in
/// [`GdsRecordReader::read_record`]).
fn parse_gds_string(payload: &[u8]) -> String {
    let end = payload.iter().rposition(|&b| b != 0).map_or(0, |p| p + 1);
    String::from_utf8_lossy(&payload[..end]).into_owned()
}

/// Recovers `dbu_per_micron` from a UNITS record payload (two GDSII 8-byte reals:
/// the user unit and the database unit in metres). Mirrors
/// [`crate::gds`]'s `dbu_per_micron_from_units`: a non-positive, unrepresentable, or
/// absent database unit falls back to [`DEFAULT_DBU_PER_MICRON`].
fn dbu_per_micron_from_units(payload: &[u8]) -> i64 {
    // The database unit (metres per DBU) is the second real, bytes 8..16.
    let Some(bytes) = payload.get(8..16) else {
        return DEFAULT_DBU_PER_MICRON;
    };
    let db_unit_meters = gds_real8_to_f64(bytes.try_into().expect("8 bytes"));
    if db_unit_meters > 0.0 {
        let per_micron = (USER_UNIT_METERS / db_unit_meters).round();
        if per_micron >= 1.0 && per_micron <= f64::from(u32::MAX) {
            return per_micron as i64;
        }
    }
    DEFAULT_DBU_PER_MICRON
}

/// Decodes a GDSII 8-byte real (excess-64, base-16 float): a sign bit and 7-bit
/// exponent in the first byte, then a 56-bit fraction. This is *not* IEEE-754, so it
/// is decoded by hand rather than transmuted.
fn gds_real8_to_f64(b: [u8; 8]) -> f64 {
    let sign = if b[0] & 0x80 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from(b[0] & 0x7f) - 64;
    let mut mantissa = 0u64;
    for &byte in &b[1..8] {
        mantissa = (mantissa << 8) | u64::from(byte);
    }
    // Fraction is mantissa / 2^56; magnitude is fraction * 16^exponent.
    sign * (mantissa as f64 / 72_057_594_037_927_936.0) * 16f64.powi(exponent)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Record types the reader skips (so it does not name them in the decoder),
    /// needed only to synthesize well-framed streams in these tests.
    const RT_HEADER: u8 = 0x00;
    const RT_BGNLIB: u8 = 0x01;
    const RT_BGNSTR: u8 = 0x05;

    #[test]
    fn event_vocabulary_constructs_and_compares() {
        // Exercise every frozen variant so the contract surface is real and used from
        // the contract commit; lane 2A/2D build the same shapes.
        let events = [
            GdsEvent::BeginLibrary {
                dbu_per_micron: 1000,
            },
            GdsEvent::BeginStruct { name: "top".into() },
            GdsEvent::Boundary {
                layer: 68,
                datatype: 20,
                xy: vec![(0, 0), (1, 0), (1, 1)],
            },
            GdsEvent::Path {
                layer: 69,
                datatype: 20,
                width: 140,
                xy: vec![(0, 0), (10, 0)],
            },
            GdsEvent::StructRef {
                name: "sub".into(),
                x: 5,
                y: 6,
            },
            GdsEvent::ArrayRef {
                name: "sub".into(),
                cols: 4,
                rows: 2,
                x: 0,
                y: 0,
            },
            GdsEvent::Text {
                layer: 68,
                texttype: 5,
                x: 1,
                y: 2,
                text: "vdd".into(),
            },
            GdsEvent::EndStruct,
            GdsEvent::EndLibrary,
        ];
        assert_eq!(events.len(), 9);
        assert_eq!(
            events[0],
            GdsEvent::BeginLibrary {
                dbu_per_micron: 1000
            }
        );
        assert_ne!(events[7], events[8]);
    }

    /// Builds one GDS record: `[len_hi, len_lo, rectype, datatype, payload...]`.
    fn record(rectype: u8, datatype: u8, payload: &[u8]) -> Vec<u8> {
        let len = (4 + payload.len()) as u16;
        let mut out = len.to_be_bytes().to_vec();
        out.push(rectype);
        out.push(datatype);
        out.extend_from_slice(payload);
        out
    }

    /// Big-endian bytes of an XY coordinate list.
    fn xy_bytes(points: &[(i32, i32)]) -> Vec<u8> {
        points
            .iter()
            .flat_map(|&(x, y)| {
                let mut b = x.to_be_bytes().to_vec();
                b.extend_from_slice(&y.to_be_bytes());
                b
            })
            .collect()
    }

    /// Encodes a positive GDSII 8-byte real (used to build a UNITS record).
    fn gds_real8(value: f64) -> [u8; 8] {
        assert!(value > 0.0);
        let mut exponent = 0i32;
        let mut v = value;
        while v >= 1.0 {
            v /= 16.0;
            exponent += 1;
        }
        while v < 1.0 / 16.0 {
            v *= 16.0;
            exponent -= 1;
        }
        let mantissa = (v * 72_057_594_037_927_936.0).round() as u64;
        let mut out = [0u8; 8];
        out[0] = ((exponent + 64) as u8) & 0x7f;
        for i in 0..7 {
            out[i + 1] = (mantissa >> (8 * (6 - i))) as u8;
        }
        out
    }

    /// A minimal well-formed library: HEADER, BGNLIB, UNITS, one struct with one
    /// boundary and one path, ENDSTR, ENDLIB.
    fn sample_library() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(record(RT_HEADER, 0x02, &[0, 3]));
        b.extend(record(RT_BGNLIB, 0x02, &[0u8; 24]));
        // UNITS: user unit (1e-3) then database unit in metres (1e-9) => 1000 dbu/µm.
        let mut units = gds_real8(1e-3).to_vec();
        units.extend_from_slice(&gds_real8(1e-9));
        b.extend(record(RT_UNITS, 0x05, &units));
        b.extend(record(RT_BGNSTR, 0x02, &[0u8; 24]));
        b.extend(record(RT_STRNAME, DT_STRING, b"top\0"));
        b.extend(record(RT_BOUNDARY, 0x00, &[]));
        b.extend(record(RT_LAYER, 0x02, &68i16.to_be_bytes()));
        b.extend(record(RT_DATATYPE, 0x02, &20i16.to_be_bytes()));
        b.extend(record(
            RT_XY,
            0x03,
            &xy_bytes(&[(0, 0), (10, 0), (10, 10), (0, 10), (0, 0)]),
        ));
        b.extend(record(RT_ENDEL, 0x00, &[]));
        b.extend(record(RT_PATH, 0x00, &[]));
        b.extend(record(RT_LAYER, 0x02, &69i16.to_be_bytes()));
        b.extend(record(RT_DATATYPE, 0x02, &20i16.to_be_bytes()));
        b.extend(record(RT_WIDTH, 0x03, &140i32.to_be_bytes()));
        b.extend(record(RT_XY, 0x03, &xy_bytes(&[(0, 0), (100, 0)])));
        b.extend(record(RT_ENDEL, 0x00, &[]));
        b.extend(record(RT_ENDSTR, 0x00, &[]));
        b.extend(record(RT_ENDLIB, 0x00, &[]));
        b
    }

    /// Drives a reader to exhaustion, collecting its events.
    fn events(bytes: &[u8]) -> Vec<GdsEvent> {
        let mut reader = GdsRecordReader::new(bytes);
        let mut out = Vec::new();
        while let Some(ev) = reader.next_event().expect("no error on valid input") {
            out.push(ev);
        }
        out
    }

    #[test]
    fn decodes_a_minimal_library_in_document_order() {
        let events = events(&sample_library());
        assert_eq!(
            events,
            vec![
                GdsEvent::BeginLibrary {
                    dbu_per_micron: 1000
                },
                GdsEvent::BeginStruct { name: "top".into() },
                GdsEvent::Boundary {
                    layer: 68,
                    datatype: 20,
                    xy: vec![(0, 0), (10, 0), (10, 10), (0, 10), (0, 0)],
                },
                GdsEvent::Path {
                    layer: 69,
                    datatype: 20,
                    width: 140,
                    xy: vec![(0, 0), (100, 0)],
                },
                GdsEvent::EndStruct,
                GdsEvent::EndLibrary,
            ]
        );
    }

    #[test]
    fn decodes_references_and_text() {
        let mut b = Vec::new();
        b.extend(record(RT_HEADER, 0x02, &[0, 3]));
        b.extend(record(RT_BGNSTR, 0x02, &[0u8; 24]));
        b.extend(record(RT_STRNAME, DT_STRING, b"top\0"));
        // SREF to "sub" at (5, 6).
        b.extend(record(RT_SREF, 0x00, &[]));
        b.extend(record(RT_SNAME, DT_STRING, b"sub\0"));
        b.extend(record(RT_XY, 0x03, &xy_bytes(&[(5, 6)])));
        b.extend(record(RT_ENDEL, 0x00, &[]));
        // AREF 4x2 of "sub" anchored at (0, 0).
        b.extend(record(RT_AREF, 0x00, &[]));
        b.extend(record(RT_SNAME, DT_STRING, b"sub\0"));
        let mut colrow = 4i16.to_be_bytes().to_vec();
        colrow.extend_from_slice(&2i16.to_be_bytes());
        b.extend(record(RT_COLROW, 0x02, &colrow));
        b.extend(record(
            RT_XY,
            0x03,
            &xy_bytes(&[(0, 0), (400, 0), (0, 200)]),
        ));
        b.extend(record(RT_ENDEL, 0x00, &[]));
        // TEXT "vdd" on 68/5 at (1, 2).
        b.extend(record(RT_TEXT, 0x00, &[]));
        b.extend(record(RT_LAYER, 0x02, &68i16.to_be_bytes()));
        b.extend(record(RT_TEXTTYPE, 0x02, &5i16.to_be_bytes()));
        b.extend(record(RT_XY, 0x03, &xy_bytes(&[(1, 2)])));
        b.extend(record(RT_STRING, DT_STRING, b"vdd\0"));
        b.extend(record(RT_ENDEL, 0x00, &[]));
        b.extend(record(RT_ENDSTR, 0x00, &[]));
        b.extend(record(RT_ENDLIB, 0x00, &[]));

        assert_eq!(
            events(&b),
            vec![
                GdsEvent::BeginStruct { name: "top".into() },
                GdsEvent::StructRef {
                    name: "sub".into(),
                    x: 5,
                    y: 6,
                },
                GdsEvent::ArrayRef {
                    name: "sub".into(),
                    cols: 4,
                    rows: 2,
                    x: 0,
                    y: 0,
                },
                GdsEvent::Text {
                    layer: 68,
                    texttype: 5,
                    x: 1,
                    y: 2,
                    text: "vdd".into(),
                },
                GdsEvent::EndStruct,
                GdsEvent::EndLibrary,
            ]
        );
    }

    #[test]
    fn skips_node_and_box_and_unknown_records() {
        let mut b = Vec::new();
        b.extend(record(RT_BGNSTR, 0x02, &[0u8; 24]));
        b.extend(record(RT_STRNAME, DT_STRING, b"c\0"));
        // A NODE element with a layer/xy is swallowed to its ENDEL.
        b.extend(record(RT_NODE, 0x00, &[]));
        b.extend(record(RT_LAYER, 0x02, &1i16.to_be_bytes()));
        b.extend(record(RT_XY, 0x03, &xy_bytes(&[(0, 0), (1, 1)])));
        b.extend(record(RT_ENDEL, 0x00, &[]));
        // An unknown record type is skipped.
        b.extend(record(0x7F, 0x02, &[0, 0]));
        b.extend(record(RT_ENDSTR, 0x00, &[]));
        b.extend(record(RT_ENDLIB, 0x00, &[]));

        assert_eq!(
            events(&b),
            vec![
                GdsEvent::BeginStruct { name: "c".into() },
                GdsEvent::EndStruct,
                GdsEvent::EndLibrary,
            ]
        );
    }

    #[test]
    fn stops_at_endlib_and_reports_done() {
        let mut b = sample_library();
        b.extend_from_slice(&[0u8; 16]); // trailing block padding after ENDLIB
        let mut reader = GdsRecordReader::new(&b[..]);
        let mut count = 0;
        while reader.next_event().expect("valid").is_some() {
            count += 1;
        }
        assert_eq!(count, 6);
        // Further pulls stay at None.
        assert_eq!(reader.next_event().unwrap(), None);
    }

    #[test]
    fn rejects_zero_length_string_record() {
        // A string-typed record with an empty payload is the gds21 read_str panic
        // class; the reader must reject it, matching the DOM importer's guard.
        let mut b = Vec::new();
        b.extend(record(RT_HEADER, 0x02, &[0, 3]));
        b.extend(record(RT_STRNAME, DT_STRING, &[]));
        let mut reader = GdsRecordReader::new(&b[..]);
        // HEADER is skipped, then the bad record errors.
        assert!(reader.next_event().is_err());
    }

    #[test]
    fn rejects_under_length_and_odd_records() {
        for bad in [
            vec![0x00, 0x03, 0x00, 0x00],       // reclen 3 < 4
            vec![0x00, 0x05, 0x10, 0x03, 0x00], // reclen 5, odd
        ] {
            let mut reader = GdsRecordReader::new(&bad[..]);
            assert!(reader.next_event().is_err(), "expected error for {bad:?}");
        }
    }

    #[test]
    fn empty_and_truncated_inputs_do_not_panic() {
        assert_eq!(events(&[]), Vec::new());
        // A record header promising a payload that is not there ends as an error,
        // never a panic or hang.
        let truncated = [0x00, 0x08, RT_STRNAME, DT_STRING, b'a']; // needs 4 payload
        let mut reader = GdsRecordReader::new(&truncated[..]);
        assert!(reader.next_event().is_err());
    }

    #[test]
    fn gds_real8_round_trips_through_decode() {
        for v in [1e-3, 1e-9, 1.0, 0.5, 12345.678] {
            let decoded = gds_real8_to_f64(gds_real8(v));
            assert!(
                (decoded - v).abs() <= v.abs() * 1e-9 + 1e-12,
                "{v} decoded as {decoded}"
            );
        }
    }
}
