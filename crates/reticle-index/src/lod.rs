//! A tile / level-of-detail (LOD) pyramid for out-of-core browsing.
//!
//! A large layout does not fit in memory or on screen at once. The renderer
//! browses it by requesting only the tiles overlapping the current viewport, at a
//! level of detail matched to the current zoom: zoomed out, it draws a few coarse
//! tiles; zoomed in, many fine ones. [`LodPyramid`] precomputes, for a fixed set
//! of world bounds and shape bounding boxes, which shapes fall into which tile at
//! every level, so a viewport query is a cheap lookup rather than a scene scan.
//!
//! # Level convention
//!
//! Level `0` is the **coarsest**: a single tile covering the whole world. Each
//! finer level doubles the tile count per axis, so level `l` has a
//! `2^l x 2^l` grid. The finest level is `num_levels - 1`. A shape is recorded in
//! every tile its bounding box overlaps at every level, so it is found regardless
//! of the LOD the renderer picks.
//!
//! Shapes are referenced by their index into the slice passed to
//! [`LodPyramid::build`], keeping the pyramid independent of the shape payload.

use reticle_geometry::{Point, Rect};

/// Identifies one tile: its level and its `(x, y)` position within that level's
/// grid. At level `l`, `x` and `y` each range over `0..2^l`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TileId {
    /// The LOD level (0 is coarsest; higher levels are finer).
    pub level: u32,
    /// Column index within the level grid, `0..2^level`.
    pub x: u32,
    /// Row index within the level grid, `0..2^level`.
    pub y: u32,
}

impl TileId {
    /// Creates a tile identifier.
    #[must_use]
    pub fn new(level: u32, x: u32, y: u32) -> Self {
        Self { level, x, y }
    }
}

/// One tile: the shapes whose bounding boxes overlap it.
#[derive(Clone, Debug, Default)]
struct Tile {
    shapes: Vec<u32>,
}

/// A precomputed level-of-detail pyramid over a set of shape bounding boxes.
///
/// Build one with [`LodPyramid::build`], then serve viewport queries with
/// [`LodPyramid::visible_tiles`] / [`LodPyramid::shapes_in_viewport`], choosing a
/// level with [`LodPyramid::level_for_viewport`].
#[derive(Debug)]
pub struct LodPyramid {
    world: Rect,
    num_levels: u32,
    /// `levels[l]` is a row-major `2^l x 2^l` grid of tiles.
    levels: Vec<Vec<Tile>>,
}

impl LodPyramid {
    /// Builds a pyramid with `num_levels` levels over the given `world` bounds,
    /// recording each shape in every tile it overlaps at every level.
    ///
    /// `shape_bboxes` supplies the bounding box of each shape; shapes are later
    /// referenced by their index into this slice. Shapes are clipped to `world`,
    /// so a bounding box extending past the world edge still lands in the border
    /// tiles rather than being dropped.
    ///
    /// # Panics
    ///
    /// Panics if `num_levels` is zero, if `world` has zero width or height, or if
    /// `num_levels` is large enough that `2^(num_levels - 1)` overflows the tile
    /// index range.
    #[must_use]
    pub fn build(world: Rect, num_levels: u32, shape_bboxes: &[Rect]) -> Self {
        assert!(num_levels > 0, "a pyramid needs at least one level");
        assert!(
            world.width() > 0 && world.height() > 0,
            "world bounds must have positive area"
        );
        assert!(
            num_levels <= 31,
            "num_levels too large: tile grid would overflow u32"
        );

        let mut levels: Vec<Vec<Tile>> = Vec::with_capacity(num_levels as usize);
        for level in 0..num_levels {
            let tiles_per_side = 1usize << level;
            let mut grid = Vec::with_capacity(tiles_per_side * tiles_per_side);
            grid.resize_with(tiles_per_side * tiles_per_side, Tile::default);
            levels.push(grid);
        }

        let mut pyramid = Self {
            world,
            num_levels,
            levels,
        };
        for (index, bbox) in shape_bboxes.iter().enumerate() {
            pyramid.insert_shape(u32::try_from(index).expect("shape count exceeds u32"), bbox);
        }
        pyramid
    }

    /// The number of levels in the pyramid.
    #[must_use]
    pub fn num_levels(&self) -> u32 {
        self.num_levels
    }

    /// The world bounds the pyramid tiles.
    #[must_use]
    pub fn world(&self) -> Rect {
        self.world
    }

    /// The number of tiles per axis at `level` (`2^level`), or `None` if `level`
    /// is out of range.
    #[must_use]
    pub fn tiles_per_side(&self, level: u32) -> Option<u32> {
        if level < self.num_levels {
            Some(1u32 << level)
        } else {
            None
        }
    }

    /// The world-space bounds of a single tile, or `None` if `id` is out of range.
    #[must_use]
    pub fn tile_bounds(&self, id: TileId) -> Option<Rect> {
        let per_side = self.tiles_per_side(id.level)?;
        if id.x >= per_side || id.y >= per_side {
            return None;
        }
        let (x0, x1) = axis_span(self.world.min.x, self.world.max.x, id.x, per_side);
        let (y0, y1) = axis_span(self.world.min.y, self.world.max.y, id.y, per_side);
        Some(Rect::new(Point::new(x0, y0), Point::new(x1, y1)))
    }

