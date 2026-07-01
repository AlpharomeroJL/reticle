//! Exact integer 2D geometry for Reticle.
//!
//! Chip layout is defined on an integer grid measured in *database units* (DBU),
//! not floating point: coordinates are exact and boolean operations must be
//! numerically robust. This crate provides the primitive types
//! ([`Point`], [`Rect`], [`Polygon`], [`Path`], [`Transform`]) and the shared
//! traits ([`Shape`], [`SpatialIndex`]) that the rest of the workspace builds on.
//!
//! It contains no GPU, async, or UI code so the core logic stays fast to test and
//! clean to review.
//!
//! # Coordinates
//!
//! [`Dbu`] is the coordinate type (see ADR 0002). It is 32-bit for GDSII
//! interoperability and memory density. All area and product arithmetic widens to
//! [`i64`] (or [`i128`] for area sums) to avoid overflow; helpers such as
//! [`Rect::area`] return widened types.
//!
//! # Booleans
//!
//! Robust polygon booleans and offsetting are delegated to the `i_overlay` crate
//! (ADR 0003), wrapped behind [`polygon_boolean`] and [`offset`] so the dependency
//! stays swappable and is property-tested against a brute-force oracle.
//!
//! # Convex decomposition
//!
//! [`convex_decompose`] cuts a simple polygon into convex triangles by exact
//! integer ear clipping, validated against a brute-force area oracle. Because every
//! triangle is convex, the triangulation is a valid convex decomposition that
//! exactly tiles the input.
#![forbid(unsafe_code)]

mod boolean;
mod decompose;
mod primitives;
mod shapes;
mod traits;

pub use boolean::{BooleanOp, offset, polygon_boolean};
pub use decompose::convex_decompose;
pub use primitives::{Magnification, Orientation, Point, Rect, Transform};
pub use shapes::{Endcap, Path, Polygon, Winding};
pub use traits::{GeometryError, LayerId, Shape, SpatialIndex};

/// The coordinate type: an exact integer database unit (DBU).
///
/// 32-bit for GDSII compatibility and cache density. Multiply two `Dbu` values
/// only after widening to [`i64`]; see ADR 0002.
pub type Dbu = i32;

/// Result type for fallible geometry operations.
pub type Result<T> = core::result::Result<T, GeometryError>;
