//! Routing for Reticle: a grid and maze router.
//!
//! [`MazeRouter`] builds a discrete routing grid over the bounding area of a
//! [`RouteRequest`], marks the tracks covered by existing geometry (grown by a DRC
//! spacing margin) as blocked, and connects each net's terminals with an A\* maze
//! search (via [`pathfinding`]). Two or more routing layers are modelled as a
//! stacked grid with via moves between planes. When a net cannot be routed because
//! earlier nets filled the channels, the router rips up the conflicting nets and
//! retries in a different order for a bounded number of iterations before reporting
//! whatever it could not complete.
//!
//! Successful routes are written back into the target cell as
//! [`Path`] shapes on the net's layer, and the run returns
//! a [`RouteReport`] of routed/failed counts and total wire length. A
//! [`MazeRouter::congestion`] summary reports grid occupancy after a run.
//!
//! The frozen Wave 0 contract is [`MazeRouter`] implementing [`Router`]; the grid,
//! search, and configuration types are additive public surface.

mod config;
mod grid;
mod maze;

pub use config::RouteConfig;
pub use grid::{GridNode, RoutingGrid};
pub use maze::{MazeSearch, NetId, NetRoute, RouteFailure};

use reticle_geometry::{Endcap, LayerId, Path, Point, Rect, Shape, SpatialIndex};
use reticle_index::RTreeIndex;
use reticle_model::{Document, DrawShape, RouteReport, RouteRequest, Router, ShapeKind};

/// A snapshot of grid usage after a routing run.
///
/// All counts are node counts over the stacked grid (`cols * rows * layers`
/// nodes total). [`CongestionSummary::utilization`] is the fraction of *free*
/// (non-obstacle) capacity that ended up carrying wire.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct CongestionSummary {
    /// Total nodes in the grid.
    pub total_nodes: usize,
    /// Nodes blocked by static obstacles (unavailable to any net).
    pub blocked_nodes: usize,
    /// Nodes occupied by routed wire.
    pub occupied_nodes: usize,
}

impl CongestionSummary {
    /// Free routing capacity: nodes that were neither obstacles nor used.
    #[must_use]
    pub fn free_nodes(&self) -> usize {
        self.total_nodes
            .saturating_sub(self.blocked_nodes)
            .saturating_sub(self.occupied_nodes)
    }

    /// Fraction (0.0–1.0) of non-obstacle capacity carrying wire. Zero when the
    /// grid has no free capacity.
    #[must_use]
    pub fn utilization(&self) -> f64 {
        let capacity = self.total_nodes.saturating_sub(self.blocked_nodes);
        if capacity == 0 {
            0.0
        } else {
            self.occupied_nodes as f64 / capacity as f64
        }
    }
}

/// The grid/maze router.
///
/// Holds a [`RouteConfig`] and, after [`Router::route`], the grid and congestion
/// snapshot from the most recent run for inspection via [`MazeRouter::congestion`]
/// and [`MazeRouter::grid`].
#[derive(Debug, Default, Clone)]
pub struct MazeRouter {
    config: RouteConfig,
    last_grid: Option<RoutingGrid>,
    last_congestion: Option<CongestionSummary>,
}

impl MazeRouter {
    /// Creates a router with the default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a router with a specific configuration.
    #[must_use]
    pub fn with_config(config: RouteConfig) -> Self {
        Self {
            config,
            last_grid: None,
            last_congestion: None,
        }
    }

    /// The active configuration.
    #[must_use]
    pub fn config(&self) -> &RouteConfig {
        &self.config
    }

    /// A mutable handle to the configuration, for tuning between runs.
    pub fn config_mut(&mut self) -> &mut RouteConfig {
        &mut self.config
    }

    /// The grid built by the most recent [`Router::route`] call, if any.
    #[must_use]
    pub fn grid(&self) -> Option<&RoutingGrid> {
        self.last_grid.as_ref()
    }

    /// The congestion snapshot from the most recent run, if any.
    #[must_use]
    pub fn congestion(&self) -> Option<CongestionSummary> {
        self.last_congestion
    }

    /// The bounding area to grid: the union of every terminal and every existing
    /// shape's bounding box in the target cell. Returns `None` if there is nothing
    /// to bound (no terminals and no geometry).
    fn request_bounds(cell_shapes: &[DrawShape], request: &RouteRequest) -> Option<Rect> {
        let mut bbox: Option<Rect> = None;
        let mut grow = |r: Rect| {
            bbox = Some(bbox.map_or(r, |acc| acc.union(&r)));
        };
        for net in &request.nets {
            for t in &net.terminals {
                grow(Rect::new(*t, *t));
            }
        }
        for shape in cell_shapes {
            grow(shape.bounding_box());
        }
        bbox
    }

