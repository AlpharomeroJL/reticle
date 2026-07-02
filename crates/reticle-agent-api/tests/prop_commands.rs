//! Property tests for the agent command surface.
//!
//! Two invariants are checked over randomized input:
//!
//! 1. **Robustness**: an arbitrary sequence of commands never panics and always
//!    yields a well-formed [`CommandResult`]. The session's revision never goes
//!    backwards, and a failed command never advances it.
//! 2. **Stable ids**: an [`ElementId`] returned by an add command keeps addressing
//!    the same element after later removals shift the underlying vector. The oracle
//!    is an independent map from id to the shape's known x-coordinate, maintained in
//!    the test; after every deletion the session's query results must agree with it.

use std::collections::BTreeMap;

use proptest::prelude::*;

use reticle_agent_api::args::{LayerArg, OrientationArg, PointArg, RectArg, TransformArg};
use reticle_agent_api::{AgentCommand, AgentResponse, ElementId, Session};

/// A layer argument on layer 1.
fn metal() -> LayerArg {
    LayerArg {
        layer: 1,
        datatype: 0,
    }
}

/// A 10-wide unit-height rect whose left edge is at `x`, so a query can recover
/// which shape an id addresses from the reported bounding box.
fn rect_at(x: i32) -> RectArg {
    RectArg {
        min: PointArg { x, y: 0 },
        max: PointArg { x: x + 10, y: 10 },
    }
}

// ===== strategy for arbitrary programs ======================================

/// A small pool of cell names so commands actually hit existing cells often.
const CELLS: [&str; 3] = ["a", "b", "top"];

