//! Drawing tools and vertex-level editing: the window-free state and geometry.
//!
//! This module is the logic behind the [`Tool::DrawRect`](crate::tool::Tool::DrawRect),
//! [`Tool::DrawPolygon`](crate::tool::Tool::DrawPolygon),
//! [`Tool::DrawPath`](crate::tool::Tool::DrawPath), and
//! [`Tool::EditVertex`](crate::tool::Tool::EditVertex) tools. As with the rest of the
//! app, everything here is pure DBU arithmetic and small state machines so the
//! interesting behavior is unit-tested without a window; the egui layer in
//! [`app`](crate::app) only converts pixels to world points, calls in, and paints the
//! preview the state exposes.
//!
//! # What lives here
//!
//! * [`rect_from_drag`], the rubber-band rectangle with the shift (square) and
//!   alt/ctrl (from-center) modifier constraints.
//! * [`PolyBuilder`] and [`PathBuilder`], the click-to-place vertex accumulators for
//!   the polygon and path tools, plus the path's width and end-cap settings.
//! * The vertex-edit geometry ([`insert_vertex_on_segment`], [`delete_vertex`],
//!   [`nearest_vertex`], [`nearest_segment_insertion`]) that drives dragging,
//!   inserting, and deleting a vertex on a selected shape.
//! * [`DrawState`], the one persistent field the app carries: the in-progress
//!   builders, the path width/end-cap the user picked, and the active vertex-edit
//!   grab.
//!
//! The document mutations these produce ([`Edit::AddShape`](reticle_model::Edit) and a
//! remove-then-add for a vertex edit) are issued by the app through its undo history;
//! this module never touches the document.

use reticle_geometry::{Dbu, Endcap, Path, Point, Polygon, Rect};

/// The modifier constraints that refine a rubber-band rectangle drag.
///
/// Both may be combined: a square drawn from its center holds when `square` and
/// `from_center` are both set.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct RectMods {
    /// Constrain to a square (the shorter side is grown to the longer). Bound to
    /// shift in the UI.
    pub square: bool,
    /// Treat the anchor as the rectangle's center rather than a corner. Bound to
    /// alt or ctrl in the UI.
    pub from_center: bool,
}

impl RectMods {
    /// Reads the modifier flags the UI passes (shift, alt, ctrl).
    ///
    /// Shift constrains to a square; either alt or ctrl draws from the center.
    #[must_use]
    pub fn new(shift: bool, alt: bool, ctrl: bool) -> Self {
        Self {
            square: shift,
            from_center: alt || ctrl,
        }
    }
}

/// Builds the rectangle for a rubber-band drag from `anchor` to `cursor` under
/// `mods`.
///
/// With no modifiers this is the axis-aligned box spanning the two points. The
/// `square` modifier grows the shorter axis so width and height match the longer
/// one, keeping the corner under the cursor in the cursor's quadrant. The
/// `from_center` modifier reinterprets `anchor` as the center and mirrors the cursor
/// offset to the opposite corner, so the rectangle grows symmetrically. The two
/// combine: a centered square uses the larger half-extent on both axes.
///
/// The returned [`Rect`] is normalized (`min <= max`); it may be empty (zero width or
/// height) when the two points share a coordinate, which the caller treats as "no
/// shape yet".
#[must_use]
pub fn rect_from_drag(anchor: Point, cursor: Point, mods: RectMods) -> Rect {
    let dx = i64::from(cursor.x) - i64::from(anchor.x);
    let dy = i64::from(cursor.y) - i64::from(anchor.y);

    if mods.from_center {
        // `anchor` is the center. Half-extents come from the cursor offset; a square
        // uses the larger magnitude on both axes. The rectangle is the center plus
        // and minus the (signed) half-extents, so the cursor stays on a corner.
        let (mut hx, mut hy) = (dx, dy);
        if mods.square {
            let m = hx.abs().max(hy.abs());
            hx = if hx < 0 { -m } else { m };
            hy = if hy < 0 { -m } else { m };
        }
        let cx = i64::from(anchor.x);
        let cy = i64::from(anchor.y);
        Rect::new(
            Point::new(sat(cx - hx), sat(cy - hy)),
            Point::new(sat(cx + hx), sat(cy + hy)),
        )
    } else {
        // `anchor` is a corner. A square grows the shorter side to the longer,
        // keeping the far corner in the cursor's quadrant (sign of the offset).
        let (mut ex, mut ey) = (dx, dy);
        if mods.square {
            let m = ex.abs().max(ey.abs());
            ex = if ex < 0 { -m } else { m };
            ey = if ey < 0 { -m } else { m };
        }
        let ax = i64::from(anchor.x);
        let ay = i64::from(anchor.y);
        Rect::new(anchor, Point::new(sat(ax + ex), sat(ay + ey)))
    }
}

