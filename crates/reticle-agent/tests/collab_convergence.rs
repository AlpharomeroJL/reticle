//! Convergence between the agent collaborator and a concurrent human client.
//!
//! These mirror `reticle-sync`'s own `convergence.rs`, but with one peer driven by
//! the [`AgentCollaborator`] bridge (editing under `AGENT_ACTOR`) and the other a
//! plain human [`SyncDocument`]. They assert two properties:
//!
//! 1. **Order-independent convergence.** Two peers making concurrent edits and
//!    exchanging their updates in either order reach an identical [`Document`].
//! 2. **No half-shape on the wire.** A multi-shape agent step is one CRDT
//!    transaction: the single update it produces materializes to the whole step at
//!    once (all its shapes, never a strict subset), so a concurrent peer never
//!    observes a partially-applied step.

use reticle_agent::{AgentCollaborator, Pacing};
use reticle_agent_api::args::{LayerArg, PointArg, RectArg, TransformArg};
use reticle_agent_api::{AgentCommand, ElementId};
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, Document, ShapeKind};
use reticle_sync::SyncDocument;

/// A met1 rectangle command in `cell`.
fn rect_cmd(cell: &str, x0: i32, y0: i32, x1: i32, y1: i32) -> AgentCommand {
    AgentCommand::AddRect {
        cell: cell.into(),
        layer: LayerArg {
            layer: 68,
            datatype: 20,
        },
        rect: RectArg {
            min: PointArg { x: x0, y: y0 },
            max: PointArg { x: x1, y: y1 },
        },
    }
}

/// A rectangle shape helper for the human peer (which edits `SyncDocument` directly).
fn human_rect(l: u16, x0: i32, y0: i32, x1: i32, y1: i32) -> reticle_model::DrawShape {
    reticle_model::DrawShape::new(
        LayerId::new(l, 0),
        reticle_model::ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
    )
}

/// A `TransformShapes` command addressing the given stable element ids by number.
fn transform_cmd(ids: &[u64], dx: i32, dy: i32) -> AgentCommand {
    AgentCommand::TransformShapes {
        ids: ids.iter().map(|&n| ElementId(n)).collect(),
        transform: TransformArg {
            dx,
            dy,
            ..TransformArg::default()
        },
    }
}

/// A `DeleteShapes` command addressing the given stable element ids by number.
fn delete_cmd(ids: &[u64]) -> AgentCommand {
    AgentCommand::DeleteShapes {
        ids: ids.iter().map(|&n| ElementId(n)).collect(),
    }
}

/// Whether `cell` in `doc` holds a met1 (layer 68) rectangle exactly equal to `want`.
fn has_rect(doc: &Document, cell: &str, want: Rect) -> bool {
    doc.cell(cell).is_some_and(|c| {
        c.shapes.iter().any(|s| {
            s.layer == LayerId::new(68, 20) && matches!(s.kind, ShapeKind::Rect(r) if r == want)
        })
    })
}

/// Exchanges the full state of two peers both ways (asymmetric: a diff one direction,
/// a full state update the other), then asserts convergence.
fn exchange_and_assert(a: &mut SyncDocument, b: &mut SyncDocument) {
    let sv_a = a.state_vector();
    let sv_b = b.state_vector();
    let a_to_b = a.encode_update(&sv_b).expect("encode a->b");
    let b_to_a = b.encode_update(&sv_a).expect("encode b->a");
    a.apply_update(&b_to_a).expect("apply b->a");
    b.apply_update(&a_to_b).expect("apply a->b");
    assert_eq!(
        a.document(),
        b.document(),
        "agent and human did not converge to an identical document"
    );
}

#[test]
fn agent_and_human_disjoint_edits_converge() {
    let mut agent = AgentCollaborator::new(Pacing::Instant);
    let mut human = SyncDocument::new("human");

    // The agent draws a two-rect cell as one atomic step.
    agent.apply_step(&[
        AgentCommand::CreateCell {
            name: "agent_cell".into(),
        },
        rect_cmd("agent_cell", 0, 0, 100, 100),
        rect_cmd("agent_cell", 200, 0, 300, 100),
    ]);

    // Concurrently, the human draws in their own cell.
    human.add_empty_cell("human_cell");
    human.add_shape("human_cell", &human_rect(69, 5, 5, 20, 20));

    exchange_and_assert(agent.sync_mut(), &mut human);

    // Both peers see both cells and the agent's two shapes.
    let doc = agent.document();
    assert!(doc.cell("agent_cell").is_some());
    assert!(doc.cell("human_cell").is_some());
    assert_eq!(doc.cell("agent_cell").unwrap().shapes.len(), 2);
    assert_eq!(doc.cell_count(), 2);
}

