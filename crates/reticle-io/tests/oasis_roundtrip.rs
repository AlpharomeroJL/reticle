//! Round-trip tests for the in-house OASIS-inspired subset ([`Oasis`]).
//!
//! The subset covers rectangles, polygons, and paths on `(layer, datatype)`,
//! plus cell instances (placements) and arrays, across multiple named cells with
//! preserved top cells. Each of those is exercised here so the documented
//! coverage stays honest.

use reticle_geometry::{Endcap, LayerId, Orientation, Path, Point, Polygon, Rect, Transform};
use reticle_io::Oasis;
use reticle_model::{
    ArrayInstance, Cell, Document, DrawShape, Exporter, Importer, Instance, ShapeKind, Technology,
};

const METAL1: LayerId = LayerId::new(1, 0);
const METAL2: LayerId = LayerId::new(4, 2);

/// Builds a document exercising every supported record: rectangles, polygons, and
/// paths across two layers, plus a placement and an array of a child cell.
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
    // A path with a non-default (custom) end cap exercises the endcap encoding.
    a.shapes.push(DrawShape::new(
        METAL1,
        ShapeKind::Path(Path::new(
            vec![Point::new(0, 0), Point::new(100, 0), Point::new(100, 60)],
            20,
            Endcap::Custom(7),
        )),
    ));
    // A single placement and an array of `cell_b`, with a mirrored orientation to
    // exercise the transform encoding (unit magnification round-trips exactly).
    a.instances.push(Instance {
        cell: "cell_b".to_string(),
        transform: Transform {
            translation: Point::new(250, -40),
            orientation: Orientation::MirrorX90,
            ..Transform::IDENTITY
        },
    });
    a.arrays.push(ArrayInstance {
        cell: "cell_b".to_string(),
        transform: Transform::translate(1000, 2000),
        columns: 3,
        rows: 4,
        column_pitch: 120,
        row_pitch: 90,
    });
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
    assert_eq!(a.shapes.len(), 3);
    let rect = a
        .shapes
        .iter()
        .find(|s| matches!(s.kind, ShapeKind::Rect(_)))
        .unwrap();
    match &rect.kind {
        ShapeKind::Rect(r) => {
            assert_eq!(*r, Rect::new(Point::new(0, 0), Point::new(500, 300)));
            assert_eq!(rect.layer, METAL1);
        }
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
    // The path round-trips its centre-line, width, and (custom) end cap.
    let path = a
        .shapes
        .iter()
        .find(|s| matches!(s.kind, ShapeKind::Path(_)))
        .unwrap();
    match &path.kind {
        ShapeKind::Path(p) => {
            assert_eq!(
                p.points(),
                &[Point::new(0, 0), Point::new(100, 0), Point::new(100, 60)]
            );
            assert_eq!(p.width(), 20);
            assert_eq!(p.endcap(), Endcap::Custom(7));
            assert_eq!(path.layer, METAL1);
        }
        other => panic!("expected path, got {other:?}"),
    }

    // The single placement round-trips its cell reference and full transform.
    assert_eq!(a.instances.len(), 1);
    let inst = &a.instances[0];
    assert_eq!(inst.cell, "cell_b");
    assert_eq!(inst.transform.translation, Point::new(250, -40));
    assert_eq!(inst.transform.orientation, Orientation::MirrorX90);
    assert!(inst.transform.magnification.is_unity());

    // The array round-trips its counts and pitches.
    assert_eq!(a.arrays.len(), 1);
    let arr = &a.arrays[0];
    assert_eq!(arr.cell, "cell_b");
    assert_eq!(arr.transform.translation, Point::new(1000, 2000));
    assert_eq!(arr.columns, 3);
    assert_eq!(arr.rows, 4);
    assert_eq!(arr.column_pitch, 120);
    assert_eq!(arr.row_pitch, 90);

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
fn oasis_roundtrips_paths_with_each_endcap() {
    // Every end-cap variant, including the custom extension, survives a round-trip.
    for (i, endcap) in [
        Endcap::Flat,
        Endcap::Square,
        Endcap::Round,
        Endcap::Custom(13),
    ]
    .into_iter()
    .enumerate()
    {
        let mut doc = Document::new();
        let mut cell = Cell::new("wire");
        cell.shapes.push(DrawShape::new(
            METAL1,
            ShapeKind::Path(Path::new(
                vec![Point::new(0, 0), Point::new(100, i as i32 * 10)],
                20,
                endcap,
            )),
        ));
        doc.insert_cell(cell);

        let bytes = Oasis.export(&doc).expect("export should succeed");
        let imported = Oasis.import(&bytes).expect("import should succeed");
        let cell = imported.cell("wire").expect("wire present");
        assert_eq!(cell.shapes.len(), 1);
        match &cell.shapes[0].kind {
            ShapeKind::Path(p) => assert_eq!(p.endcap(), endcap),
            other => panic!("expected path, got {other:?}"),
        }
    }
}

#[test]
fn oasis_roundtrips_instances() {
    // A placement (formerly unsupported) now round-trips its cell and transform.
    let mut doc = Document::new();
    let mut cell = Cell::new("parent");
    cell.instances.push(Instance {
        cell: "child".to_string(),
        transform: Transform::translate(10, 10),
    });
    doc.insert_cell(cell);
    doc.insert_cell(Cell::new("child"));

    let bytes = Oasis.export(&doc).expect("export should succeed");
    let imported = Oasis.import(&bytes).expect("import should succeed");
    let parent = imported.cell("parent").expect("parent present");
    assert_eq!(parent.instances.len(), 1);
    assert_eq!(parent.instances[0].cell, "child");
    assert_eq!(
        parent.instances[0].transform.translation,
        Point::new(10, 10)
    );
}

#[test]
fn oasis_rejects_garbage_and_truncation() {
    // Not our container.
    assert!(Oasis.import(b"not oasis at all").is_err());
    // Correct magic but truncated before the START payload.
    let mut short = b"RETICLE-OASIS\0".to_vec();
    short.push(2); // version
    short.push(0x01); // START tag, but no payload follows
    assert!(Oasis.import(&short).is_err());
}