/// Saturating cast from the widened `i64` used in rectangle math back to a [`Dbu`].
fn sat(v: i64) -> Dbu {
    v.clamp(i64::from(Dbu::MIN), i64::from(Dbu::MAX)) as Dbu
}

/// A click-to-place accumulator for the polygon tool.
///
/// Each click appends a vertex (deduplicating an immediate repeat so a double-click
/// that both places and closes does not leave a zero-length edge). The ring is
/// implicitly closed, matching [`Polygon`]; the tool finishes once three or more
/// distinct vertices are placed.
#[derive(Clone, Debug, Default)]
pub struct PolyBuilder {
    vertices: Vec<Point>,
}

impl PolyBuilder {
    /// A fresh, empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The vertices placed so far, in order.
    #[must_use]
    pub fn vertices(&self) -> &[Point] {
        &self.vertices
    }

    /// Whether no vertex has been placed yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    /// Number of vertices placed so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.vertices.len()
    }

    /// Appends a vertex, ignoring an immediate duplicate of the last point.
    ///
    /// Suppressing the repeat keeps a closing double-click (place then finish on the
    /// same pixel) from inserting a degenerate edge.
    pub fn push(&mut self, p: Point) {
        if self.vertices.last() != Some(&p) {
            self.vertices.push(p);
        }
    }

    /// Whether the ring has enough distinct vertices to form a polygon.
    #[must_use]
    pub fn can_finish(&self) -> bool {
        self.vertices.len() >= 3
    }

    /// Consumes the builder into a [`Polygon`] if it has at least three vertices.
    ///
    /// Returns `None` (and drops nothing the caller needs) when there are too few
    /// vertices, so a stray finish gesture is a no-op rather than an empty shape.
    #[must_use]
    pub fn finish(self) -> Option<Polygon> {
        if self.vertices.len() >= 3 {
            Some(Polygon::new(self.vertices))
        } else {
            None
        }
    }

    /// Clears all placed vertices.
    pub fn clear(&mut self) {
        self.vertices.clear();
    }
}

/// A click-to-place accumulator for the path tool, carrying the width and end cap.
///
/// Like [`PolyBuilder`] it dedups an immediate repeat; a path finishes with two or
/// more distinct points (a single segment is a valid wire).
#[derive(Clone, Debug)]
pub struct PathBuilder {
    points: Vec<Point>,
    width: Dbu,
    endcap: Endcap,
}

impl Default for PathBuilder {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            width: DEFAULT_PATH_WIDTH,
            endcap: Endcap::Flat,
        }
    }
}

/// The width, in DBU, a fresh path tool starts with.
pub const DEFAULT_PATH_WIDTH: Dbu = 100;

impl PathBuilder {
    /// A fresh builder with the default width and a flat cap.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The points placed so far, in order.
    #[must_use]
    pub fn points(&self) -> &[Point] {
        &self.points
    }

    /// Whether no point has been placed yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Number of points placed so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// The current path width in DBU.
    #[must_use]
    pub fn width(&self) -> Dbu {
        self.width
    }

    /// The current end-cap style.
    #[must_use]
    pub fn endcap(&self) -> Endcap {
        self.endcap
    }

    /// Sets the width in DBU, clamped to at least one so a path is never degenerate.
    pub fn set_width(&mut self, width: Dbu) {
        self.width = width.max(1);
    }

    /// Sets the end-cap style applied on finish.
    pub fn set_endcap(&mut self, endcap: Endcap) {
        self.endcap = endcap;
    }

