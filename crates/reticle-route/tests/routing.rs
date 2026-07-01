//! Integration tests for the grid/maze router.
//!
//! Each test builds a tiny [`Document`] with a single target cell, issues a
//! [`RouteRequest`], and asserts on the [`RouteReport`] and the geometry written
//! back into the cell. Grids are kept small (tens of tracks) so the tests are
//! fast and their expected wirelengths are easy to reason about by hand.

use reticle_geometry::{LayerId, Point, Rect, Shape};
use reticle_model::{Cell, Document, DrawShape, NetSpec, RouteRequest, Router, ShapeKind};
use reticle_route::{MazeRouter, RouteConfig};

/// The layer every net in these tests routes on.
const METAL1: LayerId = LayerId::new(10, 0);

/// Builds a document with one empty target cell named `top`.
fn doc_with_cell() -> Document {
    let mut doc = Document::new();
    doc.insert_cell(Cell::new("top"));
    doc.set_top_cells(vec!["top".to_owned()]);
    doc
}

/// Adds a rectangular obstacle on `layer` to the `top` cell.
fn add_obstacle(doc: &mut Document, layer: LayerId, rect: Rect) {
    let cell = doc.cell_mut("top").expect("top cell exists");
    cell.shapes
        .push(DrawShape::new(layer, ShapeKind::Rect(rect)));
}

/// Counts the `Path` shapes on any layer in the `top` cell.
fn path_count(doc: &Document) -> usize {
    doc.cell("top")
        .expect("top cell")
        .shapes
        .iter()
        .filter(|s| matches!(s.kind, ShapeKind::Path(_)))
        .count()
}

/// A request routing a single net with the given terminals on [`METAL1`].
fn single_net(terminals: Vec<Point>) -> RouteRequest {
    RouteRequest {
        cell: "top".to_owned(),
        nets: vec![NetSpec {
            name: "n0".to_owned(),
            terminals,
            layer: METAL1,
        }],
    }
}

#[test]
fn routes_two_terminal_net_on_empty_grid() {
    let mut doc = doc_with_cell();
    let request = single_net(vec![Point::new(0, 0), Point::new(100, 0)]);

    let mut router = MazeRouter::with_config(RouteConfig::new().with_pitch(10));
    let report = router.route(&mut doc, &request);

    assert_eq!(report.routed, 1, "the single net should route");
    assert_eq!(report.failed, 0, "no failures on an empty grid");

    // Straight-line distance is 100 DBU; on a clear grid the router should find a
    // path of exactly that Manhattan length.
    assert_eq!(
        report.total_length_dbu, 100,
        "clear-grid route should be the straight Manhattan length"
    );
    assert!(path_count(&doc) >= 1, "at least one wire Path is emitted");

    // The congestion snapshot should exist and show some occupancy but no blockage.
    let cong = router.congestion().expect("congestion after a run");
    assert_eq!(cong.blocked_nodes, 0, "empty grid has no obstacles");
    assert!(cong.occupied_nodes > 0, "the route occupies some nodes");
}

#[test]
fn routes_around_blocking_obstacle() {
    let mut doc = doc_with_cell();
    // A wall in the middle of the direct path between the terminals, with a gap
    // above and below so the net must detour but can still connect.
    add_obstacle(
        &mut doc,
        METAL1,
        Rect::new(Point::new(45, -30), Point::new(55, 30)),
    );
    let request = single_net(vec![Point::new(0, 0), Point::new(100, 0)]);

    let mut router = MazeRouter::with_config(RouteConfig::new().with_pitch(10));
    let report = router.route(&mut doc, &request);

    assert_eq!(report.routed, 1, "the net routes around the obstacle");
    assert_eq!(report.failed, 0);
    assert!(
        report.total_length_dbu > 100,
        "a detour must be longer than the 100 DBU straight line, got {}",
        report.total_length_dbu
    );

    let cong = router.congestion().expect("congestion");
    assert!(cong.blocked_nodes > 0, "the obstacle blocks some tracks");
}

#[test]
fn rip_up_and_reroute_completes_competing_nets() {
    let mut doc = doc_with_cell();

    // Two nets whose natural straight routes cross in the middle of a narrow
    // channel. With rip-up and reroute the router should still complete both
    // (one detours), or report the failure honestly.
    let request = RouteRequest {
        cell: "top".to_owned(),
        nets: vec![
            NetSpec {
                name: "a".to_owned(),
                terminals: vec![Point::new(0, 0), Point::new(100, 0)],
                layer: METAL1,
            },
            NetSpec {
                name: "b".to_owned(),
                terminals: vec![Point::new(50, -50), Point::new(50, 50)],
                layer: METAL1,
            },
        ],
    };

    let mut router = MazeRouter::with_config(
        RouteConfig::new()
            .with_pitch(10)
            .with_max_rip_up_iterations(12),
    );
    let report = router.route(&mut doc, &request);

    // Honest accounting: routed + failed always equals the number of nets.
    assert_eq!(
        report.routed + report.failed,
        2,
        "every net is accounted for as routed or failed"
    );
    // On an otherwise empty grid the two nets can both be routed (they cross at a
    // single node; one simply detours by one track).
    assert_eq!(
        report.routed, 2,
        "both competing nets should complete via rip-up/reroute"
    );
    assert_eq!(report.failed, 0);

    // Both nets' wires are present.
    assert!(path_count(&doc) >= 2, "both nets emit wire geometry");
}

