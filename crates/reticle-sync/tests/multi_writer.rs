//! Multi-writer collaboration: convergence and per-actor selective undo (ADR 0081).
//!
//! These are the review-critical properties of the write-multi lane:
//!
//! * **Convergence** - two editors making disjoint AND conflicting edits, then
//!   exchanging updates both ways, reach a byte-identical materialized document.
//! * **Selective undo that converges** - each editor's [`SyncDocument::undo`]
//!   reverts only that editor's own last edit and leaves the other editor's edit
//!   intact; after re-exchanging updates the two peers still converge.

use std::fmt::Write as _;

use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, DrawShape, ShapeKind};
use reticle_sync::SyncDocument;

/// A rectangle shape helper on layer `l`.
fn rect_shape(l: u16, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
    DrawShape::new(
        LayerId::new(l, 0),
        ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
    )
}

/// Exchanges everything each peer is missing from the other, both ways.
fn exchange(a: &mut SyncDocument, b: &mut SyncDocument) {
    let sv_a = a.state_vector();
    let sv_b = b.state_vector();
    let a_to_b = a.encode_update(&sv_b).expect("encode a->b");
    let b_to_a = b.encode_update(&sv_a).expect("encode b->a");
    a.apply_update(&b_to_a).expect("apply b->a");
    b.apply_update(&a_to_b).expect("apply a->b");
}

/// A canonical byte-level witness of a materialized document.
///
/// Convergence is `a.document() == b.document()`; this additionally encodes the
/// document to a deterministic byte string so two converged peers can be compared
/// byte-for-byte. Cells are sorted by name (the document's own `cells` map has no
/// intrinsic order), and each cell's shapes/instances/arrays are already ordered
/// deterministically by the CRDT materializer (sorted by element id), so the
/// resulting bytes are identical on every converged peer.
fn state_bytes(doc: &SyncDocument) -> Vec<u8> {
    let d = doc.document();
    let mut cells: Vec<_> = d.cells().collect();
    cells.sort_by(|a, b| a.name.cmp(&b.name));
    let mut out = String::new();
    for cell in cells {
        let _ = writeln!(out, "{cell:#?}");
    }
    let _ = writeln!(out, "top_cells={:?}", d.top_cells());
    out.into_bytes()
}

#[test]
fn two_writers_disjoint_and_conflicting_edits_converge_byte_identical() {
    let mut a = SyncDocument::new("alice");
    let mut b = SyncDocument::new("bob");

    // Disjoint: each writer creates a private cell.
    a.add_cell(&Cell::new("a_only"));
    a.add_shape("a_only", &rect_shape(1, 0, 0, 10, 10));
    b.add_cell(&Cell::new("b_only"));
    b.add_shape("b_only", &rect_shape(2, 20, 20, 30, 30));

    // Conflicting: both writers add shapes to the SAME shared cell concurrently.
    a.add_empty_cell("shared");
    b.add_empty_cell("shared");
    a.add_shape("shared", &rect_shape(3, 0, 0, 4, 4));
    b.add_shape("shared", &rect_shape(3, 8, 8, 12, 12));

    exchange(&mut a, &mut b);

    // Structural convergence...
    assert_eq!(
        a.document(),
        b.document(),
        "two writers did not converge to an identical document"
    );
    // ...and byte-identical materialized state.
    assert_eq!(
        state_bytes(&a),
        state_bytes(&b),
        "converged documents are not byte-identical"
    );

    // Both private cells and the union of the shared cell are present on both.
    let doc = a.document();
    assert!(doc.cell("a_only").is_some());
    assert!(doc.cell("b_only").is_some());
    assert_eq!(
        doc.cell("shared").expect("shared cell").shapes.len(),
        2,
        "the shared cell holds the union of both writers' shapes"
    );
}