    /// Appends a point, ignoring an immediate duplicate of the last one.
    pub fn push(&mut self, p: Point) {
        if self.points.last() != Some(&p) {
            self.points.push(p);
        }
    }

    /// Whether the polyline has enough distinct points to form a path.
    #[must_use]
    pub fn can_finish(&self) -> bool {
        self.points.len() >= 2
    }

    /// Consumes the builder into a [`Path`] if it has at least two points.
    ///
    /// The width and end cap carried by the builder are baked into the path.
    #[must_use]
    pub fn finish(self) -> Option<Path> {
        if self.points.len() >= 2 {
            Some(Path::new(self.points, self.width, self.endcap))
        } else {
            None
        }
    }

    /// Clears the placed points, keeping the width and end-cap settings.
    pub fn clear(&mut self) {
        self.points.clear();
    }
}

/// The index of the vertex nearest to `p`, if any is within `radius_dbu`.
///
/// Distance is exact squared DBU (no floating point), so ties resolve to the
/// lower-indexed vertex deterministically. Returns `None` for an empty ring or when
/// the closest vertex is farther than the pick radius.
#[must_use]
pub fn nearest_vertex(vertices: &[Point], p: Point, radius_dbu: i64) -> Option<usize> {
    let r2 = radius_dbu.saturating_mul(radius_dbu);
    let mut best: Option<(usize, i64)> = None;
    for (i, v) in vertices.iter().enumerate() {
        let d2 = v.distance_squared(p);
        if d2 <= r2 && best.is_none_or(|(_, bd)| d2 < bd) {
            best = Some((i, d2));
        }
    }
    best.map(|(i, _)| i)
}

/// Where a new vertex should be inserted for a click near a ring edge.
///
/// Scans every segment (for a polygon the closing edge back to the first vertex is
/// included; for an open path it is not) and returns the insertion index and the
/// snapped point on the segment nearest `p`, when that distance is within
/// `radius_dbu`. The insertion index is the position the new vertex takes in the
/// vertex list (after the segment's start vertex), so the caller can splice it in
/// directly.
///
/// `closed` selects polygon (closing edge included) versus path (open) topology.
#[must_use]
pub fn nearest_segment_insertion(
    vertices: &[Point],
    p: Point,
    radius_dbu: i64,
    closed: bool,
) -> Option<VertexInsertion> {
    if vertices.len() < 2 {
        return None;
    }
    let r2 = radius_dbu.saturating_mul(radius_dbu);
    let seg_count = if closed {
        vertices.len()
    } else {
        vertices.len() - 1
    };
    let mut best: Option<(usize, Point, i64)> = None;
    for i in 0..seg_count {
        let a = vertices[i];
        let b = vertices[(i + 1) % vertices.len()];
        let (proj, d2) = project_point_on_segment(a, b, p);
        if d2 <= r2 && best.is_none_or(|(_, _, bd)| d2 < bd) {
            best = Some((i, proj, d2));
        }
    }
    best.map(|(seg, at, _)| VertexInsertion {
        index: seg + 1,
        point: at,
    })
}

/// The result of hit-testing a click against a ring's edges for vertex insertion.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VertexInsertion {
    /// The position the new vertex takes in the vertex list.
    pub index: usize,
    /// The point on the segment nearest the click (already snapped by projection).
    pub point: Point,
}

/// Inserts `point` at `index` in a copy of `vertices`.
///
/// `index` is clamped to the valid range, so an out-of-range index appends rather
/// than panics. Used to add a vertex mid-edge during vertex editing.
#[must_use]
pub fn insert_vertex_on_segment(vertices: &[Point], index: usize, point: Point) -> Vec<Point> {
    let mut out = vertices.to_vec();
    let at = index.min(out.len());
    out.insert(at, point);
    out
}

/// Deletes the vertex at `index` in a copy of `vertices`, refusing to drop below the
/// `floor` count.
///
/// A polygon must keep at least three vertices and a path at least two, so the caller
/// passes the appropriate floor; when the ring is already at the floor the vertices
/// are returned unchanged and the boolean is `false`. An out-of-range index is also a
/// no-op.
#[must_use]
pub fn delete_vertex(vertices: &[Point], index: usize, floor: usize) -> (Vec<Point>, bool) {
    if index >= vertices.len() || vertices.len() <= floor {
        return (vertices.to_vec(), false);
    }
    let mut out = vertices.to_vec();
    out.remove(index);
    (out, true)
}

