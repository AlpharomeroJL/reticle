//! Polygon and path shapes.

use crate::{Dbu, Point, Rect};

/// A simple polygon: an ordered ring of vertices, implicitly closed (the last
/// vertex connects back to the first). May be convex or concave but is expected to
/// be non-self-intersecting; boolean operations are the way to combine polygons.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct Polygon {
    vertices: Vec<Point>,
}

impl Polygon {
    /// Creates a polygon from its vertices (implicitly closed).
    #[must_use]
    pub fn new(vertices: Vec<Point>) -> Self {
        Self { vertices }
    }

    /// Creates a rectangular polygon from a [`Rect`], wound counter-clockwise.
    #[must_use]
    pub fn from_rect(r: Rect) -> Self {
        Self::new(vec![
            r.min,
            Point::new(r.max.x, r.min.y),
            r.max,
            Point::new(r.min.x, r.max.y),
        ])
    }

    /// The polygon's vertices.
    #[must_use]
    pub fn vertices(&self) -> &[Point] {
        &self.vertices
    }

    /// Number of vertices.
    #[must_use]
    pub fn len(&self) -> usize {
        self.vertices.len()
    }

    /// Returns `true` if the polygon has no vertices.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    /// Twice the signed area via the shoelace formula, exact in [`i128`].
    ///
    /// Positive for counter-clockwise winding, negative for clockwise. Twice the
    /// area is returned so the value stays an exact integer.
    #[must_use]
    pub fn signed_double_area(&self) -> i128 {
        let n = self.vertices.len();
        if n < 3 {
            return 0;
        }
        let mut acc: i128 = 0;
        for i in 0..n {
            let a = self.vertices[i];
            let b = self.vertices[(i + 1) % n];
            acc += i128::from(a.x) * i128::from(b.y) - i128::from(b.x) * i128::from(a.y);
        }
        acc
    }

    /// The polygon's area in DBU² (always non-negative).
    #[must_use]
    pub fn area(&self) -> f64 {
        self.signed_double_area().unsigned_abs() as f64 / 2.0
    }

    /// Returns the winding of the vertex ring.
    #[must_use]
    pub fn winding(&self) -> Winding {
        match self.signed_double_area().signum() {
            1 => Winding::CounterClockwise,
            -1 => Winding::Clockwise,
            _ => Winding::Degenerate,
        }
    }

    /// Returns a copy with the vertex order reversed (flips winding).
    #[must_use]
    pub fn reversed(&self) -> Self {
        let mut v = self.vertices.clone();
        v.reverse();
        Self::new(v)
    }

    /// The axis-aligned bounding box, or a degenerate box at the origin if empty.
    #[must_use]
    pub fn bounding_box(&self) -> Rect {
        Rect::from_points(self.vertices.iter().copied()).unwrap_or_default()
    }
}

/// The winding direction of a polygon ring.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Winding {
    /// Counter-clockwise (positive signed area).
    CounterClockwise,
    /// Clockwise (negative signed area).
    Clockwise,
    /// Zero area (fewer than three distinct vertices, or collinear).
    Degenerate,
}

/// How the ends of a [`Path`] are capped.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Endcap {
    /// The path ends exactly at its endpoints (no extension).
    #[default]
    Flat,
    /// The path is extended by half its width at each end (square cap).
    Square,
    /// The ends are rounded by a half-width radius.
    Round,
    /// The path is extended by a custom amount (in DBU) at each end.
    Custom(Dbu),
}

/// A path (wire): a polyline of a given width with configurable end caps. Used for
/// routing wires and drawn geometry that is naturally a stroke rather than a fill.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct Path {
    points: Vec<Point>,
    width: Dbu,
    endcap: Endcap,
}

impl Path {
    /// Creates a path from a polyline, a width in DBU, and an end-cap style.
    #[must_use]
    pub fn new(points: Vec<Point>, width: Dbu, endcap: Endcap) -> Self {
        Self {
            points,
            width,
            endcap,
        }
    }

    /// The path's center-line points.
    #[must_use]
    pub fn points(&self) -> &[Point] {
        &self.points
    }

    /// The path width in DBU.
    #[must_use]
    pub fn width(&self) -> Dbu {
        self.width
    }

    /// The end-cap style.
    #[must_use]
    pub fn endcap(&self) -> Endcap {
        self.endcap
    }

    /// A conservative axis-aligned bounding box: the center-line box expanded by
    /// half the width (plus any end-cap extension).
    #[must_use]
    pub fn bounding_box(&self) -> Rect {
        let Some(base) = Rect::from_points(self.points.iter().copied()) else {
            return Rect::default();
        };
        let half = self.width / 2;
        // Perpendicular to the path, the box always grows by the half-width; along
        // the path, only a custom cap can exceed that. A uniform expansion by the
        // larger of the two is a correct conservative AABB for every cap style.
        let ext = match self.endcap {
            Endcap::Flat | Endcap::Square | Endcap::Round => half,
            Endcap::Custom(e) => half.max(e),
        };
        base.expanded(ext)
    }
}
