//! Round-trip tests for the in-house OASIS-inspired subset ([`Oasis`]).
//!
//! The subset covers rectangles and polygons on `(layer, datatype)` across
//! multiple named cells with preserved top cells. Paths, instances, and arrays
//! are reported as unsupported rather than silently dropped, those cases are
//! asserted here too, to keep the documented coverage honest.

use reticle_geometry::{LayerId, Point, Polygon, Rect, Transform};
use reticle_io::Oasis;
use reticle_model::{
    Cell, Document, DrawShape, Exporter, Importer, Instance, ModelError, ShapeKind, Technology,
};

const METAL1: LayerId = LayerId::new(1, 0);
const METAL2: LayerId = LayerId::new(4, 2);

/// Builds a document with two cells of rectangles and polygons on two layers.
fn sample_document() -> Document {
    let mut doc = Document::new();

    let mut a = Cell::new("cell_a");
    a.shapes.push(DrawShape::new(
        METAL1,
        ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(500, 300))),
    ));
    a.shapes.push(DrawShape::new(
        METAL2,
        ShapeKind::Polygon(Polygon::new(vec![
            Point::new(10, 10),
            Point::new(90, 10),
            Point::new(50, 80),
        ])),
    ));
    doc.insert_cell(a);

    let mut b = Cell::new("cell_b");
    b.shapes.push(DrawShape::new(
        METAL2,
        ShapeKind::Rect(Rect::new(Point::new(-100, -100), Point::new(-20, -30))),
    ));
    doc.insert_cell(b);

    doc.set_top_cells(vec!["cell_a".to_string(), "cell_b".to_string()]);
    doc.set_technology(Technology {
        name: "oasis_demo".to_string(),
        dbu_per_micron: 2000,
        ..Technology::default()
    });
    doc
}

#[test]
fn oasis_roundtrip_preserves_rectangles_and_polygons() {
    let original = sample_document();
    let bytes = Oasis.export(&original).expect("export should succeed");
    assert!(!bytes.is_empty());

    let imported = Oasis.import(&bytes).expect("import should succeed");

    assert_eq!(imported.cell_count(), 2);
    assert_eq!(imported.technology().dbu_per_micron, 2000);

    let a = imported.cell("cell_a").expect("cell_a present");
    assert_eq!(a.shapes.len(), 2);
    let rect = a.shapes.iter().find(|s| s.layer == METAL1).unwrap();
    match &rect.kind {
        ShapeKind::Rect(r) => assert_eq!(*r, Rect::new(Point::new(0, 0), Point::new(500, 300))),
        other => panic!("expected rect, got {other:?}"),
    }
    let poly = a.shapes.iter().find(|s| s.layer == METAL2).unwrap();
    match &poly.kind {
        ShapeKind::Polygon(p) => assert_eq!(
            p.vertices(),
            &[Point::new(10, 10), Point::new(90, 10), Point::new(50, 80)]
        ),
        other => panic!("expected polygon, got {other:?}"),
    }

    let b = imported.cell("cell_b").expect("cell_b present");
    assert_eq!(b.shapes.len(), 1);
    match &b.shapes[0].kind {
        ShapeKind::Rect(r) => {
            assert_eq!(*r, Rect::new(Point::new(-100, -100), Point::new(-20, -30)));
            assert_eq!(b.shapes[0].layer, METAL2);
        }
        other => panic!("expected rect, got {other:?}"),
    }

    // Top cells preserved.
    let mut tops = imported.top_cells().to_vec();
    tops.sort();
    assert_eq!(tops, vec!["cell_a".to_string(), "cell_b".to_string()]);

    // Layer table reconstructed from geometry.
    let layers: Vec<LayerId> = imported.technology().layers.iter().map(|l| l.id).collect();
    assert!(layers.contains(&METAL1));
    assert!(layers.contains(&METAL2));
}

#[test]
fn oasis_roundtrip_is_idempotent() {
    let doc = sample_document();
    let bytes1 = Oasis.export(&doc).expect("first export");
    let reimported = Oasis.import(&bytes1).expect("import");
    let bytes2 = Oasis.export(&reimported).expect("second export");
    assert_eq!(bytes1, bytes2);
}

#[test]
fn oasis_reports_unsupported_paths() {
    use reticle_geometry::{Endcap, Path};
    let mut doc = Document::new();
    let mut cell = Cell::new("wire");
    cell.shapes.push(DrawShape::new(
        METAL1,
        ShapeKind::Path(Path::new(
            vec![Point::new(0, 0), Point::new(100, 0)],
            20,
            Endcap::Flat,
        )),
    ));
    doc.insert_cell(cell);

    let err = Oasis
        .export(&doc)
        .expect_err("paths are unsupported in the subset");
    assert!(matches!(err, ModelError::Unsupported(_)));
}

#[test]
fn oasis_reports_unsupported_instances() {
    let mut doc = Document::new();
    let mut cell = Cell::new("parent");
    cell.instances.push(Instance {
        cell: "child".to_string(),
        transform: Transform::translate(10, 10),
    });
    doc.insert_cell(cell);

    let err = Oasis
        .export(&doc)
        .expect_err("instances are unsupported in the subset");
    assert!(matches!(err, ModelError::Unsupported(_)));
}

#[test]
fn oasis_rejects_garbage_and_truncation() {
    // Not our container.
    assert!(Oasis.import(b"not oasis at all").is_err());
    // Correct magic but truncated before the START payload.
    let mut short = b"RETICLE-OASIS\0".to_vec();
    short.push(1); // version
    short.push(0x01); // START tag, but no payload follows
    assert!(Oasis.import(&short).is_err());
}
