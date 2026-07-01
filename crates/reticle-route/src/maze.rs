//! The maze search: A\* over the routing grid with vias between layers.
//!
//! [`MazeSearch`] wraps a [`RoutingGrid`] and an occupancy map (which net, if any,
//! owns each node) and exposes a single-net router that connects a set of
//! terminals. Two-terminal nets are a plain point-to-point A\*; nets with more
//! terminals grow a connected tree, wiring each remaining terminal to the nearest
//! node already on the net.
//!
//! Costs are integer DBU: an in-plane step costs one `pitch`, a via costs
//! [`MazeSearch::via_cost`]. The heuristic is the Manhattan distance to the goal
//! scaled by the pitch — admissible because every unit of Manhattan distance needs
//! at least one in-plane step, and vias only add cost the heuristic never claims.

use crate::grid::{GridNode, RoutingGrid};
use pathfinding::prelude::astar;
use reticle_geometry::Point;
use std::collections::{HashMap, HashSet};

/// A net identifier used to key the occupancy map.
pub type NetId = usize;

/// Why a single-net route attempt failed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RouteFailure {
    /// The net has fewer than two terminals, so there is nothing to connect.
    NotEnoughTerminals,
    /// No obstacle-free path exists between two terminals given current occupancy.
    /// Carries the set of nets whose wires block the search frontier, so the
    /// caller can decide what to rip up.
    Blocked {
        /// Nets found adjacent to the explored frontier (rip-up candidates).
        blockers: Vec<NetId>,
    },
}

/// A completed single-net route: the grid nodes visited and the total cost.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NetRoute {
    /// Every grid node the net occupies (terminals, bends, and via stacks).
    pub nodes: Vec<GridNode>,
    /// The route cost in DBU (in-plane length plus via costs).
    pub cost: i64,
}

/// A\* maze search bound to a grid and a shared occupancy map.
///
/// The occupancy map records, per node, which [`NetId`] currently owns it. A
/// search may pass through nodes owned by the net being routed (so a growing
/// multi-terminal tree can reuse its own trunk) but treats nodes owned by other
/// nets as blocked, recording them as rip-up candidates.
#[derive(Debug)]
pub struct MazeSearch<'g> {
    grid: &'g RoutingGrid,
    occupancy: HashMap<GridNode, NetId>,
    via_cost: i64,
}

impl<'g> MazeSearch<'g> {
    /// Creates a search over `grid` with an empty occupancy map.
    ///
    /// `via_cost` is the DBU penalty for a single layer change; it is clamped to at
    /// least 1 so vias always cost something and A\* stays well-founded.
    #[must_use]
    pub fn new(grid: &'g RoutingGrid, via_cost: i64) -> Self {
        Self {
            grid,
            occupancy: HashMap::new(),
            via_cost: via_cost.max(1),
        }
    }

    /// The via cost in DBU.
    #[must_use]
    pub fn via_cost(&self) -> i64 {
        self.via_cost
    }

    /// The grid this search runs over.
    #[must_use]
    pub fn grid(&self) -> &RoutingGrid {
        self.grid
    }

    /// The number of nodes currently occupied by any net.
    #[must_use]
    pub fn occupied_count(&self) -> usize {
        self.occupancy.len()
    }

    /// Returns the net owning `node`, if any.
    #[must_use]
    pub fn owner(&self, node: GridNode) -> Option<NetId> {
        self.occupancy.get(&node).copied()
    }

    /// Marks every node of `route` as owned by `net`.
    pub fn commit(&mut self, net: NetId, route: &NetRoute) {
        for node in &route.nodes {
            self.occupancy.insert(*node, net);
        }
    }

    /// Releases every node currently owned by `net` (rip-up).
    pub fn rip_up(&mut self, net: NetId) {
        self.occupancy.retain(|_, owner| *owner != net);
    }

    /// Removes all occupancy, returning the grid to empty.
    pub fn clear(&mut self) {
        self.occupancy.clear();
    }

    /// Whether `node` is passable for `net`: in bounds, not a static obstacle, and
    /// either free or already owned by `net` itself.
    fn passable(&self, node: GridNode, net: NetId) -> bool {
        if self.grid.is_blocked(node) {
            return false;
        }
        match self.occupancy.get(&node) {
            None => true,
            Some(owner) => *owner == net,
        }
    }

