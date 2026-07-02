//! Tests for transactional editing, flattening, and recursive bounding boxes.

use reticle_geometry::{LayerId, Point, Rect, Transform};
use reticle_model::{
    ArrayInstance, Cell, Document, DocumentStore, DrawShape, Edit, EditableDocument, Instance,
    ShapeKind,
};

fn rect_shape(layer: u16, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
    DrawShape::new(
        LayerId::new(layer, 0),
        ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
    )
}

#[test]
fn apply_undo_redo_add_shape() {
    let mut editor = EditableDocument::new(Document::new());
    editor
        .apply(Edit::AddCell {
            cell: Cell::new("top"),
        })
        .unwrap();
    editor
        .apply(Edit::AddShape {
            cell: "top".to_owned(),
            shape: rect_shape(1, 0, 0, 10, 10),
        })
        .unwrap();
    assert_eq!(editor.document().cell("top").unwrap().shapes.len(), 1);

    assert!(editor.undo());
    assert_eq!(editor.document().cell("top").unwrap().shapes.len(), 0);

    assert!(editor.redo());
    assert_eq!(editor.document().cell("top").unwrap().shapes.len(), 1);
}

#[test]
fn undo_all_returns_to_initial() {
    let mut editor = EditableDocument::new(Document::new());
    editor
        .apply(Edit::AddCell {
            cell: Cell::new("a"),
        })
        .unwrap();
    editor
        .apply(Edit::AddShape {
            cell: "a".to_owned(),
            shape: rect_shape(1, 0, 0, 5, 5),
        })
        .unwrap();
    editor
        .apply(Edit::AddShape {
            cell: "a".to_owned(),
            shape: rect_shape(2, 5, 5, 10, 10),
        })
        .unwrap();

    while editor.undo() {}
    assert_eq!(editor.document().cell_count(), 0);
    assert_eq!(editor.undo_depth(), 0);
    assert_eq!(editor.redo_depth(), 3);
}

#[test]
fn remove_shape_undo_restores_at_index() {
    let mut editor = EditableDocument::new(Document::new());
    editor
        .apply(Edit::AddCell {
            cell: Cell::new("c"),
        })
        .unwrap();
    for layer in 1..=3u16 {
        editor
            .apply(Edit::AddShape {
                cell: "c".to_owned(),
                shape: rect_shape(layer, 0, 0, 1, 1),
            })
            .unwrap();
    }
    editor
        .apply(Edit::RemoveShape {
            cell: "c".to_owned(),
            index: 1,
        })
        .unwrap();
    assert_eq!(editor.document().cell("c").unwrap().shapes.len(), 2);

    editor.undo();
    let shapes = &editor.document().cell("c").unwrap().shapes;
    assert_eq!(shapes.len(), 3);
    assert_eq!(shapes[1].layer, LayerId::new(2, 0));
}

#[test]
fn add_duplicate_cell_errors() {
    let mut editor = EditableDocument::new(Document::new());
    editor
        .apply(Edit::AddCell {
            cell: Cell::new("x"),
        })
        .unwrap();
    assert!(
        editor
            .apply(Edit::AddCell {
                cell: Cell::new("x"),
            })
            .is_err()
    );
}

#[test]
fn flatten_expands_instance_with_transform() {
    let mut doc = Document::new();
    let mut child = Cell::new("child");
    child.shapes.push(rect_shape(1, 0, 0, 10, 10));
    doc.insert_cell(child);

    let mut top = Cell::new("top");
    top.instances.push(Instance {
        cell: "child".to_owned(),
        transform: Transform::translate(100, 200),
    });
    doc.insert_cell(top);

    let flat = doc.flatten("top");
    assert_eq!(flat.len(), 1);
    match &flat[0].kind {
        ShapeKind::Rect(rect) => {
            assert_eq!(*rect, Rect::new(Point::new(100, 200), Point::new(110, 210)));
        }
        other => panic!("expected a rect, got {other:?}"),
    }
}

#[test]
fn flatten_expands_array() {
    let mut doc = Document::new();
    let mut unit = Cell::new("u");
    unit.shapes.push(rect_shape(1, 0, 0, 2, 2));
    doc.insert_cell(unit);

    let mut top = Cell::new("top");
    top.arrays.push(ArrayInstance {
        cell: "u".to_owned(),
        transform: Transform::IDENTITY,
        columns: 3,
        rows: 2,
        column_pitch: 10,
        row_pitch: 10,
    });
    doc.insert_cell(top);

    assert_eq!(doc.flatten("top").len(), 6);
}

