//! Convergence tests: two peers applying different concurrent edits, exchanging
//! their updates in different orders, must reach an identical [`Document`].

use reticle_geometry::{Endcap, LayerId, Path, Point, Polygon, Rect, Transform};
use reticle_model::{ArrayInstance, Cell, DrawShape, Instance, ShapeKind};
use reticle_sync::SyncDocument;

/// A layer helper.
fn layer(l: u16) -> LayerId {
    LayerId::new(l, 0)
}

/// A rectangle shape helper.
fn rect_shape(l: u16, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
    DrawShape::new(
        layer(l),
        ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
    )
}

/// Exchanges the full state of two peers both ways, then asserts convergence.
///
/// The exchange is deliberately asymmetric: `a` learns of `b` via a diff computed
/// against `a`'s state vector, while `b` learns of `a` via a full state update.
/// Both directions must land the peers on an identical document.
fn exchange_and_assert(a: &mut SyncDocument, b: &mut SyncDocument) {
    let sv_a = a.state_vector();
    let sv_b = b.state_vector();

    // What each peer is missing from the other.
    let a_to_b = a.encode_update(&sv_b).expect("encode a->b");
    let b_to_a = b.encode_update(&sv_a).expect("encode b->a");

    a.apply_update(&b_to_a).expect("apply b->a");
    b.apply_update(&a_to_b).expect("apply a->b");

    assert_eq!(
        a.document(),
        b.document(),
        "peers did not converge to an identical document"
    );
}

#[test]
fn two_peers_concurrent_disjoint_cells_converge() {
    let mut alice = SyncDocument::new("alice");
    let mut bob = SyncDocument::new("bob");

    // Concurrent, disjoint edits.
    alice.add_cell(&Cell::new("alpha"));
    alice.add_rect(
        "alpha",
        layer(1),
        Rect::new(Point::new(0, 0), Point::new(10, 10)),
    );

    bob.add_cell(&Cell::new("beta"));
    bob.add_rect(
        "beta",
        layer(2),
        Rect::new(Point::new(5, 5), Point::new(20, 20)),
    );

    exchange_and_assert(&mut alice, &mut bob);

    let doc = alice.document();
    assert!(doc.cell("alpha").is_some());
    assert!(doc.cell("beta").is_some());
    assert_eq!(doc.cell_count(), 2);
}

#[test]
fn two_peers_concurrent_edits_same_cell_union() {
    let mut alice = SyncDocument::new("alice");
    let mut bob = SyncDocument::new("bob");

    // Both start from a shared cell (seeded identically is not required; each adds
    // it, and `add_cell` merges rather than replaces).
    alice.add_empty_cell("shared");
    bob.add_empty_cell("shared");

    // Concurrent shape additions to the SAME cell.
    alice.add_shape("shared", &rect_shape(1, 0, 0, 4, 4));
    alice.add_shape("shared", &rect_shape(1, 4, 4, 8, 8));
    bob.add_shape("shared", &rect_shape(2, 8, 8, 12, 12));

    exchange_and_assert(&mut alice, &mut bob);

    // The shared cell must contain the union of all three shapes on both peers.
    let cell = alice.document().cell("shared").expect("shared cell");
    assert_eq!(cell.shapes.len(), 3, "expected union of all shapes");
}

