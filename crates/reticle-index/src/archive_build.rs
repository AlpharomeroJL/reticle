//! The two-pass external `.rtla` archive builder (Wave 2 lane 2A).
//!
//! The builder is *external* (bounded memory): the ADR 0016 in-RAM builder holds the
//! whole archive at once, which does not scale to the multi-gigabyte dies this format
//! exists to stream. This one streams renderable records once, spilling them to disk,
//! and never holds more than one sorted chunk (pass 1) or one tile's worth of records
//! (pass 2) in memory, so peak RSS is bounded no matter how large the layout is.
//!
//! # Two passes
//!
//! 1. **Spill sorted runs.** Stream every record once. For each record and each
//!    pyramid level, compute the tiles it lands in (the `crate::lod` overlap
//!    assignment, reimplemented here so the streaming builder shares its tile math),
//!    and emit one fixed-size `(global_tile_index, record)` entry per placement into
//!    an in-memory buffer. When the buffer fills, sort it by tile index and flush it
//!    to a run file, then clear it. Peak memory is one buffer (a tunable chunk).
//! 2. **Merge and emit.** `k`-way merge the sorted runs into one globally
//!    tile-ordered stream, and walk the tile grid in directory order (level-major,
//!    row-major within a level). Each tile's records are the contiguous run of merged
//!    entries carrying its index; assemble them into a [`TilePayload`], serialize it,
//!    and append it. Peak memory is one tile's records plus the `k` merge heads.
//!
//! # On-disk framing (ADR 0068)
//!
//! [`crate::archive`] freezes the rkyv record *types* and says tiles are
//! byte-contiguous with `(offset, len)` from the start of the file, but does not say
//! how a reader locates the header and directory blocks, which are variable length.
//! This builder writes a fixed 32-byte little-endian preamble of four `u64`s
//! (`header_off`, `header_len`, `dir_off`, `dir_len`) ahead of the rkyv
//! [`RtlaHeader`] block and the rkyv `Vec<TileDirEntry>` directory block, then the
//! tiles. Every block starts on a `BLOCK_ALIGN`-byte boundary so a reader can
//! `rkyv::access` it in place from a memory map. ADR 0068 documents this so lane 2B's
//! reader is written to the same layout.
//!
//! # Coarse-level decimation
//!
//! The finest level (the last entry of [`RtlaHeader::levels`]) holds exact geometry:
//! every record reaches it. Coarser levels are "paint-only approximations" (per ADR
//! 0062), so this builder subsamples them with a per-level stride, keeping their
//! per-tile density roughly constant with the finest level. That bounds memory
//! uniformly and keeps the spill near-linear in the record count instead of
//! multiplying it by the level count. A per-tile safety cap (`MAX_TILE_RECORDS`)
//! bounds memory even under a pathological distribution.
//!
//! Every count read from the header is untrusted; the builder never reserves capacity
//! from one beyond what it can fill (the OASIS OOM lesson, commit 1b1b56b).

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use rkyv::rancor;

use crate::archive::{RTLA_MAGIC, RTLA_VERSION, RtlaHeader, TileDirEntry, TilePayload, TileRecord};

/// Byte alignment every block (header, directory, each tile) is padded up to, so a
/// reader can `rkyv::access` it in place over a memory map. Covers the 8-byte
/// alignment of the `i64`/`u64` fields in the archived types with margin.
const BLOCK_ALIGN: u64 = 16;

/// Length of the fixed little-endian preamble: four `u64`s (`header_off`,
/// `header_len`, `dir_off`, `dir_len`).
const PREAMBLE_LEN: u64 = 32;

/// Number of `(tile_index, record)` entries buffered in memory before a sorted run is
/// spilled. Each entry is [`ENTRY_LEN`] bytes, so this bounds pass-1 memory to about
/// `CHUNK_ENTRIES * ENTRY_LEN` (~112 MiB).
const CHUNK_ENTRIES: usize = 4_000_000;

