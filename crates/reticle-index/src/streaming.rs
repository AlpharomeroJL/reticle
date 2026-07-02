//! Zero-copy access to archived index payloads with [`rkyv`] 0.8, and a
//! memory-mapped, tile-organized index for out-of-core streaming.
//!
//! An [`IndexPayload`] serializes to a byte buffer laid out exactly as its in-memory
//! representation, so a caller reads shape rectangles straight from those bytes with
//! no parsing or heap allocation, and can index a single archived entry in place. That
//! is precisely what a memory-mapped, larger-than-RAM layout sits on: map a file,
//! hand these bytes in, and the OS pages in only the regions actually touched.
//!
//! [`TiledPayload`] builds on that primitive to make region queries cheap: entries are
//! bucketed into a uniform `N x N` grid and stored tile-contiguously behind a
//! per-tile directory, so a viewport query reads only the header, the directory, and
//! the entries of the tiles the viewport overlaps. [`StreamingIndex`] owns a
//! [`memmap2::Mmap`] over such a file and answers [`StreamingIndex::query_region`]
//! straight from the mapped bytes: the full entries array is never copied into a
//! `Vec`, and the OS demand-pages only the tiles a query touches. See
//! `examples/stream_demo.rs` for a measured, multi-gigabyte demonstration.
//!
//! # Safety
//!
//! The zero-copy readers here use `rkyv`'s validated [`access`](rkyv::access) entry
//! point (enabled by `rkyv`'s default `bytecheck` feature), which checks the byte
//! buffer before handing back a reference, so a truncated or corrupt file yields a
//! [`StreamError`] rather than undefined behaviour. No `access_unchecked` is used.
//!
//! The one exception is memory-mapping the file in [`StreamingIndex::open`], the
//! workspace's only `unsafe`: creating a [`memmap2::Mmap`] is unsafe because the
//! kernel, not Rust, backs those bytes, and a *different process* truncating or
//! mutating the file underneath the map is undefined behaviour (the standard mmap
//! contract). We hold the read-only [`File`] for the map's lifetime and
//! immediately validate the mapped bytes with `rkyv` `access` before reading them. The
//! `unsafe` is confined to a single, documented block; everything else stays safe.

use std::fs::File;
use std::path::Path;

use memmap2::Mmap;
use reticle_geometry::{Point, Rect};
use rkyv::{Archive, Deserialize, Serialize, rancor};

/// A rectangle in a form `rkyv` can archive, mirroring [`Rect`] as four `i32`
/// corners. Convert with [`ArchivableRect::from_rect`] / [`ArchivableRect::to_rect`].
#[derive(Archive, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[rkyv(derive(Debug))]
pub struct ArchivableRect {
    /// Minimum x corner, in DBU.
    pub min_x: i32,
    /// Minimum y corner, in DBU.
    pub min_y: i32,
    /// Maximum x corner, in DBU.
    pub max_x: i32,
    /// Maximum y corner, in DBU.
    pub max_y: i32,
}

impl ArchivableRect {
    /// Converts a [`Rect`] into its archivable form.
    #[must_use]
    pub fn from_rect(r: Rect) -> Self {
        Self {
            min_x: r.min.x,
            min_y: r.min.y,
            max_x: r.max.x,
            max_y: r.max.y,
        }
    }

    /// Reconstructs the [`Rect`] this was built from.
    #[must_use]
    pub fn to_rect(self) -> Rect {
        Rect::new(
            Point::new(self.min_x, self.min_y),
            Point::new(self.max_x, self.max_y),
        )
    }
}

/// A serializable index payload: a flat list of `(bounding box, item id)` entries.
///
/// The `u32` item id is a handle into the caller's shape table. This is the unit
/// that gets archived to disk and memory-mapped; see the [module docs](self).
#[derive(Archive, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[rkyv(derive(Debug))]
pub struct IndexPayload {
    /// The indexed entries, each a rectangle paired with an item id.
    pub entries: Vec<(ArchivableRect, u32)>,
}

