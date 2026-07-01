//! Integration tests for the document model (the Wave 0 contract).

use reticle_geometry::{LayerId, Point, Rect, Shape, Transform};
use reticle_model::{ArrayInstance, Cell, Document, DrawShape, ShapeKind};

#[test]
fn document_insert_and_lookup() {
    let mut doc = Document::new();
    doc.insert_cell(Cell::new("top"));
    assert!(doc.cell("top").is_some());
    assert_eq!(doc.cell_count(), 1);

    doc.set_top_cells(vec!["top".to_owned()]);
    assert_eq!(doc.top_cells().len(), 1);
    assert_eq!(doc.top_cells()[0], "top");
}

#[test]
fn draw_shape_reports_bbox_and_layer() {
    let shape = DrawShape::new(
        LayerId::new(1, 0),
        ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(10, 4))),
    );
    assert_eq!(shape.layer(), LayerId::new(1, 0));
    assert_eq!(shape.bounding_box().area(), 40);
}

#[test]
fn cell_shapes_bbox_unions_all_shapes() {
    let mut cell = Cell::new("c");
    cell.shapes.push(DrawShape::new(
        LayerId::new(1, 0),
        ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(2, 2))),
    ));
    cell.shapes.push(DrawShape::new(
        LayerId::new(2, 0),
        ShapeKind::Rect(Rect::new(Point::new(5, 5), Point::new(7, 7))),
    ));
    let bbox = cell.shapes_bbox().unwrap();
    assert_eq!(bbox.min, Point::new(0, 0));
    assert_eq!(bbox.max, Point::new(7, 7));
}

#[test]
fn empty_cell_has_no_bbox() {
    assert!(Cell::new("empty").shapes_bbox().is_none());
}

#[test]
fn array_instance_count() {
    let array = ArrayInstance {
        cell: "unit".to_owned(),
        transform: Transform::IDENTITY,
        columns: 10,
        rows: 20,
        column_pitch: 5,
        row_pitch: 5,
    };
    assert_eq!(array.count(), 200);
}
