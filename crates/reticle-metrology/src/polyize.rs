//! Converts document shapes into filled polygons for exact boolean metrology.
//!
//! Every metrology measurement of area and perimeter runs on polygons, so each
//! [`DrawShape`] is first reduced to the polygons that describe the region it
//! fills. Rectangles and polygons convert exactly; paths are stroked (see
//! [`shape_polygons`]).

use reticle_geometry::{Point, Polygon, Rect, Winding};
use reticle_model::{DrawShape, ShapeKind};

/// Converts one shape into the counter-clockwise polygons that fill its region.
///
/// - A rectangle becomes one polygon.
/// - A polygon is normalized to counter-clockwise winding so a boolean union
///   treats it as solid regardless of how it was authored.
/// - A path is stroked into one axis-aligned rectangle per segment, each grown
///   by half the path width. Axis-aligned (manhattan) segments are exact; a
///   diagonal segment falls back to its bounding box grown by half width, which
///   overstates its area slightly. Endcaps are treated as flat (no extension).
///   Paths are uncommon in the layouts this crate measures, so this keeps the
///   common manhattan case exact while never panicking on the rest.
///
/// Degenerate shapes (an empty rectangle, a fewer-than-three-vertex polygon, a
/// path with fewer than two points or a non-positive width) contribute no
/// polygons.
pub(crate) fn shape_polygons(shape: &DrawShape) -> Vec<Polygon> {
    match &shape.kind {
        ShapeKind::Rect(r) if !r.is_empty() => vec![Polygon::from_rect(*r)],
        ShapeKind::Rect(_) => Vec::new(),
        ShapeKind::Polygon(p) => vec![as_ccw(p.clone())],
        ShapeKind::Path(path) => stroke_path(path.points(), path.width()),
    }
}

/// Returns the polygon wound counter-clockwise, reversing a clockwise ring so a
/// non-zero-fill union reads it as a solid rather than a hole.
fn as_ccw(p: Polygon) -> Polygon {
    if p.winding() == Winding::Clockwise {
        p.reversed()
    } else {
        p
    }
}

/// Strokes a polyline of `width` into one rectangle polygon per segment.
fn stroke_path(points: &[Point], width: reticle_geometry::Dbu) -> Vec<Polygon> {
    let half = width / 2;
    if points.len() < 2 || half <= 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for seg in points.windows(2) {
        let (a, b) = (seg[0], seg[1]);
        let rect = if a.y == b.y {
            Rect::new(
                Point::new(a.x.min(b.x), a.y - half),
                Point::new(a.x.max(b.x), a.y + half),
            )
        } else if a.x == b.x {
            Rect::new(
                Point::new(a.x - half, a.y.min(b.y)),
                Point::new(a.x + half, a.y.max(b.y)),
            )
        } else {
            // Non-manhattan segment: conservative bounding box grown by half width.
            Rect::new(
                Point::new(a.x.min(b.x) - half, a.y.min(b.y) - half),
                Point::new(a.x.max(b.x) + half, a.y.max(b.y) + half),
            )
        };
        if !rect.is_empty() {
            out.push(Polygon::from_rect(rect));
        }
    }
    out
}