impl IndexPayload {
    /// Creates a payload from `(bbox, id)` pairs.
    pub fn from_entries(entries: impl IntoIterator<Item = (Rect, u32)>) -> Self {
        Self {
            entries: entries
                .into_iter()
                .map(|(r, id)| (ArchivableRect::from_rect(r), id))
                .collect(),
        }
    }

    /// The entries as `(Rect, u32)` pairs (allocates a fresh vector).
    #[must_use]
    pub fn to_entries(&self) -> Vec<(Rect, u32)> {
        self.entries
            .iter()
            .map(|(r, id)| (r.to_rect(), *id))
            .collect()
    }
}

/// An error from serializing, opening, or accessing a streamed index payload.
#[derive(Debug)]
pub enum StreamError {
    /// The bytes were not a valid archive, or (de)serialization failed. Covers a
    /// truncated or corrupt file caught by `rkyv`'s `bytecheck` validation.
    Archive(rancor::Error),
    /// Opening or memory-mapping the backing file failed.
    Io(std::io::Error),
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Archive(e) => write!(f, "index streaming error: {e}"),
            Self::Io(e) => write!(f, "index streaming I/O error: {e}"),
        }
    }
}

impl std::error::Error for StreamError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Archive(e) => Some(e),
            Self::Io(e) => Some(e),
        }
    }
}

impl From<rancor::Error> for StreamError {
    fn from(e: rancor::Error) -> Self {
        Self::Archive(e)
    }
}

impl From<std::io::Error> for StreamError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Serializes a payload to an aligned byte buffer suitable for writing to a file
/// and later memory-mapping.
///
/// The returned bytes begin at an alignment `rkyv` requires for zero-copy access;
/// write them to disk verbatim and map the file at a matching alignment.
pub fn serialize(payload: &IndexPayload) -> Result<Vec<u8>, StreamError> {
    let bytes = rkyv::to_bytes::<rancor::Error>(payload)?;
    Ok(bytes.to_vec())
}

/// Validates `bytes` and returns a zero-copy reference to the archived payload.
///
/// No data is copied or deserialized: entries are read in place from the buffer,
/// which may be a memory-mapped file. The buffer must be aligned as produced by
/// [`serialize`]. Returns [`StreamError`] if the bytes are not a valid archive.
pub fn access(bytes: &[u8]) -> Result<&ArchivedIndexPayload, StreamError> {
    Ok(rkyv::access::<ArchivedIndexPayload, rancor::Error>(bytes)?)
}

/// Reads the `index`-th entry of an archived payload as a `(Rect, u32)` pair
/// without deserializing the whole payload, or `None` if out of range.
///
/// This is the zero-copy fast path: the archived integers are decoded directly
/// from the mapped bytes.
#[must_use]
pub fn entry_at(archived: &ArchivedIndexPayload, index: usize) -> Option<(Rect, u32)> {
    // Archived tuples become `ArchivedTuple2` with fields `.0` and `.1`, and
    // archived integers are little-endian wrappers convertible with `From`.
    let entry = archived.entries.get(index)?;
    let rect = &entry.0;
    let bbox = Rect::new(
        Point::new(i32::from(rect.min_x), i32::from(rect.min_y)),
        Point::new(i32::from(rect.max_x), i32::from(rect.max_y)),
    );
    Some((bbox, u32::from(entry.1)))
}

/// The number of entries in an archived payload, read without deserializing.
#[must_use]
pub fn len(archived: &ArchivedIndexPayload) -> usize {
    archived.entries.len()
}

/// Validates and fully deserializes `bytes` back into an owned [`IndexPayload`].
///
/// Use this when you need an owned, mutable copy; prefer [`access`] + [`entry_at`]
/// for read-only streaming, which avoids the allocation.
pub fn load(bytes: &[u8]) -> Result<IndexPayload, StreamError> {
    Ok(rkyv::from_bytes::<IndexPayload, rancor::Error>(bytes)?)
}