/// The maximum number of records materialized into a single tile. A tile that would
/// exceed this is decimated (extra records dropped), bounding pass-2 memory to about
/// `MAX_TILE_RECORDS * size_of::<TileRecord>()` regardless of the input distribution.
const MAX_TILE_RECORDS: usize = 4_000_000;

/// Fixed size of one spilled entry: an 8-byte tile index, then the record's
/// `layer`/`datatype` (`u16` each) and four `i32` rectangle corners.
const ENTRY_LEN: usize = 8 + 2 + 2 + 16;

/// Why building a `.rtla` archive failed.
#[derive(Debug)]
pub enum BuildError {
    /// The supplied header is not a valid `.rtla` v1 header (bad magic, unsupported
    /// version, a degenerate world box, or a level with a zero grid dimension).
    InvalidHeader(&'static str),
    /// An I/O error while reading input or writing spill/output files.
    Io(std::io::Error),
    /// An rkyv serialization error while archiving the header, directory, or a tile.
    Serialize(rancor::Error),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidHeader(why) => write!(f, "rtla builder: invalid header: {why}"),
            Self::Io(e) => write!(f, "rtla builder I/O error: {e}"),
            Self::Serialize(e) => write!(f, "rtla builder serialization error: {e}"),
        }
    }
}

impl std::error::Error for BuildError {}

impl From<std::io::Error> for BuildError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<rancor::Error> for BuildError {
    fn from(e: rancor::Error) -> Self {
        Self::Serialize(e)
    }
}

/// One pyramid level's grid, plus the base index its tiles occupy in the level-major
/// directory.
#[derive(Clone, Copy)]
struct Level {
    cols: u32,
    rows: u32,
    /// Directory index of this level's first tile.
    base: u64,
    /// Keep-every-`stride`th-record subsample factor for this (coarse) level; 1 at
    /// the finest level, so it stays exact.
    stride: u64,
}

/// Builds a `.rtla` archive at `out_path` from `header` and a stream of renderable
/// `records`, using bounded memory (see the [module docs](self)).
///
/// `records` is consumed once, lazily, so the caller can supply a generator that
/// never materializes the whole layout. The header's `world`, `dbu_per_micron`, and
/// per-level grid dimensions are taken as given; the tiles are filled from `records`.
///
/// # Errors
///
/// Returns [`BuildError::InvalidHeader`] if the header is not a valid `.rtla` v1
/// header, [`BuildError::Io`] on any spill/output I/O failure, or
/// [`BuildError::Serialize`] if rkyv fails to archive a block.
pub fn build_rtla<I>(header: &RtlaHeader, records: I, out_path: &Path) -> Result<(), BuildError>
where
    I: IntoIterator<Item = TileRecord>,
{
    let levels = validate_and_plan(header)?;
    let total_tiles: u64 = levels
        .iter()
        .map(|l| u64::from(l.cols) * u64::from(l.rows))
        .sum();
    let world = header.world.to_rect();

    // Pass 1: spill sorted runs to a scratch directory beside the output file.
    let scratch = Scratch::create(out_path)?;
    let runs = spill_sorted_runs(records, world, &levels, &scratch)?;

    // Compute the block layout. The directory's serialized length depends only on the
    // tile count (fixed-size entries), so a zeroed placeholder measures it exactly.
    let header_bytes = rkyv::to_bytes::<rancor::Error>(header)?;
    let header_off = PREAMBLE_LEN;
    let header_len = header_bytes.len() as u64;
    let dir_off = align_up(header_off + header_len, BLOCK_ALIGN);
    let placeholder_dir = vec![TileDirEntry { offset: 0, len: 0 }; total_tiles as usize];
    let dir_bytes = rkyv::to_bytes::<rancor::Error>(&placeholder_dir)?;
    let dir_len = dir_bytes.len() as u64;
    let data_start = align_up(dir_off + dir_len, BLOCK_ALIGN);

    // Pass 2: write the preamble/header/(placeholder)directory, then merge the runs
    // and append the tiles, recording each tile's absolute offset and length.
    let file = File::create(out_path)?;
    let mut out = BufWriter::new(file);
    write_preamble(&mut out, header_len, dir_len)?;
    let mut offset = header_off;
    out.write_all(&header_bytes)?;
    offset += header_len;
    pad_to(&mut out, &mut offset, dir_off)?;
    out.write_all(&dir_bytes)?; // placeholder, backpatched below
    offset += dir_len;
    pad_to(&mut out, &mut offset, data_start)?;

    let directory = merge_and_emit(&runs, total_tiles, &mut out, &mut offset)?;

    // Backpatch the directory now that every tile's offset/len is known.
    let real_dir_bytes = rkyv::to_bytes::<rancor::Error>(&directory)?;
    if real_dir_bytes.len() as u64 != dir_len {
        return Err(BuildError::InvalidHeader(
            "directory serialized to an unexpected length",
        ));
    }
    out.flush()?;
    let mut file = out
        .into_inner()
        .map_err(std::io::IntoInnerError::into_error)?;
    file.seek(SeekFrom::Start(dir_off))?;
    file.write_all(&real_dir_bytes)?;
    file.flush()?;

    scratch.cleanup();
    Ok(())
}

