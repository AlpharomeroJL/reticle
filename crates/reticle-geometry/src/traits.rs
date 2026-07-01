//! Shared geometry traits and the layer identifier, implemented across the
//! workspace (for example [`SpatialIndex`] by `reticle-index`).

use crate::{Point, Rect};
use core::fmt;

/// A layer/datatype pair, the GDSII addressing scheme for geometry. Richer layer
/// metadata (name, color, style) lives in `reticle-model`; this is the stable
/// identifier every shape carries.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, PartialOrd, Ord)]
pub struct LayerId {
    /// GDSII layer number.
    pub layer: u16,
    /// GDSII datatype number.
    pub datatype: u16,
}

impl LayerId {
    /// Creates a layer identifier from a layer number and datatype.
    #[must_use]
    pub const fn new(layer: u16, datatype: u16) -> Self {
        Self { layer, datatype }
    }
}

/// A geometric shape that occupies a bounded region on a single layer.
///
/// This is the minimal surface the spatial index, renderer, DRC, and extraction
/// engines rely on. Concrete shapes live in `reticle-model` and wrap the
/// primitives in this crate.
pub trait Shape {
    /// The shape's axis-aligned bounding box, in DBU.
    fn bounding_box(&self) -> Rect;

    /// The layer the shape is drawn on.
    fn layer(&self) -> LayerId;
}

/// A 2D spatial index over items that each have an axis-aligned bounding box.
///
/// Implemented by `reticle-index` (a bulk-loaded R-tree, a uniform grid, and a
/// tile/LOD pyramid). The generic `Item` is typically a lightweight shape handle.
pub trait SpatialIndex {
    /// The indexed item type.
    type Item;

    /// Inserts `item` with the given bounding box.
    fn insert(&mut self, item: Self::Item, bbox: Rect);

    /// Returns references to all items whose bounding box intersects `area`.
    fn query_rect(&self, area: Rect) -> Vec<&Self::Item>;

    /// Returns the item nearest to `p` (by bounding-box distance), if any.
    fn nearest(&self, p: Point) -> Option<&Self::Item>;

    /// Returns up to `k` items nearest to `p`, in non-decreasing distance order.
    fn k_nearest(&self, p: Point, k: usize) -> Vec<&Self::Item>;

    /// The number of indexed items.
    fn len(&self) -> usize;

    /// Returns `true` if the index holds no items.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Errors produced by geometry operations.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GeometryError {
    /// The input was degenerate (for example a polygon with fewer than three
    /// vertices, or zero-area geometry where positive area was required).
    Degenerate,
    /// A computation overflowed the coordinate range (see ADR 0002).
    Overflow,
    /// The operation is not supported for the given input, with a reason.
    Unsupported(&'static str),
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Degenerate => write!(f, "degenerate geometry"),
            Self::Overflow => write!(f, "coordinate arithmetic overflow"),
            Self::Unsupported(why) => write!(f, "unsupported geometry operation: {why}"),
        }
    }
}

impl core::error::Error for GeometryError {}