/// Moves the vertex at `index` to `to` in a copy of `vertices`.
///
/// An out-of-range index leaves the ring unchanged. Used while dragging a vertex.
#[must_use]
pub fn move_vertex(vertices: &[Point], index: usize, to: Point) -> Vec<Point> {
    let mut out = vertices.to_vec();
    if let Some(v) = out.get_mut(index) {
        *v = to;
    }
    out
}

/// Projects `p` onto the segment `a..b`, returning the nearest point on the segment
/// (as integer DBU) and the squared distance from `p` to it.
///
/// The parameter is computed in `f64` and clamped to `[0, 1]` so the result never
/// leaves the segment; the returned point is rounded to the DBU grid. A degenerate
/// segment (`a == b`) projects to `a`.
fn project_point_on_segment(a: Point, b: Point, p: Point) -> (Point, i64) {
    let abx = f64::from(b.x) - f64::from(a.x);
    let aby = f64::from(b.y) - f64::from(a.y);
    let denom = abx * abx + aby * aby;
    let proj = if denom <= f64::EPSILON {
        a
    } else {
        let apx = f64::from(p.x) - f64::from(a.x);
        let apy = f64::from(p.y) - f64::from(a.y);
        let t = ((apx * abx + apy * aby) / denom).clamp(0.0, 1.0);
        Point::new(
            round_dbu(f64::from(a.x) + t * abx),
            round_dbu(f64::from(a.y) + t * aby),
        )
    };
    (proj, proj.distance_squared(p))
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

/// A live vertex-edit grab: which scene shape is being edited and which vertex is
/// held.
///
/// The app records this on drag-start over a selected shape's vertex and clears it on
/// release, replacing the shape through the undo history with the moved vertex.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VertexGrab {
    /// The scene shape index whose vertex is held.
    pub shape: usize,
    /// The vertex index within that shape's ring.
    pub vertex: usize,
}

/// The persistent drawing/vertex-edit state carried on the app.
///
/// It holds whichever builder the active tool is filling (only one is non-empty at a
/// time) plus the last path width and end cap the user chose, so those persist across
/// paths. The vertex-edit grab lives here too. Switching tools should
/// [`reset`](DrawState::reset) the in-progress builders so a half-drawn shape never
/// leaks into another tool.
#[derive(Clone, Debug, Default)]
pub struct DrawState {
    /// The polygon tool's in-progress ring.
    pub poly: PolyBuilder,
    /// The path tool's in-progress polyline, plus its width and end cap.
    pub path: PathBuilder,
    /// The vertex currently held by an [`EditVertex`](crate::tool::Tool::EditVertex)
    /// drag, if any.
    pub grab: Option<VertexGrab>,
}

impl DrawState {
    /// A fresh drawing state: empty builders, default path settings, no grab.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears any in-progress polygon/path and drops a live vertex grab.
    ///
    /// The path width and end-cap settings survive (they live on the [`PathBuilder`]
    /// and are only reset by an explicit `set`), so switching away and back keeps the
    /// user's chosen wire width.
    pub fn reset(&mut self) {
        self.poly.clear();
        self.path.clear();
        self.grab = None;
    }

    /// Whether a shape is mid-construction (any polygon or path vertices placed).
    #[must_use]
    pub fn in_progress(&self) -> bool {
        !self.poly.is_empty() || !self.path.is_empty()
    }
}

/// Extracts the editable vertex ring of a shape kind, if it has one.
///
/// A rectangle is returned as its four corners wound counter-clockwise (so it can be
/// vertex-edited into a polygon); a polygon and a path return their own vertices.
/// The boolean is `true` when the ring is closed (rectangle or polygon), which the
/// insertion hit-test uses to decide whether the closing edge participates.
#[must_use]
pub fn editable_vertices(kind: &reticle_model::ShapeKind) -> (Vec<Point>, bool) {
    use reticle_model::ShapeKind;
    match kind {
        ShapeKind::Rect(r) => (Polygon::from_rect(*r).vertices().to_vec(), true),
        ShapeKind::Polygon(p) => (p.vertices().to_vec(), true),
        ShapeKind::Path(p) => (p.points().to_vec(), false),
    }
}