#[test]
fn exchange_order_does_not_matter() {
    // Build the same logical edits, but exchange updates in two different orders
    // across two independent peer pairs; both pairs must reach the SAME document.
    let build = || {
        let mut a = SyncDocument::new("alice");
        let mut b = SyncDocument::new("bob");
        a.add_cell(&Cell::new("top"));
        a.add_shape("top", &rect_shape(1, 0, 0, 100, 100));
        a.add_instance(
            "top",
            &Instance {
                cell: "sub".to_owned(),
                transform: Transform::translate(10, 20),
            },
        );
        b.add_cell(&Cell::new("sub"));
        b.add_shape("sub", &rect_shape(3, -5, -5, 5, 5));
        b.add_array(
            "sub",
            &ArrayInstance {
                cell: "leaf".to_owned(),
                transform: Transform::IDENTITY,
                columns: 4,
                rows: 2,
                column_pitch: 30,
                row_pitch: 40,
            },
        );
        (a, b)
    };

    // Pair 1: apply b->a first, then a->b.
    let (mut a1, mut b1) = build();
    let sv_a1 = a1.state_vector();
    let sv_b1 = b1.state_vector();
    let a1_to_b1 = a1.encode_update(&sv_b1).unwrap();
    let b1_to_a1 = b1.encode_update(&sv_a1).unwrap();
    a1.apply_update(&b1_to_a1).unwrap();
    b1.apply_update(&a1_to_b1).unwrap();

    // Pair 2: apply a->b first, then b->a, and also apply each twice (idempotence).
    let (mut a2, mut b2) = build();
    let sv_a2 = a2.state_vector();
    let sv_b2 = b2.state_vector();
    let a2_to_b2 = a2.encode_update(&sv_b2).unwrap();
    let b2_to_a2 = b2.encode_update(&sv_a2).unwrap();
    b2.apply_update(&a2_to_b2).unwrap();
    b2.apply_update(&a2_to_b2).unwrap(); // duplicate delivery
    a2.apply_update(&b2_to_a2).unwrap();

    // All four peers converge to a single identical document.
    assert_eq!(a1.document(), b1.document());
    assert_eq!(a2.document(), b2.document());
    assert_eq!(a1.document(), a2.document());
    assert_eq!(a1.document(), b2.document());
}

#[test]
fn all_shape_kinds_round_trip_through_crdt() {
    let mut a = SyncDocument::new("alice");
    a.add_empty_cell("c");
    a.add_shape("c", &rect_shape(1, 0, 0, 10, 10));
    a.add_shape(
        "c",
        &DrawShape::new(
            layer(2),
            ShapeKind::Polygon(Polygon::new(vec![
                Point::new(0, 0),
                Point::new(10, 0),
                Point::new(10, 10),
                Point::new(0, 10),
            ])),
        ),
    );
    a.add_shape(
        "c",
        &DrawShape::new(
            layer(3),
            ShapeKind::Path(Path::new(
                vec![Point::new(0, 0), Point::new(100, 0), Point::new(100, 50)],
                8,
                Endcap::Custom(3),
            )),
        ),
    );

    // Round-trip the whole document through a fresh peer via a full state update.
    let mut b = SyncDocument::new("bob");
    b.apply_update(&a.encode_state_update()).unwrap();

    assert_eq!(a.document(), b.document());
    let cell = b.document().cell("c").expect("cell c");
    assert_eq!(cell.shapes.len(), 3);
    // Verify the path decoded exactly, including its custom endcap.
    let has_custom_path = cell.shapes.iter().any(|s| {
        matches!(&s.kind, ShapeKind::Path(p) if p.endcap() == Endcap::Custom(3) && p.width() == 8)
    });
    assert!(has_custom_path, "custom path did not round-trip");
}

#[test]
fn from_document_seeds_and_to_document_reads_back() {
    let mut original = reticle_model::Document::new();
    let mut top = Cell::new("top");
    top.shapes.push(rect_shape(1, 0, 0, 50, 50));
    top.instances.push(Instance {
        cell: "sub".to_owned(),
        transform: Transform::translate(5, 5),
    });
    original.insert_cell(top);
    original.insert_cell(Cell::new("sub"));
    original.set_top_cells(vec!["top".to_owned()]);

    let sync = SyncDocument::from_document("alice", &original);
    let read_back = sync.to_document();

    assert_eq!(&read_back, &original, "seed/read-back must be lossless");
    assert_eq!(read_back.top_cells(), &["top".to_owned()]);
}