/// Builds a `.rtla` archive entirely in memory, returning its bytes.
///
/// This is the in-browser counterpart to [`build_rtla`] (lane v8-6c). On
/// `wasm32-unknown-unknown` there is no filesystem for the external builder to spill
/// sorted runs to or to seek-and-backpatch an output file against, so the streaming
/// disk path cannot run in a browser Web Worker. This variant does the same work with
/// one in-memory sort instead of an external merge sort, and assembles the archive into
/// a single `Vec<u8>` (which the worker then writes to OPFS).
///
/// It is a drop-in for [`build_rtla`]: for any input whose placements fit in a single
/// sort chunk (`CHUNK_ENTRIES`), it produces **byte-identical** output. The collection
/// order, the `sort_unstable_by_key(tile_index)`, the per-tile `MAX_TILE_RECORDS` cap,
/// and the framing (preamble, header, directory, aligned tiles) are all shared with the
/// external builder, so the frozen reader parses either identically. A test in
/// `tests/rtla_to_vec.rs` pins the byte equality.
///
/// The trade-off is that peak memory is the whole archive rather than one sort chunk, so
/// this is for the browser v1 scope (layouts that fit in memory); the native
/// `reticle convert` keeps the bounded-memory streaming builder for large dies.
///
/// # Errors
///
/// Returns [`BuildError::InvalidHeader`] if `header` is not a valid `.rtla` v1 header, or
/// [`BuildError::Serialize`] if rkyv fails to archive a block. It performs no I/O, so it
/// never returns [`BuildError::Io`].
pub fn build_rtla_to_vec<I>(header: &RtlaHeader, records: I) -> Result<Vec<u8>, BuildError>
where
    I: IntoIterator<Item = TileRecord>,
{
    let levels = validate_and_plan(header)?;
    let total_tiles: u64 = levels
        .iter()
        .map(|l| u64::from(l.cols) * u64::from(l.rows))
        .sum();
    let world = header.world.to_rect();

    // Pass 1: expand every record into per-level tile placements in the same order the
    // external builder emits them, then sort once by tile index (a single in-memory sort
    // stands in for spill-then-k-way-merge; for one chunk the byte result is identical).
    let mut entries: Vec<Entry> = Vec::new();
    for (ordinal, record) in records.into_iter().enumerate() {
        let rect = record.rect.to_rect();
        let ordinal = ordinal as u64;
        for level in &levels {
            if level.stride > 1 && !ordinal.is_multiple_of(level.stride) {
                continue;
            }
            let (tx0, tx1) =
                tile_span(world.min.x, world.max.x, rect.min.x, rect.max.x, level.cols);
            let (ty0, ty1) =
                tile_span(world.min.y, world.max.y, rect.min.y, rect.max.y, level.rows);
            for ty in ty0..=ty1 {
                for tx in tx0..=tx1 {
                    let tile_index =
                        level.base + u64::from(ty) * u64::from(level.cols) + u64::from(tx);
                    entries.push(Entry { tile_index, record });
                }
            }
        }
    }
    entries.sort_unstable_by_key(|e| e.tile_index);

    // Block layout, derived exactly as in `build_rtla`: the directory's serialized length
    // depends only on the (fixed-size) tile count, so a zeroed placeholder measures it.
    let header_bytes = rkyv::to_bytes::<rancor::Error>(header)?;
    let header_off = PREAMBLE_LEN;
    let header_len = header_bytes.len() as u64;
    let dir_off = align_up(header_off + header_len, BLOCK_ALIGN);
    let placeholder_dir = vec![TileDirEntry { offset: 0, len: 0 }; total_tiles as usize];
    let dir_len = rkyv::to_bytes::<rancor::Error>(&placeholder_dir)?.len() as u64;
    let data_start = align_up(dir_off + dir_len, BLOCK_ALIGN);

    // Pass 2: walk the sorted placements in one forward scan, emitting each tile in
    // directory order and recording its absolute (offset, len). Because the whole build
    // is in memory the directory is known before the tiles are written, so no seek or
    // backpatch is needed -- unlike the disk builder, which writes a placeholder first.
    let mut tiles_blob: Vec<u8> = Vec::new();
    let mut directory: Vec<TileDirEntry> = Vec::with_capacity(total_tiles as usize);
    let mut cursor = 0usize;
    let mut offset = data_start;
    for global_idx in 0..total_tiles {
        let mut tile_records: Vec<TileRecord> = Vec::new();
        while cursor < entries.len() && entries[cursor].tile_index == global_idx {
            if tile_records.len() < MAX_TILE_RECORDS {
                tile_records.push(entries[cursor].record);
            }
            cursor += 1;
        }
        let bytes = rkyv::to_bytes::<rancor::Error>(&TilePayload {
            records: tile_records,
        })?;
        let tile_off = align_up(offset, BLOCK_ALIGN);
        push_zeros(&mut tiles_blob, (tile_off - offset) as usize);
        tiles_blob.extend_from_slice(&bytes);
        directory.push(TileDirEntry {
            offset: tile_off,
            len: bytes.len() as u64,
        });
        offset = tile_off + bytes.len() as u64;
    }

    let dir_bytes = rkyv::to_bytes::<rancor::Error>(&directory)?;
    if dir_bytes.len() as u64 != dir_len {
        return Err(BuildError::InvalidHeader(
            "directory serialized to an unexpected length",
        ));
    }

    // Assemble the archive: preamble, header, directory, then the tile blob, each block
    // padded up to its BLOCK_ALIGN start, matching `build_rtla`'s on-disk framing.
    let mut out = Vec::with_capacity(data_start as usize + tiles_blob.len());
    let mut preamble = [0u8; PREAMBLE_LEN as usize];
    preamble[0..8].copy_from_slice(&RTLA_MAGIC);
    preamble[8..12].copy_from_slice(&RTLA_VERSION.to_le_bytes());
    preamble[16..24].copy_from_slice(&header_len.to_le_bytes());
    preamble[24..32].copy_from_slice(&dir_len.to_le_bytes());
    out.extend_from_slice(&preamble);
    out.extend_from_slice(&header_bytes);
    let pad_before_dir = (dir_off - out.len() as u64) as usize;
    push_zeros(&mut out, pad_before_dir);
    out.extend_from_slice(&dir_bytes);
    let pad_before_data = (data_start - out.len() as u64) as usize;
    push_zeros(&mut out, pad_before_data);
    out.extend_from_slice(&tiles_blob);
    Ok(out)
}

