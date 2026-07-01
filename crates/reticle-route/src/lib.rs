//! Routing for Reticle.
//!
//! Wave 2 implements a grid and maze router (Lee / A* via `pathfinding`),
//! multi-net with rip-up and reroute, obstacle avoidance derived from geometry and
//! DRC spacing, and cross-layer vias, with congestion and length reporting.
//!
//! The Wave 0 contract is [`MazeRouter`], a [`Router`] implementation that
//! currently routes nothing.

use reticle_model::{Document, RouteReport, RouteRequest, Router};

/// The grid/maze router (Wave 2).
#[derive(Debug, Default, Clone)]
pub struct MazeRouter;

impl MazeRouter {
    /// Creates a router.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Router for MazeRouter {
    fn route(&mut self, _doc: &mut Document, request: &RouteRequest) -> RouteReport {
        // Wave 2: build the routing grid, run A* per net with rip-up and reroute.
        RouteReport {
            failed: request.nets.len(),
            ..RouteReport::default()
        }
    }
}