// ---------------------------------------------------------------------------
// Tile-organized payload for out-of-core region queries.
// ---------------------------------------------------------------------------

/// The fixed-size header of a [`TiledPayload`]: the world rectangle the grid spans
/// (as four `i32` corners) and the grid side `grid_n`.
///
/// The grid is `grid_n x grid_n` uniform tiles over `[min, max)` in each axis. Tile
/// column `tx` and row `ty` (both in `0..grid_n`) live at directory index
/// `ty * grid_n + tx` (row-major).
#[derive(Archive, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[rkyv(derive(Debug))]
pub struct TileHeader {
    /// World minimum x corner, in DBU.
    pub world_min_x: i32,
    /// World minimum y corner, in DBU.
    pub world_min_y: i32,
    /// World maximum x corner, in DBU.
    pub world_max_x: i32,
    /// World maximum y corner, in DBU.
    pub world_max_y: i32,
    /// Grid side: the world is split into `grid_n x grid_n` tiles. Always `>= 1`.
    pub grid_n: u32,
    /// Maximum entry width across all entries, in DBU. A region query expands its
    /// tile search by this margin in x so that an entry bucketed by its center, whose
    /// bbox spills into a neighbouring tile, is never missed. `0` when empty.
    pub max_extent_x: u32,
    /// Maximum entry height across all entries, in DBU. The y counterpart of
    /// [`max_extent_x`](Self::max_extent_x).
    pub max_extent_y: u32,
}

impl TileHeader {
    /// The world rectangle this header describes.
    #[must_use]
    pub fn world(&self) -> Rect {
        Rect::new(
            Point::new(self.world_min_x, self.world_min_y),
            Point::new(self.world_max_x, self.world_max_y),
        )
    }
}

/// One slot of a [`TiledPayload`] directory: a half-open range `offset..offset + count`
/// into the flat, tile-contiguous `entries` array.
#[derive(Archive, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[rkyv(derive(Debug))]
pub struct TileSlot {
    /// Index of this tile's first entry in the flat `entries` array.
    pub offset: u32,
    /// Number of entries in this tile.
    pub count: u32,
}

/// A tile-organized, `rkyv`-serializable index payload.
///
/// Entries are bucketed into a uniform `grid_n x grid_n` grid by the tile containing
/// each entry's center, then laid out tile-contiguously in `entries`. The `directory`
/// (length `grid_n * grid_n`, row-major) records each tile's `(offset, count)` range
/// into `entries`. This lets a region query read only the header, the directory, and
/// the entries of the tiles the region overlaps, rather than scanning everything.
///
/// # Byte layout
///
/// Serialized with `rkyv`, the archive contains, reachable from its root:
/// - a [`TileHeader`] (four `i32` world corners plus `grid_n`),
/// - a `directory` of `grid_n * grid_n` [`TileSlot`]s (`offset: u32`, `count: u32`),
///   in row-major tile order, and
/// - a flat `entries` array of `(ArchivableRect, u32)`, grouped so that tile `t`'s
///   entries occupy `directory[t].offset .. directory[t].offset + directory[t].count`.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[rkyv(derive(Debug))]
pub struct TiledPayload {
    /// World rectangle and grid side.
    pub header: TileHeader,
    /// Per-tile `(offset, count)` ranges into `entries`, row-major, length `grid_n^2`.
    pub directory: Vec<TileSlot>,
    /// All entries, grouped tile-contiguously per `directory`.
    pub entries: Vec<(ArchivableRect, u32)>,
}

/// Maps a world coordinate to a tile index along one axis.
///
/// `lo`/`hi` are the world bounds on this axis, `n` the grid side. Coordinates are
/// clamped into `0..n` so out-of-world entries and queries land on the border tiles
/// rather than out of range. `hi <= lo` collapses the axis to a single tile.
fn axis_tile(coord: i32, lo: i32, hi: i32, n: u32) -> u32 {
    debug_assert!(n >= 1);
    if hi <= lo {
        return 0;
    }
    // Widen to i64 so the multiply cannot overflow for full-i32-range worlds.
    let span = i64::from(hi) - i64::from(lo);
    let rel = (i64::from(coord) - i64::from(lo)).clamp(0, span - 1);
    let idx = rel * i64::from(n) / span;
    // idx is in 0..n by construction; clamp defensively.
    (idx as u32).min(n - 1)
}

