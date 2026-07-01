//! A uniform-grid spatial index.
//!
//! [`UniformGrid`] partitions the plane into square cells of a fixed side length
//! and buckets each shape into every cell its bounding box overlaps. It excels
//! when shapes are roughly uniform in size and evenly distributed, where a
//! rectangle query touches only a handful of cells: [`UniformGrid::query_rect`]
//! visits the cells overlapping the query and post-filters the candidates, so it
//! avoids the whole-scene scan a [`LinearIndex`](crate::LinearIndex) performs.
//!
//! Nearest-neighbour queries fall back to an exact scan so their answers match
//! the brute-force oracle bit-for-bit, including tie-breaking by insertion order.

use std::collections::HashMap;

use reticle_geometry::{Point, Rect, SpatialIndex};

/// Integer cell coordinate in the grid (a cell index, not a DBU coordinate).
type Cell = (i32, i32);

/// Squared distance from a point to the closed rectangle `[min, max]`, in DBU²,
/// computed in `i64` (identical to the oracle's metric).
#[must_use]
fn point_rect_distance_squared(p: Point, r: &Rect) -> i64 {
    let clamp = |v: i32, lo: i32, hi: i32| v.max(lo).min(hi.max(lo));
    let nx = clamp(p.x, r.min.x, r.max.x);
    let ny = clamp(p.y, r.min.y, r.max.y);
    let dx = i64::from(p.x) - i64::from(nx);
    let dy = i64::from(p.y) - i64::from(ny);
    dx * dx + dy * dy
}

/// A uniform-grid spatial index implementing [`SpatialIndex`].
///
/// Construct with [`UniformGrid::new`] (choosing a cell size) or
/// [`UniformGrid::bulk_load`]. The cell size should be on the order of a typical
/// shape's extent: too small wastes memory bucketing each shape into many cells,
/// too large degrades queries toward a linear scan.
#[derive(Debug)]
pub struct UniformGrid<T> {
    entries: Vec<(Rect, T)>,
    cells: HashMap<Cell, Vec<usize>>,
    cell_size: i32,
}

impl<T> UniformGrid<T> {
    /// Creates an empty grid with the given `cell_size` (side length in DBU).
    ///
    /// # Panics
    ///
    /// Panics if `cell_size` is not strictly positive.
    #[must_use]
    pub fn new(cell_size: i32) -> Self {
        assert!(cell_size > 0, "grid cell size must be positive");
        Self {
            entries: Vec::new(),
            cells: HashMap::new(),
            cell_size,
        }
    }

    /// Builds a grid over `items` with the given `cell_size`.
    ///
    /// # Panics
    ///
    /// Panics if `cell_size` is not strictly positive.
    pub fn bulk_load(cell_size: i32, items: impl IntoIterator<Item = (Rect, T)>) -> Self {
        let mut grid = Self::new(cell_size);
        for (bbox, item) in items {
            grid.insert(item, bbox);
        }
        grid
    }

    /// The cell side length in DBU.
    #[must_use]
    pub fn cell_size(&self) -> i32 {
        self.cell_size
    }

    /// The `(bbox, item)` pairs, in insertion order.
    #[must_use]
    pub fn entries(&self) -> &[(Rect, T)] {
        &self.entries
    }

    /// The cell index containing DBU coordinate `v` (floor division).
    fn cell_of(&self, v: i32) -> i32 {
        v.div_euclid(self.cell_size)
    }

    /// The inclusive range of cell indices a `[lo, hi]` DBU span touches.
    fn cell_range(&self, lo: i32, hi: i32) -> (i32, i32) {
        (self.cell_of(lo), self.cell_of(hi))
    }

    /// Visits every cell index a rectangle overlaps.
    fn cells_for(&self, bbox: &Rect, mut f: impl FnMut(Cell)) {
        let (cx0, cx1) = self.cell_range(bbox.min.x, bbox.max.x);
        let (cy0, cy1) = self.cell_range(bbox.min.y, bbox.max.y);
        for cx in cx0..=cx1 {
            for cy in cy0..=cy1 {
                f((cx, cy));
            }
        }
    }
}

impl<T> SpatialIndex for UniformGrid<T> {
    type Item = T;

    fn insert(&mut self, item: Self::Item, bbox: Rect) {
        let index = self.entries.len();
        let cell_size = self.cell_size;
        let cells = &mut self.cells;
        // Inlined `cells_for` to satisfy the borrow checker (we mutate `cells`).
        let cell_of = |v: i32| v.div_euclid(cell_size);
        let (cx0, cx1) = (cell_of(bbox.min.x), cell_of(bbox.max.x));
        let (cy0, cy1) = (cell_of(bbox.min.y), cell_of(bbox.max.y));
        for cx in cx0..=cx1 {
            for cy in cy0..=cy1 {
                cells.entry((cx, cy)).or_default().push(index);
            }
        }
        self.entries.push((bbox, item));
    }

    fn query_rect(&self, area: Rect) -> Vec<&Self::Item> {
        // Gather candidate entry indices from every overlapped cell. A shape can
        // appear in several cells, so track which we have already emitted.
        let mut seen: Vec<bool> = vec![false; self.entries.len()];
        let mut out = Vec::new();
        self.cells_for(&area, |cell| {
            if let Some(indices) = self.cells.get(&cell) {
                for &i in indices {
                    if !seen[i] {
                        seen[i] = true;
                        let (bbox, item) = &self.entries[i];
                        if bbox.intersects(&area) {
                            out.push(item);
                        }
                    }
                }
            }
        });
        out
    }

    fn nearest(&self, p: Point) -> Option<&Self::Item> {
        self.entries
            .iter()
            .min_by_key(|(bbox, _)| point_rect_distance_squared(p, bbox))
            .map(|(_, item)| item)
    }

    fn k_nearest(&self, p: Point, k: usize) -> Vec<&Self::Item> {
        let mut scored: Vec<(i64, &T)> = self
            .entries
            .iter()
            .map(|(bbox, item)| (point_rect_distance_squared(p, bbox), item))
            .collect();
        scored.sort_by_key(|(d, _)| *d);
        scored.into_iter().take(k).map(|(_, item)| item).collect()
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}
