//! The routing grid: the discrete coordinate system the maze router searches.
//!
//! A [`RoutingGrid`] tiles a rectangular world region (in DBU) at a fixed
//! [`pitch`](RoutingGrid::pitch) into `cols` × `rows` tracks, stacked across one
//! or more routing layers. Every reachable location is a [`GridNode`] — a
//! `(col, row, layer)` triple — and the grid converts between grid coordinates
//! and world [`Point`]s so routed paths land back on the DBU grid.
//!
//! Blockage is precomputed: each `(col, row, layer)` is marked passable or
//! blocked once, up front, by testing the track's world footprint (grown by a
//! spacing margin) against the obstacle index. The maze search then only reads
//! this bitmap, so per-node cost stays O(1).

use reticle_geometry::{Point, Rect};

/// A single node in the routing grid: a track intersection on one layer.
///
/// Coordinates are grid indices, not DBU. `col` runs along x, `row` along y, and
/// `layer` selects the routing plane (0 is the lowest). Convert to and from world
/// [`Point`]s with [`RoutingGrid::node_to_point`] and
/// [`RoutingGrid::point_to_node`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct GridNode {
    /// Column index (along x).
    pub col: i32,
    /// Row index (along y).
    pub row: i32,
    /// Routing-layer index (0 is lowest).
    pub layer: u16,
}

impl GridNode {
    /// Creates a node from its column, row, and layer indices.
    #[must_use]
    pub const fn new(col: i32, row: i32, layer: u16) -> Self {
        Self { col, row, layer }
    }
}

/// A rectangular, multi-layer routing grid over a world region.
///
/// The grid owns a per-node blocked bitmap sized `cols * rows * layers`. It is
/// built once from the request's bounding area (see
/// [`RoutingGrid::from_bounds`]) and then queried by the maze router; the grid
/// itself performs no searching.
#[derive(Clone, Debug)]
pub struct RoutingGrid {
    origin: Point,
    pitch: i32,
    cols: i32,
    rows: i32,
    layers: u16,
    /// Blocked bitmap, indexed by [`RoutingGrid::index`]; `true` == blocked.
    blocked: Vec<bool>,
}

impl RoutingGrid {
    /// Builds an empty (fully passable) grid covering `bounds` at `pitch`.
    ///
    /// The world region is padded by one pitch on every side so terminals sitting
    /// exactly on the boundary still have room to turn. `pitch` is clamped to at
    /// least 1 DBU and `layers` to at least 1 so the grid is always non-degenerate.
    #[must_use]
    pub fn from_bounds(bounds: Rect, pitch: i32, layers: u16) -> Self {
        let pitch = pitch.max(1);
        let layers = layers.max(1);
        // Snap the origin down to a pitch multiple and pad by a pitch so edge
        // terminals have a turning lane on either side.
        let pad = pitch;
        let min_x = bounds.min.x.saturating_sub(pad);
        let min_y = bounds.min.y.saturating_sub(pad);
        let max_x = bounds.max.x.saturating_add(pad);
        let max_y = bounds.max.y.saturating_add(pad);
        let origin = Point::new(snap_down(min_x, pitch), snap_down(min_y, pitch));
        // +1 so the far edge is inside the grid; both spans are non-negative.
        let span_x = i64::from(max_x) - i64::from(origin.x);
        let span_y = i64::from(max_y) - i64::from(origin.y);
        let cols = (span_x / i64::from(pitch)) as i32 + 1;
        let rows = (span_y / i64::from(pitch)) as i32 + 1;
        let cols = cols.max(1);
        let rows = rows.max(1);
        let count = (cols as usize)
            .saturating_mul(rows as usize)
            .saturating_mul(layers as usize);
        Self {
            origin,
            pitch,
            cols,
            rows,
            layers,
            blocked: vec![false; count],
        }
    }

    /// The track pitch in DBU.
    #[must_use]
    pub fn pitch(&self) -> i32 {
        self.pitch
    }

    /// The number of columns (x tracks).
    #[must_use]
    pub fn cols(&self) -> i32 {
        self.cols
    }

    /// The number of rows (y tracks).
    #[must_use]
    pub fn rows(&self) -> i32 {
        self.rows
    }

    /// The number of stacked routing layers.
    #[must_use]
    pub fn layers(&self) -> u16 {
        self.layers
    }