impl TiledPayload {
    /// Builds a tiled payload over `world`, split into a `grid_n x grid_n` grid, from
    /// `(bbox, id)` entries.
    ///
    /// Each entry is bucketed into the tile containing the center of its bounding box
    /// (clamped into the world, so entries outside `world` land on the border tiles).
    /// Entries are laid out tile-contiguously and the directory is filled to match.
    /// `grid_n` is raised to at least `1`.
    pub fn build(world: Rect, grid_n: u32, entries: impl IntoIterator<Item = (Rect, u32)>) -> Self {
        let n = grid_n.max(1);
        let tile_count = (n as usize) * (n as usize);

        // First pass: assign each entry to a tile, count per-tile occupancy, and track
        // the largest entry extent so queries know how far a center-bucketed entry can
        // reach into neighbouring tiles.
        let mut counts = vec![0u32; tile_count];
        let mut assigned: Vec<(usize, ArchivableRect, u32)> = Vec::new();
        let mut max_extent_x: u32 = 0;
        let mut max_extent_y: u32 = 0;
        for (rect, id) in entries {
            let cx = i64::midpoint(i64::from(rect.min.x), i64::from(rect.max.x)) as i32;
            let cy = i64::midpoint(i64::from(rect.min.y), i64::from(rect.max.y)) as i32;
            let tx = axis_tile(cx, world.min.x, world.max.x, n);
            let ty = axis_tile(cy, world.min.y, world.max.y, n);
            let tile = (ty as usize) * (n as usize) + (tx as usize);
            counts[tile] += 1;
            // Widths/heights are non-negative (`Rect::new` normalizes) and fit in u32.
            max_extent_x = max_extent_x.max(rect.width().min(i64::from(u32::MAX)) as u32);
            max_extent_y = max_extent_y.max(rect.height().min(i64::from(u32::MAX)) as u32);
            assigned.push((tile, ArchivableRect::from_rect(rect), id));
        }

        let header = TileHeader {
            world_min_x: world.min.x,
            world_min_y: world.min.y,
            world_max_x: world.max.x,
            world_max_y: world.max.y,
            grid_n: n,
            max_extent_x,
            max_extent_y,
        };

        // Prefix-sum the counts into directory offsets.
        let mut directory = Vec::with_capacity(tile_count);
        let mut running: u32 = 0;
        for &count in &counts {
            directory.push(TileSlot {
                offset: running,
                count,
            });
            running += count;
        }
        let total = running as usize;

        // Second pass: scatter each entry into its tile's contiguous run. `cursor`
        // tracks the next free slot within each tile.
        let mut placed: Vec<Option<(ArchivableRect, u32)>> = vec![None; total];
        let mut cursor: Vec<u32> = directory.iter().map(|slot| slot.offset).collect();
        for (tile, rect, id) in assigned {
            let pos = cursor[tile] as usize;
            cursor[tile] += 1;
            placed[pos] = Some((rect, id));
        }
        let entries = placed
            .into_iter()
            .map(|e| e.expect("every slot filled by construction"))
            .collect();

        Self {
            header,
            directory,
            entries,
        }
    }

    /// Serializes this tiled payload to an aligned byte buffer for writing to a file
    /// and later memory-mapping via [`StreamingIndex::open`].
    pub fn serialize(&self) -> Result<Vec<u8>, StreamError> {
        let bytes = rkyv::to_bytes::<rancor::Error>(self)?;
        Ok(bytes.to_vec())
    }
}