/// Appends `n` zero bytes to `buf`, the in-memory equivalent of [`pad_to`].
fn push_zeros(buf: &mut Vec<u8>, n: usize) {
    buf.resize(buf.len() + n, 0);
}

/// Validates the header and precomputes each level's directory base and coarse-level
/// subsample stride. The finest level is the last in [`RtlaHeader::levels`].
fn validate_and_plan(header: &RtlaHeader) -> Result<Vec<Level>, BuildError> {
    if header.magic != RTLA_MAGIC {
        return Err(BuildError::InvalidHeader("bad magic"));
    }
    if header.version != RTLA_VERSION {
        return Err(BuildError::InvalidHeader("unsupported version"));
    }
    let world = header.world.to_rect();
    if world.width() <= 0 || world.height() <= 0 {
        return Err(BuildError::InvalidHeader("world box has non-positive area"));
    }
    if header.levels.is_empty() {
        return Err(BuildError::InvalidHeader("header declares no levels"));
    }

    let finest_tiles = header
        .levels
        .last()
        .map_or(0, |l| u64::from(l.cols) * u64::from(l.rows));

    let mut levels = Vec::with_capacity(header.levels.len());
    let mut base = 0u64;
    for dims in &header.levels {
        if dims.cols == 0 || dims.rows == 0 {
            return Err(BuildError::InvalidHeader("level has a zero grid dimension"));
        }
        let tiles = u64::from(dims.cols) * u64::from(dims.rows);
        // Subsample coarse levels so their per-tile density matches the finest level.
        let stride = (finest_tiles / tiles).max(1);
        levels.push(Level {
            cols: dims.cols,
            rows: dims.rows,
            base,
            stride,
        });
        base += tiles;
    }
    Ok(levels)
}

