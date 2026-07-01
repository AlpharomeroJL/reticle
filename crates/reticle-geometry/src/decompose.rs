//! Convex decomposition of simple polygons by ear clipping.
//!
//! A [triangulation](https://en.wikipedia.org/wiki/Polygon_triangulation) is itself
//! a convex decomposition: every triangle is convex, so cutting a simple polygon
//! into triangles yields convex pieces that exactly tile the input. Ear clipping is
//! used because it needs only integer cross-product predicates, making it exact and
//! robust on the DBU grid (no floating point, ADR 0002).

use crate::{Point, Polygon};

/// Twice the signed area of triangle `a, b, c`, exact in [`i64`].
///
/// This is the 2D cross product of `a->b` with `a->c`. It is positive when the
/// vertices are counter-clockwise, negative when clockwise, and zero when collinear.
fn cross(a: Point, b: Point, c: Point) -> i64 {
    let abx = i64::from(b.x) - i64::from(a.x);
    let aby = i64::from(b.y) - i64::from(a.y);
    let acx = i64::from(c.x) - i64::from(a.x);
    let acy = i64::from(c.y) - i64::from(a.y);
    abx * acy - aby * acx
}

/// Returns `true` if point `p` lies inside or on the boundary of triangle `a, b, c`,
/// which must be wound counter-clockwise (strictly positive area).
///
/// Uses three sign tests on integer cross products, so the result is exact.
fn point_in_triangle(p: Point, a: Point, b: Point, c: Point) -> bool {
    // For a CCW triangle, an interior/boundary point is left-of-or-on every edge.
    cross(a, b, p) >= 0 && cross(b, c, p) >= 0 && cross(c, a, p) >= 0
}

/// Decomposes a simple polygon (no holes, no self-intersections) into convex pieces
/// by ear clipping. Every returned piece is a triangle (hence convex) and the pieces
/// exactly tile the input: their areas sum to the polygon's area and they do not
/// overlap. Returns an empty vec for a degenerate (fewer than 3 vertices or
/// zero-area) polygon.
///
/// Input may be wound clockwise or counter-clockwise; the orientation is normalized
/// internally. Collinear vertices are handled gracefully (they simply never form an
/// ear with positive area).
///
/// All predicates use exact integer arithmetic, so the decomposition is robust on
/// the integer DBU grid.
///
/// # Examples
///
/// ```
/// use reticle_geometry::{convex_decompose, Point, Polygon, Rect};
///
/// let square = Polygon::from_rect(Rect::new(Point::new(0, 0), Point::new(4, 4)));
/// let pieces = convex_decompose(&square);
/// assert_eq!(pieces.len(), 2); // a square splits into two triangles
/// let total: i128 = pieces.iter().map(|p| p.signed_double_area().abs()).sum();
/// assert_eq!(total, square.signed_double_area().abs());
/// ```
#[must_use]
pub fn convex_decompose(polygon: &Polygon) -> Vec<Polygon> {
    // A polygon needs at least three vertices and non-zero area to have any interior.
    if polygon.len() < 3 || polygon.signed_double_area() == 0 {
        return Vec::new();
    }

    // Work on a counter-clockwise copy so the convex/reflex predicate has a fixed
    // sign convention regardless of the caller's winding.
    let mut verts: Vec<Point> = polygon.vertices().to_vec();
    if polygon.signed_double_area() < 0 {
        verts.reverse();
    }

    // `remaining` holds the indices into `verts` still in the working ring. Clipping
    // an ear removes its apex from this list.
    let mut remaining: Vec<usize> = (0..verts.len()).collect();
    let mut triangles: Vec<Polygon> = Vec::with_capacity(verts.len().saturating_sub(2));

    // Each successful clip removes one vertex, so `remaining` shrinks every time an
    // ear is found. `guard` bounds the total iterations to protect against a ring
    // that stops yielding ears (which should not happen for valid simple input, but
    // keeps the loop provably terminating).
    let mut guard = remaining.len() * remaining.len() + 1;

    while remaining.len() > 3 && guard > 0 {
        guard -= 1;
        let n = remaining.len();
        let mut clipped = false;

        for i in 0..n {
            let prev = remaining[(i + n - 1) % n];
            let curr = remaining[i];
            let next = remaining[(i + 1) % n];

            let a = verts[prev];
            let b = verts[curr];
            let c = verts[next];

            // A convex ear apex turns left (positive area). Collinear (== 0) or
            // reflex (< 0) apices are skipped; collinear ones carry zero area and
            // are pruned separately below so the ring keeps shrinking.
            let area2 = cross(a, b, c);
            if area2 <= 0 {
                continue;
            }

            // The candidate is a true ear only if no other remaining vertex lies
            // inside the triangle `a, b, c`.
            let mut is_ear = true;
            for &other in &remaining {
                if other == prev || other == curr || other == next {
                    continue;
                }
                if point_in_triangle(verts[other], a, b, c) {
                    is_ear = false;
                    break;
                }
            }

            if is_ear {
                triangles.push(Polygon::new(vec![a, b, c]));
                remaining.remove(i);
                clipped = true;
                break;
            }
        }

        // If a full pass found no convex ear, the ring's frontier is entirely
        // collinear or reflex spikes. Drop a collinear vertex (zero-area corner) to
        // make progress without emitting a degenerate triangle.
        if !clipped {
            let mut removed_collinear = false;
            for i in 0..n {
                let prev = remaining[(i + n - 1) % n];
                let curr = remaining[i];
                let next = remaining[(i + 1) % n];
                if cross(verts[prev], verts[curr], verts[next]) == 0 {
                    remaining.remove(i);
                    removed_collinear = true;
                    break;
                }
            }
            // No ear and no collinear vertex to prune: the input is not a valid
            // simple polygon. Stop rather than loop; partial output still tiles the
            // portion already clipped.
            if !removed_collinear {
                return triangles;
            }
        }
    }

    // The final three indices form the last triangle, provided they are not
    // collinear (a zero-area remainder contributes nothing to the tiling).
    if remaining.len() == 3 {
        let a = verts[remaining[0]];
        let b = verts[remaining[1]];
        let c = verts[remaining[2]];
        if cross(a, b, c) != 0 {
            triangles.push(Polygon::new(vec![a, b, c]));
        }
    }

    triangles
}