/// Given the archived header and a query `region`, returns the inclusive tile-column
/// and tile-row bounds `(tx0, tx1, ty0, ty1)` whose entries could intersect `region`,
/// or `None` if no tile can (an empty region, or one disjoint from the world).
///
/// Entries are bucketed by their center, so an entry sitting in tile `T` can reach up
/// to its own extent into a neighbouring tile. To stay exact, we widen `region` by the
/// maximum entry extent before mapping it to a tile range: any entry that truly
/// intersects `region` has its center within the widened box, so its tile is included.
/// The caller still filters by `intersects`, so widening never yields false positives
/// in the final result, only a few extra tiles scanned.
///
/// This reads only the header, so a query narrows to a small tile rectangle before
/// touching any directory slot or entry.
fn overlapping_tiles(header: &ArchivedTileHeader, region: Rect) -> Option<(u32, u32, u32, u32)> {
    let n = u32::from(header.grid_n);
    if n == 0 || region.is_empty() {
        return None;
    }
    let world = Rect::new(
        Point::new(i32::from(header.world_min_x), i32::from(header.world_min_y)),
        Point::new(i32::from(header.world_max_x), i32::from(header.world_max_y)),
    );
    if !region.intersects(&world) {
        return None;
    }

    // Widen the region by the max entry extent (saturating at the i32 range) so a
    // center-bucketed entry spilling in from a neighbour is not missed.
    let ex = i64::from(u32::from(header.max_extent_x));
    let ey = i64::from(u32::from(header.max_extent_y));
    let lo_x = (i64::from(region.min.x) - ex).max(i64::from(i32::MIN)) as i32;
    let lo_y = (i64::from(region.min.y) - ey).max(i64::from(i32::MIN)) as i32;
    // The max corner is exclusive; use `max - 1` for the last touched coordinate, then
    // widen it outward.
    let hi_x = (i64::from(region.max.x - 1) + ex).min(i64::from(i32::MAX)) as i32;
    let hi_y = (i64::from(region.max.y - 1) + ey).min(i64::from(i32::MAX)) as i32;

    let tx0 = axis_tile(lo_x, world.min.x, world.max.x, n);
    let ty0 = axis_tile(lo_y, world.min.y, world.max.y, n);
    let tx1 = axis_tile(hi_x, world.min.x, world.max.x, n);
    let ty1 = axis_tile(hi_y, world.min.y, world.max.y, n);
    Some((tx0.min(tx1), tx0.max(tx1), ty0.min(ty1), ty0.max(ty1)))
}

/// A memory-mapped, tile-organized index answering zero-copy region queries.
///
/// [`open`](StreamingIndex::open) maps a file produced by [`TiledPayload::serialize`];
/// [`query_region`](StreamingIndex::query_region) reads matching entries straight from
/// the mapped bytes. The full entries array is never copied into a `Vec`; the OS
/// demand-pages only the tiles a query touches.
#[derive(Debug)]
pub struct StreamingIndex {
    // Field order matters for drop order: unmap before closing the file. `mmap` also
    // borrows conceptually from `_file`, which we keep alive for the map's lifetime.
    mmap: Mmap,
    _file: File,
}

impl StreamingIndex {
    /// Opens `path` read-only and memory-maps it, validating the mapped bytes as a
    /// [`TiledPayload`] archive before returning.
    ///
    /// Returns [`StreamError::Io`] if the file cannot be opened or mapped, or
    /// [`StreamError::Archive`] if the mapped bytes are not a valid tiled archive.
    pub fn open(path: &Path) -> Result<Self, StreamError> {
        let file = File::open(path)?;

        // SAFETY: `Mmap::map` is unsafe because the mapped memory is backed by the
        // file on disk, not by Rust's allocator: its contents can change out from
        // under us if the file is mutated or truncated by another process while the
        // map is live, which would be undefined behaviour (this is the standard mmap
        // contract, unavoidable for any memory-mapped file). We uphold the safe side
        // of that contract here: we own `file` (opened read-only just above), store it
        // in the returned `StreamingIndex` so it outlives `mmap`, and never expose a
        // way to resize or write it. Callers must not have another process truncate or
        // write the file while a `StreamingIndex` over it is alive. Immediately after
        // mapping we validate the bytes with `rkyv` `access` (bytecheck), so a corrupt
        // or truncated-at-open file is rejected as an error rather than read unchecked.
        #[allow(unsafe_code)]
        let mmap = unsafe { Mmap::map(&file)? };

        // Validate now so `query_region` can access the archive without re-checking.
        rkyv::access::<ArchivedTiledPayload, rancor::Error>(&mmap)?;

        Ok(Self { mmap, _file: file })
    }