/// Pass 1: stream `records`, expand each into per-level tile placements, and spill
/// sorted fixed-size run files. Returns the run-file paths.
fn spill_sorted_runs<I>(
    records: I,
    world: reticle_geometry::Rect,
    levels: &[Level],
    scratch: &Scratch,
) -> Result<Vec<PathBuf>, BuildError>
where
    I: IntoIterator<Item = TileRecord>,
{
    let mut buffer: Vec<Entry> = Vec::with_capacity(CHUNK_ENTRIES);
    let mut runs = Vec::new();

    for (ordinal, record) in records.into_iter().enumerate() {
        let rect = record.rect.to_rect();
        let ordinal = ordinal as u64;
        for level in levels {
            // Coarse-level decimation: keep only every `stride`th record.
            if level.stride > 1 && !ordinal.is_multiple_of(level.stride) {
                continue;
            }
            let (tx0, tx1) =
                tile_span(world.min.x, world.max.x, rect.min.x, rect.max.x, level.cols);
            let (ty0, ty1) =
                tile_span(world.min.y, world.max.y, rect.min.y, rect.max.y, level.rows);
            for ty in ty0..=ty1 {
                for tx in tx0..=tx1 {
                    let tile_index =
                        level.base + u64::from(ty) * u64::from(level.cols) + u64::from(tx);
                    buffer.push(Entry { tile_index, record });
                    if buffer.len() >= CHUNK_ENTRIES {
                        runs.push(flush_run(&mut buffer, scratch, runs.len())?);
                    }
                }
            }
        }
    }
    if !buffer.is_empty() {
        runs.push(flush_run(&mut buffer, scratch, runs.len())?);
    }
    Ok(runs)
}

