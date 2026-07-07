//! The sharer-side publish primitive: a persistent [`SyncDocument`] reconciled to the
//! editable document on each change (ADR 0063). This is the regression guard for the
//! Wave 1 review's highest finding: rebuilding a fresh document per publish reset the
//! `yrs` clock, so a viewer dropped every publish after the first as duplicate struct
//! ids. `reconcile_to` mutates one long-lived document instead, so a viewer converges.

use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, Document, DrawShape, ShapeKind};
use reticle_sync::SyncDocument;

const MET1: LayerId = LayerId::new(68, 20);

fn doc_with(rects: &[Rect]) -> Document {
    let mut cell = Cell::new("top");
    for r in rects {
        cell.shapes.push(DrawShape::new(MET1, ShapeKind::Rect(*r)));
    }
    let mut d = Document::new();
    d.insert_cell(cell);
    d.set_top_cells(vec!["top".to_string()]);
    d
}

fn shapes(doc: &SyncDocument) -> usize {
    doc.document().cell("top").map_or(0, |c| c.shapes.len())
}

#[test]
fn a_persistent_reconciled_sharer_lets_a_viewer_converge_across_publishes() {
    let a = Rect::new(Point::new(0, 0), Point::new(10, 10));
    let b = Rect::new(Point::new(20, 0), Point::new(30, 10));

    // ONE long-lived sharer document, exactly as the fixed app keeps it.
    let mut sharer = SyncDocument::new(reticle_sync_actor_sharer());
    let mut viewer = SyncDocument::new("viewer");

    // Publish 1: the editable document is [A]; send full state.
    sharer.reconcile_to(&doc_with(&[a]));
    viewer
        .apply_update(&sharer.encode_state_update())
        .expect("viewer applies publish 1");
    assert_eq!(shapes(&viewer), 1, "viewer has A after the first publish");

    // Publish 2: the user adds B ([A, B]); send the incremental delta.
    let sv = sharer.state_vector();
    sharer.reconcile_to(&doc_with(&[a, b]));
    viewer
        .apply_update(&sharer.encode_update(&sv).expect("delta encodes"))
        .expect("viewer applies publish 2");
    assert_eq!(
        shapes(&viewer),
        2,
        "viewer sees B, added AFTER the first publish (the finding-1 regression)"
    );

    // Publish 3: an offline delete of A ([B]); send the delta.
    let sv = sharer.state_vector();
    sharer.reconcile_to(&doc_with(&[b]));
    viewer
        .apply_update(&sharer.encode_update(&sv).expect("delta encodes"))
        .expect("viewer applies publish 3");
    assert_eq!(shapes(&viewer), 1, "viewer sees the delete reach it");

    // The reconnect case: a fresh viewer joins late and gets ONE full-state snapshot.
    let mut latecomer = SyncDocument::new("latecomer");
    latecomer
        .apply_update(&sharer.encode_state_update())
        .expect("late viewer applies the snapshot");
    assert_eq!(
        shapes(&latecomer),
        1,
        "a late viewer materializes the current state"
    );
}

/// The sharer actor id, kept in sync with `crate::livesync::SHARER_ACTOR` in the app.
fn reticle_sync_actor_sharer() -> &'static str {
    "sharer"
}
