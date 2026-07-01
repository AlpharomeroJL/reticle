//! A bulk-loaded R-tree spatial index built on the [`rstar`] crate.
//!
//! [`RTreeIndex`] wraps each indexed `(bbox, item)` pair in an [`Entry`], which
//! teaches `rstar` how to compute the item's bounding envelope and its distance
//! to a query point. Bulk loading (via `rstar`'s Sort-Tile-Recursive packing)
//! produces a well-balanced tree in one pass, which is the intended way to build
//! an index over a large, mostly-static layout.
//!
//! # Distance and intersection semantics
//!
//! Coordinates are database units ([`i32`](reticle_geometry::Dbu)), but the tree
//! is built over `[i64; 2]` points so that `rstar`'s internal squared-distance
//! arithmetic is performed in `i64`. An `i32`-scalar tree would overflow when
//! measuring distances across a large layout (a squared difference of two full
//! `i32`-range coordinates far exceeds `i32::MAX`); widening to `i64` matches the
//! metric [`reticle_geometry`] uses and keeps distances exact for the entire
//! practical coordinate range.
//!
//! `rstar`'s envelope intersection is inclusive (shapes that merely touch along
//! an edge count as intersecting), whereas [`Rect::intersects`] requires overlap
//! of positive area. [`RTreeIndex::query_rect`] therefore post-filters the
//! candidates `rstar` returns so its results are identical to the brute-force
//! [`LinearIndex`](crate::LinearIndex) oracle.

use reticle_geometry::{Point, Rect, SpatialIndex};
use rstar::{AABB, PointDistance, RTree, RTreeObject};

/// The point type the R-tree is built over: DBU coordinates widened to `i64` so
/// `rstar`'s squared-distance math cannot overflow (see the module docs).
type TreePoint = [i64; 2];

/// Widens a [`Point`] to the tree's `i64` coordinate space.
fn to_tree_point(p: Point) -> TreePoint {
    [i64::from(p.x), i64::from(p.y)]
}

/// True squared distance from an `i64` query point to the closed rectangle
/// `[min, max]`, in DBU², computed entirely in `i64` (identical to the oracle's
/// metric, with no narrowing of the point).
#[must_use]
fn point_rect_distance_squared(px: i64, py: i64, r: &Rect) -> i64 {
    let clamp = |v: i64, lo: i64, hi: i64| v.max(lo).min(hi.max(lo));
    let nx = clamp(px, i64::from(r.min.x), i64::from(r.max.x));
    let ny = clamp(py, i64::from(r.min.y), i64::from(r.max.y));
    let dx = px - nx;
    let dy = py - ny;
    dx * dx + dy * dy
}

/// A single indexed shape: its bounding box plus the caller's item.
///
/// This is the leaf payload stored in the R-tree. It implements the `rstar`
/// traits [`RTreeObject`] and [`PointDistance`] so the tree can index and query
/// it, while keeping the original [`Rect`] around for exact post-filtering.
#[derive(Clone, Debug)]
pub struct Entry<T> {
    bbox: Rect,
    item: T,
}

impl<T> Entry<T> {
    /// Creates an entry pairing `bbox` with `item`.
    #[must_use]
    pub fn new(bbox: Rect, item: T) -> Self {
        Self { bbox, item }
    }

    /// The entry's bounding box.
    #[must_use]
    pub fn bbox(&self) -> Rect {
        self.bbox
    }

    /// A shared reference to the stored item.
    #[must_use]
    pub fn item(&self) -> &T {
        &self.item
    }
}

impl<T> RTreeObject for Entry<T> {
    type Envelope = AABB<TreePoint>;

    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners(to_tree_point(self.bbox.min), to_tree_point(self.bbox.max))
    }
}

impl<T> PointDistance for Entry<T> {
    fn distance_2(&self, point: &TreePoint) -> i64 {
        // The point arrives in `i64` tree space; the DBU bbox is exact within
        // `i32`, so this reproduces the oracle's metric with no loss.
        point_rect_distance_squared(point[0], point[1], &self.bbox)
    }
}

/// A bulk-loaded R-tree implementing [`SpatialIndex`].
///
/// Build it with [`RTreeIndex::bulk_load`] for a balanced tree over a known set
/// of shapes, or start empty with [`RTreeIndex::new`] and [`SpatialIndex::insert`]
/// incrementally. Bulk loading is preferred for large static layouts.
#[derive(Debug)]
pub struct RTreeIndex<T> {
    tree: RTree<Entry<T>>,
}

impl<T> Default for RTreeIndex<T> {
    fn default() -> Self {
        Self { tree: RTree::new() }
    }
}

impl<T> RTreeIndex<T> {
    /// Creates an empty R-tree index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Bulk-loads an R-tree from `(bbox, item)` pairs using Sort-Tile-Recursive
    /// packing, producing a balanced tree in a single pass.
    pub fn bulk_load(items: impl IntoIterator<Item = (Rect, T)>) -> Self {
        let entries: Vec<Entry<T>> = items
            .into_iter()
            .map(|(bbox, item)| Entry::new(bbox, item))
            .collect();
        Self {
            tree: RTree::bulk_load(entries),
        }
    }

    /// An iterator over the indexed entries, in no particular order.
    pub fn entries(&self) -> impl Iterator<Item = &Entry<T>> {
        self.tree.iter()
    }
}

impl<T> SpatialIndex for RTreeIndex<T> {
    type Item = T;

    fn insert(&mut self, item: Self::Item, bbox: Rect) {
        self.tree.insert(Entry::new(bbox, item));
    }

    fn query_rect(&self, area: Rect) -> Vec<&Self::Item> {
        let envelope = AABB::from_corners(to_tree_point(area.min), to_tree_point(area.max));
        self.tree
            .locate_in_envelope_intersecting(envelope)
            // Post-filter: `rstar` intersection is inclusive; the contract wants
            // positive-area overlap, matching `LinearIndex`.
            .filter(|entry| entry.bbox.intersects(&area))
            .map(Entry::item)
            .collect()
    }

    fn nearest(&self, p: Point) -> Option<&Self::Item> {
        self.tree
            .nearest_neighbor(to_tree_point(p))
            .map(Entry::item)
    }

    fn k_nearest(&self, p: Point, k: usize) -> Vec<&Self::Item> {
        self.tree
            .nearest_neighbor_iter(to_tree_point(p))
            .take(k)
            .map(Entry::item)
            .collect()
    }

    fn len(&self) -> usize {
        self.tree.size()
    }
}
