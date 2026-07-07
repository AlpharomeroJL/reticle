//! Concrete [`TileSource`](crate::archive::TileSource) implementations (Wave 2 lane 2B).
//!
//! ADR 0062 freezes the [`TileSource`](crate::archive::TileSource) trait and the
//! `.rtla` types in [`crate::archive`]; this module fills the read side of that
//! contract with three sources plus a small viewport query layer:
//!
//! - [`MmapTileSource`] (native): memory-maps a `.rtla` file, reads the header and
//!   directory once, and serves a tile by slicing the mapped range. It reuses the
//!   [`crate::streaming`] mmap discipline exactly (validated `rkyv` access, the one
//!   documented `unsafe`).
//! - [`HttpRangeTileSource`] (wasm): fetches the header with two ranged GETs, then a
//!   tile per `fetch` with a `Range: bytes=offset-end` header, in front of an
//!   in-memory [`LruByteCache`] (a byte-budgeted LRU) and an OPFS persistent cache
//!   keyed by the archive so a revisit is instant. The pure cache and cache-key logic
//!   is target-independent and unit-tested here; only the fetch/OPFS glue is
//!   `cfg(target_arch = "wasm32")`.
//! - [`MemTileSource`] (any target): an in-memory map of tiles, the double the
//!   headline proptest streams against and compares to the in-RAM R-tree.
//!
//! # Physical framing of a `.rtla` file
//!
//! ADR 0062 fixes the logical layout (header block, tile directory, byte-contiguous
//! tiles) and the frozen types, but not the byte framing that lets a reader find each
//! block. This module fixes that framing, documented here as the concrete realization
//! of the contract (see ADR 0063). Every block starts at a 16-byte-aligned file offset
//! so a memory-mapped block validates zero-copy and a fetched block is aligned enough
//! for `rkyv`:
//!
//! ```text
//! offset 0    [8]  RTLA_MAGIC                       raw bytes, checkable before parsing
//! offset 8    [4]  u32 LE version                   refused if not RTLA_VERSION
//! offset 12   [4]  u32 LE flags (reserved, 0)
//! offset 16   [8]  u64 LE header_len                archived RtlaHeader byte length
//! offset 24   [8]  u64 LE directory_len             archived Vec<TileDirEntry> length
//! offset 32        header_len bytes                 rkyv RtlaHeader (32 is 16-aligned)
//! (pad to 16)      directory_len bytes              rkyv Vec<TileDirEntry>
//! (pad to 16)      tile payloads                    each rkyv TilePayload, byte-contiguous
//! ```
//!
//! A [`TileDirEntry`]'s `offset` is an absolute file offset (16-aligned) and `len` is
//! the tile's archived byte length, so one `Range` request over `[offset, offset+len)`
//! yields exactly one independently-validatable tile.
//!
//! # Untrusted input
//!
//! The header's per-level grid dimensions and the directory arrive from the network
//! and are untrusted. No `Vec` is ever reserved from a count or length read out of
//! them: tile counts are summed with checked `u64` arithmetic (overflow is an error,
//! not a panic or a giant allocation), a header whose level dimensions disagree with
//! the actual directory length is rejected as [`TileSourceError::Malformed`], and the
//! header plus directory fetch is capped so a lying preamble cannot ask for gigabytes.
//! This carries the OASIS out-of-memory lesson (commit 1b1b56b) into the reader.

use std::collections::HashMap;

use reticle_geometry::Rect;
use rkyv::rancor;

use crate::archive::{
    ArchivedTilePayload, RTLA_MAGIC, RTLA_VERSION, RtlaHeader, TileCoord, TileDirEntry,
    TilePayload, TileRecord, TileSource, TileSourceError,
};

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

// ---------------------------------------------------------------------------
// Physical framing constants and helpers (all targets).
// ---------------------------------------------------------------------------

/// Bytes in the fixed preamble: magic, version, flags, `header_len`, `directory_len`.
const PREAMBLE_LEN: u64 = 32;

/// Alignment every block (header, directory, each tile) begins on, so a mapped block
/// validates zero-copy and a freshly allocated fetched block is aligned enough for the
/// archived types (which need at most 8-byte alignment).
const BLOCK_ALIGN: u64 = 16;

/// Cap on the combined header + directory byte length a reader will fetch or trust
/// before parsing, so a corrupt or hostile preamble claiming a huge header/directory
/// is rejected up front rather than driving a gigabyte-sized request or allocation.
/// 128 MiB is far above any real header/directory and far below a memory hazard.
const MAX_HEADER_DIR_BYTES: u64 = 128 * 1024 * 1024;

/// Rounds `value` up to the next multiple of `align` (a power of two).
const fn align_up(value: u64, align: u64) -> u64 {
    (value + align - 1) & !(align - 1)
}

/// The fixed preamble at the start of a `.rtla` file: the archived byte lengths of the
/// header and directory blocks, validated against the magic and version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Preamble {
    header_len: u64,
    directory_len: u64,
}

impl Preamble {
    /// Parses and validates the 32-byte preamble at the start of `bytes`.
    ///
    /// Checks the magic and version before trusting the length fields. The lengths
    /// themselves are bounded by the caller (against the actual file length or the
    /// [`MAX_HEADER_DIR_BYTES`] cap) before any slice or allocation uses them.
    fn parse(bytes: &[u8]) -> Result<Self, TileSourceError> {
        if bytes.len() < PREAMBLE_LEN as usize {
            return Err(TileSourceError::Malformed(format!(
                "archive shorter than the {PREAMBLE_LEN}-byte preamble"
            )));
        }
        if bytes[0..8] != RTLA_MAGIC {
            return Err(TileSourceError::Malformed(
                "bad magic: not an .rtla archive".to_owned(),
            ));
        }
        let version = u32::from_le_bytes(bytes[8..12].try_into().expect("4 bytes"));
        if version != RTLA_VERSION {
            return Err(TileSourceError::Malformed(format!(
                "unsupported .rtla version {version} (this build reads {RTLA_VERSION})"
            )));
        }
        // bytes[12..16] are reserved flags, ignored in v1.
        let header_len = u64::from_le_bytes(bytes[16..24].try_into().expect("8 bytes"));
        let directory_len = u64::from_le_bytes(bytes[24..32].try_into().expect("8 bytes"));
        Ok(Self {
            header_len,
            directory_len,
        })
    }

    /// Encodes the 32-byte preamble for the given block lengths (archive writer side).
    ///
    /// Only the test archive writer needs this in lane 2B (the production writer is lane
    /// 2A); gated to tests so the read-only build carries no unused writer code.
    #[cfg(test)]
    fn encode(self) -> [u8; PREAMBLE_LEN as usize] {
        let mut out = [0u8; PREAMBLE_LEN as usize];
        out[0..8].copy_from_slice(&RTLA_MAGIC);
        out[8..12].copy_from_slice(&RTLA_VERSION.to_le_bytes());
        out[16..24].copy_from_slice(&self.header_len.to_le_bytes());
        out[24..32].copy_from_slice(&self.directory_len.to_le_bytes());
        out
    }

    /// The file offset where the header block starts.
    const fn header_start() -> u64 {
        PREAMBLE_LEN
    }

