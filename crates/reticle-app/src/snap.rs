//! Geometry-aware snapping and user guides, all pure DBU arithmetic.
//!
//! The grid snap in [`crate::grid`] rounds a point onto the background grid. This
//! module adds the two other things a layout editor snaps to:
//!
//! * **Nearby geometry.** The cursor snaps to the vertices, edge projections,
//!   edge midpoints, and centers of the shapes around it, so a new point lands
//!   exactly on existing geometry instead of merely on-grid.
//! * **User guides.** Draggable horizontal and vertical guide lines, pulled off
//!   the rulers, that the cursor also snaps to.
//!
//! The interesting logic is the nearest-candidate search, and it is a pure
//! function of a raw point, a radius, and a set of candidates, so it lives here
//! and is unit-tested without a scene, a camera, or any drawing. The egui layer
//! feeds candidates from [`crate::culling::SceneIndex`] and turns the returned
//! [`SnapHint`] into an on-canvas indicator; guides are view/session state and do
//! not touch the document.

use reticle_geometry::{Dbu, Point};
use reticle_model::{DrawShape, ShapeKind};

/// What a snap landed on, used to label and color the on-canvas indicator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SnapKind {
    /// A shape vertex (rectangle corner, polygon or path vertex).
    Vertex,
    /// The nearest point along a shape edge (the perpendicular projection).
    Edge,
    /// The midpoint of a shape edge.
    Midpoint,
    /// The center of a shape's bounding box.
    Center,
    /// The intersection with a user guide line.
    Guide,
}

impl SnapKind {
    /// A short human label for the snap indicator caption.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            SnapKind::Vertex => "vertex",
            SnapKind::Edge => "edge",
            SnapKind::Midpoint => "midpoint",
            SnapKind::Center => "center",
            SnapKind::Guide => "guide",
        }
    }
}

/// The result of a successful snap: where the cursor landed and what it caught.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SnapHint {
    /// The snapped world point, in DBU.
    pub point: Point,
    /// What the point snapped to.
    pub kind: SnapKind,
}

/// A single thing the cursor can snap to.
///
/// A [`SnapCandidate::Point`] is a discrete target (a vertex, midpoint, or
/// center); a [`SnapCandidate::Segment`] is an edge, whose snap point is the raw
/// cursor projected onto the segment and clamped to its ends. Segments let the
/// cursor slide along an edge rather than only catching its endpoints.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SnapCandidate {
    /// A discrete point target of the given kind.
    Point {
        /// The target position in DBU.
        at: Point,
        /// What kind of feature this point is.
        kind: SnapKind,
    },
    /// An edge from `a` to `b`; the cursor snaps to its nearest point.
    Segment {
        /// One endpoint in DBU.
        a: Point,
        /// The other endpoint in DBU.
        b: Point,
    },
}

impl SnapCandidate {
    /// The point on this candidate nearest `raw`, with the kind it represents.
    ///
    /// For a discrete point this is the point itself; for a segment it is the
    /// perpendicular projection of `raw` onto the segment, clamped to the ends.
    #[must_use]
    pub fn resolve(self, raw: Point) -> (Point, SnapKind) {
        match self {
            SnapCandidate::Point { at, kind } => (at, kind),
            SnapCandidate::Segment { a, b } => (closest_on_segment(raw, a, b), SnapKind::Edge),
        }
    }
}

/// Which way a guide line runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    /// A horizontal guide, pinning a world `y`. Pulled from the top ruler.
    Horizontal,
    /// A vertical guide, pinning a world `x`. Pulled from the left ruler.
    Vertical,
}

/// A single draggable guide line at a fixed world coordinate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Guide {
    /// Whether the guide is horizontal (pins `y`) or vertical (pins `x`).
    pub axis: Axis,
    /// The world coordinate the guide sits at, in DBU: `y` if horizontal, `x` if
    /// vertical.
    pub coord: Dbu,
}

