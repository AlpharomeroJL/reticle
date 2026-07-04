//! The document-open seam: bytes plus a format hint to an opened document.
//!
//! This is the single, platform-neutral entry point for loading a layout file
//! into the editor. It takes **bytes** (not a path) and a [`DocFormat`] hint, so
//! the same call works on native and in the browser where there is no filesystem;
//! the caller is responsible for obtaining the bytes (a file read on native, a
//! file-input `ArrayBuffer` on wasm). It imports through the hardened
//! `reticle-io` path, so no input can panic or hang the app, and returns an
//! [`OpenOutcome`] carrying the opened document, the top cell to frame, and a list
//! of structured, non-fatal [`OpenWarning`]s (empty for a clean file), or a clean
//! [`OpenError`] on a hard failure.
//!
//! # What this seam is (and is not)
//!
//! It is the contract other parts of the app route file-opening through: a Start
//! screen's "open a file" button, an example gallery, or a drag-and-drop handler
//! all call [`open_document_bytes`] and then hand the [`OpenOutcome`] to
//! [`App::open_outcome`](crate::app::App::open_outcome) to load it into the editor.
//! It is deliberately free of any browser specifics and any Start-screen UI: it is
//! pure model glue over `reticle-io`, unit-tested without a window or a GPU, so the
//! interesting behavior (which formats parse, what warnings surface, how errors
//! read) is proven in plain code.
//!
//! # Warnings vs errors
//!
//! A *warning* is a recoverable problem: the file opened, but something in it was
//! skipped or clamped (a degenerate shape, an out-of-range value). The document is
//! still usable and is returned; the warnings ride alongside it so the UI can show
//! the user what was dropped. An *error* means no usable document could be produced
//! (the bytes are not the claimed format, or are malformed past recovery); nothing
//! is returned but a clear message.

use reticle_io::{Gds, ImportWarning, Oasis};
use reticle_model::{Document, Importer};

/// The layout file format to interpret the bytes as.
///
/// A hint, not a guess: the caller states which importer to use (from a file
/// extension, a picker filter, or an explicit choice). Unknown or unsupported
/// formats are not represented here; a caller that cannot classify the bytes
/// should surface that itself rather than calling the seam.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DocFormat {
    /// GDSII binary stream, imported through the hardened `reticle-io` GDS path.
    Gds,
    /// The Reticle OASIS-inspired container (see `reticle_io::Oasis`).
    Oasis,
}

impl DocFormat {
    /// A short, human-readable name for the format, for messages.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            DocFormat::Gds => "GDSII",
            DocFormat::Oasis => "OASIS",
        }
    }

    /// Guesses a format from a file name's extension, or `None` if unrecognized.
    ///
    /// Recognizes `.gds`/`.gdsii`/`.gds2` as [`DocFormat::Gds`] and `.oas`/`.oasis`
    /// as [`DocFormat::Oasis`], case-insensitively. Provided as a convenience for
    /// callers that have a file name; the seam itself never touches the filesystem.
    #[must_use]
    pub fn from_extension(name: &str) -> Option<Self> {
        let ext = name.rsplit('.').next()?.to_ascii_lowercase();
        match ext.as_str() {
            "gds" | "gdsii" | "gds2" => Some(DocFormat::Gds),
            "oas" | "oasis" => Some(DocFormat::Oasis),
            _ => None,
        }
    }
}

/// A structured, non-fatal problem found while opening a document.
///
/// Plain owned data so it crosses the wasm boundary and lives in the UI freely: a
/// one-line [`summary`](OpenWarning::summary) and a longer
/// [`detail`](OpenWarning::detail). Produced from the importer's own
/// [`reticle_io::ImportWarning`]s, so the app does not need to know the importer's
/// warning taxonomy.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OpenWarning {
    /// A short, human-readable one-liner naming what happened.
    pub summary: String,
    /// A longer explanation of what was skipped, clamped, or defaulted.
    pub detail: String,
}