    /// The file offset where the header block ends (exclusive).
    const fn header_end(self) -> u64 {
        Self::header_start() + self.header_len
    }

    /// The 16-aligned file offset where the directory block starts.
    const fn directory_start(self) -> u64 {
        align_up(self.header_end(), BLOCK_ALIGN)
    }

    /// The file offset where the directory block ends (exclusive).
    const fn directory_end(self) -> u64 {
        self.directory_start() + self.directory_len
    }
}

/// Serializes a [`TilePayload`] to its archived bytes.
fn serialize_tile(payload: &TilePayload) -> Vec<u8> {
    rkyv::to_bytes::<rancor::Error>(payload)
        .expect("serializing an in-memory TilePayload cannot fail")
        .to_vec()
}

/// Validates and deserializes an archived [`RtlaHeader`], checking the magic and
/// version fields as well as the `rkyv` byte validity.
fn deserialize_header(bytes: &[u8]) -> Result<RtlaHeader, TileSourceError> {
    let header = rkyv::from_bytes::<RtlaHeader, rancor::Error>(bytes)
        .map_err(|e| TileSourceError::Malformed(format!("header archive invalid: {e}")))?;
    if header.magic != RTLA_MAGIC {
        return Err(TileSourceError::Malformed(
            "header magic field does not match RTLA_MAGIC".to_owned(),
        ));
    }
    if header.version != RTLA_VERSION {
        return Err(TileSourceError::Malformed(format!(
            "header version {} is not {RTLA_VERSION}",
            header.version
        )));
    }
    Ok(header)
}

/// Validates and deserializes an archived tile directory.
fn deserialize_directory(bytes: &[u8]) -> Result<Vec<TileDirEntry>, TileSourceError> {
    rkyv::from_bytes::<Vec<TileDirEntry>, rancor::Error>(bytes)
        .map_err(|e| TileSourceError::Malformed(format!("directory archive invalid: {e}")))
}

/// Validates a tile's bytes as an archived [`TilePayload`] without deserializing it,
/// exactly as [`crate::streaming`] validates a mapped payload. A truncated or corrupt
/// tile becomes an error, never undefined behaviour.
fn validate_tile(bytes: &[u8]) -> Result<(), TileSourceError> {
    rkyv::access::<ArchivedTilePayload, rancor::Error>(bytes)
        .map(|_| ())
        .map_err(|e| TileSourceError::Malformed(format!("tile archive invalid: {e}")))
}

/// The total number of tiles a header's levels imply, summed with checked `u64`
/// arithmetic. Returns `None` on overflow, which the callers turn into a
/// [`TileSourceError::Malformed`] rather than allocating from an untrusted count.
fn expected_tile_count(header: &RtlaHeader) -> Option<u64> {
    let mut total: u64 = 0;
    for dims in &header.levels {
        let tiles = u64::from(dims.cols).checked_mul(u64::from(dims.rows))?;
        total = total.checked_add(tiles)?;
    }
    Some(total)
}

/// Rejects a header whose level dimensions do not match the actual directory length,
/// the load-bearing untrusted-input check: a header claiming billions of tiles behind
/// a small directory errors here (bounded work), never allocates from the claim.
fn check_header_directory_consistency(
    header: &RtlaHeader,
    directory_len: usize,
) -> Result<(), TileSourceError> {
    match expected_tile_count(header) {
        Some(expected) if expected == directory_len as u64 => Ok(()),
        Some(expected) => Err(TileSourceError::Malformed(format!(
            "directory has {directory_len} entries but the header's level dimensions imply \
             {expected} tiles"
        ))),
        None => Err(TileSourceError::Malformed(
            "header level dimensions overflow the tile-count range".to_owned(),
        )),
    }
}

/// The flat directory index of `coord`, given the header's level-major, row-major
/// directory ordering, or an error if `coord` is out of range for the header.
///
/// All arithmetic is checked `u64`: a coordinate implied by out-of-range level
/// dimensions cannot overflow into a bogus in-range index, and nothing is allocated
/// from a level's tile count.
fn tile_directory_index(header: &RtlaHeader, coord: TileCoord) -> Result<usize, TileSourceError> {
    let level = coord.level as usize;
    let dims = header
        .levels
        .get(level)
        .ok_or(TileSourceError::OutOfRange(coord))?;
    if coord.col >= dims.cols || coord.row >= dims.rows {
        return Err(TileSourceError::OutOfRange(coord));
    }
    let overflow = || TileSourceError::Malformed("tile index arithmetic overflow".to_owned());

    let mut base: u64 = 0;
    for prior in &header.levels[..level] {
        let tiles = u64::from(prior.cols)
            .checked_mul(u64::from(prior.rows))
            .ok_or_else(overflow)?;
        base = base.checked_add(tiles).ok_or_else(overflow)?;
    }
    let within = u64::from(coord.row)
        .checked_mul(u64::from(dims.cols))
        .and_then(|v| v.checked_add(u64::from(coord.col)))
        .ok_or_else(overflow)?;
    let index = base.checked_add(within).ok_or_else(overflow)?;
    usize::try_from(index).map_err(|_| overflow())
}

// ---------------------------------------------------------------------------
// Viewport -> tile mapping (all targets).
// ---------------------------------------------------------------------------

/// The tile index of world coordinate `q` along an axis running `lo..hi` split into `n`
/// equal tiles, floored and clamped to `0..n`. Mirrors the uniform grid mapping used by
/// [`crate::streaming`] and [`crate::lod`]: `hi <= lo` or `n <= 1` collapses to tile 0.
fn axis_tile_index(lo: i32, hi: i32, q: i32, n: u32) -> u32 {
    if n <= 1 || hi <= lo {
        return 0;
    }
    let span = i64::from(hi) - i64::from(lo);
    let rel = i64::from(q) - i64::from(lo);
    let idx = (rel * i64::from(n)).div_euclid(span);
    idx.clamp(0, i64::from(n) - 1) as u32
}

/// The inclusive `[start, end]` tile index range along an axis (`lo..hi`, split into
/// `n` tiles) that the query span `[q0, q1]` overlaps.
fn axis_tile_range(lo: i32, hi: i32, q0: i32, q1: i32, n: u32) -> (u32, u32) {
    let a = axis_tile_index(lo, hi, q0, n);
    let b = axis_tile_index(lo, hi, q1, n);
    (a.min(b), a.max(b))
}

/// The finest level's index and its grid dimensions, or `None` if the header has no
/// levels. Queries always resolve against the finest level (ADR 0062).
fn finest_level(header: &RtlaHeader) -> Option<(u32, u32, u32)> {
    let level = header.levels.len().checked_sub(1)?;
    let dims = header.levels[level];
    Some((level as u32, dims.cols, dims.rows))
}

// ---------------------------------------------------------------------------
// Pure LRU byte cache (all targets, unit-tested off-wasm).
// ---------------------------------------------------------------------------