impl Guide {
    /// A horizontal guide pinning world `y`.
    #[must_use]
    pub fn horizontal(y: Dbu) -> Self {
        Self {
            axis: Axis::Horizontal,
            coord: y,
        }
    }

    /// A vertical guide pinning world `x`.
    #[must_use]
    pub fn vertical(x: Dbu) -> Self {
        Self {
            axis: Axis::Vertical,
            coord: x,
        }
    }
}

/// The set of user guides, plus the tuning knobs for geometry and guide snapping.
///
/// Grid visibility and grid snapping live on [`crate::grid::GridSettings`]; this
/// struct owns only the settings unique to geometry and guide snapping. The panel
/// surfaces both together. Guides are pure view state and are never written to the
/// document, so no undo integration is needed.
#[derive(Clone, PartialEq, Debug)]
pub struct SnapState {
    /// Whether the cursor snaps to nearby shape geometry.
    pub geometry_enabled: bool,
    /// Whether the cursor snaps to user guide lines.
    pub guide_enabled: bool,
    /// The snap radius, in screen pixels. A candidate only catches the cursor when
    /// it is within this many pixels; the world-space radius is derived from the
    /// zoom so the feel is zoom-independent.
    pub radius_px: f32,
    /// The user guides, in insertion order.
    pub guides: Vec<Guide>,
}

impl Default for SnapState {
    fn default() -> Self {
        Self {
            geometry_enabled: true,
            guide_enabled: true,
            radius_px: 12.0,
            guides: Vec::new(),
        }
    }
}

/// The smallest snap radius the UI will accept, in pixels.
pub const MIN_RADIUS_PX: f32 = 2.0;
/// The largest snap radius the UI will accept, in pixels.
pub const MAX_RADIUS_PX: f32 = 40.0;

impl SnapState {
    /// The snap radius in world DBU at the given zoom (pixels per DBU).
    ///
    /// A candidate within this many DBU of the cursor is eligible. Returns at least
    /// one DBU so snapping still works when fully zoomed in, and saturates to a
    /// large value at a degenerate (non-positive) zoom rather than dividing by zero.
    #[must_use]
    pub fn radius_dbu(&self, pixels_per_dbu: f64) -> i64 {
        if pixels_per_dbu <= 0.0 {
            return i64::from(Dbu::MAX);
        }
        let r = f64::from(self.radius_px) / pixels_per_dbu;
        (r.ceil() as i64).max(1)
    }

    /// Adds a guide, keeping the list free of exact duplicates.
    pub fn add_guide(&mut self, guide: Guide) {
        if !self.guides.contains(&guide) {
            self.guides.push(guide);
        }
    }

    /// Removes the guide at `index`, if it exists.
    pub fn remove_guide(&mut self, index: usize) {
        if index < self.guides.len() {
            self.guides.remove(index);
        }
    }

    /// Removes every guide.
    pub fn clear_guides(&mut self) {
        self.guides.clear();
    }

    /// Snaps `raw` to the nearest guide within `radius_dbu`, if guide snapping is on.
    ///
    /// A horizontal guide pins `y` and leaves `x`; a vertical guide pins `x` and
    /// leaves `y`. When a horizontal and a vertical guide are both in range their
    /// intersection is used, so the cursor lands on the crossing point. The nearest
    /// guide on each axis wins. Returns `None` when guide snapping is disabled, no
    /// guide is within range, or the radius is non-positive.
    #[must_use]
    pub fn snap_to_guides(&self, raw: Point, radius_dbu: i64) -> Option<SnapHint> {
        if !self.guide_enabled || radius_dbu <= 0 {
            return None;
        }
        let mut best_x: Option<(i64, Dbu)> = None;
        let mut best_y: Option<(i64, Dbu)> = None;
        for g in &self.guides {
            match g.axis {
                Axis::Vertical => {
                    let d = (i64::from(g.coord) - i64::from(raw.x)).abs();
                    if d <= radius_dbu && best_x.is_none_or(|(bd, _)| d < bd) {
                        best_x = Some((d, g.coord));
                    }
                }
                Axis::Horizontal => {
                    let d = (i64::from(g.coord) - i64::from(raw.y)).abs();
                    if d <= radius_dbu && best_y.is_none_or(|(bd, _)| d < bd) {
                        best_y = Some((d, g.coord));
                    }
                }
            }
        }
        match (best_x, best_y) {
            (None, None) => None,
            (bx, by) => {
                let x = bx.map_or(raw.x, |(_, c)| c);
                let y = by.map_or(raw.y, |(_, c)| c);
                Some(SnapHint {
                    point: Point::new(x, y),
                    kind: SnapKind::Guide,
                })
            }
        }
    }
}