/// Encloses the point `(100, 0)` in a solid obstacle ring on `layer`, two tracks
/// (20 DBU) thick, leaving only the terminal's own pocket free. Nothing on that
/// layer can reach the enclosed terminal, so a single-layer route must fail.
fn enclose_right_terminal(doc: &mut Document, layer: LayerId) {
    // Ring around (100, 0): outer box (60,-40)-(140,40), inner hole (90,-10)-(110,10).
    let bars = [
        Rect::new(Point::new(60, -40), Point::new(80, 40)), // left
        Rect::new(Point::new(120, -40), Point::new(140, 40)), // right
        Rect::new(Point::new(60, 20), Point::new(140, 40)), // top
        Rect::new(Point::new(60, -40), Point::new(140, -20)), // bottom
    ];
    for bar in bars {
        add_obstacle(doc, layer, bar);
    }
}

#[test]
fn impossible_obstacle_wall_reports_failure() {
    let mut doc = doc_with_cell();

    // Fully enclose the right terminal on the (only) routing layer: no path can
    // reach it, so the net must fail rather than fabricate a bogus route.
    enclose_right_terminal(&mut doc, METAL1);
    let request = single_net(vec![Point::new(0, 0), Point::new(100, 0)]);

    let mut router = MazeRouter::with_config(RouteConfig::new().with_pitch(10));
    let report = router.route(&mut doc, &request);

    assert_eq!(report.routed, 0, "no route can enter a full enclosure");
    assert_eq!(report.failed, 1, "the blocked net is reported as failed");
    assert_eq!(
        report.total_length_dbu, 0,
        "a failed run emits no wire length"
    );
    assert_eq!(path_count(&doc), 0, "nothing is written back on failure");
}

#[test]
fn multi_layer_via_route_connects_across_a_wall() {
    let mut doc = doc_with_cell();

    // Enclose the right terminal on layer 0 (METAL1) so no in-plane route can
    // reach it. With a second routing layer available and clear, the router can
    // via up out of the pocket, cross over the enclosure on the upper plane, and
    // via back down at the start — so the net still completes.
    enclose_right_terminal(&mut doc, METAL1);
    let request = single_net(vec![Point::new(0, 0), Point::new(100, 0)]);

    let mut router = MazeRouter::with_config(
        RouteConfig::new()
            .with_pitch(10)
            .with_layers(2)
            .with_via_cost(15),
    );
    let report = router.route(&mut doc, &request);

    assert_eq!(
        report.routed, 1,
        "the net should cross the wall using the upper layer via vias"
    );
    assert_eq!(report.failed, 0);
    // The cost must include two via traversals plus the in-plane length, so it is
    // strictly greater than the 100 DBU straight line.
    assert!(
        report.total_length_dbu > 100,
        "a via detour costs more than the straight line, got {}",
        report.total_length_dbu
    );

    // Geometry should appear on the base layer and on the upper (datatype-bumped)
    // plane, confirming the via crossed layers.
    let cell = doc.cell("top").expect("top cell");
    let base_paths = cell
        .shapes
        .iter()
        .filter(|s| matches!(s.kind, ShapeKind::Path(_)) && s.layer() == METAL1)
        .count();
    let upper_paths = cell
        .shapes
        .iter()
        .filter(|s| matches!(s.kind, ShapeKind::Path(_)) && s.layer().datatype > METAL1.datatype)
        .count();
    assert!(base_paths > 0, "wire exists on the base layer");
    assert!(upper_paths > 0, "wire exists on the upper routing layer");
}

#[test]
fn missing_cell_reports_all_nets_failed() {
    let mut doc = Document::new(); // no "top" cell
    let request = single_net(vec![Point::new(0, 0), Point::new(50, 0)]);

    let mut router = MazeRouter::new();
    let report = router.route(&mut doc, &request);

    assert_eq!(report.routed, 0);
    assert_eq!(report.failed, 1, "a missing target cell fails every net");
}

mod properties {
    use super::{doc_with_cell, single_net};
    use proptest::prelude::*;
    use reticle_geometry::Point;
    use reticle_model::Router;
    use reticle_route::{MazeRouter, RouteConfig};

    proptest! {
        // On a clear grid, a 2-terminal net always routes, and its optimal length
        // is exactly the pitch-aligned Manhattan distance between the terminals'
        // snapped grid nodes. Coordinates are kept modest so grids stay small.
        #[test]
        fn clear_grid_route_is_manhattan_optimal(
            x0 in -200i32..=200,
            y0 in -200i32..=200,
            x1 in -200i32..=200,
            y1 in -200i32..=200,
        ) {
            // Skip coincident terminals (nothing to route).
            prop_assume!((x0, y0) != (x1, y1));

            let pitch = 10;
            let mut doc = doc_with_cell();
            let request = single_net(vec![Point::new(x0, y0), Point::new(x1, y1)]);
            let mut router = MazeRouter::with_config(RouteConfig::new().with_pitch(pitch));
            let report = router.route(&mut doc, &request);

            prop_assert_eq!(report.routed, 1);
            prop_assert_eq!(report.failed, 0);

            // Expected optimum: Manhattan distance between the snapped grid nodes.
            let grid = router.grid().expect("grid built");
            let a = grid.point_to_node(Point::new(x0, y0));
            let b = grid.point_to_node(Point::new(x1, y1));
            let expected = i64::from((a.col - b.col).abs() + (a.row - b.row).abs())
                * i64::from(pitch);
            prop_assert_eq!(report.total_length_dbu, expected);
        }
    }
}
