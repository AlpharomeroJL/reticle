//! Round-trip tests for the external `.rtla` builder ([`reticle_index::build_rtla`]).
//!
//! These build a real archive to a temp file and read it back the way lane 2B's
//! `MmapTileSource` will: memory-map the file, parse the fixed preamble, `rkyv::access`
//! the header and directory blocks in place, then `rkyv::access` each tile's bytes by
//! its `(offset, len)` directory entry. The load-bearing assertion is that the finest
//! level round-trips **exactly** (ADR 0062: "the finest level holds exact geometry"),
//! so every input record is recovered from it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use memmap2::Mmap;
use rkyv::rancor;
use rkyv::vec::ArchivedVec;

use reticle_geometry::{Point, Rect};
use reticle_index::archive::{
    ArchivedRtlaHeader, ArchivedTileDirEntry, ArchivedTilePayload, LevelDims, RTLA_MAGIC,
    RTLA_VERSION, RtlaHeader, TileRecord,
};
use reticle_index::build_rtla;
use reticle_index::streaming::ArchivableRect;

/// A comparable, order-independent record key: `(layer, datatype, corners)`.
type RecordKey = (u16, u16, i32, i32, i32, i32);

fn key(r: &TileRecord) -> RecordKey {
    (
        r.layer,
        r.datatype,
        r.rect.min_x,
        r.rect.min_y,
        r.rect.max_x,
        r.rect.max_y,
    )
}

/// A unique temp path for one test (no two tests share a file).
fn temp_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("reticle_rtla_{tag}.rtla"))
}

/// A path for the large builds, preferring the (roomier) target drive when
/// `CARGO_TARGET_DIR` is set so multi-gigabyte archives and their spill do not fill
/// the system temp volume.
fn big_temp_path(tag: &str) -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => {
            let base = PathBuf::from(dir).join("rtla-bigtest");
            let _ = std::fs::create_dir_all(&base);
            base.join(format!("reticle_rtla_{tag}.rtla"))
        }
        None => temp_path(tag),
    }
}

/// The `i`th deterministic small record spread across `world`. All arithmetic is
/// `i64` so it is safe up to the 30M-record build; casts land back inside the world.
fn record_at(i: u32, world: Rect) -> TileRecord {
    let span_x = i64::from(world.max.x - world.min.x - 16).max(1);
    let span_y = i64::from(world.max.y - world.min.y - 16).max(1);
    let min_x = i64::from(world.min.x) + (i64::from(i) * 13) % span_x;
    let min_y = i64::from(world.min.y) + (i64::from(i) * 97) % span_y;
    let (min_x, min_y) = (min_x as i32, min_y as i32);
    TileRecord {
        layer: (i % 4) as u16 + 60,
        datatype: (i % 2) as u16 * 20,
        rect: ArchivableRect::from_rect(Rect::new(
            Point::new(min_x, min_y),
            Point::new(min_x + 8, min_y + 8),
        )),
    }
}

/// A deterministic set of `n` unique small records spread across `world`.
fn sample_records(n: u32, world: Rect) -> Vec<TileRecord> {
    (0..n).map(|i| record_at(i, world)).collect()
}

/// A minimal in-memory reader over a built archive, shaped like the eventual
/// `MmapTileSource`: it maps the file, validates each block with `rkyv::access`, and
/// exposes the header, directory, and per-tile records.
struct ArchiveReader {
    mmap: Mmap,
    header_off: u64,
    header_len: u64,
    dir_off: u64,
    dir_len: u64,
}

impl ArchiveReader {
    fn open(path: &Path) -> Self {
        let file = std::fs::File::open(path).expect("open archive");
        // SAFETY: the file is read-only for the map's lifetime and not mutated by any
        // other process during the test; this mirrors `StreamingIndex::open`.
        #[allow(unsafe_code)]
        let mmap = unsafe { Mmap::map(&file).expect("map archive") };
        assert!(mmap.len() >= 32, "archive smaller than its preamble");
        // Canonical .rtla preamble (ADR 0069, shared with the real reader): magic[0..8],
        // version[8..12], flags[12..16], header_len[16..24], dir_len[24..32]. Offsets are
        // derived (header at 32, directory at the next 16-aligned boundary), exactly as
        // crate::tile_source::MmapTileSource derives them.
        assert_eq!(&mmap[0..8], &RTLA_MAGIC, "bad .rtla magic");
        let rd = |at: usize| u64::from_le_bytes(mmap[at..at + 8].try_into().unwrap());
        let header_len = rd(16);
        let dir_len = rd(24);
        let header_off = 32u64;
        let dir_off = (header_off + header_len).div_ceil(16) * 16;
        Self {
            mmap,
            header_off,
            header_len,
            dir_off,
            dir_len,
        }
    }