    /// The in-plane and via neighbours of `node` that are passable for `net`,
    /// each paired with its move cost. Occupied-by-others neighbours are skipped
    /// but recorded in `blockers`.
    fn successors(
        &self,
        node: GridNode,
        net: NetId,
        blockers: &mut HashSet<NetId>,
    ) -> Vec<(GridNode, i64)> {
        let pitch = i64::from(self.grid.pitch());
        let mut out = Vec::with_capacity(6);
        let in_plane = [
            GridNode::new(node.col + 1, node.row, node.layer),
            GridNode::new(node.col - 1, node.row, node.layer),
            GridNode::new(node.col, node.row + 1, node.layer),
            GridNode::new(node.col, node.row - 1, node.layer),
        ];
        for next in in_plane {
            self.push_neighbor(next, net, pitch, &mut out, blockers);
        }
        // Via moves: change layer at the same (col, row).
        if node.layer + 1 < self.grid.layers() {
            let up = GridNode::new(node.col, node.row, node.layer + 1);
            self.push_neighbor(up, net, self.via_cost, &mut out, blockers);
        }
        if node.layer > 0 {
            let down = GridNode::new(node.col, node.row, node.layer - 1);
            self.push_neighbor(down, net, self.via_cost, &mut out, blockers);
        }
        out
    }

    /// Helper for [`Self::successors`]: adds `next` at `cost` if passable, else
    /// records any foreign owner as a rip-up candidate.
    fn push_neighbor(
        &self,
        next: GridNode,
        net: NetId,
        cost: i64,
        out: &mut Vec<(GridNode, i64)>,
        blockers: &mut HashSet<NetId>,
    ) {
        if self.passable(next, net) {
            out.push((next, cost));
        } else if let Some(owner) = self.occupancy.get(&next)
            && *owner != net
        {
            blockers.insert(*owner);
        }
    }

    /// Manhattan distance from `node` to `goal`, scaled to DBU. Ignores the layer
    /// axis so it never overestimates (via cost is real cost the heuristic omits).
    fn heuristic(&self, node: GridNode, goal: GridNode) -> i64 {
        let pitch = i64::from(self.grid.pitch());
        let dc = i64::from((node.col - goal.col).abs());
        let dr = i64::from((node.row - goal.row).abs());
        (dc + dr) * pitch
    }

    /// Runs A\* from `start` to any node in `targets`, for `net`.
    ///
    /// Succeeds at the first target reached. Returns the path and cost, or the set
    /// of foreign nets seen on the frontier when no path exists.
    fn search(
        &self,
        start: GridNode,
        targets: &HashSet<GridNode>,
        net: NetId,
    ) -> Result<(Vec<GridNode>, i64), Vec<NetId>> {
        // Heuristic aims at the nearest target so it stays admissible for a
        // multi-source goal.
        let mut blockers: HashSet<NetId> = HashSet::new();
        let result = astar(
            &start,
            |&node| self.successors(node, net, &mut blockers),
            |&node| {
                targets
                    .iter()
                    .map(|t| self.heuristic(node, *t))
                    .min()
                    .unwrap_or(0)
            },
            |node| targets.contains(node),
        );
        if let Some((path, cost)) = result {
            Ok((path, cost))
        } else {
            let mut v: Vec<NetId> = blockers.into_iter().collect();
            v.sort_unstable();
            Err(v)
        }
    }

    /// Routes `net` so all of its `terminals` are connected, growing a tree.
    ///
    /// The first terminal seeds the connected set; each further terminal is wired
    /// to the nearest already-connected node. Every node found is added to the
    /// connected set so later terminals may join anywhere on the net, which keeps
    /// multi-terminal wirelength low without a separate Steiner pass.
    ///
    /// # Errors
    ///
    /// Returns [`RouteFailure::NotEnoughTerminals`] for degenerate nets, or
    /// [`RouteFailure::Blocked`] (with rip-up candidates) when a terminal cannot be
    /// reached under the current occupancy.
    pub fn route_net(&self, net: NetId, terminals: &[Point]) -> Result<NetRoute, RouteFailure> {
        if terminals.len() < 2 {
            return Err(RouteFailure::NotEnoughTerminals);
        }
        // Map each terminal to its nearest grid node on layer 0.
        let mut pending: Vec<GridNode> = terminals
            .iter()
            .map(|p| self.grid.point_to_node(*p))
            .collect();
        // De-duplicate coincident terminals while preserving order.
        let mut seen = HashSet::new();
        pending.retain(|n| seen.insert(*n));
        if pending.len() < 2 {
            // All terminals collapsed onto one node: a zero-length "connection".
            let node = pending
                .pop()
                .unwrap_or_else(|| self.grid.point_to_node(terminals[0]));
            return Ok(NetRoute {
                nodes: vec![node],
                cost: 0,
            });
        }

        let mut connected: HashSet<GridNode> = HashSet::new();
        let mut ordered: Vec<GridNode> = Vec::new();
        let mut total_cost: i64 = 0;

        let first = pending.remove(0);
        connected.insert(first);
        ordered.push(first);

        for terminal in pending {
            if connected.contains(&terminal) {
                continue;
            }
            match self.search(terminal, &connected, net) {
                Ok((path, cost)) => {
                    total_cost += cost;
                    for node in path {
                        if connected.insert(node) {
                            ordered.push(node);
                        }
                    }
                }
                Err(blockers) => {
                    return Err(RouteFailure::Blocked { blockers });
                }
            }
        }

        Ok(NetRoute {
            nodes: ordered,
            cost: total_cost,
        })
    }
}
