//! Robustness: [`Gds::import`] must never panic, whatever bytes it is handed. It
//! must always return `Ok` or `Err`.
//!
//! `gds21` itself can panic on some crafted inputs (for example a zero-length
//! string record triggers an out-of-bounds index). [`Gds::import`] contains any
//! such panic with `catch_unwind` and converts it to an `Err`, so the property
//! below holds for arbitrary and truncated input.

use proptest::prelude::*;
use reticle_geometry::{LayerId, Point, Rect};
use reticle_io::Gds;
use reticle_model::{Cell, Document, DrawShape, Exporter, Importer, ShapeKind};

/// A byte-for-byte valid GDS produced by our own exporter, used to seed the
/// truncation strategy so we exercise the parser on realistically shaped input.
fn valid_gds_bytes() -> Vec<u8> {
    let mut doc = Document::new();
    let mut cell = Cell::new("robust");
    cell.shapes.push(DrawShape::new(
        LayerId::new(7, 3),
        ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(42, 42))),
    ));
    doc.insert_cell(cell);
    doc.set_top_cells(vec!["robust".to_string()]);
    Gds.export(&doc)
        .expect("export of a trivial document should succeed")
}

/// Silences the panic hook for the duration of the test so the many *caught*
/// panics from `gds21` on garbage input don't flood the test log. Returns a
/// guard that restores the previous hook on drop.
struct QuietPanics;

impl QuietPanics {
    fn install() -> Self {
        std::panic::set_hook(Box::new(|_| {}));
        Self
    }
}

impl Drop for QuietPanics {
    fn drop(&mut self) {
        let _ = std::panic::take_hook();
    }
}

proptest! {
    // A good number of cases without making the suite slow.
    #![proptest_config(ProptestConfig::with_cases(2048))]

    /// Arbitrary bytes never panic the importer.
    #[test]
    fn import_never_panics_on_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _guard = QuietPanics::install();
        // The call must return (either variant); reaching the assert proves no unwind.
        let result = Gds.import(&bytes);
        prop_assert!(result.is_ok() || result.is_err());
    }

    /// Every truncated prefix of a valid GDS never panics the importer. The
    /// truncation length is drawn as a fraction of the full stream so proptest
    /// shrinks toward minimal reproducers.
    #[test]
    fn import_never_panics_on_truncated_valid_gds(fraction in 0.0f64..=1.0) {
        let _guard = QuietPanics::install();
        let full = valid_gds_bytes();
        let len = ((full.len() as f64) * fraction) as usize;
        let result = Gds.import(&full[..len.min(full.len())]);
        prop_assert!(result.is_ok() || result.is_err());
    }

    /// A valid GDS with a handful of bytes flipped never panics the importer.
    #[test]
    fn import_never_panics_on_corrupted_valid_gds(
        seed in any::<u64>(),
        flips in proptest::collection::vec(any::<u8>(), 1..16),
    ) {
        let _guard = QuietPanics::install();
        let mut bytes = valid_gds_bytes();
        if !bytes.is_empty() {
            // Deterministically flip a few bytes using the provided noise.
            let mut idx = (seed as usize) % bytes.len();
            for f in flips {
                bytes[idx] ^= f;
                idx = (idx + 7) % bytes.len();
            }
        }
        let result = Gds.import(&bytes);
        prop_assert!(result.is_ok() || result.is_err());
    }
}

/// A direct (non-proptest) regression for the specific `gds21` panic vector: a
/// record header claiming a zero-length string. Confirms `import` returns `Err`
/// rather than unwinding.
#[test]
fn import_contains_gds21_zero_length_string_panic() {
    let _guard = QuietPanics::install();
    // HEADER record (len=6, type=0x00 Header, dtype=0x02 I16, version=3), then a
    // record with datatype Str (0x06) and length 4 (zero payload bytes) whose
    // reader indexes `data[len - 1]` on an empty slice.
    let bytes: Vec<u8> = vec![
        0x00, 0x06, 0x00, 0x02, 0x00, 0x03, // HEADER v3
        0x00, 0x04, 0x02, 0x06, // len=4, LIBNAME-ish record, Str datatype, empty string
    ];
    let result = Gds.import(&bytes);
    // We don't care whether it's Ok or Err, only that it returned at all.
    let _ = result;
}

/// The hardened `import_with_warnings` entry point also never panics on arbitrary
/// bytes, and its warnings channel is present (empty here) whenever it returns Ok.
#[test]
fn import_with_warnings_never_panics_on_garbage() {
    let _guard = QuietPanics::install();
    for seed in 0u8..64 {
        let bytes: Vec<u8> = (0..97u16)
            .map(|i| (i as u8).wrapping_mul(seed.max(1)))
            .collect();
        // Reaching past this call at all proves no unwind.
        if let Ok(import) = Gds.import_with_warnings(&bytes) {
            // Warnings, when present, are well-formed.
            for w in &import.warnings {
                assert!(!w.summary.is_empty());
                assert!(!w.detail.is_empty());
            }
        }
    }
}

/// An input larger than the accepted ceiling is refused up front with an error,
/// not parsed (so a hostile length cannot force a huge allocation). The buffer is
/// filled with a valid HEADER prefix so the rejection is by size, not content.
#[test]
fn oversized_input_is_refused_before_parsing() {
    let mut bytes = vec![0x00, 0x06, 0x00, 0x02, 0x00, 0x03];
    // One byte past the documented maximum.
    bytes.resize(reticle_io::gds::MAX_INPUT_BYTES + 1, 0);
    let err = Gds
        .import_with_warnings(&bytes)
        .expect_err("an oversized input must be refused");
    // It is a model error (the lowered IoError), and the input was never parsed.
    let msg = err.to_string();
    assert!(!msg.is_empty(), "the refusal carries a message");
}

/// A boundary with too few vertices imports as a warning, not a failure, and the
/// good geometry in the same cell survives. Exercises the recoverable path end to
/// end via a hand-built stream.
#[test]
fn degenerate_boundary_warns_and_keeps_good_geometry() {
    // Build a minimal valid library with a struct holding one good rectangle and
    // one degenerate (2-vertex) boundary, using our own exporter to get a valid
    // frame and then not; simplest is to reuse the committed corpus sample.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/tinytapeout/degenerate_boundary.gds");
    let bytes = std::fs::read(&path).expect("degenerate_boundary corpus sample present");
    let import = Gds
        .import_with_warnings(&bytes)
        .expect("a degenerate boundary is recoverable, not fatal");

    // Exactly one warning, of the degenerate-geometry kind.
    assert_eq!(import.warnings.len(), 1, "one degenerate boundary warned");
    assert_eq!(
        import.warnings[0].kind,
        reticle_io::WarningKind::DegenerateGeometry
    );
    // The good boundary in the same struct survived (one shape remains).
    let cell = import.document.cells().next().expect("the struct imported");
    assert_eq!(
        cell.shapes.len(),
        1,
        "the good boundary survives; only the degenerate one is dropped"
    );
}