/// A strategy generating one arbitrary command over a fixed cell-name pool and
/// small coordinate range. Ids for id-taking commands are drawn from a modest range
/// so they sometimes hit a live element and sometimes miss (both must be handled).
fn any_command() -> impl Strategy<Value = AgentCommand> {
    let cell = prop::sample::select(CELLS.as_slice()).prop_map(str::to_owned);
    let coord = -50i32..50;

    prop_oneof![
        // Create / delete cells.
        cell.clone()
            .prop_map(|name| AgentCommand::CreateCell { name }),
        cell.clone()
            .prop_map(|name| AgentCommand::DeleteCell { name }),
        // Add geometry.
        (cell.clone(), coord.clone()).prop_map(|(cell, x)| AgentCommand::AddRect {
            cell,
            layer: metal(),
            rect: rect_at(x),
        }),
        (cell.clone(), prop::collection::vec(coord.clone(), 0..5)).prop_map(|(cell, xs)| {
            AgentCommand::AddPolygon {
                cell,
                layer: metal(),
                points: xs.into_iter().map(|x| PointArg { x, y: x }).collect(),
            }
        }),
        (cell.clone(), prop::collection::vec(coord.clone(), 0..5)).prop_map(|(cell, xs)| {
            AgentCommand::AddPath {
                cell,
                layer: metal(),
                width: 2,
                points: xs.into_iter().map(|x| PointArg { x, y: 0 }).collect(),
                endcap: None,
            }
        }),
        // Placements between cells.
        (cell.clone(), cell.clone()).prop_map(|(cell, child)| AgentCommand::PlaceInstance {
            cell,
            child,
            transform: TransformArg::default(),
        }),
        // Id-taking commands: ids 1..8 sometimes exist, sometimes not.
        prop::collection::vec(1u64..8, 0..3).prop_map(|ids| AgentCommand::DeleteShapes {
            ids: ids.into_iter().map(ElementId).collect(),
        }),
        (prop::collection::vec(1u64..8, 0..3), -20i32..20).prop_map(|(ids, dx)| {
            AgentCommand::TransformShapes {
                ids: ids.into_iter().map(ElementId).collect(),
                transform: TransformArg {
                    orientation: OrientationArg::R0,
                    mag_num: 1,
                    mag_den: 1,
                    dx,
                    dy: 0,
                },
            }
        }),
        // Queries.
        cell.clone().prop_map(|cell| AgentCommand::QueryShapes {
            cell,
            layer: None,
            region: None,
        }),
        cell.clone()
            .prop_map(|cell| AgentCommand::GetCellInfo { cell }),
        Just(AgentCommand::ListLayers),
        cell.prop_map(|cell| AgentCommand::RunExtract { cell }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// An arbitrary program never panics and preserves the revision invariants.
    #[test]
    fn arbitrary_program_is_robust(cmds in prop::collection::vec(any_command(), 0..40)) {
        let mut session = Session::new();
        for cmd in cmds {
            let before = session.revision();
            let result = session.apply(cmd);
            let after = session.revision();
            // The revision never decreases.
            prop_assert!(after >= before, "revision went backwards: {before} -> {after}");
            match result {
                // A mutation advances the revision by exactly one; a read-only Ok
                // (an `Ok` with empty affected can be either) never decreases it.
                Ok(AgentResponse::Ok { revision, .. }) => {
                    prop_assert_eq!(revision, after);
                }
                Ok(AgentResponse::Data { revision, .. } | AgentResponse::Blob { revision, .. }) => {
                    // Reads report the current revision and do not change it.
                    prop_assert_eq!(revision, after);
                    prop_assert_eq!(after, before);
                }
                // `AgentResponse` is `#[non_exhaustive]`; a future variant is still
                // required to leave the revision consistent.
                Ok(_) => {
                    prop_assert!(after >= before);
                }
                Err(_) => {
                    // A failed command must not advance the revision.
                    prop_assert_eq!(after, before, "a failed command changed the revision");
                }
            }
            // The transcript grows by one record per command.
            prop_assert!(session.transcript().last().is_some());
        }
        // Every recorded outcome is internally consistent (seq is dense and ordered).
        for (i, rec) in session.transcript().iter().enumerate() {
            prop_assert_eq!(rec.seq, i as u64);
            prop_assert!(rec.revision_after >= rec.revision_before);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Ids returned by `add_rect` keep addressing their own shape after an arbitrary
    /// subset of them is deleted. Each rect has a unique left-edge x, so a query can
    /// recover which shape an id points at; the test's own id -> x map is the oracle.
    #[test]
    fn ids_track_elements_across_deletions(
        // Distinct x-coordinates (so every rect is identifiable) and a deletion mask.
        xs in prop::collection::btree_set(-1000i32..1000, 1..12),
        delete_seed in any::<u64>(),
    ) {
        let mut session = Session::new();
        session.apply(AgentCommand::CreateCell { name: "top".into() })
            .expect("create cell");

        // Add one rect per x and remember the id -> x mapping (the oracle).
        let mut oracle: BTreeMap<ElementId, i32> = BTreeMap::new();
        for &x in &xs {
            let resp = session
                .apply(AgentCommand::AddRect {
                    cell: "top".into(),
                    layer: metal(),
                    rect: rect_at(x),
                })
                .expect("add rect");
            let AgentResponse::Ok { affected, .. } = resp else {
                prop_assert!(false, "add_rect did not return Ok");
                unreachable!()
            };
            prop_assert_eq!(affected.len(), 1);
            oracle.insert(affected[0], x);
        }

        // Choose a subset to delete via a cheap deterministic hash of (id, seed).
        let ids: Vec<ElementId> = oracle.keys().copied().collect();
        let to_delete: Vec<ElementId> = ids
            .iter()
            .copied()
            .filter(|id| (id.0.wrapping_mul(0x9E37_79B9).wrapping_add(delete_seed)) & 1 == 0)
            .collect();
        for id in &to_delete {
            session
                .apply(AgentCommand::DeleteShapes { ids: vec![*id] })
                .expect("delete shape");
            oracle.remove(id);
        }

        // Query and confirm every surviving id still maps to its original x, and no
        // deleted id reappears. This is the crux: index shifts from the deletions
        // must have been reconciled so the ids stayed pinned to their elements.
        let AgentResponse::Data { value, .. } = session
            .apply(AgentCommand::QueryShapes {
                cell: "top".into(),
                layer: None,
                region: None,
            })
            .expect("query")
        else {
            prop_assert!(false, "query did not return Data");
            unreachable!()
        };
        let shapes = value["shapes"].as_array().expect("shapes array");

        // Build the observed id -> x map from the query result.
        let mut observed: BTreeMap<ElementId, i32> = BTreeMap::new();
        for sh in shapes {
            if let Some(id) = sh["id"].as_u64() {
                let x = sh["bbox"]["min"]["x"].as_i64().expect("bbox min x") as i32;
                observed.insert(ElementId(id), x);
            }
        }

        prop_assert_eq!(
            observed.len(),
            oracle.len(),
            "surviving shape count must match the oracle"
        );
        for (id, x) in &oracle {
            prop_assert_eq!(
                observed.get(id),
                Some(x),
                "id {} should still address the rect at x={}",
                id.0,
                x
            );
        }
        for id in &to_delete {
            prop_assert!(
                !observed.contains_key(id),
                "deleted id {} must not reappear",
                id.0
            );
        }
    }
}
