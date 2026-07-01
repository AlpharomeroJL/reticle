//! GDSII export/import round-trip: build a small document, serialize it with
//! [`Gds`], read it back, and assert geometry, cells, layers, and hierarchy
//! survive the trip.

use reticle_geometry::{LayerId, Orientation, Point, Polygon, Rect, Transform};
use reticle_io::Gds;
use reticle_model::{
    ArrayInstance, Cell, Document, DrawShape, Exporter, Importer, Instance, ShapeKind, Technology,
};

/// Layer 1 / datatype 0 (e.g. metal1).
const METAL1: LayerId = LayerId::new(1, 0);
/// Layer 2 / datatype 0 (e.g. via1).
const VIA1: LayerId = LayerId::new(2, 5);

/// Builds a document with a top cell holding a rectangle and a polygon on two
/// layers, an instance of a child cell, and an array of that child.
fn sample_document() -> Document {
    let mut doc = Document::new();

    // A leaf cell with a single rectangle on METAL1.
    let mut child = Cell::new("leaf");
    child.shapes.push(DrawShape::new(
        METAL1,
        ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(100, 200))),
    ));
    doc.insert_cell(child);

    // The top cell: a rectangle, a (non-rectangular) polygon, one instance, and
    // one array of the leaf.
    let mut top = Cell::new("top");
    top.shapes.push(DrawShape::new(
        METAL1,
        ShapeKind::Rect(Rect::new(Point::new(-50, -50), Point::new(50, 75))),
    ));
    top.shapes.push(DrawShape::new(
        VIA1,
        ShapeKind::Polygon(Polygon::new(vec![
            Point::new(0, 0),
            Point::new(300, 0),
            Point::new(300, 100),
            Point::new(150, 250),
            Point::new(0, 100),
        ])),
    ));
    top.instances.push(Instance {
        cell: "leaf".to_string(),
        transform: Transform {
            translation: Point::new(1000, 2000),
            orientation: Orientation::R90,
            ..Transform::IDENTITY
        },
    });
    top.arrays.push(ArrayInstance {
        cell: "leaf".to_string(),
        transform: Transform::translate(5000, 6000),
        columns: 4,
        rows: 3,
        column_pitch: 250,
        row_pitch: 400,
    });
    doc.insert_cell(top);

    doc.set_top_cells(vec!["top".to_string()]);

    let tech = Technology {
        name: "roundtrip".to_string(),
        dbu_per_micron: 1000,
        ..Technology::default()
    };
    doc.set_technology(tech);
    doc
}

#[test]
fn gds_roundtrip_preserves_geometry_cells_and_layers() {
    let original = sample_document();

    let bytes = Gds.export(&original).expect("export should succeed");
    assert!(!bytes.is_empty(), "GDS output must not be empty");

    let imported = Gds.import(&bytes).expect("import should succeed");

    // Cells survive by name.
    assert_eq!(imported.cell_count(), 2);
    let top = imported.cell("top").expect("top cell present");
    let leaf = imported.cell("leaf").expect("leaf cell present");

    // Database resolution survives via the UNITS record.
    assert_eq!(imported.technology().dbu_per_micron, 1000);

    // The leaf's rectangle survives as a rectangle.
    assert_eq!(leaf.shapes.len(), 1);
    match &leaf.shapes[0].kind {
        ShapeKind::Rect(r) => {
            assert_eq!(*r, Rect::new(Point::new(0, 0), Point::new(100, 200)));
            assert_eq!(leaf.shapes[0].layer, METAL1);
        }
        other => panic!("expected rectangle, got {other:?}"),
    }

    // The top cell has both shapes; find them by layer.
    assert_eq!(top.shapes.len(), 2);
    let rect_shape = top
        .shapes
        .iter()
        .find(|s| s.layer == METAL1)
        .expect("metal1 rect present");
    match &rect_shape.kind {
        ShapeKind::Rect(r) => {
            assert_eq!(*r, Rect::new(Point::new(-50, -50), Point::new(50, 75)));
        }
        other => panic!("expected rectangle, got {other:?}"),
    }
    let poly_shape = top
        .shapes
        .iter()
        .find(|s| s.layer == VIA1)
        .expect("via1 polygon present");
    match &poly_shape.kind {
        ShapeKind::Polygon(p) => {
            assert_eq!(
                p.vertices(),
                &[
                    Point::new(0, 0),
                    Point::new(300, 0),
                    Point::new(300, 100),
                    Point::new(150, 250),
                    Point::new(0, 100),
                ]
            );
        }
        other => panic!("expected polygon, got {other:?}"),
    }

    // The instance survives with its transform (translation + orientation).
    assert_eq!(top.instances.len(), 1);
    let inst = &top.instances[0];
    assert_eq!(inst.cell, "leaf");
    assert_eq!(inst.transform.translation, Point::new(1000, 2000));
    assert_eq!(inst.transform.orientation, Orientation::R90);

    // The array survives with counts and pitches.
    assert_eq!(top.arrays.len(), 1);
    let arr = &top.arrays[0];
    assert_eq!(arr.cell, "leaf");
    assert_eq!(arr.columns, 4);
    assert_eq!(arr.rows, 3);
    assert_eq!(arr.column_pitch, 250);
    assert_eq!(arr.row_pitch, 400);
    assert_eq!(arr.transform.translation, Point::new(5000, 6000));

    // The derived layer table lists both layers used.
    let layers: Vec<LayerId> = imported.technology().layers.iter().map(|l| l.id).collect();
    assert!(layers.contains(&METAL1));
    assert!(layers.contains(&VIA1));

    // Top-cell tracking: only `top` is a root (leaf is referenced).
    assert_eq!(imported.top_cells(), &["top".to_string()]);
}

#[test]
fn gds_roundtrip_is_idempotent() {
    // Export -> import -> export again should reproduce the same bytes, proving
    // the mapping is stable (no drift across cycles).
    let doc = sample_document();
    let bytes1 = Gds.export(&doc).expect("first export");
    let reimported = Gds.import(&bytes1).expect("import");
    let bytes2 = Gds.export(&reimported).expect("second export");
    assert_eq!(
        bytes1, bytes2,
        "GDS export must be stable across a round-trip"
    );
}

#[test]
fn gds_export_defaults_units_when_no_technology() {
    // A document without a technology still exports with the default 1000 DBU/µm.
    let mut doc = Document::new();
    let mut cell = Cell::new("only");
    cell.shapes.push(DrawShape::new(
        METAL1,
        ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(10, 10))),
    ));
    doc.insert_cell(cell);
    doc.set_top_cells(vec!["only".to_string()]);

    let bytes = Gds.export(&doc).expect("export");
    let imported = Gds.import(&bytes).expect("import");
    assert_eq!(imported.technology().dbu_per_micron, 1000);
}