/// Finds the nearest geometry candidate to `raw` within `radius_dbu`.
///
/// Each candidate is resolved to its closest point (a vertex, midpoint, or center
/// as-is; an edge to its projection) and the closest resolved point within the
/// radius wins. Ties break toward the earlier candidate, and toward the
/// higher-priority kind so an exact vertex is preferred over the edge it lies on.
/// Returns `None` when nothing is in range or the radius is non-positive.
#[must_use]
pub fn nearest_geometry<I>(raw: Point, radius_dbu: i64, candidates: I) -> Option<SnapHint>
where
    I: IntoIterator<Item = SnapCandidate>,
{
    if radius_dbu <= 0 {
        return None;
    }
    let radius_sq = radius_dbu.saturating_mul(radius_dbu);
    let mut best: Option<(i64, u8, SnapHint)> = None;
    for cand in candidates {
        let (point, kind) = cand.resolve(raw);
        let dist_sq = raw.distance_squared(point);
        if dist_sq > radius_sq {
            continue;
        }
        let prio = kind_priority(kind);
        let better = match best {
            None => true,
            Some((bd, bp, _)) => dist_sq < bd || (dist_sq == bd && prio < bp),
        };
        if better {
            best = Some((dist_sq, prio, SnapHint { point, kind }));
        }
    }
    best.map(|(_, _, hint)| hint)
}

/// Combines a geometry snap and a guide snap, preferring whichever is closer.
///
/// This is the single entry point the canvas uses: it runs [`nearest_geometry`]
/// over the supplied candidates and [`SnapState::snap_to_guides`], then returns the
/// hint whose point is nearer `raw`. Geometry wins ties so a snap lands on real
/// geometry rather than a guide passing through it. Returns `None` when neither
/// produces a hit.
#[must_use]
pub fn best_snap<I>(
    state: &SnapState,
    raw: Point,
    radius_dbu: i64,
    candidates: I,
) -> Option<SnapHint>
where
    I: IntoIterator<Item = SnapCandidate>,
{
    let geom = if state.geometry_enabled {
        nearest_geometry(raw, radius_dbu, candidates)
    } else {
        None
    };
    let guide = state.snap_to_guides(raw, radius_dbu);
    match (geom, guide) {
        (Some(g), Some(gu)) => {
            if raw.distance_squared(gu.point) < raw.distance_squared(g.point) {
                Some(gu)
            } else {
                Some(g)
            }
        }
        (Some(g), None) => Some(g),
        (None, Some(gu)) => Some(gu),
        (None, None) => None,
    }
}

/// Emits every snap candidate a single shape offers: its vertices, edge segments,
/// edge midpoints, and bounding-box center.
///
/// Rectangles contribute four corners, four edges, four edge midpoints, and the
/// box center. Polygons and paths contribute each vertex, each segment between
/// consecutive vertices (the polygon also closes the loop), the midpoints of those
/// segments, and the bounding-box center. The canvas gathers these across the
/// shapes near the cursor and feeds them to [`nearest_geometry`].
#[must_use]
pub fn shape_candidates(shape: &DrawShape) -> Vec<SnapCandidate> {
    match &shape.kind {
        ShapeKind::Rect(r) => {
            let corners = [
                Point::new(r.min.x, r.min.y),
                Point::new(r.max.x, r.min.y),
                Point::new(r.max.x, r.max.y),
                Point::new(r.min.x, r.max.y),
            ];
            candidates_from_ring(&corners, true)
        }
        ShapeKind::Polygon(p) => candidates_from_ring(p.vertices(), true),
        ShapeKind::Path(p) => candidates_from_ring(p.points(), false),
    }
}