    /// The total number of nodes (`cols * rows * layers`).
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.blocked.len()
    }

    /// Returns `true` if `node` lies inside the grid extents.
    #[must_use]
    pub fn in_bounds(&self, node: GridNode) -> bool {
        node.col >= 0
            && node.row >= 0
            && node.col < self.cols
            && node.row < self.rows
            && node.layer < self.layers
    }

    /// The flat bitmap index of `node`, or `None` if out of bounds.
    #[must_use]
    fn index(&self, node: GridNode) -> Option<usize> {
        if !self.in_bounds(node) {
            return None;
        }
        let per_layer = (self.cols as usize) * (self.rows as usize);
        let within = (node.row as usize) * (self.cols as usize) + (node.col as usize);
        Some((node.layer as usize) * per_layer + within)
    }

    /// The world-space center [`Point`] of `node`.
    #[must_use]
    pub fn node_to_point(&self, node: GridNode) -> Point {
        let x = i64::from(self.origin.x) + i64::from(node.col) * i64::from(self.pitch);
        let y = i64::from(self.origin.y) + i64::from(node.row) * i64::from(self.pitch);
        Point::new(clamp_dbu(x), clamp_dbu(y))
    }

    /// The nearest in-plane node (on layer 0) to a world point, clamped to the grid.
    #[must_use]
    pub fn point_to_node(&self, p: Point) -> GridNode {
        self.point_to_node_on(p, 0)
    }

    /// The nearest node to `p` on a specific `layer`, clamped into the grid.
    #[must_use]
    pub fn point_to_node_on(&self, p: Point, layer: u16) -> GridNode {
        let col = div_round(
            i64::from(p.x) - i64::from(self.origin.x),
            i64::from(self.pitch),
        );
        let row = div_round(
            i64::from(p.y) - i64::from(self.origin.y),
            i64::from(self.pitch),
        );
        let col = col.clamp(0, i64::from(self.cols - 1)) as i32;
        let row = row.clamp(0, i64::from(self.rows - 1)) as i32;
        GridNode::new(col, row, layer.min(self.layers - 1))
    }

    /// The world rectangle covered by `node`'s track cell: a pitch-sized box
    /// centered on the node. Used for obstacle testing.
    #[must_use]
    pub fn node_cell_rect(&self, node: GridNode) -> Rect {
        let center = self.node_to_point(node);
        let half = self.pitch / 2;
        Rect::new(center.translate(-half, -half), center.translate(half, half))
    }

    /// Returns `true` if `node` is blocked (out-of-bounds counts as blocked).
    #[must_use]
    pub fn is_blocked(&self, node: GridNode) -> bool {
        match self.index(node) {
            Some(i) => self.blocked[i],
            None => true,
        }
    }

    /// Marks `node` blocked or passable. Out-of-bounds nodes are ignored.
    pub fn set_blocked(&mut self, node: GridNode, blocked: bool) {
        if let Some(i) = self.index(node) {
            self.blocked[i] = blocked;
        }
    }

    /// Marks every node whose cell rectangle intersects `obstacle` (already grown
    /// by any spacing margin) as blocked, across all layers.
    ///
    /// Only the tracks the obstacle actually overlaps are visited, so this is
    /// proportional to the obstacle's footprint rather than the whole grid.
    pub fn block_obstacle(&mut self, obstacle: Rect) {
        self.for_each_overlapping(obstacle, |grid, col, row| {
            for layer in 0..grid.layers {
                grid.set_blocked(GridNode::new(col, row, layer), true);
            }
        });
    }

    /// Marks the tracks a spacing-grown `obstacle` overlaps as blocked, but only on
    /// the single routing `layer` (out-of-range layers are ignored).
    ///
    /// This models a real obstacle that occupies one metal plane: a wire on another
    /// layer can still cross above or below it via a via.
    pub fn block_obstacle_on_layer(&mut self, obstacle: Rect, layer: u16) {
        if layer >= self.layers {
            return;
        }
        self.for_each_overlapping(obstacle, |grid, col, row| {
            grid.set_blocked(GridNode::new(col, row, layer), true);
        });
    }

    /// Visits every `(col, row)` whose track cell overlaps `obstacle`, calling `f`.
    ///
    /// The scan widens the obstacle's node span by one track each way because a
    /// cell centered just outside the obstacle can still overlap it once the
    /// half-pitch cell extent is accounted for.
    fn for_each_overlapping(&mut self, obstacle: Rect, mut f: impl FnMut(&mut Self, i32, i32)) {
        let lo = self.point_to_node(obstacle.min);
        let hi = self.point_to_node(obstacle.max);
        let c0 = (lo.col - 1).max(0);
        let c1 = (hi.col + 1).min(self.cols - 1);
        let r0 = (lo.row - 1).max(0);
        let r1 = (hi.row + 1).min(self.rows - 1);
        for row in r0..=r1 {
            for col in c0..=c1 {
                if self
                    .node_cell_rect(GridNode::new(col, row, 0))
                    .intersects(&obstacle)
                {
                    f(self, col, row);
                }
            }
        }
    }

    /// The number of blocked nodes across all layers.
    #[must_use]
    pub fn blocked_count(&self) -> usize {
        self.blocked.iter().filter(|b| **b).count()
    }
}

/// Rounds `value` to the nearest integer when dividing by a positive `divisor`.
fn div_round(value: i64, divisor: i64) -> i64 {
    debug_assert!(divisor > 0);
    if value >= 0 {
        (value + divisor / 2) / divisor
    } else {
        (value - divisor / 2) / divisor
    }
}

/// Snaps `value` down to the nearest lower multiple of a positive `step`.
fn snap_down(value: i32, step: i32) -> i32 {
    let v = i64::from(value);
    let s = i64::from(step);
    let m = v.rem_euclid(s);
    clamp_dbu(v - m)
}

/// Clamps a widened coordinate back into the DBU (`i32`) range.
fn clamp_dbu(v: i64) -> i32 {
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_node_and_point() {
        let grid =
            RoutingGrid::from_bounds(Rect::new(Point::new(0, 0), Point::new(100, 100)), 10, 2);
        for col in 0..grid.cols() {
            for row in 0..grid.rows() {
                let node = GridNode::new(col, row, 0);
                let back = grid.point_to_node(grid.node_to_point(node));
                assert_eq!(node.col, back.col);
                assert_eq!(node.row, back.row);
            }
        }
    }

    #[test]
    fn blocks_only_overlapping_tracks() {
        let mut grid =
            RoutingGrid::from_bounds(Rect::new(Point::new(0, 0), Point::new(100, 100)), 10, 1);
        let before = grid.blocked_count();
        assert_eq!(before, 0);
        grid.block_obstacle(Rect::new(Point::new(40, 40), Point::new(60, 60)));
        assert!(grid.blocked_count() > 0);
        // A far corner is untouched.
        assert!(!grid.is_blocked(grid.point_to_node(Point::new(0, 0))));
    }
}