/// Builds a two-level hierarchy: a `unit` leaf, a `block` that instances and
/// arrays `unit`, and a `top` that owns a shape plus an instance of `block`.
fn hierarchical_doc() -> Document {
    let mut doc = Document::new();

    let mut unit = Cell::new("unit");
    unit.shapes.push(rect_shape(1, 0, 0, 4, 4));
    doc.insert_cell(unit);

    let mut block = Cell::new("block");
    block.instances.push(Instance {
        cell: "unit".to_owned(),
        transform: Transform::translate(-10, -10),
    });
    block.arrays.push(ArrayInstance {
        cell: "unit".to_owned(),
        transform: Transform::IDENTITY,
        columns: 4,
        rows: 3,
        column_pitch: 20,
        row_pitch: 20,
    });
    doc.insert_cell(block);

    let mut top = Cell::new("top");
    top.shapes.push(rect_shape(2, -50, -50, -40, -40));
    top.instances.push(Instance {
        cell: "block".to_owned(),
        transform: Transform::translate(100, 100),
    });
    doc.insert_cell(top);

    doc
}

#[test]
fn cached_cell_bbox_equals_uncached_recompute() {
    let doc = hierarchical_doc();
    let editor = EditableDocument::new(doc.clone());
    for name in ["unit", "block", "top", "missing"] {
        let uncached = doc.cell_bbox(name);
        // First call populates the cache; second call hits it. Both must match.
        assert_eq!(editor.cell_bbox(name), uncached, "first read of {name}");
        assert_eq!(editor.cell_bbox(name), uncached, "cached read of {name}");
    }
}

#[test]
fn add_shape_invalidates_cached_bbox() {
    let mut editor = EditableDocument::new(hierarchical_doc());
    let before = editor.cell_bbox("top").unwrap();
    // Prime the cache for the child too, so we know it is dropped on edit.
    let unit_before = editor.cell_bbox("unit").unwrap();

    // Extend `unit` far to the lower-left; every placement of it grows `top`.
    editor
        .apply(Edit::AddShape {
            cell: "unit".to_owned(),
            shape: rect_shape(1, -1000, -1000, 0, 0),
        })
        .unwrap();

    let unit_after = editor.cell_bbox("unit").unwrap();
    assert_ne!(unit_after, unit_before);
    assert_eq!(unit_after, editor.document().cell_bbox("unit").unwrap());

    let after = editor.cell_bbox("top").unwrap();
    assert_ne!(after, before, "top bbox must reflect the added shape");
    assert_eq!(after, editor.document().cell_bbox("top").unwrap());
}

#[test]
fn remove_shape_invalidates_cached_bbox() {
    let mut editor = EditableDocument::new(Document::new());
    editor
        .apply(Edit::AddCell {
            cell: Cell::new("c"),
        })
        .unwrap();
    editor
        .apply(Edit::AddShape {
            cell: "c".to_owned(),
            shape: rect_shape(1, 0, 0, 10, 10),
        })
        .unwrap();
    editor
        .apply(Edit::AddShape {
            cell: "c".to_owned(),
            shape: rect_shape(1, 90, 90, 100, 100),
        })
        .unwrap();
    let two_shapes = editor.cell_bbox("c").unwrap();
    assert_eq!(
        two_shapes,
        Rect::new(Point::new(0, 0), Point::new(100, 100))
    );

    // Remove the far shape; the cached bbox must shrink to the remaining one.
    editor
        .apply(Edit::RemoveShape {
            cell: "c".to_owned(),
            index: 1,
        })
        .unwrap();
    let one_shape = editor.cell_bbox("c").unwrap();
    assert_ne!(one_shape, two_shapes);
    assert_eq!(one_shape, Rect::new(Point::new(0, 0), Point::new(10, 10)));
    assert_eq!(one_shape, editor.document().cell_bbox("c").unwrap());
}

#[test]
fn undo_redo_restore_cached_bbox() {
    let mut editor = EditableDocument::new(hierarchical_doc());
    let original = editor.cell_bbox("top").unwrap();

    editor
        .apply(Edit::AddShape {
            cell: "unit".to_owned(),
            shape: rect_shape(1, -1000, -1000, 0, 0),
        })
        .unwrap();
    let edited = editor.cell_bbox("top").unwrap();
    assert_ne!(edited, original);

    // Undo must restore the original cached box.
    assert!(editor.undo());
    let restored = editor.cell_bbox("top").unwrap();
    assert_eq!(restored, original);
    assert_eq!(restored, editor.document().cell_bbox("top").unwrap());

    // Redo must reproduce the edited cached box.
    assert!(editor.redo());
    let redone = editor.cell_bbox("top").unwrap();
    assert_eq!(redone, edited);
    assert_eq!(redone, editor.document().cell_bbox("top").unwrap());
}