#[test]
fn agent_and_human_same_cell_union_converges() {
    let mut agent = AgentCollaborator::new(Pacing::Instant);
    let mut human = SyncDocument::new("human");

    // Both edit a shared cell concurrently (add_* / add_empty_cell merge, not
    // replace), so the shared cell ends as the union of both peers' shapes.
    agent.apply_step(&[
        AgentCommand::CreateCell {
            name: "shared".into(),
        },
        rect_cmd("shared", 0, 0, 4, 4),
        rect_cmd("shared", 4, 4, 8, 8),
    ]);
    human.add_empty_cell("shared");
    human.add_shape("shared", &human_rect(70, 8, 8, 12, 12));

    exchange_and_assert(agent.sync_mut(), &mut human);

    let cell = agent.document().cell("shared").expect("shared cell");
    assert_eq!(
        cell.shapes.len(),
        3,
        "union of agent (2) and human (1) shapes"
    );
}

#[test]
fn exchange_order_does_not_matter_agent_human() {
    // The same logical edits, exchanged in two different orders across two independent
    // agent/human pairs; both pairs must reach the SAME document.
    let build = || {
        let mut agent = AgentCollaborator::new(Pacing::Instant);
        let mut human = SyncDocument::new("human");
        agent.apply_step(&[
            AgentCommand::CreateCell { name: "top".into() },
            rect_cmd("top", 0, 0, 100, 100),
        ]);
        agent.apply_step(&[AgentCommand::PlaceInstance {
            cell: "top".into(),
            child: "sub".into(),
            transform: TransformArg {
                dx: 10,
                dy: 20,
                ..TransformArg::default()
            },
        }]);
        human.add_cell(&Cell::new("sub"));
        human.add_shape("sub", &human_rect(3, -5, -5, 5, 5));
        (agent, human)
    };

    // Pair 1: apply human->agent first, then agent->human.
    let (mut agent1, mut human1) = build();
    let sv_a1 = agent1.sync().state_vector();
    let sv_h1 = human1.state_vector();
    let a1_to_h1 = agent1.sync().encode_update(&sv_h1).unwrap();
    let h1_to_a1 = human1.encode_update(&sv_a1).unwrap();
    agent1.sync_mut().apply_update(&h1_to_a1).unwrap();
    human1.apply_update(&a1_to_h1).unwrap();

    // Pair 2: apply agent->human first, then human->agent, and deliver one twice
    // (idempotence).
    let (mut agent2, mut human2) = build();
    let sv_a2 = agent2.sync().state_vector();
    let sv_h2 = human2.state_vector();
    let a2_to_h2 = agent2.sync().encode_update(&sv_h2).unwrap();
    let h2_to_a2 = human2.encode_update(&sv_a2).unwrap();
    human2.apply_update(&a2_to_h2).unwrap();
    human2.apply_update(&a2_to_h2).unwrap(); // duplicate delivery
    agent2.sync_mut().apply_update(&h2_to_a2).unwrap();

    // All four peers converge to a single identical document.
    assert_eq!(agent1.document(), &human1.to_document());
    assert_eq!(agent2.document(), &human2.to_document());
    assert_eq!(agent1.document(), agent2.document());
    assert_eq!(agent1.document(), &human2.to_document());
}

#[test]
fn a_multi_shape_step_is_atomic_on_the_wire_never_a_partial() {
    // Capture the exact wire update produced by one multi-shape agent step, then
    // replay it in isolation on a fresh peer. Because the step is a single yrs
    // transaction, the update materializes to the WHOLE step at once: all three
    // shapes, never a strict subset. There is no intermediate update that carries
    // only some of the shapes.
    let mut agent = AgentCollaborator::new(Pacing::Instant);

    // Snapshot the agent's state vector BEFORE the step, so the diff captures exactly
    // (and only) what the step produced.
    let before = agent.sync().state_vector();

    let report = agent.apply_step(&[
        AgentCommand::CreateCell {
            name: "atomic".into(),
        },
        rect_cmd("atomic", 0, 0, 10, 10),
        rect_cmd("atomic", 20, 0, 30, 10),
        rect_cmd("atomic", 40, 0, 50, 10),
    ]);
    assert_eq!(report.placed.len(), 3, "three shapes placed this step");

    // The single update the step contributed.
    let step_update = agent
        .sync()
        .encode_update(&before)
        .expect("encode step diff");

    // Apply that one update to a fresh peer and materialize: it is all-or-nothing.
    let mut observer = SyncDocument::new("observer");
    observer
        .apply_update(&step_update)
        .expect("apply the atomic step update");

    let cell = observer
        .document()
        .cell("atomic")
        .expect("the step's cell is present");
    assert_eq!(
        cell.shapes.len(),
        3,
        "the atomic step materializes to all three shapes at once, never a partial subset"
    );
    // And the observer's whole document equals the agent's: no drift.
    assert_eq!(observer.document(), agent.document());
}

