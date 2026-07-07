//! The GDSII AREF (array reference) decode is exact: a hierarchy of arrays exports,
//! re-imports, and flattens to exactly `product(columns * rows) * shapes_per_leaf`
//! leaf shapes.
//!
//! This is the regression test for a filed-then-retracted "AREF COLROW off-by-one"
//! (ADR 0057). The AREF import copies the COLROW counts verbatim and `Document::flatten`
//! loops `0..columns`, so the count is exactly the product of the per-level array sizes.
//! The counts here are deliberately asymmetric (`columns != rows`) across two levels, so
//! an off-by-one in the COLROW encode/decode, or a column/row transposition, changes the
//! leaf count and fails an assertion. The design mirrors the shape of `xtask gen-layout`'s
//! nested square arrays (the "sample" that surfaced the phantom), with distinct
//! per-dimension counts for sharper coverage.

use reticle_geometry::{LayerId, Point, Rect, Transform};
use reticle_io::Gds;
use reticle_model::{ArrayInstance, Cell, Document, DrawShape, Exporter, Importer, ShapeKind};

/// Layer 1 / datatype 0.
const METAL1: LayerId = LayerId::new(1, 0);

/// A leaf cell carrying `shape_count` distinct rectangles on metal 1.
fn leaf_cell(name: &str, shape_count: i32) -> Cell {
    let mut cell = Cell::new(name);
    for i in 0..shape_count {
        let x = i * 100;
        cell.shapes.push(DrawShape::new(
            METAL1,
            ShapeKind::Rect(Rect::new(Point::new(x, 0), Point::new(x + 50, 50))),
        ));
    }
    cell
}

/// A cell holding one `columns x rows` array of `child`, with a non-zero pitch on each
/// axis so the exported AREF carries a valid COLROW plus placement vectors.
fn array_cell(name: &str, child: &str, columns: u32, rows: u32) -> Cell {
    let mut cell = Cell::new(name);
    cell.arrays.push(ArrayInstance {
        cell: child.to_owned(),
        transform: Transform::IDENTITY,
        columns,
        rows,
        column_pitch: 1000,
        row_pitch: 1000,
    });
    cell
}

/// Round-trips `doc` through GDSII and returns the flattened leaf-shape count of `top`.
fn reimport_and_flatten(doc: &Document, top: &str) -> usize {
    let bytes = Gds.export(doc).expect("export GDSII");
    let reimported = Gds.import(&bytes).expect("re-import GDSII");
    reimported.flatten(top).len()
}

#[test]
fn nested_asymmetric_arrays_flatten_to_the_exact_product() {
    // leaf (2 shapes) -> level1 (3 x 4 = 12) -> level2/top (2 x 5 = 10).
    // Expected leaves: 2 * 12 * 10 = 240.
    let shapes_per_leaf = 2;
    let mut doc = Document::new();
    doc.insert_cell(leaf_cell("leaf", shapes_per_leaf));
    doc.insert_cell(array_cell("level1", "leaf", 3, 4));
    doc.insert_cell(array_cell("level2", "level1", 2, 5));
    doc.set_top_cells(vec!["level2".to_owned()]);

    let expected = (shapes_per_leaf as usize) * (3 * 4) * (2 * 5);
    assert_eq!(expected, 240, "hand arithmetic check");

    // The direct flatten pins the flatten loop; the round-trip pins the GDSII COLROW
    // encode/decode. An off-by-one in either changes the count.
    assert_eq!(
        doc.flatten("level2").len(),
        expected,
        "direct flatten must place exactly product(columns*rows) * shapes_per_leaf leaves"
    );
    assert_eq!(
        reimport_and_flatten(&doc, "level2"),
        expected,
        "GDSII round-trip must preserve the exact array leaf count"
    );
}

#[test]
fn single_row_and_single_column_arrays_are_exact() {
    // Degenerate dimensions are where an off-by-one hides most easily.
    for (columns, rows) in [(6_u32, 1_u32), (1, 8)] {
        let mut doc = Document::new();
        doc.insert_cell(leaf_cell("unit", 1));
        doc.insert_cell(array_cell("top", "unit", columns, rows));
        doc.set_top_cells(vec!["top".to_owned()]);

        let expected = (columns * rows) as usize;
        assert_eq!(
            doc.flatten("top").len(),
            expected,
            "direct flatten of a {columns}x{rows} array"
        );
        assert_eq!(
            reimport_and_flatten(&doc, "top"),
            expected,
            "round-trip of a {columns}x{rows} array"
        );
    }
}