impl OpenWarning {
    /// Builds an [`OpenWarning`] from a `reticle-io` import warning.
    fn from_import(w: &ImportWarning) -> Self {
        Self {
            summary: w.summary.clone(),
            detail: w.detail.clone(),
        }
    }
}

/// The successful result of opening a document: the document plus what to frame
/// and any non-fatal warnings.
///
/// The document is always valid and safe to install. [`top_cell`](OpenOutcome::top_cell)
/// is the cell the editor should frame on load (the document's first declared top
/// cell, or its first cell if it declares none, or an empty string for an empty
/// document). [`warnings`](OpenOutcome::warnings) lists every recoverable problem
/// (empty for a clean file).
#[derive(Clone, Debug)]
pub struct OpenOutcome {
    /// The opened document, ready to load into the editor.
    pub document: Document,
    /// The top cell to frame when the document loads.
    pub top_cell: String,
    /// Non-fatal problems found while opening, in encounter order.
    pub warnings: Vec<OpenWarning>,
}

impl OpenOutcome {
    /// Whether the open produced any non-fatal warnings.
    #[must_use]
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }
}

/// A hard failure that produced no usable document.
///
/// `#[non_exhaustive]` so more precise categories can be added without breaking
/// callers, who should match with a `_ =>` arm or format the [`Display`](std::fmt::Display) text.
#[derive(Clone, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum OpenError {
    /// The importer rejected the bytes: not the claimed format, or malformed past
    /// recovery. Carries the format tried and a human-readable reason.
    Import {
        /// The format the bytes were interpreted as.
        format: DocFormat,
        /// A human-readable reason, lowered from the importer's error.
        reason: String,
    },
    /// The bytes imported but yielded a document with no cells at all, so there is
    /// nothing to open. Kept distinct from a parse error because the input was
    /// well-formed; it is simply empty.
    Empty {
        /// The format the (empty) document was read as.
        format: DocFormat,
    },
}

impl core::fmt::Display for OpenError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            OpenError::Import { format, reason } => {
                write!(f, "could not open {} file: {reason}", format.label())
            }
            OpenError::Empty { format } => {
                write!(
                    f,
                    "the {} file opened but contains no cells",
                    format.label()
                )
            }
        }
    }
}

impl core::error::Error for OpenError {}

/// Opens a layout document from `bytes`, interpreting them as `format`.
///
/// This is the app's single document-open entry point (see the [module
/// docs](self)). It never panics and never hangs on any input: GDSII goes through
/// the hardened [`reticle_io::Gds::import_with_warnings`] path (which contains the
/// `gds21` panic vectors and caps input size), and OASIS through its bounded
/// reader. On success it returns an [`OpenOutcome`] with the document, the top cell
/// to frame, and any non-fatal warnings; on failure a clean [`OpenError`].
///
/// # Errors
///
/// Returns [`OpenError::Import`] when the bytes are not the claimed format or are
/// malformed past recovery, and [`OpenError::Empty`] when they parse into a
/// document with no cells.
pub fn open_document_bytes(bytes: &[u8], format: DocFormat) -> Result<OpenOutcome, OpenError> {
    let (document, warnings) = match format {
        DocFormat::Gds => match Gds.import_with_warnings(bytes) {
            Ok(import) => (
                import.document,
                import
                    .warnings
                    .iter()
                    .map(OpenWarning::from_import)
                    .collect(),
            ),
            Err(e) => {
                return Err(OpenError::Import {
                    format,
                    reason: e.to_string(),
                });
            }
        },
        DocFormat::Oasis => match Oasis.import(bytes) {
            // The OASIS importer has no warning channel yet; a clean import carries
            // no warnings.
            Ok(document) => (document, Vec::new()),
            Err(e) => {
                return Err(OpenError::Import {
                    format,
                    reason: e.to_string(),
                });
            }
        },
    };

    if document.cell_count() == 0 {
        return Err(OpenError::Empty { format });
    }

    let top_cell = choose_top_cell(&document);
    Ok(OpenOutcome {
        document,
        top_cell,
        warnings,
    })
}