/// Builds candidates from an ordered ring or open chain of vertices.
///
/// Each vertex becomes a [`SnapKind::Vertex`] point; each segment between
/// consecutive vertices becomes an edge candidate and an edge-midpoint point; the
/// overall bounding-box center becomes a [`SnapKind::Center`] point. When `closed`
/// is set the segment from the last vertex back to the first is included too, so a
/// polygon's closing edge snaps like any other.
fn candidates_from_ring(verts: &[Point], closed: bool) -> Vec<SnapCandidate> {
    let mut out = Vec::new();
    if verts.is_empty() {
        return out;
    }
    for &v in verts {
        out.push(SnapCandidate::Point {
            at: v,
            kind: SnapKind::Vertex,
        });
    }
    let segment_count = if closed {
        verts.len()
    } else {
        verts.len().saturating_sub(1)
    };
    for i in 0..segment_count {
        let a = verts[i];
        let b = verts[(i + 1) % verts.len()];
        out.push(SnapCandidate::Segment { a, b });
        out.push(SnapCandidate::Point {
            at: midpoint(a, b),
            kind: SnapKind::Midpoint,
        });
    }
    if let Some(c) = ring_center(verts) {
        out.push(SnapCandidate::Point {
            at: c,
            kind: SnapKind::Center,
        });
    }
    out
}

/// The bounding-box center of a set of vertices, or `None` if empty.
fn ring_center(verts: &[Point]) -> Option<Point> {
    let first = *verts.first()?;
    let mut min_x = first.x;
    let mut min_y = first.y;
    let mut max_x = first.x;
    let mut max_y = first.y;
    for &p in &verts[1..] {
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
    }
    Some(Point::new(
        midpoint_scalar(min_x, max_x),
        midpoint_scalar(min_y, max_y),
    ))
}

/// The midpoint of two points, each coordinate rounded toward zero on a half.
fn midpoint(a: Point, b: Point) -> Point {
    Point::new(midpoint_scalar(a.x, b.x), midpoint_scalar(a.y, b.y))
}

/// The integer midpoint of two DBU scalars, computed in [`i64`] to avoid overflow.
fn midpoint_scalar(a: Dbu, b: Dbu) -> Dbu {
    let m = i64::midpoint(i64::from(a), i64::from(b));
    m.clamp(i64::from(Dbu::MIN), i64::from(Dbu::MAX)) as Dbu
}

/// The point on the segment `seg_a`..`seg_b` closest to `point`, clamped to the
/// endpoints.
///
/// Solves the standard projection `t = (p - a) . (b - a) / |b - a|^2`, clamps `t`
/// to `[0, 1]`, and returns `a + t (b - a)` rounded to DBU. A degenerate segment
/// (`seg_a == seg_b`) returns `seg_a`. All intermediate math is in `f64` so a long
/// edge does not overflow.
#[must_use]
pub fn closest_on_segment(point: Point, seg_a: Point, seg_b: Point) -> Point {
    let ax = f64::from(seg_a.x);
    let ay = f64::from(seg_a.y);
    let bx = f64::from(seg_b.x);
    let by = f64::from(seg_b.y);
    let px = f64::from(point.x);
    let py = f64::from(point.y);
    let dx = bx - ax;
    let dy = by - ay;
    let len_sq = dx * dx + dy * dy;
    if len_sq <= 0.0 {
        return seg_a;
    }
    let along = (((px - ax) * dx + (py - ay) * dy) / len_sq).clamp(0.0, 1.0);
    let foot_x = ax + along * dx;
    let foot_y = ay + along * dy;
    Point::new(round_dbu(foot_x), round_dbu(foot_y))
}