/// A least-recently-used tile cache under a fixed byte budget, keyed by [`TileCoord`].
///
/// The wasm [`HttpRangeTileSource`] keeps one of these in front of the network so a
/// tile revisited while panning does not re-fetch. The logic is deliberately
/// target-independent and has no DOM dependency, so eviction is proven in plain unit
/// tests: inserting past the budget evicts the least-recently-used tiles first, a
/// [`get`](LruByteCache::get) marks a tile most-recently-used, and a single tile larger
/// than the whole budget is simply not retained (the cache ends empty rather than
/// exceeding its budget).
#[derive(Debug, Default)]
pub struct LruByteCache {
    budget: usize,
    used: usize,
    map: HashMap<TileCoord, Vec<u8>>,
    /// Access order, least-recently-used at the front, most-recently-used at the back.
    order: Vec<TileCoord>,
}

impl LruByteCache {
    /// A cache holding at most `budget` bytes of tile payloads. A budget of `0`
    /// disables caching (every insert is immediately evicted).
    #[must_use]
    pub fn with_budget(budget: usize) -> Self {
        Self {
            budget,
            used: 0,
            map: HashMap::new(),
            order: Vec::new(),
        }
    }

    /// The byte budget the cache holds within.
    #[must_use]
    pub fn budget(&self) -> usize {
        self.budget
    }

    /// The total bytes currently cached.
    #[must_use]
    pub fn used_bytes(&self) -> usize {
        self.used
    }

    /// The number of tiles currently cached.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the cache holds no tiles.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Returns a cached tile's bytes (a clone) and marks it most-recently-used, or
    /// `None` on a miss.
    pub fn get(&mut self, coord: TileCoord) -> Option<Vec<u8>> {
        if !self.map.contains_key(&coord) {
            return None;
        }
        self.touch(coord);
        self.map.get(&coord).cloned()
    }

    /// Inserts (or replaces) a tile, then evicts least-recently-used tiles until the
    /// cache is within budget.
    pub fn insert(&mut self, coord: TileCoord, bytes: Vec<u8>) {
        let len = bytes.len();
        if let Some(old) = self.map.insert(coord, bytes) {
            self.used -= old.len();
            self.order.retain(|c| *c != coord);
        }
        self.used += len;
        self.order.push(coord);
        self.evict_to_budget();
    }

    /// Moves `coord` to the most-recently-used end of the order.
    fn touch(&mut self, coord: TileCoord) {
        self.order.retain(|c| *c != coord);
        self.order.push(coord);
    }