/// Rebuilds a shape kind from an edited vertex ring, preserving the shape family.
///
/// A path keeps its width and end cap and takes the new points. A polygon (or a
/// rectangle, which promotes to a polygon once its corners are individually edited)
/// takes the new ring. Returns `None` if the ring is too small to be valid for the
/// family (fewer than three vertices for a polygon, two points for a path), so the
/// caller can decline the edit instead of writing a degenerate shape.
#[must_use]
pub fn rebuild_kind(
    original: &reticle_model::ShapeKind,
    vertices: Vec<Point>,
) -> Option<reticle_model::ShapeKind> {
    use reticle_model::ShapeKind;
    match original {
        ShapeKind::Path(p) => {
            if vertices.len() >= 2 {
                Some(ShapeKind::Path(Path::new(vertices, p.width(), p.endcap())))
            } else {
                None
            }
        }
        ShapeKind::Rect(_) | ShapeKind::Polygon(_) => {
            if vertices.len() >= 3 {
                Some(ShapeKind::Polygon(Polygon::new(vertices)))
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_model::ShapeKind;

    fn p(x: Dbu, y: Dbu) -> Point {
        Point::new(x, y)
    }

    // ---- rect_from_drag ----

    #[test]
    fn plain_drag_spans_the_two_points() {
        let r = rect_from_drag(p(10, 20), p(50, 5), RectMods::default());
        assert_eq!(r.min, p(10, 5));
        assert_eq!(r.max, p(50, 20));
    }

    #[test]
    fn square_grows_shorter_side_to_longer() {
        // dx = 40, dy = 10: the square side is 40, kept in the cursor's quadrant.
        let r = rect_from_drag(p(0, 0), p(40, 10), RectMods::new(true, false, false));
        assert_eq!(r.min, p(0, 0));
        assert_eq!(r.max, p(40, 40));
    }

    #[test]
    fn square_respects_negative_quadrant() {
        // Cursor up-left of the anchor: the square extends to negative x and y.
        let r = rect_from_drag(p(0, 0), p(-10, -40), RectMods::new(true, false, false));
        assert_eq!(r.min, p(-40, -40));
        assert_eq!(r.max, p(0, 0));
    }

    #[test]
    fn from_center_mirrors_the_offset() {
        // Anchor is the center; cursor at (30, 20) makes a 60x40 box centered there.
        let r = rect_from_drag(p(100, 100), p(130, 120), RectMods::new(false, true, false));
        assert_eq!(r.min, p(70, 80));
        assert_eq!(r.max, p(130, 120));
    }

    #[test]
    fn from_center_via_ctrl_matches_alt() {
        let a = rect_from_drag(p(0, 0), p(10, 25), RectMods::new(false, true, false));
        let b = rect_from_drag(p(0, 0), p(10, 25), RectMods::new(false, false, true));
        assert_eq!(a, b);
    }

    #[test]
    fn centered_square_uses_larger_half_extent() {
        // Half-extents 30 and 20; the square uses 30 on both, centered at anchor.
        let r = rect_from_drag(
            p(0, 0),
            p(30, 20),
            RectMods {
                square: true,
                from_center: true,
            },
        );
        assert_eq!(r.min, p(-30, -30));
        assert_eq!(r.max, p(30, 30));
    }

    #[test]
    fn degenerate_drag_is_empty() {
        let r = rect_from_drag(p(5, 5), p(5, 5), RectMods::default());
        assert!(r.is_empty());
    }

    // ---- PolyBuilder ----

    #[test]
    fn poly_dedups_immediate_repeat() {
        let mut b = PolyBuilder::new();
        b.push(p(0, 0));
        b.push(p(0, 0));
        b.push(p(10, 0));
        assert_eq!(b.len(), 2);
    }

    #[test]
    fn poly_needs_three_vertices_to_finish() {
        let mut b = PolyBuilder::new();
        b.push(p(0, 0));
        b.push(p(10, 0));
        assert!(!b.can_finish());
        assert!(b.clone().finish().is_none());
        b.push(p(10, 10));
        assert!(b.can_finish());
        let poly = b.finish().expect("three vertices form a polygon");
        assert_eq!(poly.len(), 3);
    }

    // ---- PathBuilder ----

    #[test]
    fn path_finishes_with_two_points_and_keeps_settings() {
        let mut b = PathBuilder::new();
        b.set_width(250);
        b.set_endcap(Endcap::Round);
        b.push(p(0, 0));
        assert!(!b.can_finish());
        b.push(p(100, 0));
        assert!(b.can_finish());
        let path = b.finish().expect("two points form a path");
        assert_eq!(path.points().len(), 2);
        assert_eq!(path.width(), 250);
        assert_eq!(path.endcap(), Endcap::Round);
    }

    #[test]
    fn path_width_is_clamped_to_at_least_one() {
        let mut b = PathBuilder::new();
        b.set_width(0);
        assert_eq!(b.width(), 1);
        b.set_width(-5);
        assert_eq!(b.width(), 1);
    }

    #[test]
    fn path_clear_keeps_width_and_cap() {
        let mut b = PathBuilder::new();
        b.set_width(400);
        b.set_endcap(Endcap::Square);
        b.push(p(0, 0));
        b.clear();
        assert!(b.is_empty());
        assert_eq!(b.width(), 400);
        assert_eq!(b.endcap(), Endcap::Square);
    }

    // ---- nearest_vertex ----

    #[test]
    fn nearest_vertex_finds_closest_within_radius() {
        let ring = [p(0, 0), p(100, 0), p(100, 100)];
        assert_eq!(nearest_vertex(&ring, p(103, 4), 10), Some(1));
    }

    #[test]
    fn nearest_vertex_rejects_beyond_radius() {
        let ring = [p(0, 0), p(100, 0)];
        assert_eq!(nearest_vertex(&ring, p(50, 50), 10), None);
    }

    #[test]
    fn nearest_vertex_ties_pick_lower_index() {
        // Equidistant from index 0 and index 1: the lower index wins.
        let ring = [p(0, 0), p(10, 0)];
        assert_eq!(nearest_vertex(&ring, p(5, 0), 100), Some(0));
    }

    // ---- nearest_segment_insertion ----

    #[test]
    fn insertion_hits_the_nearest_edge_and_projects() {
        // Square ring; a click just below the bottom edge midpoint inserts after v0.
        let ring = [p(0, 0), p(100, 0), p(100, 100), p(0, 100)];
        let ins = nearest_segment_insertion(&ring, p(50, 3), 10, true).expect("edge hit");
        assert_eq!(ins.index, 1);
        assert_eq!(ins.point, p(50, 0));
    }

    #[test]
    fn insertion_uses_closing_edge_only_when_closed() {
        // A point near the closing edge (v3 -> v0, the left side) of a square.
        let ring = [p(0, 0), p(100, 0), p(100, 100), p(0, 100)];
        let closed = nearest_segment_insertion(&ring, p(3, 50), 10, true).expect("closed hit");
        assert_eq!(closed.index, 4); // after the last vertex
        assert_eq!(closed.point, p(0, 50));
        // Open topology does not consider the closing edge, so the same click misses.
        assert!(nearest_segment_insertion(&ring, p(3, 50), 10, false).is_none());
    }

    #[test]
    fn insertion_rejects_far_clicks() {
        let ring = [p(0, 0), p(100, 0)];
        assert!(nearest_segment_insertion(&ring, p(50, 500), 10, false).is_none());
    }

    // ---- insert / delete / move ----

    #[test]
    fn insert_splices_at_index() {
        let ring = [p(0, 0), p(100, 0), p(100, 100)];
        let out = insert_vertex_on_segment(&ring, 1, p(50, 0));
        assert_eq!(out, vec![p(0, 0), p(50, 0), p(100, 0), p(100, 100)]);
    }

    #[test]
    fn insert_out_of_range_appends() {
        let ring = [p(0, 0)];
        let out = insert_vertex_on_segment(&ring, 99, p(5, 5));
        assert_eq!(out, vec![p(0, 0), p(5, 5)]);
    }

    #[test]
    fn delete_respects_the_floor() {
        let tri = [p(0, 0), p(10, 0), p(10, 10)];
        // A triangle is at the polygon floor of 3: deletion is refused.
        let (out, ok) = delete_vertex(&tri, 0, 3);
        assert!(!ok);
        assert_eq!(out.len(), 3);
        // A quad can lose a vertex.
        let quad = [p(0, 0), p(10, 0), p(10, 10), p(0, 10)];
        let (out, ok) = delete_vertex(&quad, 2, 3);
        assert!(ok);
        assert_eq!(out, vec![p(0, 0), p(10, 0), p(0, 10)]);
    }

    #[test]
    fn delete_out_of_range_is_noop() {
        let quad = [p(0, 0), p(10, 0), p(10, 10), p(0, 10)];
        let (out, ok) = delete_vertex(&quad, 9, 3);
        assert!(!ok);
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn move_relocates_the_vertex() {
        let ring = [p(0, 0), p(10, 0), p(10, 10)];
        let out = move_vertex(&ring, 1, p(20, 5));
        assert_eq!(out, vec![p(0, 0), p(20, 5), p(10, 10)]);
    }

    #[test]
    fn move_out_of_range_is_noop() {
        let ring = [p(0, 0)];
        assert_eq!(move_vertex(&ring, 5, p(9, 9)), vec![p(0, 0)]);
    }

    // ---- editable_vertices / rebuild_kind ----

    #[test]
    fn rect_exposes_four_corners_closed() {
        let kind = ShapeKind::Rect(Rect::new(p(0, 0), p(10, 20)));
        let (verts, closed) = editable_vertices(&kind);
        assert_eq!(verts.len(), 4);
        assert!(closed);
    }

    #[test]
    fn path_exposes_open_points() {
        let kind = ShapeKind::Path(Path::new(vec![p(0, 0), p(50, 0)], 100, Endcap::Flat));
        let (verts, closed) = editable_vertices(&kind);
        assert_eq!(verts, vec![p(0, 0), p(50, 0)]);
        assert!(!closed);
    }

    #[test]
    fn editing_a_rect_promotes_it_to_a_polygon() {
        let kind = ShapeKind::Rect(Rect::new(p(0, 0), p(10, 10)));
        let (mut verts, _) = editable_vertices(&kind);
        verts[2] = p(20, 20); // drag a corner out of axis-alignment
        let rebuilt = rebuild_kind(&kind, verts).expect("still a valid ring");
        assert!(matches!(rebuilt, ShapeKind::Polygon(_)));
    }

    #[test]
    fn rebuilding_a_path_preserves_width_and_cap() {
        let kind = ShapeKind::Path(Path::new(vec![p(0, 0), p(50, 0)], 300, Endcap::Round));
        let moved = move_vertex(&[p(0, 0), p(50, 0)], 1, p(60, 10));
        let rebuilt = rebuild_kind(&kind, moved).expect("two points remain");
        match rebuilt {
            ShapeKind::Path(path) => {
                assert_eq!(path.width(), 300);
                assert_eq!(path.endcap(), Endcap::Round);
                assert_eq!(path.points(), &[p(0, 0), p(60, 10)]);
            }
            other => panic!("expected a path, got {other:?}"),
        }
    }

    #[test]
    fn rebuild_declines_degenerate_rings() {
        let poly = ShapeKind::Polygon(Polygon::new(vec![p(0, 0), p(10, 0), p(10, 10)]));
        assert!(rebuild_kind(&poly, vec![p(0, 0), p(10, 0)]).is_none());
        let path = ShapeKind::Path(Path::new(vec![p(0, 0), p(5, 0)], 10, Endcap::Flat));
        assert!(rebuild_kind(&path, vec![p(0, 0)]).is_none());
    }

    // ---- DrawState ----

    #[test]
    fn reset_clears_builders_but_keeps_path_settings() {
        let mut s = DrawState::new();
        s.path.set_width(500);
        s.path.push(p(0, 0));
        s.poly.push(p(1, 1));
        s.grab = Some(VertexGrab {
            shape: 2,
            vertex: 1,
        });
        assert!(s.in_progress());
        s.reset();
        assert!(!s.in_progress());
        assert!(s.grab.is_none());
        assert_eq!(s.path.width(), 500);
    }
}
