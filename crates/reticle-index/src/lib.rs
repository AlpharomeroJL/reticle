//! Spatial indices for Reticle.
//!
//! This crate provides the query structures that make browsing massive layouts
//! interactive: a bulk-loaded R-tree (`rstar`, Wave 1), a uniform grid, and a
//! tile/LOD pyramid for out-of-core streaming (`rkyv`, Wave 1).
//!
//! The Wave 0 contract is [`LinearIndex`], a correct but unoptimized
//! implementation of [`SpatialIndex`]. It is intentionally simple so it can serve
//! as the brute-force oracle the fast indices are property-tested against.
//!
//! # Indices
//!
//! - [`LinearIndex`], brute-force scan; the oracle.
//! - [`RTreeIndex`], bulk-loaded R-tree (`rstar`); the general-purpose index.
//! - [`UniformGrid`], bucket-per-cell grid; strong for evenly distributed shapes.
//! - [`LodPyramid`], tile/level-of-detail pyramid for out-of-core browsing.
//! - [`streaming`], `rkyv` zero-copy (de)serialization of an index payload for
//!   memory-mapped, out-of-core layouts.
//! - [`archive`], the `.rtla` streamed-archive format and the [`TileSource`]
//!   transport seam (Wave 2 contract, ADR 0062); built by [`archive_build`] and read
//!   through [`tile_source`] over mmap (native) or HTTP Range (wasm).

pub mod archive;
pub mod archive_build;
mod grid;
mod lod;
mod rtree;
pub mod streaming;
pub mod tile_source;

pub use archive::{
    LevelDims, RTLA_MAGIC, RTLA_VERSION, RtlaHeader, TileCoord, TileDirEntry, TilePayload,
    TileRecord, TileSource, TileSourceError,
};
pub use archive_build::{BuildError, build_rtla, build_rtla_to_vec};
pub use grid::UniformGrid;
pub use lod::{LodPyramid, TileId};
pub use rtree::{Entry, RTreeIndex};

use reticle_geometry::{Point, Rect, SpatialIndex};

/// Squared distance from a point to a rectangle (0 if the point is inside), in DBU².
#[must_use]
fn point_rect_distance_squared(p: Point, r: &Rect) -> i64 {
    let clamp = |v: i32, lo: i32, hi: i32| v.max(lo).min(hi.max(lo));
    // Nearest point on the rectangle to `p` (rectangle is `[min, max)`; treat the
    // closed box for distance purposes).
    let nx = clamp(p.x, r.min.x, r.max.x);
    let ny = clamp(p.y, r.min.y, r.max.y);
    let dx = i64::from(p.x) - i64::from(nx);
    let dy = i64::from(p.y) - i64::from(ny);
    dx * dx + dy * dy
}

/// A brute-force spatial index: stores `(bbox, item)` pairs and answers queries by
/// linear scan. Correct for any input; used as the oracle for the fast indices and
/// as a fallback where indexing overhead is not worthwhile.
#[derive(Clone, Debug)]
pub struct LinearIndex<T> {
    items: Vec<(Rect, T)>,
}

impl<T> Default for LinearIndex<T> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<T> LinearIndex<T> {
    /// Creates an empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds an index from `(bbox, item)` pairs.
    pub fn bulk_load(items: impl IntoIterator<Item = (Rect, T)>) -> Self {
        Self {
            items: items.into_iter().collect(),
        }
    }

    /// The `(bbox, item)` pairs, in insertion order.
    #[must_use]
    pub fn entries(&self) -> &[(Rect, T)] {
        &self.items
    }
}

impl<T> SpatialIndex for LinearIndex<T> {
    type Item = T;

    fn insert(&mut self, item: Self::Item, bbox: Rect) {
        self.items.push((bbox, item));
    }

    fn query_rect(&self, area: Rect) -> Vec<&Self::Item> {
        self.items
            .iter()
            .filter(|(bbox, _)| bbox.intersects(&area))
            .map(|(_, item)| item)
            .collect()
    }

    fn nearest(&self, p: Point) -> Option<&Self::Item> {
        self.items
            .iter()
            .min_by_key(|(bbox, _)| point_rect_distance_squared(p, bbox))
            .map(|(_, item)| item)
    }

    fn k_nearest(&self, p: Point, k: usize) -> Vec<&Self::Item> {
        let mut scored: Vec<(i64, &T)> = self
            .items
            .iter()
            .map(|(bbox, item)| (point_rect_distance_squared(p, bbox), item))
            .collect();
        scored.sort_by_key(|(d, _)| *d);
        scored.into_iter().take(k).map(|(_, item)| item).collect()
    }

    fn len(&self) -> usize {
        self.items.len()
    }
}