#[test]
fn no_intermediate_update_between_two_steps_carries_a_half_step() {
    // Two sequential multi-shape steps, streamed to a peer one step at a time (as a
    // live session would). Each step's update is captured immediately after it lands
    // (a diff against the state vector taken just before that step), so it carries a
    // COMPLETE step: after step 1 the peer has exactly 2 shapes, after step 2 exactly
    // 4. A peer polling between steps therefore sees whole steps only, never a
    // half-applied one.
    let mut agent = AgentCollaborator::new(Pacing::Instant);
    let mut peer = SyncDocument::new("peer");

    // --- Step 1: capture and stream its update before step 2 exists. ---
    let sv0 = agent.sync().state_vector();
    agent.apply_step(&[
        AgentCommand::CreateCell { name: "c".into() },
        rect_cmd("c", 0, 0, 4, 4),
        rect_cmd("c", 4, 0, 8, 4),
    ]);
    let step1 = agent.sync().encode_update(&sv0).unwrap();
    peer.apply_update(&step1).unwrap();
    assert_eq!(
        peer.document().cell("c").unwrap().shapes.len(),
        2,
        "step 1 lands exactly two shapes, never one"
    );

    // --- Step 2: capture its incremental update and stream it on top of step 1. ---
    let sv1 = agent.sync().state_vector();
    agent.apply_step(&[rect_cmd("c", 8, 0, 12, 4), rect_cmd("c", 12, 0, 16, 4)]);
    let step2 = agent.sync().encode_update(&sv1).unwrap();
    peer.apply_update(&step2).unwrap();
    assert_eq!(
        peer.document().cell("c").unwrap().shapes.len(),
        4,
        "step 2 brings the peer to exactly four shapes"
    );
    assert_eq!(peer.document(), agent.document());
}

// ----- id-addressed edits (ADR 0022's closed gap) ----------------------------
//
// These prove the new mirroring: the agent transforms and deletes shapes it created,
// addressed by their stable ElementIds, and those edits reach the CRDT and converge
// with a concurrent human peer's edits in either exchange order. They also prove the
// honest failure mode: an id the collaborator never learned is skipped with a warning
// rather than applied incorrectly or silently dropped.

#[test]
fn agent_transform_of_its_own_shape_converges_with_a_concurrent_human_edit_both_orders() {
    // Each pair: the agent draws two rects (E1, E2) and moves E2 right by 200 DBU, while a
    // human concurrently draws in its own cell. The two pairs exchange updates in opposite
    // orders and must reach the same document, with E2 actually moved.
    let build = || {
        let mut agent = AgentCollaborator::new(Pacing::Instant);
        agent.apply_step(&[
            AgentCommand::CreateCell { name: "top".into() },
            rect_cmd("top", 0, 0, 10, 10),    // E1
            rect_cmd("top", 100, 0, 110, 10), // E2
        ]);
        let report = agent.apply_step(&[transform_cmd(&[2], 200, 0)]);
        assert!(
            report.skipped.is_empty(),
            "the agent's own id resolves, nothing skipped"
        );
        let mut human = SyncDocument::new("human");
        human.add_empty_cell("human_cell");
        human.add_shape("human_cell", &human_rect(69, 5, 5, 20, 20));
        (agent, human)
    };

    // Pair 1: human -> agent first, then agent -> human.
    let (mut agent1, mut human1) = build();
    let sv_a1 = agent1.sync().state_vector();
    let sv_h1 = human1.state_vector();
    let a1_to_h1 = agent1.sync().encode_update(&sv_h1).unwrap();
    let h1_to_a1 = human1.encode_update(&sv_a1).unwrap();
    agent1.sync_mut().apply_update(&h1_to_a1).unwrap();
    human1.apply_update(&a1_to_h1).unwrap();

    // Pair 2: agent -> human first (delivered twice for idempotence), then human -> agent.
    let (mut agent2, mut human2) = build();
    let sv_a2 = agent2.sync().state_vector();
    let sv_h2 = human2.state_vector();
    let a2_to_h2 = agent2.sync().encode_update(&sv_h2).unwrap();
    let h2_to_a2 = human2.encode_update(&sv_a2).unwrap();
    human2.apply_update(&a2_to_h2).unwrap();
    human2.apply_update(&a2_to_h2).unwrap();
    agent2.sync_mut().apply_update(&h2_to_a2).unwrap();

    // Both pairs converge to a single identical document, regardless of exchange order.
    assert_eq!(agent1.document(), &human1.to_document());
    assert_eq!(agent2.document(), &human2.to_document());
    assert_eq!(agent1.document(), agent2.document());

    // The transform actually moved E2: it now sits at (300,0)-(310,10), not its original
    // (100,0)-(110,10); E1 is unmoved; the human's cell arrived.
    let doc = agent1.document();
    assert!(
        has_rect(
            doc,
            "top",
            Rect::new(Point::new(300, 0), Point::new(310, 10))
        ),
        "E2 moved to the transformed location"
    );
    assert!(
        !has_rect(
            doc,
            "top",
            Rect::new(Point::new(100, 0), Point::new(110, 10))
        ),
        "E2 is no longer at its original location"
    );
    assert!(
        has_rect(doc, "top", Rect::new(Point::new(0, 0), Point::new(10, 10))),
        "E1 is unmoved"
    );
    assert_eq!(doc.cell("top").unwrap().shapes.len(), 2, "still two shapes");
    assert!(
        doc.cell("human_cell").is_some(),
        "the human's cell converged"
    );
}