    /// Builds the routing grid for a request: covers [`Self::request_bounds`] at the
    /// configured pitch and layer count, then blocks the tracks covered by each
    /// existing shape grown by the spacing margin (queried through an R-tree so only
    /// the shapes near the routing area are considered).
    fn build_grid(&self, cell_shapes: &[DrawShape], request: &RouteRequest) -> RoutingGrid {
        let bounds = Self::request_bounds(cell_shapes, request)
            .unwrap_or_else(|| Rect::new(Point::ORIGIN, Point::ORIGIN));
        let mut grid = RoutingGrid::from_bounds(bounds, self.config.pitch, self.config.layers);

        // Index the obstacles so a large host cell does not force a linear scan.
        let index: RTreeIndex<Rect> = RTreeIndex::bulk_load(
            cell_shapes
                .iter()
                .map(|s| (s.bounding_box(), s.bounding_box())),
        );
        // Query the whole gridded area (plus margin) once and block each hit. Static
        // geometry lives on the primary routing plane (grid layer 0, where the
        // terminals sit); upper planes stay clear so a net can via up and cross over
        // an obstacle. With a single layer this blocks the only plane, as expected.
        let query = bounds.expanded(self.config.pitch + self.config.spacing);
        for obstacle in index.query_rect(query) {
            grid.block_obstacle_on_layer(obstacle.expanded(self.config.spacing), 0);
        }
        grid
    }

    /// Emits the wire [`Path`] shapes for a routed net onto the correct layer.
    ///
    /// A net's nodes form a connected tree in grid space. This reconstructs the
    /// tree's edges from spatial adjacency (the node list is in discovery order,
    /// not path order) and merges collinear edges into maximal straight, single-layer
    /// runs, emitting one [`Path`] per run. Each layer change contributes a short
    /// via stub on both planes so the connection is represented in geometry. The
    /// net's own [`LayerId`] addresses grid layer 0; higher planes are offset by
    /// datatype so distinct routing layers stay distinguishable in the output.
    fn emit_paths(
        grid: &RoutingGrid,
        base_layer: LayerId,
        route: &NetRoute,
        width: i32,
        endcap: Endcap,
    ) -> Vec<DrawShape> {
        let nodes: std::collections::HashSet<GridNode> = route.nodes.iter().copied().collect();
        let mut shapes = Vec::new();

        // Merge horizontal runs: a run starts at a node whose left neighbour is not
        // in the net, and extends right while the next node is. Vertical runs are
        // symmetric. Every in-plane edge belongs to exactly one such run, so no edge
        // is emitted twice and collinear segments coalesce into one `Path`.
        for &node in &nodes {
            // Horizontal run start.
            if !nodes.contains(&GridNode::new(node.col - 1, node.row, node.layer)) {
                let mut end = node;
                while nodes.contains(&GridNode::new(end.col + 1, end.row, end.layer)) {
                    end = GridNode::new(end.col + 1, end.row, end.layer);
                }
                if end.col > node.col {
                    shapes.push(Self::run_path(grid, base_layer, node, end, width, endcap));
                }
            }
            // Vertical run start.
            if !nodes.contains(&GridNode::new(node.col, node.row - 1, node.layer)) {
                let mut end = node;
                while nodes.contains(&GridNode::new(end.col, end.row + 1, end.layer)) {
                    end = GridNode::new(end.col, end.row + 1, end.layer);
                }
                if end.row > node.row {
                    shapes.push(Self::run_path(grid, base_layer, node, end, width, endcap));
                }
            }
            // Via stub: emitted once per adjacent layer pair, keyed off the lower node.
            let up = GridNode::new(node.col, node.row, node.layer + 1);
            if nodes.contains(&up) {
                let center = grid.node_to_point(node);
                for layer_idx in [node.layer, node.layer + 1] {
                    let layer = layer_for(base_layer, layer_idx);
                    shapes.push(DrawShape::new(
                        layer,
                        ShapeKind::Path(Path::new(vec![center, center], width, endcap)),
                    ));
                }
            }
        }

        shapes
    }

    /// Builds a single straight-run [`Path`] `DrawShape` between two collinear nodes.
    fn run_path(
        grid: &RoutingGrid,
        base_layer: LayerId,
        from: GridNode,
        to: GridNode,
        width: i32,
        endcap: Endcap,
    ) -> DrawShape {
        let p0 = grid.node_to_point(from);
        let p1 = grid.node_to_point(to);
        let layer = layer_for(base_layer, from.layer);
        DrawShape::new(
            layer,
            ShapeKind::Path(Path::new(vec![p0, p1], width, endcap)),
        )
    }

