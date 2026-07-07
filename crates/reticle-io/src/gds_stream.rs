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
    /// **Contract stub**: returns an error until lane 2A implements the decoder. It is
    /// wired to touch `self.inner`/`self.done` so the shape is real and the fields are
    /// live from the contract commit.
    ///
    /// # Errors
    ///
    /// Returns [`IoError`] on a malformed record or transport failure. Currently always
    /// returns an `Unsupported` error (the reader is not yet implemented).
    pub fn next_event(&mut self) -> Result<Option<GdsEvent>> {
        if self.done {
            return Ok(None);
        }
        // Read nothing meaningful yet; the stub proves the field is live without
        // decoding. Lane 2A replaces this whole body with the record loop.
        let mut probe = [0u8; 0];
        let _ = self.inner.read(&mut probe);
        Err(IoError::Unsupported("gds_stream reader: implemented in Wave 2 lane 2A").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn reader_stub_fails_honestly_then_reports_done() {
        // The stub errors rather than silently returning an empty stream. Lane 2A
        // replaces this with a real fixture-driven decode test.
        let bytes: &[u8] = &[0x00, 0x06, 0x00, 0x02, 0x00, 0x03];
        let mut reader = GdsRecordReader::new(bytes);
        assert!(reader.next_event().is_err());
        reader.done = true;
        assert_eq!(reader.next_event().unwrap(), None);
        let _ = reader.into_inner();
    }
}