/// Sorts `buffer` by tile index and writes it to a fresh run file, then clears it.
fn flush_run(
    buffer: &mut Vec<Entry>,
    scratch: &Scratch,
    index: usize,
) -> Result<PathBuf, BuildError> {
    buffer.sort_unstable_by_key(|e| e.tile_index);
    let path = scratch.dir.join(format!("run{index}.bin"));
    let mut writer = BufWriter::new(File::create(&path)?);
    let mut bytes = [0u8; ENTRY_LEN];
    for entry in buffer.iter() {
        entry.encode(&mut bytes);
        writer.write_all(&bytes)?;
    }
    writer.flush()?;
    buffer.clear();
    Ok(path)
}

/// Pass 2: `k`-way merge the sorted runs and emit tiles in directory order, returning
/// the completed directory.
fn merge_and_emit(
    runs: &[PathBuf],
    total_tiles: u64,
    out: &mut BufWriter<File>,
    offset: &mut u64,
) -> Result<Vec<TileDirEntry>, BuildError> {
    let mut merger = KMerge::open(runs)?;
    let mut lookahead = merger.next_entry()?;
    let mut directory = Vec::with_capacity(total_tiles as usize);

    for global_idx in 0..total_tiles {
        let mut records: Vec<TileRecord> = Vec::new();
        while let Some(entry) = lookahead {
            if entry.tile_index != global_idx {
                break;
            }
            if records.len() < MAX_TILE_RECORDS {
                records.push(entry.record);
            }
            lookahead = merger.next_entry()?;
        }

        let payload = TilePayload { records };
        let bytes = rkyv::to_bytes::<rancor::Error>(&payload)?;
        pad_to(out, offset, align_up(*offset, BLOCK_ALIGN))?;
        let tile_off = *offset;
        out.write_all(&bytes)?;
        *offset += bytes.len() as u64;
        directory.push(TileDirEntry {
            offset: tile_off,
            len: bytes.len() as u64,
        });
    }
    Ok(directory)
}

/// Writes the fixed 32-byte preamble in the canonical `.rtla` framing shared with
/// the reader ([`crate::tile_source`], ADR 0069): magic, version, reserved flags,
/// then the header and directory byte lengths. The header and directory *offsets*
/// are not stored because they are derivable (`header` at [`PREAMBLE_LEN`], the
/// directory at the next [`BLOCK_ALIGN`] boundary after it), and the reader derives
/// them identically. Magic-and-version first means a reader rejects a foreign or
/// wrong-version file before trusting any length.
fn write_preamble(
    out: &mut BufWriter<File>,
    header_len: u64,
    dir_len: u64,
) -> Result<(), BuildError> {
    let mut preamble = [0u8; PREAMBLE_LEN as usize];
    preamble[0..8].copy_from_slice(&RTLA_MAGIC);
    preamble[8..12].copy_from_slice(&RTLA_VERSION.to_le_bytes());
    // preamble[12..16] are reserved flags, left zero in v1.
    preamble[16..24].copy_from_slice(&header_len.to_le_bytes());
    preamble[24..32].copy_from_slice(&dir_len.to_le_bytes());
    out.write_all(&preamble)?;
    Ok(())
}

/// A block of zeros for padding, one [`BLOCK_ALIGN`]-sized alignment unit.
const ZEROS: [u8; BLOCK_ALIGN as usize] = [0u8; BLOCK_ALIGN as usize];

/// Writes zero padding until `offset` reaches `target`.
fn pad_to(out: &mut BufWriter<File>, offset: &mut u64, target: u64) -> Result<(), BuildError> {
    debug_assert!(target >= *offset);
    let mut remaining = target - *offset;
    while remaining > 0 {
        let n = remaining.min(ZEROS.len() as u64) as usize;
        out.write_all(&ZEROS[..n])?;
        remaining -= n as u64;
    }
    *offset = target;
    Ok(())
}

/// Rounds `value` up to the next multiple of `align` (a power of two).
fn align_up(value: u64, align: u64) -> u64 {
    (value + align - 1) & !(align - 1)
}