/// Chooses the cell to frame when a document loads: its first declared top cell,
/// else its first cell in name order (so the choice is deterministic), else an
/// empty string (only for a document with no cells, which the caller has already
/// rejected as [`OpenError::Empty`]).
fn choose_top_cell(document: &Document) -> String {
    if let Some(top) = document.top_cells().first() {
        return top.clone();
    }
    // No declared top: pick the first cell by name for a stable result.
    let mut names: Vec<&str> = document.cells().map(|c| c.name.as_str()).collect();
    names.sort_unstable();
    names.first().map(|s| (*s).to_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{Cell, DrawShape, Exporter, ShapeKind};

    /// A tiny valid GDS produced by our own exporter: one cell, one rectangle.
    fn valid_gds() -> Vec<u8> {
        let mut doc = Document::new();
        let mut cell = Cell::new("OPEN_TEST");
        cell.shapes.push(DrawShape::new(
            LayerId::new(10, 0),
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(1000, 1000))),
        ));
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["OPEN_TEST".to_owned()]);
        Gds.export(&doc).expect("export a trivial doc")
    }

    #[test]
    fn opens_a_valid_gds_cleanly() {
        let outcome = open_document_bytes(&valid_gds(), DocFormat::Gds).expect("opens");
        assert_eq!(outcome.top_cell, "OPEN_TEST");
        assert!(!outcome.has_warnings(), "a clean file has no warnings");
        assert!(outcome.document.cell("OPEN_TEST").is_some());
        // The framed cell has a finite, positive bbox.
        let bbox = outcome
            .document
            .cell_bbox(&outcome.top_cell)
            .expect("bbox present");
        assert!(bbox.width() > 0 && bbox.height() > 0);
    }

    #[test]
    fn rejects_non_gds_bytes_with_a_clean_error() {
        let err =
            open_document_bytes(b"not a gds file at all", DocFormat::Gds).expect_err("must reject");
        match err {
            OpenError::Import { format, .. } => assert_eq!(format, DocFormat::Gds),
            other => panic!("expected an import error, got {other:?}"),
        }
        // The message is human-readable and names the format.
        assert!(err.to_string().contains("GDSII"));
    }

    #[test]
    fn empty_bytes_are_a_clean_error_not_a_panic() {
        let err = open_document_bytes(&[], DocFormat::Gds).expect_err("empty is an error");
        // An empty input is not valid GDSII, so it is an import error (it never
        // reaches the empty-document check).
        assert!(matches!(err, OpenError::Import { .. }));
    }

    #[test]
    fn round_trips_through_oasis() {
        // Build a doc, export to OASIS, and open it back through the seam.
        let mut doc = Document::new();
        let mut cell = Cell::new("OAS_TEST");
        cell.shapes.push(DrawShape::new(
            LayerId::new(7, 0),
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(50, 80))),
        ));
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["OAS_TEST".to_owned()]);
        let bytes = Oasis.export(&doc).expect("oasis export");

        let outcome = open_document_bytes(&bytes, DocFormat::Oasis).expect("opens oasis");
        assert_eq!(outcome.top_cell, "OAS_TEST");
        assert!(outcome.document.cell("OAS_TEST").is_some());
    }

    #[test]
    fn format_from_extension_is_case_insensitive() {
        assert_eq!(DocFormat::from_extension("chip.gds"), Some(DocFormat::Gds));
        assert_eq!(DocFormat::from_extension("CHIP.GDS"), Some(DocFormat::Gds));
        assert_eq!(DocFormat::from_extension("x.oas"), Some(DocFormat::Oasis));
        assert_eq!(DocFormat::from_extension("readme.txt"), None);
        assert_eq!(DocFormat::from_extension("noext"), None);
    }

    #[test]
    fn a_document_with_no_declared_top_still_frames_a_cell() {
        // Export a doc, then re-open and confirm a top is chosen even if we clear
        // the declared tops by importing a stream that names none. We simulate this
        // by building a doc with a cell but empty tops and checking `choose_top_cell`.
        let mut doc = Document::new();
        doc.insert_cell(Cell::new("LONELY"));
        assert_eq!(choose_top_cell(&doc), "LONELY");
    }
}
