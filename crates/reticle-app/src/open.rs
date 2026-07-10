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
//! # Formats
//!
//! [`DocFormat`] names five readers: GDSII, CIF, DXF, and two OASIS dialects
//! that share the `.oas`/`.oasis` extension. `.oas`/`.oasis` bytes are routed by
//! a cheap content sniff (`looks_like_conformant_oasis`) rather than the
//! extension alone: the in-house `reticle_io::Oasis` container and the
//! conformant SEMI P39 `reticle_io::oasis_std::OasisStd` reader (the one
//! `KLayout` also reads) start with different magic bytes, so the right one
//! runs regardless of which tool wrote the file. CIF and DXF assign their own
//! synthetic `(layer, datatype)` numbers to each distinct layer *name* they see
//! (first-seen order, recorded in the opened document's
//! [`Technology`](reticle_model::Technology)); DXF's numbering can be remapped
//! after opening through [`crate::dxf_dialog`].
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

use reticle_io::{Gds, ImportWarning, Oasis, cif::Cif, dxf::Dxf, oasis_std::OasisStd};
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
    /// A `.oas`/`.oasis` stream: either the Reticle OASIS-inspired container
    /// (`reticle_io::Oasis`) or a genuine conformant OASIS (SEMI P39) stream
    /// (`reticle_io::oasis_std::OasisStd`). The two share this one extension, so
    /// [`open_document_bytes`] picks the reader by content, not by name; see the
    /// [module docs](self).
    Oasis,
    /// CIF (Caltech Intermediate Format), the classic MOSIS-era subset (see
    /// `reticle_io::cif::Cif`). Import only; there is no CIF exporter.
    Cif,
    /// DXF (Drawing Exchange Format), the layout-relevant 2D `ENTITIES` subset
    /// (see `reticle_io::dxf::Dxf`). Import only; there is no DXF exporter.
    Dxf,
}

impl DocFormat {
    /// A short, human-readable name for the format, for messages.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            DocFormat::Gds => "GDSII",
            DocFormat::Oasis => "OASIS",
            DocFormat::Cif => "CIF",
            DocFormat::Dxf => "DXF",
        }
    }

    /// Guesses a format from a file name's extension, or `None` if unrecognized.
    ///
    /// Recognizes `.gds`/`.gdsii`/`.gds2` as [`DocFormat::Gds`], `.oas`/`.oasis`
    /// as [`DocFormat::Oasis`] (which OASIS dialect is decided later, from the
    /// bytes), `.cif` as [`DocFormat::Cif`], and `.dxf` as [`DocFormat::Dxf`],
    /// case-insensitively. Provided as a convenience for callers that have a file
    /// name; the seam itself never touches the filesystem.
    #[must_use]
    pub fn from_extension(name: &str) -> Option<Self> {
        let ext = name.rsplit('.').next()?.to_ascii_lowercase();
        match ext.as_str() {
            "gds" | "gdsii" | "gds2" => Some(DocFormat::Gds),
            "oas" | "oasis" => Some(DocFormat::Oasis),
            "cif" => Some(DocFormat::Cif),
            "dxf" => Some(DocFormat::Dxf),
            _ => None,
        }
    }
}

/// The byte prefix every conformant-OASIS (SEMI P39) stream begins with: the
/// standard's own magic string. Distinct from the in-house [`Oasis`]
/// container's magic (`RETICLE-OASIS`), which is how
/// `looks_like_conformant_oasis` tells the two `.oas`/`.oasis` dialects apart.
const CONFORMANT_OASIS_MAGIC: &[u8] = b"%SEMI-OASIS\r\n";

