//! Robust polygon boolean operations and offsetting.
//!
//! Booleans run on the exact `i_overlay` integer engine (ADR 0003) over our DBU
//! coordinates. Offsetting runs on `i_overlay`'s float outline engine and rounds
//! back to DBU. Both are wrapped behind the functions here so the dependency stays
//! swappable and is property-tested against a brute-force oracle.

use crate::{Dbu, Point, Polygon, Result};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay::Overlay;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::i_float::int::point::IntPoint;
use i_overlay::mesh::outline::offset::OutlineOffset;
use i_overlay::mesh::style::{LineJoin, OutlineStyle};

/// A polygon boolean operation.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BooleanOp {
    /// The union `A ∪ B`.
    Union,
    /// The intersection `A ∩ B`.
    Intersection,
    /// The difference `A \ B`.
    Difference,
    /// The symmetric difference `A △ B`.
    Xor,
}

impl BooleanOp {
    fn overlay_rule(self) -> OverlayRule {
        match self {
            Self::Union => OverlayRule::Union,
            Self::Intersection => OverlayRule::Intersect,
            Self::Difference => OverlayRule::Difference,
            Self::Xor => OverlayRule::Xor,
        }
    }
}

/// Converts a set of our polygons into `i_overlay` integer contours.
fn to_int_contours(polygons: &[Polygon]) -> Vec<Vec<IntPoint>> {
    polygons
        .iter()
        .filter(|p| p.len() >= 3)
        .map(|p| {
            p.vertices()
                .iter()
                .map(|v| IntPoint::new(v.x, v.y))
                .collect()
        })
        .collect()
}

/// Converts `i_overlay` integer shapes back into flat polygons.
///
/// Each output contour becomes one [`Polygon`]; outer boundaries wind
/// counter-clockwise and holes wind clockwise, so callers can distinguish them by
/// [`Polygon::winding`](crate::Polygon::winding).
fn from_int_shapes(shapes: Vec<Vec<Vec<IntPoint>>>) -> Vec<Polygon> {
    let mut out = Vec::new();
    for shape in shapes {
        for contour in shape {
            let verts = contour.iter().map(|p| Point::new(p.x, p.y)).collect();
            out.push(Polygon::new(verts));
        }
    }
    out
}

/// Computes a boolean operation between two polygon sets, returning the resulting
/// simple polygons. Input contours are interpreted by their winding under the
/// non-zero fill rule, so a clockwise ring is treated as a hole.
///
/// # Errors
///
/// Currently infallible for in-range coordinates; returns
/// [`GeometryError`](crate::GeometryError) reserved for future overflow detection.
pub fn polygon_boolean(op: BooleanOp, a: &[Polygon], b: &[Polygon]) -> Result<Vec<Polygon>> {
    let subj = to_int_contours(a);
    let clip = to_int_contours(b);
    if subj.is_empty() && clip.is_empty() {
        return Ok(Vec::new());
    }
    let mut overlay = Overlay::with_contours(&subj, &clip);
    let shapes = overlay.overlay(op.overlay_rule(), FillRule::NonZero);
    Ok(from_int_shapes(shapes))
}

/// Offsets (grows for positive `delta`, shrinks for negative) a polygon set by
/// `delta` DBU, merging overlapping results. Corners are mitered to keep
/// rectilinear layout sharp.
///
/// The offset is computed on `i_overlay`'s float engine and rounded to the nearest
/// DBU, so results are on-grid but not guaranteed bit-exact for pathological input.
///
/// # Errors
///
/// Currently infallible for in-range coordinates; returns
/// [`GeometryError`](crate::GeometryError) reserved for future overflow detection.
pub fn offset(polygons: &[Polygon], delta: Dbu) -> Result<Vec<Polygon>> {
    let shapes: Vec<Vec<Vec<[f64; 2]>>> = polygons
        .iter()
        .filter(|p| p.len() >= 3)
        .map(|p| {
            vec![
                p.vertices()
                    .iter()
                    .map(|v| [f64::from(v.x), f64::from(v.y)])
                    .collect::<Vec<_>>(),
            ]
        })
        .collect();
    if shapes.is_empty() || delta == 0 {
        return Ok(polygons.to_vec());
    }
    // Miter joins keep 90-degree layout corners sharp.
    let style = OutlineStyle::new(f64::from(delta)).line_join(LineJoin::Miter(0.5));
    let result: Vec<Vec<Vec<[f64; 2]>>> = shapes.outline(&style);
    let mut out = Vec::new();
    for shape in result {
        for contour in shape {
            let verts = contour
                .iter()
                .map(|pt| Point::new(round_dbu(pt[0]), round_dbu(pt[1])))
                .collect();
            out.push(Polygon::new(verts));
        }
    }
    Ok(out)
}

/// Rounds a float coordinate to the nearest DBU, clamped to the coordinate range.
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