#[test]
fn undo_reverts_only_the_local_actors_edit_and_still_converges() {
    // Two editors share a cell; each adds one distinct shape, they sync, then A
    // undoes. A's undo must remove ONLY A's shape and leave B's shape, and after a
    // re-exchange both peers must converge on the same {B's shape} document.
    let mut a = SyncDocument::new("alice");
    let mut b = SyncDocument::new("bob");

    a.add_empty_cell("shared");
    b.add_empty_cell("shared");

    // A distinctive layer per author so we can identify whose shape survives.
    let a_layer = 10u16;
    let b_layer = 20u16;
    a.add_shape("shared", &rect_shape(a_layer, 0, 0, 4, 4));
    b.add_shape("shared", &rect_shape(b_layer, 8, 8, 12, 12));

    // Interleave: both edits are now on both peers.
    exchange(&mut a, &mut b);
    assert_eq!(a.document(), b.document(), "converge before undo");
    assert_eq!(
        a.document().cell("shared").unwrap().shapes.len(),
        2,
        "both shapes present before undo"
    );

    // A undoes its own last edit. B's applied edit is NOT on A's undo stack.
    assert!(a.can_undo(), "A has a local edit to undo");
    assert!(a.undo(), "A undoes its own last edit");

    // Locally on A: only B's shape remains.
    let a_shapes = &a.document().cell("shared").unwrap().shapes;
    assert_eq!(a_shapes.len(), 1, "A's own shape is gone, B's remains");
    assert_eq!(
        a_shapes[0].layer.layer, b_layer,
        "the surviving shape is B's, not A's"
    );

    // Propagate the undo to B and converge.
    exchange(&mut a, &mut b);
    assert_eq!(
        a.document(),
        b.document(),
        "peers do not converge after a selective undo"
    );
    assert_eq!(
        state_bytes(&a),
        state_bytes(&b),
        "post-undo converged state is not byte-identical"
    );

    // The converged document keeps exactly B's shape.
    let shared = a.document().cell("shared").unwrap();
    assert_eq!(
        shared.shapes.len(),
        1,
        "only B's shape survives on both peers"
    );
    assert_eq!(shared.shapes[0].layer.layer, b_layer);
}

#[test]
fn each_actor_undoes_independently_and_converges() {
    // Symmetric check: A.undo() removes A's edit, B.undo() removes B's edit, and
    // the doubly-undone document (empty shared cell) still converges.
    let mut a = SyncDocument::new("alice");
    let mut b = SyncDocument::new("bob");
    a.add_empty_cell("shared");
    b.add_empty_cell("shared");
    a.add_shape("shared", &rect_shape(10, 0, 0, 4, 4));
    b.add_shape("shared", &rect_shape(20, 8, 8, 12, 12));
    exchange(&mut a, &mut b);

    assert!(a.undo(), "A undoes A's edit");
    assert!(b.undo(), "B undoes B's edit");
    // Each removed only its own shape; neither undo touched the other's.
    exchange(&mut a, &mut b);

    assert_eq!(a.document(), b.document(), "converge after both undo");
    assert_eq!(
        a.document().cell("shared").unwrap().shapes.len(),
        0,
        "both shapes are gone once each author undid their own"
    );
}

#[test]
fn redo_reapplies_only_the_local_edit_and_converges() {
    let mut a = SyncDocument::new("alice");
    let mut b = SyncDocument::new("bob");
    a.add_empty_cell("shared");
    b.add_empty_cell("shared");
    a.add_shape("shared", &rect_shape(10, 0, 0, 4, 4));
    b.add_shape("shared", &rect_shape(20, 8, 8, 12, 12));
    exchange(&mut a, &mut b);

    a.undo();
    exchange(&mut a, &mut b);
    assert_eq!(a.document().cell("shared").unwrap().shapes.len(), 1);

    // A redoes; its own shape returns, B's stays, and they converge to both shapes.
    assert!(a.can_redo(), "A has an undone edit to redo");
    assert!(a.redo(), "A redoes its own edit");
    exchange(&mut a, &mut b);
    assert_eq!(a.document(), b.document(), "converge after redo");
    assert_eq!(
        a.document().cell("shared").unwrap().shapes.len(),
        2,
        "A's redone shape and B's shape are both present"
    );
}

#[test]
fn undoing_does_not_touch_a_remote_edit_applied_after_the_local_edit() {
    // Order matters for CRDT undo: A edits, THEN receives B's edit, THEN undoes.
    // B's edit arrived after A's edit was captured, so it must not be swept up.
    let mut a = SyncDocument::new("alice");
    let mut b = SyncDocument::new("bob");
    a.add_empty_cell("shared");
    a.add_shape("shared", &rect_shape(10, 0, 0, 4, 4));

    // B independently edits the same cell, then A integrates it.
    b.add_empty_cell("shared");
    b.add_shape("shared", &rect_shape(20, 8, 8, 12, 12));
    let sv_a = a.state_vector();
    let b_to_a = b.encode_update(&sv_a).expect("b->a");
    a.apply_update(&b_to_a).expect("apply b->a");
    assert_eq!(
        a.document().cell("shared").unwrap().shapes.len(),
        2,
        "A sees both shapes after integrating B"
    );

    // A undoes: only A's shape leaves.
    assert!(a.undo());
    let shapes = &a.document().cell("shared").unwrap().shapes;
    assert_eq!(
        shapes.len(),
        1,
        "the integrated remote shape survives A's undo"
    );
    assert_eq!(shapes[0].layer.layer, 20, "B's shape is the survivor");
}