/// Whether `bytes` open with the conformant-OASIS magic string, so
/// [`open_document_bytes`] can route a `.oas`/`.oasis` file to
/// [`OasisStd`] (the real SEMI P39 reader `KLayout` also reads) instead of the
/// in-house [`Oasis`] container.
///
/// A cheap prefix compare (no allocation, no scan past the first 13 bytes).
/// Bytes shorter than the magic, or bytes that simply do not match, both
/// return `false`, falling through to the in-house reader, which reports its
/// own clean error for bytes that are neither (see the `DocFormat::Oasis` arm
/// of [`open_document_bytes`]).
#[must_use]
fn looks_like_conformant_oasis(bytes: &[u8]) -> bool {
    bytes.starts_with(CONFORMANT_OASIS_MAGIC)
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
/// `gds21` panic vectors and caps input size); CIF and DXF through their own
/// bounded, warning-carrying readers; and OASIS through whichever of the two
/// bounded OASIS readers `looks_like_conformant_oasis` selects. On success it
/// returns an [`OpenOutcome`] with the document, the top cell to frame, and any
/// non-fatal warnings; on failure a clean [`OpenError`].
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
        DocFormat::Oasis => {
            // `.oas`/`.oasis` covers two dialects (see the module docs); the bytes
            // themselves, not the extension, decide which reader runs.
            let imported = if looks_like_conformant_oasis(bytes) {
                OasisStd.import(bytes)
            } else {
                Oasis.import(bytes)
            };
            match imported {
                // Neither OASIS importer has a warning channel yet; a clean import
                // carries no warnings.
                Ok(document) => (document, Vec::new()),
                Err(e) => {
                    return Err(OpenError::Import {
                        format,
                        reason: e.to_string(),
                    });
                }
            }
        }
        DocFormat::Cif => match Cif.import_with_warnings(bytes) {
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
        DocFormat::Dxf => match Dxf.import_with_warnings(bytes) {
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

    // --- CIF, DXF, and the conformant-OASIS (OasisStd) reader --------------------

    /// A minimal, hand-written CIF file: no `DS`/`DF`, so the one box lands in the
    /// synthetic `TOP` cell (see `reticle_io::cif`'s module docs).
    fn valid_cif() -> Vec<u8> {
        b"L M1;\nB 100 100 0 0;\nE;\n".to_vec()
    }

    /// A minimal, hand-written DXF file: an `ENTITIES` section with one `LINE`.
    fn valid_dxf() -> Vec<u8> {
        concat!(
            "0\nSECTION\n2\nENTITIES\n",
            "0\nLINE\n8\nL1\n10\n0\n20\n0\n11\n100\n21\n100\n",
            "0\nENDSEC\n0\nEOF\n",
        )
        .as_bytes()
        .to_vec()
    }

    #[test]
    fn opens_a_valid_cif_cleanly() {
        let outcome = open_document_bytes(&valid_cif(), DocFormat::Cif).expect("opens");
        assert_eq!(outcome.top_cell, "TOP");
        assert!(!outcome.has_warnings(), "a clean file has no warnings");
        assert!(outcome.document.cell("TOP").is_some());
    }

    #[test]
    fn opens_a_valid_dxf_cleanly() {
        let outcome = open_document_bytes(&valid_dxf(), DocFormat::Dxf).expect("opens");
        assert_eq!(outcome.top_cell, "TOP");
        assert!(!outcome.has_warnings(), "a clean file has no warnings");
        let cell = outcome.document.cell("TOP").expect("TOP cell present");
        assert_eq!(cell.shapes.len(), 1);
        // The DXF reader assigns the first-seen layer name "L1" number 0.
        assert_eq!(cell.shapes[0].layer, LayerId::new(0, 0));
        assert_eq!(outcome.document.technology().layers[0].name, "L1");
    }

    #[test]
    fn rejects_corrupt_cif_with_a_clean_error_not_a_panic() {
        // Invalid UTF-8: rejected before any statement parsing begins.
        let err = open_document_bytes(&[0xFF, 0xFE, 0xFD, b'X'], DocFormat::Cif)
            .expect_err("must reject");
        assert!(matches!(
            err,
            OpenError::Import {
                format: DocFormat::Cif,
                ..
            }
        ));
        assert!(err.to_string().contains("CIF"));

        // Well-formed UTF-8 but structurally invalid CIF (B expects 4 or 6 numbers).
        let err =
            open_document_bytes(b"L M1;\nB 1 1;\nE;\n", DocFormat::Cif).expect_err("must reject");
        assert!(matches!(
            err,
            OpenError::Import {
                format: DocFormat::Cif,
                ..
            }
        ));
    }

    #[test]
    fn rejects_corrupt_dxf_with_a_clean_error_not_a_panic() {
        // Invalid UTF-8: rejected before any tokenizing begins.
        let err = open_document_bytes(&[0xFF, 0xFE, 0xFD, b'X'], DocFormat::Dxf)
            .expect_err("must reject");
        assert!(matches!(
            err,
            OpenError::Import {
                format: DocFormat::Dxf,
                ..
            }
        ));
        assert!(err.to_string().contains("DXF"));

        // Well-formed UTF-8 but an odd number of group-code/value lines.
        let err = open_document_bytes(b"0\nSECTION\n2\n", DocFormat::Dxf).expect_err("must reject");
        assert!(matches!(
            err,
            OpenError::Import {
                format: DocFormat::Dxf,
                ..
            }
        ));
    }

    #[test]
    fn looks_like_conformant_oasis_matches_only_the_real_magic() {
        assert!(looks_like_conformant_oasis(b"%SEMI-OASIS\r\nrest"));
        assert!(!looks_like_conformant_oasis(b"RETICLE-OASIS\0rest"));
        assert!(!looks_like_conformant_oasis(b"garbage"));
        assert!(!looks_like_conformant_oasis(b""));
        // A prefix of the magic is not a match (must be the whole 13 bytes).
        assert!(!looks_like_conformant_oasis(b"%SEMI-OASIS"));
    }

    #[test]
    fn opens_a_valid_conformant_oasis_via_content_sniff() {
        // Build a doc, export through the conformant-OASIS writer (a distinct
        // reader from the in-house `Oasis` container), and open it back purely by
        // format hint `DocFormat::Oasis`: the sniff, not the caller, must pick
        // `OasisStd`.
        let mut doc = Document::new();
        let mut cell = Cell::new("STD_TEST");
        cell.shapes.push(DrawShape::new(
            LayerId::new(3, 0),
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(40, 60))),
        ));
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["STD_TEST".to_owned()]);
        let bytes = OasisStd.export(&doc).expect("conformant oasis export");
        assert!(looks_like_conformant_oasis(&bytes));

        let outcome = open_document_bytes(&bytes, DocFormat::Oasis).expect("opens");
        assert_eq!(outcome.top_cell, "STD_TEST");
        assert!(outcome.document.cell("STD_TEST").is_some());
    }

    #[test]
    fn in_house_oasis_round_trip_is_unaffected_by_the_sniff() {
        // The pre-existing in-house container never carries the conformant magic,
        // so adding the sniff must not change its behavior.
        let bytes = Oasis.export(&Document::new()).expect("export empty doc");
        assert!(!looks_like_conformant_oasis(&bytes));
    }

    #[test]
    fn rejects_corrupt_conformant_oasis_with_a_clean_error_not_a_panic() {
        // Carries the conformant magic (so the sniff routes it to `OasisStd`) but
        // the record stream past it is nonsense.
        let mut bytes = CONFORMANT_OASIS_MAGIC.to_vec();
        bytes.extend_from_slice(&[0xAB; 16]);
        let err = open_document_bytes(&bytes, DocFormat::Oasis).expect_err("must reject");
        assert!(matches!(
            err,
            OpenError::Import {
                format: DocFormat::Oasis,
                ..
            }
        ));
        assert!(err.to_string().contains("OASIS"));
    }

    #[test]
    fn format_from_extension_recognizes_cif_and_dxf() {
        assert_eq!(DocFormat::from_extension("part.cif"), Some(DocFormat::Cif));
        assert_eq!(DocFormat::from_extension("PART.CIF"), Some(DocFormat::Cif));
        assert_eq!(DocFormat::from_extension("part.dxf"), Some(DocFormat::Dxf));
        assert_eq!(DocFormat::from_extension("PART.DXF"), Some(DocFormat::Dxf));
    }

    #[test]
    fn every_format_labels_cleanly() {
        assert_eq!(DocFormat::Gds.label(), "GDSII");
        assert_eq!(DocFormat::Oasis.label(), "OASIS");
        assert_eq!(DocFormat::Cif.label(), "CIF");
        assert_eq!(DocFormat::Dxf.label(), "DXF");
    }

    #[test]
    fn garbage_bytes_never_panic_for_any_format() {
        // Non-empty bytes that are neither valid UTF-8 nor any format's magic:
        // always a clean import error, for every format, never a panic.
        let garbage: &[u8] = b"\xff\xfe\xfd not a layout file";
        for format in [
            DocFormat::Gds,
            DocFormat::Oasis,
            DocFormat::Cif,
            DocFormat::Dxf,
        ] {
            let err = open_document_bytes(garbage, format)
                .expect_err("garbage input is always a clean error");
            assert!(
                matches!(err, OpenError::Import { format: f, .. } if f == format),
                "{format:?}: {err:?}"
            );
        }
    }

    #[test]
    fn empty_bytes_never_panic_for_any_format() {
        // Every format cleanly rejects a zero-length input, though which clean
        // error varies with the format's own grammar: GDS and OASIS (either
        // dialect) require a header or magic string a zero-length input can never
        // carry, so they report `Import`. CIF and DXF are text grammars where
        // zero statements/entities is syntactically valid, merely empty, so they
        // report `Empty` instead (see `opens_a_valid_cif_cleanly`'s sibling case).
        // Neither outcome is a panic; that is the property this test guards.
        for format in [
            DocFormat::Gds,
            DocFormat::Oasis,
            DocFormat::Cif,
            DocFormat::Dxf,
        ] {
            let err =
                open_document_bytes(&[], format).expect_err("empty input is always a clean error");
            assert!(
                matches!(err, OpenError::Import { .. } | OpenError::Empty { .. }),
                "{format:?}: {err:?}"
            );
        }
    }
}
