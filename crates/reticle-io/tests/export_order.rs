//! GDS/OASIS cell output order is deterministic.
//!
//! `Document` stores cells in a `HashMap`, whose iteration order is randomized per
//! process, so both exporters (and `document_hash`) canonicalize by sorting cells by
//! name before writing. This test pins that canonical order DIRECTLY: the same logical
//! document, built with its cells inserted in different orders, must produce the same
//! cell sequence in the export. Without the sort the order would vary across processes,
//! and even across insertion orders within one process (a `HashMap`'s layout depends on
//! insertion order), so a regression that dropped the sort would fail here.

use reticle_geometry::{LayerId, Point, Rect};
use reticle_io::{Gds, Oasis};
use reticle_model::{Cell, Document, DrawShape, Exporter, ShapeKind};

const METAL1: LayerId = LayerId::new(1, 0);

/// A document with named leaf cells inserted in `order`, each carrying one rectangle,
/// and no declared top cells (so the exporter orders all of them by name).
fn doc_with_cells(order: &[&str]) -> Document {
    let mut doc = Document::new();
    for name in order {
        let mut cell = Cell::new(*name);
        cell.shapes.push(DrawShape::new(
            METAL1,
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(100, 100))),
        ));
        doc.insert_cell(cell);
    }
    doc
}

/// The struct (cell) names in an exported GDS, in file order.
fn gds_struct_order(bytes: &[u8]) -> Vec<String> {
    gds21::GdsLibrary::from_bytes(bytes.to_vec())
        .expect("our own export parses")
        .structs
        .iter()
        .map(|s| s.name.clone())
        .collect()
}

#[test]
fn gds_cell_order_is_sorted_and_insertion_order_independent() {
    // Same three cells, inserted in two different (both non-sorted) orders.
    let a = Gds
        .export(&doc_with_cells(&["gamma", "alpha", "beta"]))
        .expect("export a");
    let b = Gds
        .export(&doc_with_cells(&["beta", "gamma", "alpha"]))
        .expect("export b");

    let order_a = gds_struct_order(&a);
    let order_b = gds_struct_order(&b);

    assert_eq!(
        order_a,
        ["alpha", "beta", "gamma"],
        "GDS writes cells in name-sorted order"
    );
    assert_eq!(
        order_a, order_b,
        "GDS cell order does not depend on the insertion order"
    );
}

#[test]
fn oasis_export_is_insertion_order_independent() {
    // OASIS export is byte-reproducible (no timestamp), so the whole file must be
    // identical regardless of the order the cells were inserted.
    let a = Oasis
        .export(&doc_with_cells(&["gamma", "alpha", "beta"]))
        .expect("export a");
    let b = Oasis
        .export(&doc_with_cells(&["beta", "gamma", "alpha"]))
        .expect("export b");
    assert_eq!(
        a, b,
        "OASIS export is identical regardless of cell insertion order"
    );
}
