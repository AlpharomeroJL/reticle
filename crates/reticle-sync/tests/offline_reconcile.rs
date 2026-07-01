//! Offline-then-reconcile: two peers that share a base, disconnect, edit
//! independently while "offline", then reconcile by exchanging updates on
//! reconnect.

use reticle_geometry::Transform;
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, DrawShape, Instance, ShapeKind};
use reticle_sync::SyncDocument;

fn layer(l: u16) -> LayerId {
    LayerId::new(l, 0)
}

fn rect_shape(l: u16, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
    DrawShape::new(
        layer(l),
        ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
    )
}

#[test]
fn offline_edits_reconcile_on_reconnect() {
    // A shared base document both peers start from.
    let mut base = reticle_model::Document::new();
    let mut top = Cell::new("top");
    top.shapes.push(rect_shape(1, 0, 0, 100, 100));
    base.insert_cell(top);
    base.set_top_cells(vec!["top".to_owned()]);

    // Both peers seed from the same base and sync once so they share history.
    let mut alice = SyncDocument::from_document("alice", &base);
    let mut bob = SyncDocument::new("bob");
    bob.apply_update(&alice.encode_state_update()).unwrap();
    assert_eq!(alice.document(), bob.document(), "peers start in sync");

    // Capture each peer's state vector at the moment of disconnect.
    let sv_alice_offline = alice.state_vector();
    let sv_bob_offline = bob.state_vector();

    // --- Offline period: no updates exchanged. ---
    alice.add_shape("top", &rect_shape(2, 10, 10, 20, 20));
    alice.add_cell(&Cell::new("alice_only"));

    bob.add_shape("top", &rect_shape(3, 30, 30, 40, 40));
    bob.add_instance(
        "top",
        &Instance {
            cell: "alice_only".to_owned(),
            transform: Transform::translate(1, 2),
        },
    );

    // Peers have diverged while offline.
    assert_ne!(
        alice.document(),
        bob.document(),
        "offline edits should diverge before reconnect"
    );

    // --- Reconnect: exchange exactly the changes made since the split. ---
    let alice_delta = alice.encode_update(&sv_bob_offline).unwrap();
    let bob_delta = bob.encode_update(&sv_alice_offline).unwrap();

    alice.apply_update(&bob_delta).unwrap();
    bob.apply_update(&alice_delta).unwrap();

    // Converged, and every offline edit survived on both sides.
    assert_eq!(
        alice.document(),
        bob.document(),
        "peers must converge after reconnect"
    );
    let top_cell = alice.document().cell("top").expect("top");
    assert_eq!(
        top_cell.shapes.len(),
        3,
        "base shape plus both offline shapes"
    );
    assert_eq!(top_cell.instances.len(), 1, "bob's offline instance");
    assert!(alice.document().cell("alice_only").is_some());
}

#[test]
fn full_state_resync_after_offline_is_idempotent() {
    // If a peer loses its state vector, a full-state resync must still converge.
    let mut alice = SyncDocument::new("alice");
    alice.add_cell(&Cell::new("a"));
    alice.add_shape("a", &rect_shape(1, 0, 0, 5, 5));

    let mut bob = SyncDocument::new("bob");
    bob.add_cell(&Cell::new("b"));
    bob.add_shape("b", &rect_shape(2, 5, 5, 10, 10));

    // Full-state exchange both ways (no state vectors involved).
    let alice_full = alice.encode_state_update();
    let bob_full = bob.encode_state_update();

    bob.apply_update(&alice_full).unwrap();
    alice.apply_update(&bob_full).unwrap();
    // Re-deliver the same full states again: must be a no-op.
    bob.apply_update(&alice_full).unwrap();
    alice.apply_update(&bob_full).unwrap();

    assert_eq!(alice.document(), bob.document());
    assert_eq!(alice.document().cell_count(), 2);
}
