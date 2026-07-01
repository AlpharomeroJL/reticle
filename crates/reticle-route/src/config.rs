//! Tunable parameters for a routing run.

use reticle_geometry::{Dbu, Endcap};

/// Configuration for the [`MazeRouter`](crate::MazeRouter): grid resolution,
/// spacing, wire style, layer count, and the rip-up/reroute budget.
///
/// Construct with [`RouteConfig::default`] and adjust fields, or use the builder
/// methods. All distances are in DBU.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RouteConfig {
    /// Track pitch: the spacing between adjacent grid lines, in DBU.
    pub pitch: Dbu,
    /// Extra clearance added around every obstacle before it blocks tracks, in DBU.
    pub spacing: Dbu,
    /// Width of the emitted wire [`Path`](reticle_geometry::Path)s, in DBU.
    pub wire_width: Dbu,
    /// End-cap style of the emitted wires.
    pub endcap: Endcap,
    /// Number of stacked routing layers (>= 1). Values above 1 enable vias.
    pub layers: u16,
    /// DBU cost of a single via (layer change) during the maze search.
    pub via_cost: i64,
    /// Maximum rip-up-and-reroute passes before giving up on the remaining nets.
    pub max_rip_up_iterations: usize,
}

impl Default for RouteConfig {
    fn default() -> Self {
        Self {
            pitch: 10,
            spacing: 0,
            wire_width: 2,
            endcap: Endcap::Flat,
            layers: 1,
            via_cost: 20,
            max_rip_up_iterations: 8,
        }
    }
}

impl RouteConfig {
    /// Creates a default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the track pitch (clamped to at least 1 DBU) and returns `self`.
    #[must_use]
    pub fn with_pitch(mut self, pitch: Dbu) -> Self {
        self.pitch = pitch.max(1);
        self
    }

    /// Sets the obstacle spacing margin and returns `self`.
    #[must_use]
    pub fn with_spacing(mut self, spacing: Dbu) -> Self {
        self.spacing = spacing.max(0);
        self
    }

    /// Sets the emitted wire width and returns `self`.
    #[must_use]
    pub fn with_wire_width(mut self, width: Dbu) -> Self {
        self.wire_width = width.max(0);
        self
    }

    /// Sets the number of routing layers (clamped to at least 1) and returns `self`.
    #[must_use]
    pub fn with_layers(mut self, layers: u16) -> Self {
        self.layers = layers.max(1);
        self
    }

    /// Sets the via cost and returns `self`.
    #[must_use]
    pub fn with_via_cost(mut self, via_cost: i64) -> Self {
        self.via_cost = via_cost.max(1);
        self
    }

    /// Sets the rip-up/reroute iteration budget and returns `self`.
    #[must_use]
    pub fn with_max_rip_up_iterations(mut self, iterations: usize) -> Self {
        self.max_rip_up_iterations = iterations;
        self
    }
}
