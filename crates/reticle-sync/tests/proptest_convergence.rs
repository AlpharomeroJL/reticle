//! Property test: randomized concurrent operations across two peers, exchanged in
//! a random order, must always converge to an identical [`Document`].

use proptest::prelude::*;
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{DrawShape, ShapeKind};
use reticle_sync::SyncDocument;

/// One randomized operation a peer can perform.
#[derive(Clone, Debug)]
enum Op {
    /// Create an empty cell named `cell{n}`.
    AddCell(u8),
    /// Add a rectangle on `layer` to `cell{n}` (creating the cell if needed).
    AddShape { cell: u8, layer: u16, coord: i32 },
}

/// Strategy for a single operation over a small, deliberately-overlapping name
/// space (so both peers frequently touch the same cells).
fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0u8..4).prop_map(Op::AddCell),
        (0u8..4, 0u16..3, -50i32..50).prop_map(|(cell, layer, coord)| Op::AddShape {
            cell,
            layer,
            coord
        }),
    ]
}

/// Applies one operation to a peer.
fn apply_op(doc: &mut SyncDocument, op: &Op) {
    match op {
        Op::AddCell(n) => doc.add_empty_cell(&format!("cell{n}")),
        Op::AddShape { cell, layer, coord } => {
            let c = *coord;
            let shape = DrawShape::new(
                LayerId::new(*layer, 0),
                ShapeKind::Rect(Rect::new(Point::new(c, c), Point::new(c + 10, c + 10))),
            );
            doc.add_shape(&format!("cell{cell}"), &shape);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Two peers apply independent random op sequences, then reconcile by
    /// exchanging state-vector diffs both ways. The `swap` flag flips which
    /// direction is applied first, exercising order independence.
    #[test]
    fn random_concurrent_ops_converge(
        alice_ops in prop::collection::vec(op_strategy(), 0..12),
        bob_ops in prop::collection::vec(op_strategy(), 0..12),
        swap in any::<bool>(),
        duplicate in any::<bool>(),
    ) {
        let mut alice = SyncDocument::new("alice");
        let mut bob = SyncDocument::new("bob");

        for op in &alice_ops {
            apply_op(&mut alice, op);
        }
        for op in &bob_ops {
            apply_op(&mut bob, op);
        }

        // Compute the differential updates each peer needs from the other.
        let sv_alice = alice.state_vector();
        let sv_bob = bob.state_vector();
        let alice_to_bob = alice.encode_update(&sv_bob).unwrap();
        let bob_to_alice = bob.encode_update(&sv_alice).unwrap();

        // Apply in an order chosen by `swap`, optionally delivering duplicates.
        if swap {
            bob.apply_update(&alice_to_bob).unwrap();
            alice.apply_update(&bob_to_alice).unwrap();
        } else {
            alice.apply_update(&bob_to_alice).unwrap();
            bob.apply_update(&alice_to_bob).unwrap();
        }
        if duplicate {
            alice.apply_update(&bob_to_alice).unwrap();
            bob.apply_update(&alice_to_bob).unwrap();
        }

        prop_assert_eq!(
            alice.document(),
            bob.document(),
            "peers diverged after random concurrent edits"
        );
    }

    /// A stronger three-peer variant: a third peer receives both peers' full
    /// state updates in the opposite order and must match too.
    #[test]
    fn third_peer_full_state_matches(
        alice_ops in prop::collection::vec(op_strategy(), 0..10),
        bob_ops in prop::collection::vec(op_strategy(), 0..10),
    ) {
        let mut alice = SyncDocument::new("alice");
        let mut bob = SyncDocument::new("bob");
        for op in &alice_ops {
            apply_op(&mut alice, op);
        }
        for op in &bob_ops {
            apply_op(&mut bob, op);
        }

        // Reconcile alice and bob.
        let sv_alice = alice.state_vector();
        let sv_bob = bob.state_vector();
        let a2b = alice.encode_update(&sv_bob).unwrap();
        let b2a = bob.encode_update(&sv_alice).unwrap();
        alice.apply_update(&b2a).unwrap();
        bob.apply_update(&a2b).unwrap();

        // A third peer applies full states in the reverse order.
        let mut carol = SyncDocument::new("carol");
        carol.apply_update(&bob.encode_state_update()).unwrap();
        carol.apply_update(&alice.encode_state_update()).unwrap();

        prop_assert_eq!(alice.document(), bob.document());
        prop_assert_eq!(alice.document(), carol.document());
    }
}