/// The inclusive range of tile indices `[i0, i1]` along one axis (world `lo..hi`,
/// `n` tiles) that the query span `[q0, q1]` overlaps, clamped to `0..n`. This is the
/// [`crate::lod`] `tile_index_span` math, reimplemented so the streaming builder does
/// not depend on the pyramid's private helpers.
fn tile_span(lo: i32, hi: i32, q0: i32, q1: i32, n: u32) -> (u32, u32) {
    let extent = i64::from(hi) - i64::from(lo);
    if extent <= 0 || n == 0 {
        return (0, 0);
    }
    let per = i64::from(n);
    let to_index = |q: i32| -> i64 {
        let rel = i64::from(q) - i64::from(lo);
        (rel * per).div_euclid(extent)
    };
    let last = per - 1;
    let start = to_index(q0).clamp(0, last);
    let end = to_index(q1).clamp(0, last);
    (start as u32, end as u32)
}

/// One spilled `(tile_index, record)` placement.
#[derive(Clone, Copy)]
struct Entry {
    tile_index: u64,
    record: TileRecord,
}

impl Entry {
    fn encode(&self, out: &mut [u8; ENTRY_LEN]) {
        out[0..8].copy_from_slice(&self.tile_index.to_le_bytes());
        out[8..10].copy_from_slice(&self.record.layer.to_le_bytes());
        out[10..12].copy_from_slice(&self.record.datatype.to_le_bytes());
        out[12..16].copy_from_slice(&self.record.rect.min_x.to_le_bytes());
        out[16..20].copy_from_slice(&self.record.rect.min_y.to_le_bytes());
        out[20..24].copy_from_slice(&self.record.rect.max_x.to_le_bytes());
        out[24..28].copy_from_slice(&self.record.rect.max_y.to_le_bytes());
    }

    fn decode(bytes: &[u8; ENTRY_LEN]) -> Self {
        let rd_i32 = |at: usize| i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        Self {
            tile_index: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            record: TileRecord {
                layer: u16::from_le_bytes(bytes[8..10].try_into().unwrap()),
                datatype: u16::from_le_bytes(bytes[10..12].try_into().unwrap()),
                rect: crate::streaming::ArchivableRect {
                    min_x: rd_i32(12),
                    min_y: rd_i32(16),
                    max_x: rd_i32(20),
                    max_y: rd_i32(24),
                },
            },
        }
    }
}

/// A `k`-way merge over sorted run files, yielding entries in ascending tile-index
/// order while holding only one head entry per run in memory.
struct KMerge {
    readers: Vec<BufReader<File>>,
    /// Min-heap of `(tile_index, run_index)`, so the next entry is always the run
    /// whose head has the smallest tile index.
    heap: std::collections::BinaryHeap<std::cmp::Reverse<(u64, usize)>>,
    /// The buffered head entry for each run (refilled after it is taken).
    heads: Vec<Option<Entry>>,
}

impl KMerge {
    fn open(runs: &[PathBuf]) -> Result<Self, BuildError> {
        let mut readers = Vec::with_capacity(runs.len());
        let mut heads = Vec::with_capacity(runs.len());
        let mut heap = std::collections::BinaryHeap::new();
        for (i, path) in runs.iter().enumerate() {
            let mut reader = BufReader::new(File::open(path)?);
            let head = read_entry(&mut reader)?;
            if let Some(entry) = head {
                heap.push(std::cmp::Reverse((entry.tile_index, i)));
            }
            readers.push(reader);
            heads.push(head);
        }
        Ok(Self {
            readers,
            heap,
            heads,
        })
    }

    /// Returns the next entry in global tile-index order, or `None` when every run is
    /// exhausted.
    fn next_entry(&mut self) -> Result<Option<Entry>, BuildError> {
        let Some(std::cmp::Reverse((_, run))) = self.heap.pop() else {
            return Ok(None);
        };
        let entry = self.heads[run].take().expect("head present for heaped run");
        if let Some(next) = read_entry(&mut self.readers[run])? {
            self.heads[run] = Some(next);
            self.heap.push(std::cmp::Reverse((next.tile_index, run)));
        }
        Ok(Some(entry))
    }
}

