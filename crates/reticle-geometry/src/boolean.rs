//! Robust polygon boolean operations and offsetting.
//!
//! These are delegated to the `i_overlay` crate (ADR 0003) and wrapped behind the
//! functions here so the dependency stays swappable and is property-tested against
//! a brute-force oracle. The Wave 1 `reticle-geometry` lane implements the bodies;
//! the signatures are the frozen contract.

use crate::{Dbu, Polygon, Result};

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

/// Computes a boolean operation between two polygon sets, returning the resulting
/// simple polygons (holes represented by winding).
///
/// # Errors
///
/// Returns [`GeometryError`](crate::GeometryError) if the input is degenerate or a
/// coordinate overflows.
pub fn polygon_boolean(op: BooleanOp, a: &[Polygon], b: &[Polygon]) -> Result<Vec<Polygon>> {
    let _ = (op, a, b);
    todo!("Wave 1: implement polygon booleans via i_overlay (ADR 0003)")
}

/// Offsets (grows for positive `delta`, shrinks for negative) a polygon set by
/// `delta` DBU.
///
/// # Errors
///
/// Returns [`GeometryError`](crate::GeometryError) if the input is degenerate or a
/// coordinate overflows.
pub fn offset(polygons: &[Polygon], delta: Dbu) -> Result<Vec<Polygon>> {
    let _ = (polygons, delta);
    todo!("Wave 1: implement offsetting via i_overlay (ADR 0003)")
}
