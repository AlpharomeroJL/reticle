//! Property test for the transactional editor: undoing then redoing every edit
//! must return to the exact same document, and undoing all edits must return to
//! the initial state.

use proptest::prelude::*;
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, Document, DocumentStore, DrawShape, Edit, EditableDocument, ShapeKind};

fn rect_shape(value: i32) -> DrawShape {
    DrawShape::new(
        LayerId::new(1, 0),
        ShapeKind::Rect(Rect::new(
            Point::new(value, value),
            Point::new(value + 1, value + 1),
        )),
    )
}

proptest! {
    #[test]
    fn undo_then_redo_is_identity(values in prop::collection::vec(0i32..1000, 0..40)) {
        let mut editor = EditableDocument::new(Document::new());
        editor.apply(Edit::AddCell { cell: Cell::new("c") }).unwrap();
        for value in &values {
            editor
                .apply(Edit::AddShape { cell: "c".to_owned(), shape: rect_shape(*value) })
                .unwrap();
        }
        let full = editor.document().clone();
        let depth = editor.undo_depth();

        for _ in 0..depth {
            prop_assert!(editor.undo());
        }
        // Undoing every edit (including the AddCell) empties the document.
        prop_assert_eq!(editor.document().cell_count(), 0);

        for _ in 0..depth {
            prop_assert!(editor.redo());
        }
        prop_assert_eq!(editor.document(), &full);
    }
}