/// Rounds an `f64` DBU coordinate to the nearest integer, saturating into range.
fn round_dbu(v: f64) -> Dbu {
    let r = v.round();
    if r >= f64::from(Dbu::MAX) {
        Dbu::MAX
    } else if r <= f64::from(Dbu::MIN) {
        Dbu::MIN
    } else {
        r as Dbu
    }
}

/// Tie-break priority for a snap kind: lower wins.
///
/// A vertex beats a midpoint or center, which beat a bare edge projection, so when
/// two candidates sit at exactly the same distance the more specific feature is
/// chosen. Guides rank last because a real geometry hit at the same spot is the
/// more useful snap.
fn kind_priority(kind: SnapKind) -> u8 {
    match kind {
        SnapKind::Vertex => 0,
        SnapKind::Midpoint => 1,
        SnapKind::Center => 2,
        SnapKind::Edge => 3,
        SnapKind::Guide => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::{Endcap, LayerId, Path, Polygon, Rect};

    /// A layer for test shapes; snapping ignores the layer, so the value is
    /// arbitrary.
    const L: LayerId = LayerId::new(0, 0);

    fn rect_shape(min: Point, max: Point) -> DrawShape {
        DrawShape::new(L, ShapeKind::Rect(Rect::new(min, max)))
    }

    #[test]
    fn nearest_vertex_within_radius_wins() {
        // A rectangle corner at (100, 100); a cursor diagonally outside the corner
        // is nearest the corner itself (both incident edges clamp back to it), so
        // the vertex wins.
        let shape = rect_shape(Point::new(100, 100), Point::new(500, 300));
        let cands = shape_candidates(&shape);
        let hint = nearest_geometry(Point::new(95, 95), 20, cands).expect("a snap");
        assert_eq!(hint.point, Point::new(100, 100));
        assert_eq!(hint.kind, SnapKind::Vertex);
    }

    #[test]
    fn no_snap_when_all_candidates_outside_radius() {
        let shape = rect_shape(Point::new(100, 100), Point::new(500, 300));
        let cands = shape_candidates(&shape);
        // Far from every corner, edge, midpoint, and center.
        assert!(nearest_geometry(Point::new(100_000, 100_000), 20, cands).is_none());
    }

    #[test]
    fn snaps_onto_edge_projection() {
        // Cursor near the bottom edge (y = 100), away from both corners and away
        // from the edge midpoint (x = 500), snaps to the perpendicular foot on the
        // edge rather than to any discrete point.
        let shape = rect_shape(Point::new(0, 100), Point::new(1000, 400));
        let cands = shape_candidates(&shape);
        let hint = nearest_geometry(Point::new(300, 108), 20, cands).expect("a snap");
        assert_eq!(hint.kind, SnapKind::Edge);
        assert_eq!(hint.point, Point::new(300, 100));
    }

    #[test]
    fn snaps_to_edge_midpoint() {
        // Exactly at the midpoint of the bottom edge: the midpoint candidate (a
        // discrete point) is at distance zero and outranks the edge projection.
        let shape = rect_shape(Point::new(0, 0), Point::new(1000, 400));
        let cands = shape_candidates(&shape);
        let hint = nearest_geometry(Point::new(500, 0), 20, cands).expect("a snap");
        assert_eq!(hint.point, Point::new(500, 0));
        assert_eq!(hint.kind, SnapKind::Midpoint);
    }

    #[test]
    fn snaps_to_center() {
        let shape = rect_shape(Point::new(0, 0), Point::new(400, 200));
        let cands = shape_candidates(&shape);
        let hint = nearest_geometry(Point::new(203, 98), 20, cands).expect("a snap");
        assert_eq!(hint.point, Point::new(200, 100));
        assert_eq!(hint.kind, SnapKind::Center);
    }

    #[test]
    fn vertex_beats_edge_on_a_tie() {
        // At a corner the vertex and both incident edges all resolve to the same
        // point; the vertex must win the tie.
        let shape = rect_shape(Point::new(0, 0), Point::new(1000, 400));
        let cands = shape_candidates(&shape);
        let hint = nearest_geometry(Point::new(0, 0), 20, cands).expect("a snap");
        assert_eq!(hint.kind, SnapKind::Vertex);
    }

    #[test]
    fn polygon_vertices_and_edges_snap() {
        // A triangle: snap to a vertex and to an edge projection.
        let poly = DrawShape::new(
            L,
            ShapeKind::Polygon(Polygon::new(vec![
                Point::new(0, 0),
                Point::new(1000, 0),
                Point::new(0, 1000),
            ])),
        );
        let cands = shape_candidates(&poly);
        // Diagonally outside the (0, 0) corner: the incident edges clamp back to the
        // corner, so the vertex is the nearest point.
        let v = nearest_geometry(Point::new(-3, -3), 20, cands.clone()).expect("vertex");
        assert_eq!(v.point, Point::new(0, 0));
        assert_eq!(v.kind, SnapKind::Vertex);
        // Near the bottom edge (y = 0), off the midpoint (x = 500), between the two
        // corners: the edge projection wins.
        let e = nearest_geometry(Point::new(300, 6), 20, cands).expect("edge");
        assert_eq!(e.kind, SnapKind::Edge);
        assert_eq!(e.point, Point::new(300, 0));
    }

    #[test]
    fn path_endpoints_are_vertices_and_closing_edge_is_absent() {
        // An open two-point path: its two points are vertices, the single segment
        // between them snaps, but there is no closing edge back to the start.
        let path = DrawShape::new(
            L,
            ShapeKind::Path(Path::new(
                vec![Point::new(0, 0), Point::new(1000, 0)],
                50,
                Endcap::Flat,
            )),
        );
        let cands = shape_candidates(&path);
        let seg_count = cands
            .iter()
            .filter(|c| matches!(c, SnapCandidate::Segment { .. }))
            .count();
        assert_eq!(seg_count, 1, "open path has exactly one segment");
        let hint = nearest_geometry(Point::new(1003, -2), 20, cands).expect("endpoint");
        assert_eq!(hint.point, Point::new(1000, 0));
        assert_eq!(hint.kind, SnapKind::Vertex);
    }

    #[test]
    fn polygon_has_closing_edge() {
        let poly = DrawShape::new(
            L,
            ShapeKind::Polygon(Polygon::new(vec![
                Point::new(0, 0),
                Point::new(1000, 0),
                Point::new(1000, 1000),
                Point::new(0, 1000),
            ])),
        );
        let cands = shape_candidates(&poly);
        let seg_count = cands
            .iter()
            .filter(|c| matches!(c, SnapCandidate::Segment { .. }))
            .count();
        // Four vertices, closed: four segments including the closing edge.
        assert_eq!(seg_count, 4);
    }

    #[test]
    fn closest_on_segment_clamps_to_ends() {
        let a = Point::new(0, 0);
        let b = Point::new(100, 0);
        // Beyond `b`: clamps to `b`.
        assert_eq!(closest_on_segment(Point::new(200, 50), a, b), b);
        // Before `a`: clamps to `a`.
        assert_eq!(closest_on_segment(Point::new(-50, 50), a, b), a);
        // Perpendicular foot in the middle.
        assert_eq!(
            closest_on_segment(Point::new(40, 30), a, b),
            Point::new(40, 0)
        );
    }

    #[test]
    fn closest_on_degenerate_segment_is_the_point() {
        let a = Point::new(7, 9);
        assert_eq!(closest_on_segment(Point::new(100, 100), a, a), a);
    }

    #[test]
    fn guide_snaps_single_axis() {
        let mut s = SnapState::default();
        s.add_guide(Guide::vertical(500));
        // Near the vertical guide: x pins to 500, y is left untouched.
        let hint = s.snap_to_guides(Point::new(496, 123), 20).expect("guide");
        assert_eq!(hint.point, Point::new(500, 123));
        assert_eq!(hint.kind, SnapKind::Guide);
        // Too far: no snap.
        assert!(s.snap_to_guides(Point::new(400, 123), 20).is_none());
    }

    #[test]
    fn crossing_guides_snap_to_intersection() {
        let mut s = SnapState::default();
        s.add_guide(Guide::vertical(500));
        s.add_guide(Guide::horizontal(300));
        let hint = s
            .snap_to_guides(Point::new(505, 296), 20)
            .expect("intersection");
        assert_eq!(hint.point, Point::new(500, 300));
    }

    #[test]
    fn nearest_guide_on_axis_wins() {
        let mut s = SnapState::default();
        s.add_guide(Guide::vertical(500));
        s.add_guide(Guide::vertical(508));
        // Cursor at 505 is nearer 508.
        let hint = s.snap_to_guides(Point::new(505, 0), 20).expect("guide");
        assert_eq!(hint.point.x, 508);
    }

    #[test]
    fn guide_snap_respects_toggle() {
        let mut s = SnapState::default();
        s.add_guide(Guide::vertical(500));
        s.guide_enabled = false;
        assert!(s.snap_to_guides(Point::new(500, 0), 20).is_none());
    }

    #[test]
    fn add_guide_dedupes_and_remove_works() {
        let mut s = SnapState::default();
        s.add_guide(Guide::vertical(500));
        s.add_guide(Guide::vertical(500));
        assert_eq!(s.guides.len(), 1, "exact duplicate is not added twice");
        s.add_guide(Guide::horizontal(500));
        assert_eq!(s.guides.len(), 2, "different axis is a distinct guide");
        s.remove_guide(0);
        assert_eq!(s.guides, vec![Guide::horizontal(500)]);
        s.clear_guides();
        assert!(s.guides.is_empty());
    }

    #[test]
    fn radius_scales_inversely_with_zoom() {
        let s = SnapState::default();
        // Zoomed out (few pixels per DBU): a large world radius.
        let wide = s.radius_dbu(0.01);
        // Zoomed in (many pixels per DBU): a small world radius, but never zero.
        let tight = s.radius_dbu(10.0);
        assert!(wide > tight);
        assert!(tight >= 1);
        // Degenerate zoom does not divide by zero.
        assert!(s.radius_dbu(0.0) > 0);
    }

    #[test]
    fn best_snap_prefers_the_nearer_of_geometry_and_guide() {
        let mut s = SnapState::default();
        // A guide loosely in x-range, and a shape corner right under the cursor.
        s.add_guide(Guide::vertical(80));
        let shape = rect_shape(Point::new(100, 100), Point::new(500, 300));
        let cands = shape_candidates(&shape);
        // Diagonally outside the (100, 100) corner: geometry catches the corner at
        // distance ~7, nearer than the guide 15 DBU away in x, so geometry wins.
        let hint = best_snap(&s, Point::new(95, 95), 40, cands).expect("a snap");
        assert_eq!(hint.point, Point::new(100, 100));
        assert_eq!(hint.kind, SnapKind::Vertex);
    }

    #[test]
    fn best_snap_uses_guide_when_geometry_disabled() {
        let mut s = SnapState {
            geometry_enabled: false,
            ..SnapState::default()
        };
        s.add_guide(Guide::horizontal(200));
        let shape = rect_shape(Point::new(0, 0), Point::new(50, 50));
        let cands = shape_candidates(&shape);
        // Geometry is off; only the guide can catch, pinning y to 200.
        let hint = best_snap(&s, Point::new(9999, 205), 20, cands).expect("guide");
        assert_eq!(hint.point.y, 200);
        assert_eq!(hint.kind, SnapKind::Guide);
    }

    #[test]
    fn best_snap_none_when_nothing_in_range() {
        let s = SnapState::default();
        let shape = rect_shape(Point::new(0, 0), Point::new(50, 50));
        let cands = shape_candidates(&shape);
        assert!(best_snap(&s, Point::new(100_000, 100_000), 20, cands).is_none());
    }
}