    /// Runs the rip-up-and-reroute loop and returns the committed routes keyed by
    /// net id, together with the final search state (for the congestion snapshot).
    fn route_all<'g>(
        &self,
        grid: &'g RoutingGrid,
        nets: &[(NetId, Vec<Point>)],
    ) -> (Vec<(NetId, NetRoute)>, MazeSearch<'g>) {
        let mut search = MazeSearch::new(grid, self.config.via_cost);
        let mut order: Vec<usize> = (0..nets.len()).collect();
        let mut best: Vec<(NetId, NetRoute)> = Vec::new();
        let mut best_routed = 0usize;

        let iterations = self.config.max_rip_up_iterations.max(1);
        for iter in 0..iterations {
            search.clear();
            let mut committed: Vec<(NetId, NetRoute)> = Vec::new();
            let mut failed_positions: Vec<usize> = Vec::new();

            for &pos in &order {
                let (net_id, terminals) = &nets[pos];
                match search.route_net(*net_id, terminals) {
                    Ok(route) => {
                        search.commit(*net_id, &route);
                        committed.push((*net_id, route));
                    }
                    Err(RouteFailure::NotEnoughTerminals) => {
                        // Nothing to do; counts as unroutable but not a conflict.
                        failed_positions.push(pos);
                    }
                    Err(RouteFailure::Blocked { blockers }) => {
                        // Rip up the conflicting nets so a later net (this one, on the
                        // next pass) has room, then record the failure for reordering.
                        for blocker in blockers {
                            search.rip_up(blocker);
                            committed.retain(|(id, _)| *id != blocker);
                        }
                        failed_positions.push(pos);
                    }
                }
            }

            if committed.len() > best_routed {
                best_routed = committed.len();
                best.clone_from(&committed);
            }

            if failed_positions.is_empty() {
                // Everything routed; done.
                best = committed;
                break;
            }
            if iter + 1 == iterations {
                break;
            }
            // Reorder for the next pass: move failed nets to the front so they claim
            // channels first. This is a deterministic rotation, not randomness, so
            // runs are reproducible.
            let mut next: Vec<usize> = failed_positions.clone();
            for &pos in &order {
                if !failed_positions.contains(&pos) {
                    next.push(pos);
                }
            }
            order = next;
        }

        // Rebuild occupancy to reflect exactly the returned `best` set so the
        // congestion snapshot is consistent with what we report and write back.
        search.clear();
        for (net_id, route) in &best {
            search.commit(*net_id, route);
        }
        (best, search)
    }
}

/// Maps a grid layer index onto a concrete [`LayerId`] derived from the net's base
/// layer: layer 0 is the net layer itself; higher planes bump the datatype so
/// each routing plane is a distinct addressable layer.
fn layer_for(base: LayerId, grid_layer: u16) -> LayerId {
    if grid_layer == 0 {
        base
    } else {
        LayerId::new(base.layer, base.datatype.saturating_add(grid_layer))
    }
}

impl Router for MazeRouter {
    fn route(&mut self, doc: &mut Document, request: &RouteRequest) -> RouteReport {
        // Snapshot the target cell's existing geometry (obstacles). If the cell is
        // missing there is nothing to route into.
        let Some(cell) = doc.cell(&request.cell) else {
            self.last_grid = None;
            self.last_congestion = None;
            return RouteReport {
                routed: 0,
                failed: request.nets.len(),
                total_length_dbu: 0,
            };
        };
        let obstacles: Vec<DrawShape> = cell.shapes.clone();

        let grid = self.build_grid(&obstacles, request);

        // Prepare the net worklist (id, terminals). The id indexes `request.nets`.
        let nets: Vec<(NetId, Vec<Point>)> = request
            .nets
            .iter()
            .enumerate()
            .map(|(i, n)| (i, n.terminals.clone()))
            .collect();

        let (routes, search) = self.route_all(&grid, &nets);

        // Write successful routes back as Path shapes and total their length.
        let mut total_length: i64 = 0;
        let mut emitted: Vec<DrawShape> = Vec::new();
        for (net_id, route) in &routes {
            total_length += route.cost;
            let net = &request.nets[*net_id];
            emitted.extend(Self::emit_paths(
                &grid,
                net.layer,
                route,
                self.config.wire_width,
                self.config.endcap,
            ));
        }

        let routed = routes.len();
        let failed = request.nets.len().saturating_sub(routed);

        if let Some(cell) = doc.cell_mut(&request.cell) {
            cell.shapes.extend(emitted);
        }

        self.last_congestion = Some(CongestionSummary {
            total_nodes: grid.node_count(),
            blocked_nodes: grid.blocked_count(),
            occupied_nodes: search.occupied_count(),
        });
        self.last_grid = Some(grid);

        RouteReport {
            routed,
            failed,
            total_length_dbu: total_length,
        }
    }
}