    /// Evicts least-recently-used tiles until `used <= budget`.
    fn evict_to_budget(&mut self) {
        while self.used > self.budget && !self.order.is_empty() {
            let lru = self.order.remove(0);
            if let Some(bytes) = self.map.remove(&lru) {
                self.used -= bytes.len();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// OPFS persistent-cache key (pure, all targets).
// ---------------------------------------------------------------------------

/// FNV-1a 64-bit hash of `bytes` continuing from `seed`.
fn fnv1a64(seed: u64, bytes: &[u8]) -> u64 {
    let mut hash = seed;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// A stable OPFS directory name for an archive, keyed by its URL and (when the server
/// sends one) its `ETag`.
///
/// The persistent tile cache lives under this name so revisiting the same archive URL
/// reuses cached tiles, while a changed `ETag` (the archive was rebuilt at the same
/// URL) derives a different key so stale tiles are never served. Pure and unit-tested;
/// the OPFS read/write that uses it is the wasm glue.
#[must_use]
pub fn opfs_cache_key(url: &str, etag: Option<&str>) -> String {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    let mut hash = fnv1a64(FNV_OFFSET, url.as_bytes());
    hash = fnv1a64(hash, b"\x00");
    if let Some(tag) = etag {
        hash = fnv1a64(hash, tag.as_bytes());
    }
    format!("rtla-{hash:016x}")
}

/// The OPFS file name a single tile is cached under, within its archive's directory.
#[cfg(target_arch = "wasm32")]
fn opfs_tile_file_name(coord: TileCoord) -> String {
    format!("l{}_c{}_r{}", coord.level, coord.col, coord.row)
}

// ---------------------------------------------------------------------------
// Tile packing shared by the in-memory double and the test archive writer.
// ---------------------------------------------------------------------------

/// Buckets `records` into the tiles they overlap at each level and returns the header
/// plus a map from tile coordinate to that tile's archived [`TilePayload`] bytes
/// (empty tiles are omitted from the map).
///
/// Each record is placed in every tile its rectangle overlaps at every level, so a
/// viewport query over the finest level, filtering by `intersects`, finds exactly the
/// records that intersect the viewport (a record intersecting the viewport shares at
/// least one tile with it). `level_dims` is coarsest-first; the last entry is the
/// finest level queries resolve against.
fn pack_records(
    world: Rect,
    dbu_per_micron: i64,
    level_dims: &[crate::archive::LevelDims],
    records: &[TileRecord],
) -> (RtlaHeader, HashMap<TileCoord, Vec<u8>>) {
    use crate::streaming::ArchivableRect;

    let header = RtlaHeader {
        magic: RTLA_MAGIC,
        version: RTLA_VERSION,
        world: ArchivableRect::from_rect(world),
        dbu_per_micron,
        levels: level_dims.to_vec(),
    };

    let mut buckets: HashMap<TileCoord, Vec<TileRecord>> = HashMap::new();
    for (level, dims) in level_dims.iter().enumerate() {
        let level = level as u32;
        for record in records {
            let rect = record.rect.to_rect();
            let (tx0, tx1) =
                axis_tile_range(world.min.x, world.max.x, rect.min.x, rect.max.x, dims.cols);
            let (ty0, ty1) =
                axis_tile_range(world.min.y, world.max.y, rect.min.y, rect.max.y, dims.rows);
            for row in ty0..=ty1 {
                for col in tx0..=tx1 {
                    buckets
                        .entry(TileCoord { level, col, row })
                        .or_default()
                        .push(*record);
                }
            }
        }
    }

    let tiles = buckets
        .into_iter()
        .map(|(coord, records)| (coord, serialize_tile(&TilePayload { records })))
        .collect();
    (header, tiles)
}

// ---------------------------------------------------------------------------
// MemTileSource: the in-memory test double (all targets).
// ---------------------------------------------------------------------------

/// An in-memory [`TileSource`] over a map of tiles, the double the headline proptest
/// streams against and compares to the in-RAM R-tree.
///
/// Build one from a set of records with [`MemTileSource::from_records`], which packs
/// them into per-level tiles per the frozen format. It serves any in-range tile that
/// has no records as an empty [`TilePayload`], so a viewport query behaves the same as
/// against a real archive.
#[derive(Debug)]
pub struct MemTileSource {
    header: RtlaHeader,
    tiles: HashMap<TileCoord, Vec<u8>>,
    /// A cached archived empty payload, served for in-range tiles absent from `tiles`.
    empty_tile: Vec<u8>,
}

impl MemTileSource {
    /// Builds a double from `records` tiled over `world` into the given per-level grids
    /// (coarsest first). The finest level is the last entry; queries resolve there.
    #[must_use]
    pub fn from_records(
        world: Rect,
        dbu_per_micron: i64,
        level_dims: &[crate::archive::LevelDims],
        records: &[TileRecord],
    ) -> Self {
        let (header, tiles) = pack_records(world, dbu_per_micron, level_dims, records);
        Self {
            header,
            tiles,
            empty_tile: serialize_tile(&TilePayload::default()),
        }
    }

    /// Builds a double directly from a header and a tile map, validating that the
    /// header's level dimensions do not overflow the tile-count range and that every
    /// supplied tile coordinate is in range for the header.
    ///
    /// This is the untrusted-input entry point the header/directory checks flow
    /// through: a header whose level dimensions overflow `u64` is rejected here rather
    /// than allocating from the claim.
    ///
    /// # Errors
    ///
    /// [`TileSourceError::Malformed`] if the header dimensions overflow, or
    /// [`TileSourceError::OutOfRange`] if a tile coordinate is outside the header grid.
    pub fn new(
        header: RtlaHeader,
        tiles: HashMap<TileCoord, Vec<u8>>,
    ) -> Result<Self, TileSourceError> {
        expected_tile_count(&header).ok_or_else(|| {
            TileSourceError::Malformed(
                "header level dimensions overflow the tile-count range".to_owned(),
            )
        })?;
        for coord in tiles.keys() {
            tile_directory_index(&header, *coord)?;
        }
        Ok(Self {
            header,
            tiles,
            empty_tile: serialize_tile(&TilePayload::default()),
        })
    }
}

impl TileSource for MemTileSource {
    async fn header(&self) -> Result<RtlaHeader, TileSourceError> {
        Ok(self.header.clone())
    }

    async fn tile_bytes(&self, coord: TileCoord) -> Result<Vec<u8>, TileSourceError> {
        // Range-check against the header; an out-of-range coordinate errors rather than
        // silently returning an empty tile.
        tile_directory_index(&self.header, coord)?;
        Ok(self
            .tiles
            .get(&coord)
            .cloned()
            .unwrap_or_else(|| self.empty_tile.clone()))
    }
}

// ---------------------------------------------------------------------------
// MmapTileSource: native memory-mapped source.
// ---------------------------------------------------------------------------

/// A native [`TileSource`] over a memory-mapped `.rtla` file.
///
/// [`open`](MmapTileSource::open) maps the file, validates and reads the header and
/// directory once, and checks them for consistency; [`tile_bytes`](TileSource::tile_bytes)
/// slices the mapped range for a tile and validates it as a [`TilePayload`] archive
/// before returning it. The map reuses the [`crate::streaming`] discipline: the one
/// documented `unsafe` is confined to [`open`](MmapTileSource::open), and every block
/// read from the map is validated with `rkyv` `access`.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct MmapTileSource {
    // Field order matters for drop order: unmap before closing the file (mirrors
    // `StreamingIndex`). `mmap` conceptually borrows from `_file`, kept alive for the
    // map's lifetime.
    mmap: memmap2::Mmap,
    _file: std::fs::File,
    header: RtlaHeader,
    directory: Vec<TileDirEntry>,
}

#[cfg(not(target_arch = "wasm32"))]
impl MmapTileSource {
    /// Opens and memory-maps the `.rtla` file at `path`, validating its preamble,
    /// header, and directory.
    ///
    /// # Errors
    ///
    /// [`TileSourceError::Transport`] if the file cannot be opened or mapped;
    /// [`TileSourceError::Malformed`] if the preamble, header, or directory is invalid,
    /// truncated, or internally inconsistent.
    pub fn open(path: &Path) -> Result<Self, TileSourceError> {
        let file = std::fs::File::open(path)
            .map_err(|e| TileSourceError::Transport(format!("open {}: {e}", path.display())))?;

        // SAFETY: `Mmap::map` is unsafe because the mapped bytes are backed by the file
        // on disk, not by Rust's allocator: a different process truncating or writing
        // the file while the map is live is undefined behaviour (the standard mmap
        // contract, unavoidable for any memory-mapped file). We uphold the safe side
        // here exactly as `StreamingIndex::open` does: we own `file` (opened read-only
        // just above), store it in the returned value so it outlives `mmap`, and never
        // expose a way to resize or write it. Every block read from the map below is
        // validated with `rkyv` `access`, so a corrupt or truncated-at-open file is
        // rejected as an error rather than read unchecked.
        #[allow(unsafe_code)]
        let mmap = unsafe {
            memmap2::Mmap::map(&file)
                .map_err(|e| TileSourceError::Transport(format!("mmap {}: {e}", path.display())))?
        };

        let map_len = mmap.len() as u64;
        let preamble = Preamble::parse(&mmap)?;
        let header_end = preamble.header_end();
        let directory_end = preamble.directory_end();
        if preamble.header_len.saturating_add(preamble.directory_len) > MAX_HEADER_DIR_BYTES {
            return Err(TileSourceError::Malformed(
                "header and directory exceed the maximum readable size".to_owned(),
            ));
        }
        if directory_end > map_len {
            return Err(TileSourceError::Malformed(
                "archive truncated: header/directory extend past end of file".to_owned(),
            ));
        }

        let header =
            deserialize_header(&mmap[Preamble::header_start() as usize..header_end as usize])?;
        let directory = deserialize_directory(
            &mmap[preamble.directory_start() as usize..directory_end as usize],
        )?;
        check_header_directory_consistency(&header, directory.len())?;

        Ok(Self {
            mmap,
            _file: file,
            header,
            directory,
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl TileSource for MmapTileSource {
    async fn header(&self) -> Result<RtlaHeader, TileSourceError> {
        Ok(self.header.clone())
    }

    async fn tile_bytes(&self, coord: TileCoord) -> Result<Vec<u8>, TileSourceError> {
        let index = tile_directory_index(&self.header, coord)?;
        let entry = self
            .directory
            .get(index)
            .ok_or(TileSourceError::OutOfRange(coord))?;
        let start = usize::try_from(entry.offset)
            .map_err(|_| TileSourceError::Malformed("tile offset exceeds usize".to_owned()))?;
        let len = usize::try_from(entry.len)
            .map_err(|_| TileSourceError::Malformed("tile length exceeds usize".to_owned()))?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| TileSourceError::Malformed("tile range overflow".to_owned()))?;
        if end > self.mmap.len() {
            return Err(TileSourceError::Malformed(
                "tile range extends past end of file".to_owned(),
            ));
        }
        let bytes = self.mmap[start..end].to_vec();
        validate_tile(&bytes)?;
        Ok(bytes)
    }
}

// ---------------------------------------------------------------------------
// HttpRangeTileSource: wasm HTTP-Range source with LRU + OPFS caches.
// ---------------------------------------------------------------------------

/// A wasm [`TileSource`] that fetches the header and each tile over HTTP `Range`
/// requests, in front of an in-memory [`LruByteCache`] and an OPFS persistent cache.
///
/// [`open`](HttpRangeTileSource::open) reads the preamble with a ranged GET of the
/// leading bytes, then the header and directory with a second ranged GET, capturing the
/// response `ETag` so the OPFS cache key changes when the archive is rebuilt.
/// [`tile_bytes`](TileSource::tile_bytes) checks the in-memory cache, then OPFS, then
/// issues a `Range: bytes=offset-end` fetch, validating the tile and populating both
/// caches. The layout assumption is documented at the module level: a `Range` request
/// over a directory entry's `[offset, offset+len)` returns exactly that tile.
#[cfg(target_arch = "wasm32")]
pub struct HttpRangeTileSource {
    url: String,
    etag: Option<String>,
    header: RtlaHeader,
    directory: Vec<TileDirEntry>,
    cache: core::cell::RefCell<LruByteCache>,
}

#[cfg(target_arch = "wasm32")]
impl std::fmt::Debug for HttpRangeTileSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpRangeTileSource")
            .field("url", &self.url)
            .field("etag", &self.etag)
            .field("levels", &self.header.levels.len())
            .field("tiles", &self.directory.len())
            .finish_non_exhaustive()
    }
}

#[cfg(target_arch = "wasm32")]
impl HttpRangeTileSource {
    /// Opens the archive at `url`, fetching and validating its preamble, header, and
    /// directory, and sizing the in-memory tile cache to `cache_budget_bytes`.
    ///
    /// # Errors
    ///
    /// [`TileSourceError::Transport`] on a network/CORS/HTTP failure, or
    /// [`TileSourceError::Malformed`] if the fetched preamble, header, or directory is
    /// invalid, oversized, or inconsistent.
    pub async fn open(url: &str, cache_budget_bytes: usize) -> Result<Self, TileSourceError> {
        let (preamble_bytes, etag) = wasm_glue::fetch_range(url, 0, PREAMBLE_LEN - 1).await?;
        let preamble = Preamble::parse(&preamble_bytes)?;
        if preamble.header_len.saturating_add(preamble.directory_len) > MAX_HEADER_DIR_BYTES {
            return Err(TileSourceError::Malformed(
                "header and directory exceed the maximum readable size".to_owned(),
            ));
        }

        let header_start = Preamble::header_start();
        let directory_end = preamble.directory_end();
        let (block, _) = wasm_glue::fetch_range(url, header_start, directory_end - 1).await?;

        let header_len = preamble.header_len as usize;
        if block.len() < (directory_end - header_start) as usize {
            return Err(TileSourceError::Malformed(
                "server returned fewer bytes than the header/directory range".to_owned(),
            ));
        }
        let header = deserialize_header(&block[..header_len])?;
        let dir_start = (preamble.directory_start() - header_start) as usize;
        let dir_end = (directory_end - header_start) as usize;
        let directory = deserialize_directory(&block[dir_start..dir_end])?;
        check_header_directory_consistency(&header, directory.len())?;

        Ok(Self {
            url: url.to_owned(),
            etag,
            header,
            directory,
            cache: core::cell::RefCell::new(LruByteCache::with_budget(cache_budget_bytes)),
        })
    }
}

#[cfg(target_arch = "wasm32")]
impl TileSource for HttpRangeTileSource {
    async fn header(&self) -> Result<RtlaHeader, TileSourceError> {
        Ok(self.header.clone())
    }

    async fn tile_bytes(&self, coord: TileCoord) -> Result<Vec<u8>, TileSourceError> {
        let index = tile_directory_index(&self.header, coord)?;
        let entry = *self
            .directory
            .get(index)
            .ok_or(TileSourceError::OutOfRange(coord))?;

        // 1. In-memory LRU.
        if let Some(bytes) = self.cache.borrow_mut().get(coord) {
            return Ok(bytes);
        }

        let archive_key = opfs_cache_key(&self.url, self.etag.as_deref());

        // 2. OPFS persistent cache.
        if let Some(bytes) = wasm_glue::opfs_read_tile(&archive_key, coord).await
            && validate_tile(&bytes).is_ok()
        {
            self.cache.borrow_mut().insert(coord, bytes.clone());
            return Ok(bytes);
        }

        // 3. Network Range fetch. `[offset, offset+len)` is inclusive `offset..=end`.
        let end = entry
            .offset
            .checked_add(entry.len)
            .ok_or_else(|| TileSourceError::Malformed("tile range overflow".to_owned()))?;
        let (bytes, _) = wasm_glue::fetch_range(&self.url, entry.offset, end - 1).await?;
        validate_tile(&bytes)?;

        wasm_glue::opfs_write_tile(&archive_key, coord, &bytes).await;
        self.cache.borrow_mut().insert(coord, bytes.clone());
        Ok(bytes)
    }
}

/// The DOM-bound half of the wasm source: the `fetch` Range request and the OPFS
/// read/write. These cannot run in a headless unit test, so they are kept thin over
/// the pure logic above; their end-to-end behaviour (a ranged GET streams a tile, an
/// OPFS revisit is instant) is lane 2E/8's browser pass. See the module's honest gaps.
#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    use super::{TileCoord, TileSourceError, opfs_tile_file_name};
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_futures::JsFuture;

    /// Renders a `JsValue` error into a short string for a [`TileSourceError`].
    fn describe(value: &wasm_bindgen::JsValue) -> String {
        value
            .as_string()
            .or_else(|| {
                value
                    .dyn_ref::<js_sys::Error>()
                    .map(|e| String::from(e.message()))
            })
            .unwrap_or_else(|| "network error".to_owned())
    }

    /// Fetches `url` over an HTTP `Range: bytes=start-end` request (inclusive bounds),
    /// returning the body bytes and the response `ETag` if present.
    ///
    /// A non-2xx status or a network/CORS failure becomes a
    /// [`TileSourceError::Transport`]. The server is assumed to honour `Range` and
    /// reply `206` with exactly the requested bytes (documented at the module level).
    pub async fn fetch_range(
        url: &str,
        start: u64,
        end: u64,
    ) -> Result<(Vec<u8>, Option<String>), TileSourceError> {
        let window = web_sys::window()
            .ok_or_else(|| TileSourceError::Transport("no browser window".to_owned()))?;
        let request = web_sys::Request::new_with_str(url)
            .map_err(|e| TileSourceError::Transport(describe(&e)))?;
        let range = format!("bytes={start}-{end}");
        request
            .headers()
            .set("Range", &range)
            .map_err(|e| TileSourceError::Transport(describe(&e)))?;

        let resp_value = JsFuture::from(window.fetch_with_request(&request))
            .await
            .map_err(|e| {
                TileSourceError::Transport(format!(
                    "fetch {url} [{range}]: {}. The server may be unreachable or may not \
                     allow cross-origin range requests (CORS).",
                    describe(&e)
                ))
            })?;
        let response: web_sys::Response = resp_value
            .dyn_into()
            .map_err(|_| TileSourceError::Transport("unexpected fetch result".to_owned()))?;
        if !response.ok() {
            return Err(TileSourceError::Transport(format!(
                "fetch {url} [{range}]: server responded {} {}",
                response.status(),
                response.status_text()
            )));
        }
        let etag = response.headers().get("ETag").ok().flatten();
        let buf_promise = response
            .array_buffer()
            .map_err(|e| TileSourceError::Transport(describe(&e)))?;
        let buf = JsFuture::from(buf_promise)
            .await
            .map_err(|e| TileSourceError::Transport(describe(&e)))?;
        let array = js_sys::Uint8Array::new(&buf);
        Ok((array.to_vec(), etag))
    }

    /// Opens (creating if asked) the OPFS directory for an archive, or `None` on any
    /// failure. OPFS is best-effort: a browser without it, or a non-worker context,
    /// simply falls back to the network.
    async fn opfs_dir(
        archive_key: &str,
        create: bool,
    ) -> Option<web_sys::FileSystemDirectoryHandle> {
        let window = web_sys::window()?;
        let storage = window.navigator().storage();
        let root_value = JsFuture::from(storage.get_directory()).await.ok()?;
        let root: web_sys::FileSystemDirectoryHandle = root_value.dyn_into().ok()?;
        let options = web_sys::FileSystemGetDirectoryOptions::new();
        options.set_create(create);
        let dir_value =
            JsFuture::from(root.get_directory_handle_with_options(archive_key, &options))
                .await
                .ok()?;
        dir_value.dyn_into().ok()
    }

    /// Reads a cached tile from OPFS, or `None` if it is not cached or OPFS is
    /// unavailable.
    pub async fn opfs_read_tile(archive_key: &str, coord: TileCoord) -> Option<Vec<u8>> {
        let dir = opfs_dir(archive_key, false).await?;
        let file_value = JsFuture::from(dir.get_file_handle(&opfs_tile_file_name(coord)))
            .await
            .ok()?;
        let file: web_sys::FileSystemFileHandle = file_value.dyn_into().ok()?;
        let sync_value = JsFuture::from(file.create_sync_access_handle())
            .await
            .ok()?;
        let sync: web_sys::FileSystemSyncAccessHandle = sync_value.dyn_into().ok()?;
        let size = sync.get_size().ok()? as usize;
        let mut buf = vec![0u8; size];
        let read = sync.read_with_u8_array(&mut buf).ok();
        sync.close();
        let read = read? as usize;
        buf.truncate(read);
        Some(buf)
    }

    /// Writes a tile to the OPFS cache, best-effort: any failure is swallowed since the
    /// tile is already in hand and losing the persistent copy only costs a re-fetch.
    pub async fn opfs_write_tile(archive_key: &str, coord: TileCoord, bytes: &[u8]) {
        let Some(dir) = opfs_dir(archive_key, true).await else {
            return;
        };
        let options = web_sys::FileSystemGetFileOptions::new();
        options.set_create(true);
        let Ok(file_value) =
            JsFuture::from(dir.get_file_handle_with_options(&opfs_tile_file_name(coord), &options))
                .await
        else {
            return;
        };
        let Ok(file) = file_value.dyn_into::<web_sys::FileSystemFileHandle>() else {
            return;
        };
        let Ok(sync_value) = JsFuture::from(file.create_sync_access_handle()).await else {
            return;
        };
        let Ok(sync) = sync_value.dyn_into::<web_sys::FileSystemSyncAccessHandle>() else {
            return;
        };
        let _ = sync.write_with_u8_array(bytes);
        let _ = sync.flush();
        sync.close();
    }
}

// ---------------------------------------------------------------------------
// Viewport query layer (all targets).
// ---------------------------------------------------------------------------

/// Streams the records visible in `viewport` from `source`, resolving against the
/// archive's finest level (ADR 0062).
///
/// Given `header` (already fetched via [`TileSource::header`]) and a `viewport`
/// rectangle, this resolves the finest level's tiles the viewport overlaps, fetches
/// each through `source`, validates it, and returns every [`TileRecord`] whose
/// rectangle actually intersects `viewport`, deduplicated (a record placed in several
/// tiles is returned once). The result set equals an exact in-RAM spatial query over
/// the same records, which the headline proptest proves against the R-tree.
///
/// # Errors
///
/// Propagates any [`TileSourceError`] from fetching or validating a tile.
pub async fn query_viewport<S: TileSource>(
    source: &S,
    header: &RtlaHeader,
    viewport: Rect,
) -> Result<Vec<TileRecord>, TileSourceError> {
    let Some((level, cols, rows)) = finest_level(header) else {
        return Ok(Vec::new());
    };
    if viewport.is_empty() {
        return Ok(Vec::new());
    }
    let world = header.world_rect();
    if !viewport.intersects(&world) {
        return Ok(Vec::new());
    }

    let (tx0, tx1) = axis_tile_range(
        world.min.x,
        world.max.x,
        viewport.min.x,
        viewport.max.x,
        cols,
    );
    let (ty0, ty1) = axis_tile_range(
        world.min.y,
        world.max.y,
        viewport.min.y,
        viewport.max.y,
        rows,
    );

    let mut seen: std::collections::HashSet<(u16, u16, i32, i32, i32, i32)> =
        std::collections::HashSet::new();
    let mut out = Vec::new();
    for row in ty0..=ty1 {
        for col in tx0..=tx1 {
            let coord = TileCoord { level, col, row };
            let bytes = source.tile_bytes(coord).await?;
            let payload = rkyv::from_bytes::<TilePayload, rancor::Error>(&bytes)
                .map_err(|e| TileSourceError::Malformed(format!("tile archive invalid: {e}")))?;
            for record in payload.records {
                let r = record.rect.to_rect();
                if r.intersects(&viewport) {
                    let key = (
                        record.layer,
                        record.datatype,
                        record.rect.min_x,
                        record.rect.min_y,
                        record.rect.max_x,
                        record.rect.max_y,
                    );
                    if seen.insert(key) {
                        out.push(record);
                    }
                }
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::LevelDims;
    use crate::streaming::ArchivableRect;
    use reticle_geometry::Point;

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
        Rect::new(Point::new(x0, y0), Point::new(x1, y1))
    }

    fn record(layer: u16, r: Rect) -> TileRecord {
        TileRecord {
            layer,
            datatype: 0,
            rect: ArchivableRect::from_rect(r),
        }
    }

    fn coord(level: u32, col: u32, row: u32) -> TileCoord {
        TileCoord { level, col, row }
    }

    // ---- framing helpers -------------------------------------------------

    #[test]
    fn align_up_rounds_to_block_boundary() {
        assert_eq!(align_up(0, 16), 0);
        assert_eq!(align_up(1, 16), 16);
        assert_eq!(align_up(16, 16), 16);
        assert_eq!(align_up(17, 16), 32);
    }

    #[test]
    fn preamble_round_trips_and_rejects_bad_magic_and_version() {
        let p = Preamble {
            header_len: 100,
            directory_len: 48,
        };
        let bytes = p.encode();
        assert_eq!(Preamble::parse(&bytes).unwrap(), p);

        let mut bad_magic = bytes;
        bad_magic[0] = b'X';
        assert!(matches!(
            Preamble::parse(&bad_magic),
            Err(TileSourceError::Malformed(_))
        ));

        let mut bad_version = bytes;
        bad_version[8] = 0xFF;
        assert!(matches!(
            Preamble::parse(&bad_version),
            Err(TileSourceError::Malformed(_))
        ));

        assert!(matches!(
            Preamble::parse(&bytes[..16]),
            Err(TileSourceError::Malformed(_))
        ));
    }

    // ---- directory index & untrusted bounds ------------------------------

    fn header_with_levels(levels: Vec<LevelDims>) -> RtlaHeader {
        RtlaHeader {
            magic: RTLA_MAGIC,
            version: RTLA_VERSION,
            world: ArchivableRect::from_rect(rect(0, 0, 1000, 1000)),
            dbu_per_micron: 1000,
            levels,
        }
    }

    #[test]
    fn tile_directory_index_is_level_major_row_major() {
        let header = header_with_levels(vec![
            LevelDims { cols: 1, rows: 1 },
            LevelDims { cols: 2, rows: 2 },
        ]);
        // Level 0: one tile at index 0.
        assert_eq!(tile_directory_index(&header, coord(0, 0, 0)).unwrap(), 0);
        // Level 1: 4 tiles, base 1, row-major.
        assert_eq!(tile_directory_index(&header, coord(1, 0, 0)).unwrap(), 1);
        assert_eq!(tile_directory_index(&header, coord(1, 1, 0)).unwrap(), 2);
        assert_eq!(tile_directory_index(&header, coord(1, 0, 1)).unwrap(), 3);
        assert_eq!(tile_directory_index(&header, coord(1, 1, 1)).unwrap(), 4);
    }

    #[test]
    fn tile_directory_index_rejects_out_of_range() {
        let header = header_with_levels(vec![LevelDims { cols: 2, rows: 2 }]);
        assert!(matches!(
            tile_directory_index(&header, coord(0, 2, 0)),
            Err(TileSourceError::OutOfRange(_))
        ));
        assert!(matches!(
            tile_directory_index(&header, coord(1, 0, 0)),
            Err(TileSourceError::OutOfRange(_))
        ));
    }

    #[test]
    fn header_claiming_billions_of_tiles_errors_not_ooms() {
        // Way 1 of the untrusted two-way check: a header whose level dimensions imply
        // billions of tiles behind a tiny directory is rejected as inconsistent, with
        // bounded work and no allocation from the claim.
        let header = header_with_levels(vec![LevelDims {
            cols: 60_000,
            rows: 60_000,
        }]);
        // 3.6e9 tiles claimed; a real directory here would have 1 entry.
        let err = check_header_directory_consistency(&header, 1).unwrap_err();
        assert!(matches!(err, TileSourceError::Malformed(_)));

        // The count is summed without overflow panic even at the u32::MAX extreme.
        let extreme = header_with_levels(vec![LevelDims {
            cols: u32::MAX,
            rows: u32::MAX,
        }]);
        assert!(expected_tile_count(&extreme).is_some());
        assert!(matches!(
            check_header_directory_consistency(&extreme, 0),
            Err(TileSourceError::Malformed(_))
        ));
    }

    // ---- LRU cache -------------------------------------------------------

    #[test]
    fn lru_evicts_least_recently_used_first() {
        let mut cache = LruByteCache::with_budget(20);
        cache.insert(coord(0, 0, 0), vec![0u8; 10]);
        cache.insert(coord(0, 1, 0), vec![0u8; 10]);
        assert_eq!(cache.used_bytes(), 20);
        assert_eq!(cache.len(), 2);

        // Touch tile (0,0,0) so it is most-recently-used.
        assert!(cache.get(coord(0, 0, 0)).is_some());

        // Inserting a third 10-byte tile evicts the LRU, which is now (0,1,0).
        cache.insert(coord(0, 2, 0), vec![0u8; 10]);
        assert_eq!(cache.used_bytes(), 20);
        assert!(cache.get(coord(0, 1, 0)).is_none(), "LRU tile was evicted");
        assert!(cache.get(coord(0, 0, 0)).is_some());
        assert!(cache.get(coord(0, 2, 0)).is_some());
    }

    #[test]
    fn lru_replacing_a_key_updates_used_bytes() {
        let mut cache = LruByteCache::with_budget(100);
        cache.insert(coord(0, 0, 0), vec![0u8; 10]);
        cache.insert(coord(0, 0, 0), vec![0u8; 30]);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.used_bytes(), 30);
    }

    #[test]
    fn lru_item_larger_than_budget_is_not_retained() {
        let mut cache = LruByteCache::with_budget(16);
        cache.insert(coord(0, 0, 0), vec![0u8; 64]);
        assert!(
            cache.is_empty(),
            "an over-budget tile is evicted immediately"
        );
        assert_eq!(cache.used_bytes(), 0);
    }

    #[test]
    fn lru_zero_budget_caches_nothing() {
        let mut cache = LruByteCache::with_budget(0);
        cache.insert(coord(0, 0, 0), vec![0u8; 1]);
        assert!(cache.is_empty());
    }

    // ---- OPFS cache key --------------------------------------------------

    #[test]
    fn opfs_cache_key_is_stable_and_etag_sensitive() {
        let base = opfs_cache_key("https://h/chip.rtla", Some("v1"));
        let same = opfs_cache_key("https://h/chip.rtla", Some("v1"));
        assert_eq!(base, same, "same url+etag yields the same key");

        let new_etag = opfs_cache_key("https://h/chip.rtla", Some("v2"));
        assert_ne!(
            base, new_etag,
            "a rebuilt archive (new etag) gets a fresh key"
        );

        let other_url = opfs_cache_key("https://h/other.rtla", Some("v1"));
        assert_ne!(base, other_url, "a different url gets a different key");

        let no_etag = opfs_cache_key("https://h/chip.rtla", None);
        assert_ne!(base, no_etag, "presence of an etag changes the key");
        assert!(base.starts_with("rtla-"));
    }

    // ---- MemTileSource + query -------------------------------------------

    #[test]
    fn mem_source_serves_header_and_tiles() {
        let world = rect(0, 0, 100, 100);
        let levels = [LevelDims { cols: 2, rows: 2 }];
        let recs = [record(1, rect(10, 10, 20, 20))];
        let source = MemTileSource::from_records(world, 1000, &levels, &recs);

        let header = pollster::block_on(source.header()).unwrap();
        assert_eq!(header.level_count(), 1);
        // An in-range but empty tile returns a valid empty payload.
        let bytes = pollster::block_on(source.tile_bytes(coord(0, 1, 1))).unwrap();
        validate_tile(&bytes).unwrap();
        // An out-of-range tile errors.
        assert!(matches!(
            pollster::block_on(source.tile_bytes(coord(0, 2, 0))),
            Err(TileSourceError::OutOfRange(_))
        ));
    }

    #[test]
    fn query_viewport_matches_a_hand_checked_case() {
        let world = rect(0, 0, 100, 100);
        let levels = [LevelDims { cols: 4, rows: 4 }];
        let recs = [
            record(1, rect(5, 5, 15, 15)),   // near origin
            record(2, rect(80, 80, 95, 95)), // far corner
            record(3, rect(45, 45, 55, 55)), // center, spans tile boundary
        ];
        let source = MemTileSource::from_records(world, 1000, &levels, &recs);
        let header = pollster::block_on(source.header()).unwrap();

        // A viewport around the origin sees only record 1.
        let got = pollster::block_on(query_viewport(&source, &header, rect(0, 0, 20, 20))).unwrap();
        let layers: Vec<u16> = got.iter().map(|r| r.layer).collect();
        assert_eq!(layers, vec![1]);

        // A viewport over the center sees record 3 once, despite it spanning tiles.
        let got =
            pollster::block_on(query_viewport(&source, &header, rect(40, 40, 60, 60))).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].layer, 3);

        // A whole-world viewport sees all three, deduplicated.
        let got = pollster::block_on(query_viewport(&source, &header, world)).unwrap();
        assert_eq!(got.len(), 3);
    }

    #[test]
    fn mem_source_new_rejects_out_of_range_tiles() {
        let header = header_with_levels(vec![LevelDims { cols: 2, rows: 2 }]);
        let mut tiles = HashMap::new();
        tiles.insert(coord(0, 5, 0), serialize_tile(&TilePayload::default()));
        assert!(matches!(
            MemTileSource::new(header, tiles),
            Err(TileSourceError::OutOfRange(_))
        ));
    }

    // ---- MmapTileSource round-trip and rejection -------------------------

    #[cfg(not(target_arch = "wasm32"))]
    mod mmap {
        use super::*;

        /// Lays out a full `.rtla` byte image from a header and a tile map, filling the
        /// directory with absolute 16-aligned offsets. Test scaffolding for the mmap
        /// source (the real external builder is lane 2A); it packs exactly the frozen
        /// framing this module reads.
        fn build_image(header: &RtlaHeader, tiles: &HashMap<TileCoord, Vec<u8>>) -> Vec<u8> {
            let header_bytes = rkyv::to_bytes::<rancor::Error>(header).unwrap().to_vec();

            // Directory in level-major, row-major order; every in-range tile gets an
            // entry (empty tiles use an archived empty payload).
            let empty = serialize_tile(&TilePayload::default());
            let mut ordered: Vec<Vec<u8>> = Vec::new();
            for (level, dims) in header.levels.iter().enumerate() {
                for row in 0..dims.rows {
                    for col in 0..dims.cols {
                        let c = coord(level as u32, col, row);
                        ordered.push(tiles.get(&c).cloned().unwrap_or_else(|| empty.clone()));
                    }
                }
            }

            // The directory's archived length is fixed by its entry count (each entry is
            // two fixed-size u64s), so we can size it from a placeholder before we know
            // the tile offsets, then place tiles at 16-aligned offsets after it.
            let mut directory = Vec::with_capacity(ordered.len());
            let placeholder: Vec<TileDirEntry> = ordered
                .iter()
                .map(|_| TileDirEntry { offset: 0, len: 0 })
                .collect();
            let dir_len = rkyv::to_bytes::<rancor::Error>(&placeholder).unwrap().len() as u64;

            let preamble = Preamble {
                header_len: header_bytes.len() as u64,
                directory_len: dir_len,
            };
            let mut cursor = preamble.directory_end();
            for tile in &ordered {
                cursor = align_up(cursor, BLOCK_ALIGN);
                directory.push(TileDirEntry {
                    offset: cursor,
                    len: tile.len() as u64,
                });
                cursor += tile.len() as u64;
            }
            let dir_bytes = rkyv::to_bytes::<rancor::Error>(&directory)
                .unwrap()
                .to_vec();
            assert_eq!(dir_bytes.len() as u64, dir_len, "directory length stable");

            // Second pass: assemble the file.
            let mut out = Vec::with_capacity(cursor as usize);
            out.extend_from_slice(&preamble.encode());
            out.extend_from_slice(&header_bytes);
            while out.len() as u64 != preamble.directory_start() {
                out.push(0);
            }
            out.extend_from_slice(&dir_bytes);
            for (entry, tile) in directory.iter().zip(&ordered) {
                while (out.len() as u64) < entry.offset {
                    out.push(0);
                }
                out.extend_from_slice(tile);
            }
            out
        }

        struct TempFile(std::path::PathBuf);
        impl TempFile {
            fn new(bytes: &[u8]) -> Self {
                static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let mut p = std::env::temp_dir();
                p.push(format!(
                    "reticle_rtla_test_{}_{}.rtla",
                    std::process::id(),
                    NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                ));
                std::fs::write(&p, bytes).unwrap();
                Self(p)
            }
        }
        impl Drop for TempFile {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }

        #[test]
        fn mmap_round_trips_header_and_tiles() {
            let world = rect(0, 0, 100, 100);
            let levels = [
                LevelDims { cols: 1, rows: 1 },
                LevelDims { cols: 2, rows: 2 },
            ];
            let recs = [
                record(1, rect(5, 5, 15, 15)),
                record(2, rect(60, 60, 80, 80)),
            ];
            let (header, tiles) = pack_records(world, 1000, &levels, &recs);
            let image = build_image(&header, &tiles);
            let file = TempFile::new(&image);

            let source = MmapTileSource::open(&file.0).unwrap();
            let got_header = pollster::block_on(source.header()).unwrap();
            assert_eq!(got_header, header);

            // The finest-level viewport query over the mmap source matches the records.
            let got = pollster::block_on(query_viewport(&source, &got_header, world)).unwrap();
            assert_eq!(got.len(), 2);

            let near = pollster::block_on(query_viewport(&source, &got_header, rect(0, 0, 20, 20)))
                .unwrap();
            assert_eq!(near.len(), 1);
            assert_eq!(near[0].layer, 1);
        }

        #[test]
        fn mmap_rejects_a_non_archive_file() {
            let file = TempFile::new(b"definitely not an rtla archive at all, no sir");
            assert!(MmapTileSource::open(&file.0).is_err());
        }

        #[test]
        fn mmap_rejects_a_billions_tile_header_without_ooming() {
            // Way 2 of the untrusted two-way check, through the real open path: a header
            // that claims billions of tiles but ships a 1-entry directory is rejected as
            // inconsistent, in bounded time and memory.
            let header = header_with_levels(vec![LevelDims {
                cols: 60_000,
                rows: 60_000,
            }]);
            // Build an image with a directory of exactly one entry that lies about the
            // header's implied tile count.
            let header_bytes = rkyv::to_bytes::<rancor::Error>(&header).unwrap().to_vec();
            let directory = vec![TileDirEntry { offset: 0, len: 0 }];
            let dir_bytes = rkyv::to_bytes::<rancor::Error>(&directory)
                .unwrap()
                .to_vec();
            let preamble = Preamble {
                header_len: header_bytes.len() as u64,
                directory_len: dir_bytes.len() as u64,
            };
            let mut image = Vec::new();
            image.extend_from_slice(&preamble.encode());
            image.extend_from_slice(&header_bytes);
            while image.len() as u64 != preamble.directory_start() {
                image.push(0);
            }
            image.extend_from_slice(&dir_bytes);
            let file = TempFile::new(&image);

            let err = MmapTileSource::open(&file.0).unwrap_err();
            assert!(matches!(err, TileSourceError::Malformed(_)));
        }

        #[test]
        fn mmap_rejects_a_truncated_directory() {
            // A preamble that claims a longer directory than the file holds must error,
            // never read past the map.
            let header = header_with_levels(vec![LevelDims { cols: 1, rows: 1 }]);
            let header_bytes = rkyv::to_bytes::<rancor::Error>(&header).unwrap().to_vec();
            let preamble = Preamble {
                header_len: header_bytes.len() as u64,
                directory_len: 4096, // far more than the file will contain
            };
            let mut image = Vec::new();
            image.extend_from_slice(&preamble.encode());
            image.extend_from_slice(&header_bytes);
            let file = TempFile::new(&image);
            assert!(matches!(
                MmapTileSource::open(&file.0),
                Err(TileSourceError::Malformed(_))
            ));
        }
    }
}