    /// The archived payload, re-validated from the mapped bytes.
    ///
    /// `open` already validated the map, so in practice this does not fail; it returns
    /// a `Result` to keep the zero-copy `access` contract honest.
    fn payload(&self) -> Result<&ArchivedTiledPayload, StreamError> {
        Ok(rkyv::access::<ArchivedTiledPayload, rancor::Error>(
            &self.mmap,
        )?)
    }

    /// The world rectangle and grid side of the mapped index.
    pub fn header(&self) -> Result<TileHeader, StreamError> {
        let p = self.payload()?;
        Ok(TileHeader {
            world_min_x: i32::from(p.header.world_min_x),
            world_min_y: i32::from(p.header.world_min_y),
            world_max_x: i32::from(p.header.world_max_x),
            world_max_y: i32::from(p.header.world_max_y),
            grid_n: u32::from(p.header.grid_n),
            max_extent_x: u32::from(p.header.max_extent_x),
            max_extent_y: u32::from(p.header.max_extent_y),
        })
    }

    /// The total number of entries in the mapped index (read from the archive without
    /// materializing them).
    pub fn total_entries(&self) -> usize {
        self.payload().map_or(0, |p| p.entries.len())
    }

    /// Returns every entry whose bounding box intersects `region`, read zero-copy from
    /// the mapped bytes.
    ///
    /// Only the header, the directory, and the entries of the tiles overlapping
    /// `region` are touched; the rest of the entries array is never read (so its pages
    /// are never faulted in). Entries whose bbox does not actually intersect `region`
    /// are filtered out, so the result is exact, not merely tile-approximate.
    pub fn query_region(&self, region: Rect) -> Vec<(Rect, u32)> {
        self.query_region_counted(region).0
    }

    /// Like [`query_region`](Self::query_region) but also returns the number of tiles
    /// and entries actually touched, for measurement and demonstration.
    ///
    /// The returned tuple is `(results, tiles_touched, entries_scanned)`.
    pub fn query_region_counted(&self, region: Rect) -> (Vec<(Rect, u32)>, usize, usize) {
        let Ok(archived) = self.payload() else {
            return (Vec::new(), 0, 0);
        };
        let n = u32::from(archived.header.grid_n);
        let Some((tx0, tx1, ty0, ty1)) = overlapping_tiles(&archived.header, region) else {
            return (Vec::new(), 0, 0);
        };

        let mut results = Vec::new();
        let mut tiles_touched = 0usize;
        let mut entries_scanned = 0usize;
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let tile = (ty as usize) * (n as usize) + (tx as usize);
                let Some(slot) = archived.directory.get(tile) else {
                    continue;
                };
                tiles_touched += 1;
                let start = u32::from(slot.offset) as usize;
                let count = u32::from(slot.count) as usize;
                for i in start..start + count {
                    // Zero-copy: this indexes into the mapped entries array and only
                    // faults in the page(s) holding this tile's run.
                    let Some(entry) = archived.entries.get(i) else {
                        break;
                    };
                    entries_scanned += 1;
                    let rect = &entry.0;
                    let bbox = Rect::new(
                        Point::new(i32::from(rect.min_x), i32::from(rect.min_y)),
                        Point::new(i32::from(rect.max_x), i32::from(rect.max_y)),
                    );
                    if bbox.intersects(&region) {
                        results.push((bbox, u32::from(entry.1)));
                    }
                }
            }
        }
        (results, tiles_touched, entries_scanned)
    }
}
