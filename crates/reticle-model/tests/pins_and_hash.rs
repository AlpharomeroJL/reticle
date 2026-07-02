//! Label edit round-trips and document-hash determinism.

use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{
    Cell, DocumentStore, DrawShape, Edit, EditableDocument, Label, ShapeKind, document_hash,
};

fn seeded() -> EditableDocument {
    let mut doc = EditableDocument::default();
    doc.apply(Edit::AddCell {
        cell: Cell::new("top"),
    })
    .expect("add cell");
    doc
}

#[test]
fn add_label_then_undo_reverts() {
    let mut doc = seeded();
    let label = Label::new("VDD", Point::new(5, 5), LayerId::new(68, 5));
    doc.apply(Edit::AddLabel {
        cell: "top".into(),
        label: label.clone(),
    })
    .expect("add label");
    assert_eq!(doc.document().cell("top").unwrap().labels, vec![label]);

    assert!(doc.undo());
    assert!(doc.document().cell("top").unwrap().labels.is_empty());
}

#[test]
fn remove_label_round_trips_through_undo() {
    let mut doc = seeded();
    let a = Label::new("A", Point::new(0, 0), LayerId::new(67, 5));
    let b = Label::new("B", Point::new(1, 1), LayerId::new(67, 5));
    doc.apply(Edit::AddLabel {
        cell: "top".into(),
        label: a.clone(),
    })
    .unwrap();
    doc.apply(Edit::AddLabel {
        cell: "top".into(),
        label: b.clone(),
    })
    .unwrap();
    doc.apply(Edit::RemoveLabel {
        cell: "top".into(),
        index: 0,
    })
    .expect("remove label");
    assert_eq!(doc.document().cell("top").unwrap().labels, vec![b.clone()]);

    // Undo restores the removed label at its original index.
    assert!(doc.undo());
    assert_eq!(doc.document().cell("top").unwrap().labels, vec![a, b]);
}

#[test]
fn document_hash_is_deterministic_and_sensitive() {
    // Re-executing the same edits reproduces the hash.
    let mut a = seeded();
    let mut b = seeded();
    assert_eq!(document_hash(a.document()), document_hash(b.document()));

    let shape = DrawShape::new(
        LayerId::new(68, 20),
        ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(10, 10))),
    );
    let before = document_hash(a.document());
    a.apply(Edit::AddShape {
        cell: "top".into(),
        shape: shape.clone(),
    })
    .unwrap();
    assert_ne!(
        document_hash(a.document()),
        before,
        "a mutation must change the hash"
    );

    // The same mutation on b converges to the same hash.
    b.apply(Edit::AddShape {
        cell: "top".into(),
        shape,
    })
    .unwrap();
    assert_eq!(document_hash(a.document()), document_hash(b.document()));
}