/// Applies `edit` to `editor` and asserts the revision advanced by exactly 1.
fn apply_bumps(editor: &mut EditableDocument, edit: Edit) {
    let before = editor.revision();
    editor.apply(edit).unwrap();
    assert_eq!(
        editor.revision(),
        before + 1,
        "apply must bump revision by 1"
    );
}

#[test]
fn every_edit_variant_bumps_revision() {
    let mut editor = EditableDocument::new(Document::new());
    assert_eq!(editor.revision(), 0, "a fresh editor starts at revision 0");

    // One apply per Edit variant, each bumping the revision by exactly 1.
    apply_bumps(
        &mut editor,
        Edit::AddCell {
            cell: Cell::new("leaf"),
        },
    );
    apply_bumps(
        &mut editor,
        Edit::AddCell {
            cell: Cell::new("top"),
        },
    );
    apply_bumps(
        &mut editor,
        Edit::AddShape {
            cell: "leaf".to_owned(),
            shape: rect_shape(1, 0, 0, 10, 10),
        },
    );
    apply_bumps(
        &mut editor,
        Edit::AddInstance {
            cell: "top".to_owned(),
            instance: Instance {
                cell: "leaf".to_owned(),
                transform: Transform::translate(5, 5),
            },
        },
    );
    apply_bumps(
        &mut editor,
        Edit::AddArray {
            cell: "top".to_owned(),
            array: ArrayInstance {
                cell: "leaf".to_owned(),
                transform: Transform::IDENTITY,
                columns: 2,
                rows: 2,
                column_pitch: 20,
                row_pitch: 20,
            },
        },
    );
    apply_bumps(
        &mut editor,
        Edit::RemoveShape {
            cell: "leaf".to_owned(),
            index: 0,
        },
    );
    apply_bumps(
        &mut editor,
        Edit::RemoveCell {
            name: "leaf".to_owned(),
        },
    );
    assert_eq!(editor.revision(), 7);
}

#[test]
fn undo_and_redo_bump_revision() {
    let mut editor = EditableDocument::new(Document::new());
    editor
        .apply(Edit::AddCell {
            cell: Cell::new("c"),
        })
        .unwrap();
    assert_eq!(editor.revision(), 1);

    // Undo applies a mutation (the reverse), so it counts as a revision.
    assert!(editor.undo());
    assert_eq!(editor.revision(), 2, "undo must bump revision");

    // Redo re-applies the edit; that is a mutation too.
    assert!(editor.redo());
    assert_eq!(editor.revision(), 3, "redo must bump revision");
}

#[test]
fn failed_and_noop_operations_leave_revision_unchanged() {
    let mut editor = EditableDocument::new(Document::new());

    // Nothing to undo or redo: no mutation, no bump.
    assert!(!editor.undo());
    assert!(!editor.redo());
    assert_eq!(editor.revision(), 0);

    // A failing apply (shape into a missing cell) must not bump.
    assert!(
        editor
            .apply(Edit::AddShape {
                cell: "missing".to_owned(),
                shape: rect_shape(1, 0, 0, 1, 1),
            })
            .is_err()
    );
    assert_eq!(editor.revision(), 0);

    // A duplicate-cell apply fails and must not bump either.
    editor
        .apply(Edit::AddCell {
            cell: Cell::new("x"),
        })
        .unwrap();
    assert!(
        editor
            .apply(Edit::AddCell {
                cell: Cell::new("x"),
            })
            .is_err()
    );
    assert_eq!(editor.revision(), 1);
}

#[test]
fn cell_bbox_includes_children() {
    let mut doc = Document::new();
    let mut child = Cell::new("child");
    child.shapes.push(rect_shape(1, 0, 0, 10, 10));
    doc.insert_cell(child);

    let mut top = Cell::new("top");
    top.shapes.push(rect_shape(1, -5, -5, 0, 0));
    top.instances.push(Instance {
        cell: "child".to_owned(),
        transform: Transform::translate(100, 100),
    });
    doc.insert_cell(top);

    let bbox = doc.cell_bbox("top").unwrap();
    assert_eq!(bbox.min, Point::new(-5, -5));
    assert_eq!(bbox.max, Point::new(110, 110));
}