    /// The indices of shapes overlapping the given tile, or `None` if `id` is out
    /// of range. Indices are into the `shape_bboxes` slice passed to
    /// [`LodPyramid::build`]; each is listed at most once per tile.
    #[must_use]
    pub fn shapes_in_tile(&self, id: TileId) -> Option<&[u32]> {
        let per_side = self.tiles_per_side(id.level)?;
        if id.x >= per_side || id.y >= per_side {
            return None;
        }
        let flat = (id.y * per_side + id.x) as usize;
        Some(&self.levels[id.level as usize][flat].shapes)
    }

    /// The tiles at `level` whose bounds intersect `viewport`, in row-major order.
    /// Returns an empty vector if `level` is out of range or the viewport misses
    /// the world entirely.
    #[must_use]
    pub fn visible_tiles(&self, viewport: Rect, level: u32) -> Vec<TileId> {
        let Some(per_side) = self.tiles_per_side(level) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let (tx0, tx1) = tile_index_span(
            self.world.min.x,
            self.world.max.x,
            viewport.min.x,
            viewport.max.x,
            per_side,
        );
        let (ty0, ty1) = tile_index_span(
            self.world.min.y,
            self.world.max.y,
            viewport.min.y,
            viewport.max.y,
            per_side,
        );
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let id = TileId::new(level, tx, ty);
                // Guard against a viewport touching only tile borders (positive
                // area is required to count as visible).
                if self
                    .tile_bounds(id)
                    .is_some_and(|b| b.intersects(&viewport))
                {
                    out.push(id);
                }
            }
        }
        out
    }

    /// The deduplicated shape indices visible in `viewport` at `level`, in
    /// ascending index order. Convenience over iterating [`Self::visible_tiles`]
    /// and [`Self::shapes_in_tile`] and merging.
    #[must_use]
    pub fn shapes_in_viewport(&self, viewport: Rect, level: u32) -> Vec<u32> {
        let mut shapes: Vec<u32> = self
            .visible_tiles(viewport, level)
            .into_iter()
            .filter_map(|id| self.shapes_in_tile(id))
            .flatten()
            .copied()
            .collect();
        shapes.sort_unstable();
        shapes.dedup();
        shapes
    }

    /// Chooses the coarsest level whose tiles are no larger than `target_tile_dbu`
    /// along either axis — i.e. the least detail that still resolves features of
    /// that size in the given `viewport`. Clamped to the available level range.
    ///
    /// A renderer typically passes `target_tile_dbu` = viewport extent divided by
    /// the number of screen tiles it wants to draw, trading tile count for detail.
    #[must_use]
    pub fn level_for_viewport(&self, target_tile_dbu: i64) -> u32 {
        let target = target_tile_dbu.max(1);
        let world_extent = self.world.width().max(self.world.height());
        for level in 0..self.num_levels {
            let per_side = i64::from(1u32 << level);
            // Ceil-divide the world extent by the tile count for this level.
            // (Manual ceil-div; both operands are positive.)
            let tile_extent = (world_extent + per_side - 1) / per_side;
            if tile_extent <= target {
                return level;
            }
        }
        self.num_levels - 1
    }

    /// Records `shape` in every tile it overlaps at every level.
    fn insert_shape(&mut self, shape: u32, bbox: &Rect) {
        // Clip to the world so out-of-bounds boxes still land in border tiles.
        let Some(clipped) = bbox.intersection(&self.world) else {
            return;
        };
        for level in 0..self.num_levels {
            let per_side = 1u32 << level;
            let (tx0, tx1) = tile_index_span(
                self.world.min.x,
                self.world.max.x,
                clipped.min.x,
                clipped.max.x,
                per_side,
            );
            let (ty0, ty1) = tile_index_span(
                self.world.min.y,
                self.world.max.y,
                clipped.min.y,
                clipped.max.y,
                per_side,
            );
            let grid = &mut self.levels[level as usize];
            for ty in ty0..=ty1 {
                for tx in tx0..=tx1 {
                    let flat = (ty * per_side + tx) as usize;
                    grid[flat].shapes.push(shape);
                }
            }
        }
    }
}

/// The `[start, end)` world-space span of tile index `i` along an axis running
/// from `lo` to `hi`, split into `per_side` equal tiles. The last tile absorbs
/// any rounding remainder so the tiling exactly covers `[lo, hi]`.
fn axis_span(lo: i32, hi: i32, i: u32, per_side: u32) -> (i32, i32) {
    let lo64 = i64::from(lo);
    let extent = i64::from(hi) - lo64;
    let per = i64::from(per_side);
    let start = lo64 + extent * i64::from(i) / per;
    let end = if i + 1 == per_side {
        i64::from(hi)
    } else {
        lo64 + extent * i64::from(i + 1) / per
    };
    // Spans lie within [lo, hi], both `i32`, so the casts cannot truncate.
    (start as i32, end as i32)
}

/// The inclusive range of tile indices along one axis (running `lo..hi`, split
/// into `per_side` tiles) that a query span `[q0, q1]` overlaps, clamped to
/// `0..per_side`.
fn tile_index_span(lo: i32, hi: i32, q0: i32, q1: i32, per_side: u32) -> (u32, u32) {
    let extent = i64::from(hi) - i64::from(lo);
    debug_assert!(extent > 0);
    let per = i64::from(per_side);
    let to_index = |q: i32| -> i64 {
        let rel = i64::from(q) - i64::from(lo);
        // Floor-divide the relative position into a tile index.
        (rel * per).div_euclid(extent)
    };
    let last = i64::from(per_side) - 1;
    let start = to_index(q0).clamp(0, last);
    // The query's max edge is exclusive at a tile boundary, but clamping to
    // `last` and letting the caller's rect intersection decide keeps this simple
    // and correct.
    let end = to_index(q1).clamp(0, last);
    (start as u32, end as u32)
}
