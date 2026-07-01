//! Exact integer geometry helpers used by the rule checks.
//!
//! Every helper works on axis-aligned [`Rect`]s in database units and keeps
//! arithmetic in [`i64`] so DBU² products never overflow (see ADR 0002). The DRC
//! engine reduces each shape to its bounding box before calling these, so the
//! whole checker is a rectangle problem: conservative for polygons and paths, and
//! exact for rectangles (documented at each call site in [`crate`]).

use reticle_geometry::Rect;

/// The signed separation of two closed intervals `[a0, a1]` and `[b0, b1]`.
///
/// Returns the positive gap between them when they are disjoint, `0` when they
/// merely touch, and a negative value equal to minus their overlap when they
/// overlap. Widened to [`i64`] so the caller can square it without overflow.
fn interval_gap(a0: i64, a1: i64, b0: i64, b1: i64) -> i64 {
    if b0 > a1 {
        b0 - a1 // b is to the right of a
    } else if a0 > b1 {
        a0 - b1 // a is to the right of b
    } else {
        // The intervals overlap or touch; return minus the overlap length (<= 0).
        -(a1.min(b1) - a0.max(b0))
    }
}

/// Whether two rectangles overlap in a region of strictly positive area.
///
/// This is [`Rect::intersects`] restated in [`i64`] for local use; touching along
/// an edge or corner is *not* an overlap.
#[must_use]
pub fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.intersects(b)
}

/// The exact minimum edge-to-edge distance between two axis-aligned rectangles,
/// in DBU.
///
/// * Returns `0` when the rectangles overlap or touch (share any boundary point).
/// * Otherwise returns the true Euclidean distance between their nearest edges or
///   corners, rounded **down** to the nearest whole DBU.
///
/// The distance is computed from the per-axis gaps: when the projections overlap
/// on one axis the separation is purely along the other axis (an exact integer);
/// only the diagonal (corner-to-corner) case needs a square root, and rounding
/// down keeps the check conservative, a pair reported as `gap` is never closer
/// than `gap`.
#[must_use]
pub fn rect_gap(a: &Rect, b: &Rect) -> i64 {
    let gx = interval_gap(
        i64::from(a.min.x),
        i64::from(a.max.x),
        i64::from(b.min.x),
        i64::from(b.max.x),
    );
    let gy = interval_gap(
        i64::from(a.min.y),
        i64::from(a.max.y),
        i64::from(b.min.y),
        i64::from(b.max.y),
    );
    // A positive gap on an axis means the rectangles are separated along it.
    let dx = gx.max(0);
    let dy = gy.max(0);
    match (dx > 0, dy > 0) {
        (false, false) => 0,                      // overlapping or touching
        (true, false) => dx,                      // separated horizontally only
        (false, true) => dy,                      // separated vertically only
        (true, true) => isqrt(dx * dx + dy * dy), // diagonal corner gap
    }
}

/// Whether every point of `inner` lies inside the closed rectangle `outer`.
///
/// Uses the closed-box convention (shared edges count as contained) so a shape
/// flush against its enclosing boundary is treated as enclosed with zero margin.
#[must_use]
pub fn contains_rect(outer: &Rect, inner: &Rect) -> bool {
    outer.min.x <= inner.min.x
        && outer.min.y <= inner.min.y
        && inner.max.x <= outer.max.x
        && inner.max.y <= outer.max.y
}

/// The enclosure margin of `outer` around `inner`: the smallest of the four side
/// overhangs (left, right, bottom, top), in DBU.
///
/// A positive value is the guaranteed clearance on every side. The value is
/// negative when `inner` pokes outside `outer` on some side (equal to minus the
/// largest shortfall), and is only meaningful when `outer` actually contains
/// `inner`; callers gate on [`contains_rect`] first.
#[must_use]
pub fn enclosure_margin(outer: &Rect, inner: &Rect) -> i64 {
    let left = i64::from(inner.min.x) - i64::from(outer.min.x);
    let right = i64::from(outer.max.x) - i64::from(inner.max.x);
    let bottom = i64::from(inner.min.y) - i64::from(outer.min.y);
    let top = i64::from(outer.max.y) - i64::from(inner.max.y);
    left.min(right).min(bottom).min(top)
}

/// Integer square root (floor) of a non-negative [`i64`], by Newton's method.
///
/// Used for the diagonal corner distance in [`rect_gap`]. Rounding down is what
/// makes the spacing check conservative.
#[must_use]
fn isqrt(n: i64) -> i64 {
    if n < 2 {
        return n.max(0);
    }
    let mut x = n;
    let mut y = i64::midpoint(x, 1);
    while y < x {
        x = y;
        y = i64::midpoint(x, n / x);
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::Point;

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
        Rect::new(Point::new(x0, y0), Point::new(x1, y1))
    }

    #[test]
    fn isqrt_floor() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(2), 1);
        assert_eq!(isqrt(3), 1);
        assert_eq!(isqrt(4), 2);
        assert_eq!(isqrt(8), 2);
        assert_eq!(isqrt(9), 3);
        assert_eq!(isqrt(10_000), 100);
        assert_eq!(isqrt(10_001), 100);
    }

    #[test]
    fn gap_horizontal() {
        let a = rect(0, 0, 10, 10);
        let b = rect(15, 0, 25, 10); // 5 to the right
        assert_eq!(rect_gap(&a, &b), 5);
        assert_eq!(rect_gap(&b, &a), 5);
    }

    #[test]
    fn gap_vertical() {
        let a = rect(0, 0, 10, 10);
        let b = rect(0, 13, 10, 20); // 3 above
        assert_eq!(rect_gap(&a, &b), 3);
    }

    #[test]
    fn gap_touching_and_overlapping_is_zero() {
        let a = rect(0, 0, 10, 10);
        let touch = rect(10, 0, 20, 10); // shares an edge
        let overlap = rect(5, 5, 15, 15);
        assert_eq!(rect_gap(&a, &touch), 0);
        assert_eq!(rect_gap(&a, &overlap), 0);
        assert!(
            !overlaps(&a, &touch),
            "edge touch is not positive-area overlap"
        );
        assert!(overlaps(&a, &overlap));
    }

    #[test]
    fn gap_diagonal_is_corner_distance_floored() {
        let a = rect(0, 0, 10, 10);
        let b = rect(13, 14, 20, 20); // corner offset (3, 4) -> exactly 5
        assert_eq!(rect_gap(&a, &b), 5);
        let c = rect(11, 11, 20, 20); // corner offset (1, 1) -> sqrt(2) -> floor 1
        assert_eq!(rect_gap(&a, &c), 1);
    }

    #[test]
    fn enclosure_positive_and_negative() {
        let outer = rect(0, 0, 100, 100);
        let inner = rect(10, 20, 90, 70); // margins: 10,10,20,30 -> min 10
        assert!(contains_rect(&outer, &inner));
        assert_eq!(enclosure_margin(&outer, &inner), 10);

        let poking = rect(-5, 10, 50, 50); // sticks out on the left
        assert!(!contains_rect(&outer, &poking));
        assert_eq!(enclosure_margin(&outer, &poking), -5);
    }
}