    fn header(&self) -> &ArchivedRtlaHeader {
        let block =
            &self.mmap[self.header_off as usize..(self.header_off + self.header_len) as usize];
        rkyv::access::<ArchivedRtlaHeader, rancor::Error>(block).expect("validate header block")
    }

    fn directory(&self) -> &ArchivedVec<ArchivedTileDirEntry> {
        let block = &self.mmap[self.dir_off as usize..(self.dir_off + self.dir_len) as usize];
        rkyv::access::<ArchivedVec<ArchivedTileDirEntry>, rancor::Error>(block)
            .expect("validate directory block")
    }

    /// The records of the tile at directory index `tile`, validated in place.
    fn tile_records(&self, tile: usize) -> Vec<TileRecord> {
        let entry = &self.directory()[tile];
        let offset = u64::from(entry.offset) as usize;
        let len = u64::from(entry.len) as usize;
        let block = &self.mmap[offset..offset + len];
        let payload =
            rkyv::access::<ArchivedTilePayload, rancor::Error>(block).expect("validate tile block");
        payload
            .records
            .iter()
            .map(|r| TileRecord {
                layer: r.layer.into(),
                datatype: r.datatype.into(),
                rect: ArchivableRect {
                    min_x: r.rect.min_x.into(),
                    min_y: r.rect.min_y.into(),
                    max_x: r.rect.max_x.into(),
                    max_y: r.rect.max_y.into(),
                },
            })
            .collect()
    }
}

fn header_for(world: Rect, levels: Vec<LevelDims>) -> RtlaHeader {
    RtlaHeader {
        magic: RTLA_MAGIC,
        version: RTLA_VERSION,
        world: ArchivableRect::from_rect(world),
        dbu_per_micron: 1000,
        levels,
    }
}