#[test]
fn agent_deletes_its_own_shape_and_the_delete_converges_with_a_concurrent_human_edit() {
    let mut agent = AgentCollaborator::new(Pacing::Instant);
    agent.apply_step(&[
        AgentCommand::CreateCell { name: "top".into() },
        rect_cmd("top", 0, 0, 10, 10),    // E1
        rect_cmd("top", 100, 0, 110, 10), // E2
    ]);
    // The agent deletes E1 (its first rect).
    let report = agent.apply_step(&[delete_cmd(&[1])]);
    assert!(report.skipped.is_empty(), "the agent's own id resolves");

    // Concurrently, the human adds a shape to the SAME cell (a union merge).
    let mut human = SyncDocument::new("human");
    human.add_empty_cell("top");
    human.add_shape("top", &human_rect(70, 8, 8, 12, 12));

    exchange_and_assert(agent.sync_mut(), &mut human);

    let doc = agent.document();
    // E1 was deleted (gone), E2 survived, the human's shape merged in: two shapes total.
    assert!(
        !has_rect(doc, "top", Rect::new(Point::new(0, 0), Point::new(10, 10))),
        "the deleted shape E1 is gone on both peers"
    );
    assert!(
        has_rect(
            doc,
            "top",
            Rect::new(Point::new(100, 0), Point::new(110, 10))
        ),
        "the surviving shape E2 is present"
    );
    assert_eq!(
        doc.cell("top").unwrap().shapes.len(),
        2,
        "E2 plus the human's shape; E1 removed"
    );
}

#[test]
fn a_transform_of_an_unknown_id_is_skipped_and_does_not_poison_the_step() {
    let mut agent = AgentCollaborator::new(Pacing::Instant);
    agent.apply_step(&[
        AgentCommand::CreateCell { name: "top".into() },
        rect_cmd("top", 0, 0, 10, 10), // E1
    ]);
    // One step transforms an id no shape has (999) AND adds a sibling rect. The unknown
    // transform is skipped; the sibling still lands, so the step is not poisoned.
    let report = agent.apply_step(&[
        transform_cmd(&[999], 50, 0),
        rect_cmd("top", 200, 0, 210, 10),
    ]);
    assert_eq!(
        report.skipped.len(),
        1,
        "the unknown id is recorded as skipped"
    );
    assert!(
        report.skipped[0].contains("e999"),
        "the warning names the unresolved id: {}",
        report.skipped[0]
    );
    assert_eq!(report.placed.len(), 1, "the sibling add still landed");
    assert_eq!(
        agent.document().cell("top").unwrap().shapes.len(),
        2,
        "E1 plus the sibling; the step committed despite the skipped transform"
    );
}

#[test]
fn a_transform_after_the_id_map_is_forgotten_is_skipped_not_a_silent_no_op() {
    // Seeded-bad variant: the collaborator learned E1, then the map is deliberately
    // cleared (as if it attached mid-session). The authoritative session still has E1, so
    // it accepts the transform; the collaborator must record a skip, not silently move
    // nothing, and the CRDT shape must stay put.
    let mut agent = AgentCollaborator::new(Pacing::Instant);
    agent.apply_step(&[
        AgentCommand::CreateCell { name: "top".into() },
        rect_cmd("top", 0, 0, 10, 10), // E1
    ]);
    assert_eq!(agent.tracked_id_count(), 1, "E1 was learned");

    agent.forget_element_ids();
    assert_eq!(agent.tracked_id_count(), 0, "the map was cleared");

    let report = agent.apply_step(&[transform_cmd(&[1], 500, 0)]);
    assert_eq!(
        report.skipped.len(),
        1,
        "the forgotten id yields a skipped entry, not a silent no-op"
    );
    assert!(report.skipped[0].contains("e1"));

    // The CRDT shape did not move (the mirror could not resolve the id).
    assert!(
        has_rect(
            agent.document(),
            "top",
            Rect::new(Point::new(0, 0), Point::new(10, 10))
        ),
        "the shape stayed at its original location on the CRDT"
    );
}