/// Reads one fixed-size entry, or `None` at end of the run file.
fn read_entry(reader: &mut BufReader<File>) -> Result<Option<Entry>, BuildError> {
    let mut bytes = [0u8; ENTRY_LEN];
    match reader.read_exact(&mut bytes) {
        Ok(()) => Ok(Some(Entry::decode(&bytes))),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
        Err(e) => Err(BuildError::Io(e)),
    }
}

/// A scratch directory for spill runs, cleaned up on [`Self::cleanup`] or on drop.
struct Scratch {
    dir: PathBuf,
    cleaned: bool,
}

impl Scratch {
    /// Creates a fresh scratch directory beside `out_path` (same volume, so spill I/O
    /// stays local to the archive's drive).
    fn create(out_path: &Path) -> Result<Self, BuildError> {
        let mut file_name = out_path.file_name().map_or_else(
            || std::ffi::OsString::from("rtla"),
            std::ffi::OsString::from,
        );
        file_name.push(".build-scratch");
        let dir = out_path.with_file_name(file_name);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            cleaned: false,
        })
    }

    /// Removes the scratch directory. Best-effort on drop as a backstop.
    fn cleanup(mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        self.cleaned = true;
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if !self.cleaned {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::{LevelDims, RtlaHeader};
    use crate::streaming::ArchivableRect;
    use reticle_geometry::{Point, Rect};

    fn header(world: Rect, levels: Vec<LevelDims>) -> RtlaHeader {
        RtlaHeader {
            magic: RTLA_MAGIC,
            version: RTLA_VERSION,
            world: ArchivableRect::from_rect(world),
            dbu_per_micron: 1000,
            levels,
        }
    }

    #[test]
    fn align_up_rounds_to_block() {
        assert_eq!(align_up(0, 16), 0);
        assert_eq!(align_up(1, 16), 16);
        assert_eq!(align_up(16, 16), 16);
        assert_eq!(align_up(17, 16), 32);
    }

    #[test]
    fn entry_round_trips_through_encoding() {
        let entry = Entry {
            tile_index: 0x0102_0304_0506_0708,
            record: TileRecord {
                layer: 68,
                datatype: 20,
                rect: ArchivableRect {
                    min_x: -5,
                    min_y: 6,
                    max_x: 7,
                    max_y: 8,
                },
            },
        };
        let mut bytes = [0u8; ENTRY_LEN];
        entry.encode(&mut bytes);
        let decoded = Entry::decode(&bytes);
        assert_eq!(decoded.tile_index, entry.tile_index);
        assert_eq!(decoded.record, entry.record);
    }

    #[test]
    fn tile_span_matches_lod_convention() {
        // A single-tile axis maps everything to index 0.
        assert_eq!(tile_span(0, 100, 10, 20, 1), (0, 0));
        // Four tiles over [0,100): the span [10,60] covers tiles 0..=2.
        assert_eq!(tile_span(0, 100, 10, 60, 4), (0, 2));
        // Out-of-world coordinates clamp into range.
        assert_eq!(tile_span(0, 100, -50, 200, 4), (0, 3));
    }

    #[test]
    fn rejects_a_header_with_bad_magic() {
        let mut h = header(
            Rect::new(Point::new(0, 0), Point::new(10, 10)),
            vec![LevelDims { cols: 1, rows: 1 }],
        );
        h.magic = *b"NOTRTLA0";
        let tmp = std::env::temp_dir().join("rtla_bad_magic.rtla");
        let err = build_rtla(&h, std::iter::empty(), &tmp).unwrap_err();
        assert!(matches!(err, BuildError::InvalidHeader(_)));
    }
}