#[test]
fn finest_level_round_trips_exactly() {
    let world = Rect::new(Point::new(0, 0), Point::new(100_000, 100_000));
    let levels = vec![
        LevelDims { cols: 1, rows: 1 },
        LevelDims { cols: 4, rows: 4 },
        LevelDims { cols: 16, rows: 16 },
    ];
    let records = sample_records(500, world);
    let expected: BTreeSet<RecordKey> = records.iter().map(key).collect();

    let path = temp_path("finest_roundtrip");
    build_rtla(&header_for(world, levels.clone()), records.clone(), &path).expect("build");

    let reader = ArchiveReader::open(&path);

    // Header round-trips.
    let h = reader.header();
    assert_eq!(h.magic, RTLA_MAGIC);
    assert_eq!(u32::from(h.version), RTLA_VERSION);
    assert_eq!(i64::from(h.dbu_per_micron), 1000);
    assert_eq!(h.levels.len(), levels.len());
    assert_eq!(u32::from(h.levels.last().unwrap().cols), 16);

    // Directory is level-major and sized to the total tile count.
    let total_tiles: usize = levels
        .iter()
        .map(|l| l.cols as usize * l.rows as usize)
        .sum();
    assert_eq!(reader.directory().len(), total_tiles);

    // The finest level is the last; its tiles are the final `16*16` directory slots.
    let finest_tiles = 16 * 16;
    let finest_base = total_tiles - finest_tiles;
    let mut recovered: BTreeSet<RecordKey> = BTreeSet::new();
    for tile in finest_base..total_tiles {
        for record in reader.tile_records(tile) {
            recovered.insert(key(&record));
        }
    }
    assert_eq!(
        recovered, expected,
        "finest level did not recover every input record exactly"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn directory_offsets_are_ordered_and_in_bounds() {
    let world = Rect::new(Point::new(0, 0), Point::new(10_000, 10_000));
    let levels = vec![
        LevelDims { cols: 1, rows: 1 },
        LevelDims { cols: 8, rows: 8 },
    ];
    let records = sample_records(200, world);
    let path = temp_path("dir_ordered");
    build_rtla(&header_for(world, levels), records, &path).expect("build");

    let reader = ArchiveReader::open(&path);
    let dir = reader.directory();
    let file_len = reader.mmap.len() as u64;
    let mut prev_end = reader.dir_off + reader.dir_len;
    for i in 0..dir.len() {
        let entry: &ArchivedTileDirEntry = &dir[i];
        let offset = u64::from(entry.offset);
        let len = u64::from(entry.len);
        // Tiles are byte-contiguous and in directory order, so each starts at or after
        // the previous tile's end and stays within the file.
        assert!(offset >= prev_end, "tile {i} overlaps the previous block");
        assert!(offset + len <= file_len, "tile {i} runs past end of file");
        prev_end = offset + len;
    }

    let _ = std::fs::remove_file(&path);
}

#[test]
#[ignore = "large: builds a 30M-entry (multi-GB) archive; run explicitly to measure peak RSS"]
fn build_30m_entry_archive_under_memory_budget() {
    // Four pyramid levels; the finest is 256x256 = 65_536 tiles, so 30M records land
    // ~458 per finest tile. The record source is a lazy iterator: the 30M records are
    // never all in memory, and the builder spills to disk, so peak RSS is bounded by
    // one sort chunk (~128 MiB) regardless of the record count. The measurement
    // harness (scratch/lanes/v8-2a-reader-builder/measure-mem.ps1) samples this
    // process's peak working set while it runs.
    let world = Rect::new(Point::new(0, 0), Point::new(1_000_000, 1_000_000));
    let levels = vec![
        LevelDims { cols: 1, rows: 1 },
        LevelDims { cols: 8, rows: 8 },
        LevelDims { cols: 64, rows: 64 },
        LevelDims {
            cols: 256,
            rows: 256,
        },
    ];
    let n: u32 = 30_000_000;

    let world_for_records = world;
    let records = (0..n).map(move |i| record_at(i, world_for_records));

    let path = temp_path("build_30m");
    let start = std::time::Instant::now();
    build_rtla(&header_for(world, levels.clone()), records, &path).expect("build 30M archive");
    let elapsed = start.elapsed();

    let bytes = std::fs::metadata(&path).expect("stat archive").len();
    // Machine-readable line the measurement harness / RESULT.md pick up.
    println!(
        "RTLA_30M_RESULT records={n} archive_bytes={bytes} elapsed_secs={:.1}",
        elapsed.as_secs_f64()
    );

    // Spot-check that the archive is well-formed: header and directory validate, and
    // the finest level's tile count matches.
    let reader = ArchiveReader::open(&path);
    let total_tiles: usize = levels
        .iter()
        .map(|l| l.cols as usize * l.rows as usize)
        .sum();
    assert_eq!(reader.directory().len(), total_tiles);
    assert_eq!(u32::from(reader.header().levels.last().unwrap().cols), 256);

    let _ = std::fs::remove_file(&path);
}

#[test]
#[ignore = "very large: writes a multi-GB archive to the target drive; run explicitly"]
fn build_multi_gb_archive_completes() {
    // 120M finest records at ~20 bytes each serialize to a >2 GiB archive. The point
    // is that a multi-gigabyte archive builds to completion under the same bounded
    // memory as the smaller builds (the record source is lazy and the builder spills
    // to disk), so this only asserts completion and the multi-GB size.
    let world = Rect::new(Point::new(0, 0), Point::new(2_000_000, 2_000_000));
    let levels = vec![
        LevelDims { cols: 1, rows: 1 },
        LevelDims { cols: 16, rows: 16 },
        LevelDims {
            cols: 512,
            rows: 512,
        },
    ];
    let n: u32 = 120_000_000;

    let world_for_records = world;
    let records = (0..n).map(move |i| record_at(i, world_for_records));

    let path = big_temp_path("multi_gb");
    let start = std::time::Instant::now();
    build_rtla(&header_for(world, levels), records, &path).expect("build multi-GB archive");
    let elapsed = start.elapsed();

    let bytes = std::fs::metadata(&path).expect("stat archive").len();
    println!(
        "RTLA_MULTIGB_RESULT records={n} archive_bytes={bytes} elapsed_secs={:.1}",
        elapsed.as_secs_f64()
    );
    assert!(
        bytes > 2 * 1024 * 1024 * 1024,
        "expected a multi-GB archive, got {bytes} bytes"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn builds_an_empty_layout() {
    // No records: every tile is present but empty, and the archive still validates.
    let world = Rect::new(Point::new(0, 0), Point::new(1000, 1000));
    let levels = vec![
        LevelDims { cols: 1, rows: 1 },
        LevelDims { cols: 2, rows: 2 },
    ];
    let path = temp_path("empty");
    build_rtla(&header_for(world, levels), std::iter::empty(), &path).expect("build empty");

    let reader = ArchiveReader::open(&path);
    assert_eq!(reader.directory().len(), 1 + 4);
    for tile in 0..5 {
        assert!(reader.tile_records(tile).is_empty());
    }

    let _ = std::fs::remove_file(&path);
}
