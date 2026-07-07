//! GDSII export is byte-reproducible and never embeds wall-clock time.
//!
//! `gds21`'s writer stamps every BGNLIB/BGNSTR record with `Utc::now` by default,
//! so two exports of the same document (for example two `xtask gen-layout` runs)
//! differed only in their embedded build time. The exporter now writes a fixed
//! date, so identical documents export to byte-identical files.
//!
//! The complementary property, that cell-storage order (a `HashMap`'s randomized
//! iteration order) never leaks into the bytes because both exporters sort cells
//! by name, is pinned separately in `export_order.rs`. This file covers the
//! timestamp behaviour that `export_order.rs` does not exercise.

use chrono::NaiveDate;
use gds21::GdsLibrary;
use reticle_geometry::{LayerId, Point, Rect};
use reticle_io::Gds;
use reticle_model::{Cell, Document, DrawShape, Exporter, ShapeKind};

/// Layer 1 / datatype 0.
const METAL1: LayerId = LayerId::new(1, 0);

/// A representative document with several distinct cell names, one rectangle each.
fn sample_document() -> Document {
    let mut doc = Document::new();
    for name in ["top", "alpha", "middle", "zeta"] {
        let mut cell = Cell::new(name);
        cell.shapes.push(DrawShape::new(
            METAL1,
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(100, 200))),
        ));
        doc.insert_cell(cell);
    }
    doc.set_top_cells(vec!["top".to_string()]);
    doc
}

/// The fixed timestamp the GDSII exporter stamps into every date record, kept in
/// sync with the exporter (and the corpus generator's `valid_dates`).
fn fixed_stamp() -> chrono::NaiveDateTime {
    NaiveDate::from_ymd_opt(2023, 1, 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .expect("2023-01-01T00:00:00 is a valid timestamp")
}

#[test]
fn gds_export_is_byte_reproducible() {
    let doc = sample_document();
    let first = Gds.export(&doc).expect("first GDSII export");
    let second = Gds.export(&doc).expect("second GDSII export");
    assert_eq!(
        first, second,
        "exporting the same document twice must produce identical GDSII bytes"
    );
}

#[test]
fn gds_export_carries_no_wallclock_time() {
    let doc = sample_document();
    let bytes = Gds.export(&doc).expect("GDSII export");
    let lib = GdsLibrary::from_bytes(bytes).expect("re-parse exported GDSII");

    let fixed = fixed_stamp();
    assert_eq!(
        lib.dates.modified, fixed,
        "library modified date must be the fixed stamp, not wall-clock time"
    );
    assert_eq!(
        lib.dates.accessed, fixed,
        "library accessed date must be the fixed stamp, not wall-clock time"
    );
    for strukt in &lib.structs {
        assert_eq!(
            strukt.dates.modified, fixed,
            "struct '{}' modified date must be the fixed stamp, got {}",
            strukt.name, strukt.dates.modified
        );
        assert_eq!(
            strukt.dates.accessed, fixed,
            "struct '{}' accessed date must be the fixed stamp, got {}",
            strukt.name, strukt.dates.accessed
        );
    }
}
