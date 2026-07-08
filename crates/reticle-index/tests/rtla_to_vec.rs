//! Tests for [`reticle_index::build_rtla_to_vec`], the in-memory `.rtla` builder used
//! by the in-browser converter (lane v8-6c), where there is no filesystem for the
//! external [`reticle_index::build_rtla`] to spill and seek against.
//!
//! The load-bearing guarantee is that the in-memory builder is a drop-in for the
//! external one: for any input that fits in a single sort chunk (every browser v1
//! input, and these fixtures), it produces **byte-identical** archive bytes. That one
//! equality means the frozen `MmapTileSource`/streaming reader -- already proven to
//! read `build_rtla` output -- reads `build_rtla_to_vec` output the same way, so the
//! browser archive and the native `reticle convert` archive are the same file.

use std::path::PathBuf;

use rkyv::rancor;
use rkyv::vec::ArchivedVec;

use reticle_geometry::{Point, Rect};
use reticle_index::archive::{
    ArchivedTileDirEntry, ArchivedTilePayload, LevelDims, RTLA_MAGIC, RTLA_VERSION, RtlaHeader,
    TileRecord,
};
use reticle_index::streaming::ArchivableRect;
use reticle_index::{build_rtla, build_rtla_to_vec};

/// A unique temp path for one test.
fn temp_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("reticle_rtla_to_vec_{tag}.rtla"))
}

/// The `i`th deterministic small record spread across `world`.
fn record_at(i: u32, world: Rect) -> TileRecord {
    let span_x = i64::from(world.max.x - world.min.x - 16).max(1);
    let span_y = i64::from(world.max.y - world.min.y - 16).max(1);
    let min_x = (i64::from(world.min.x) + (i64::from(i) * 13) % span_x) as i32;
    let min_y = (i64::from(world.min.y) + (i64::from(i) * 97) % span_y) as i32;
    TileRecord {
        layer: (i % 4) as u16 + 60,
        datatype: (i % 2) as u16 * 20,
        rect: ArchivableRect::from_rect(Rect::new(
            Point::new(min_x, min_y),
            Point::new(min_x + 8, min_y + 8),
        )),
    }
}

fn sample_records(n: u32, world: Rect) -> Vec<TileRecord> {
    (0..n).map(|i| record_at(i, world)).collect()
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
fn in_memory_bytes_equal_the_external_builder() {
    let world = Rect::new(Point::new(0, 0), Point::new(100_000, 100_000));
    let levels = vec![
        LevelDims { cols: 1, rows: 1 },
        LevelDims { cols: 4, rows: 4 },
        LevelDims { cols: 16, rows: 16 },
    ];
    let records = sample_records(500, world);

    let path = temp_path("equal");
    build_rtla(&header_for(world, levels.clone()), records.clone(), &path).expect("external build");
    let disk_bytes = std::fs::read(&path).expect("read external archive");
    let _ = std::fs::remove_file(&path);

    let mem_bytes =
        build_rtla_to_vec(&header_for(world, levels), records).expect("in-memory build");

    assert!(!mem_bytes.is_empty(), "the in-memory archive is non-empty");
    assert_eq!(
        mem_bytes, disk_bytes,
        "in-memory builder must produce byte-identical output to the external builder"
    );
}

#[test]
fn in_memory_bytes_are_deterministic() {
    let world = Rect::new(Point::new(0, 0), Point::new(10_000, 10_000));
    let levels = vec![
        LevelDims { cols: 1, rows: 1 },
        LevelDims { cols: 8, rows: 8 },
    ];
    let records = sample_records(200, world);

    let a = build_rtla_to_vec(&header_for(world, levels.clone()), records.clone()).expect("a");
    let b = build_rtla_to_vec(&header_for(world, levels), records).expect("b");
    assert_eq!(a, b, "same input converts to identical bytes");
}

#[test]
fn in_memory_archive_reads_back_from_the_returned_vec() {
    let world = Rect::new(Point::new(0, 0), Point::new(100_000, 100_000));
    let levels = vec![
        LevelDims { cols: 1, rows: 1 },
        LevelDims { cols: 4, rows: 4 },
        LevelDims { cols: 16, rows: 16 },
    ];
    let records = sample_records(500, world);
    let expected: std::collections::BTreeSet<(u16, u16, i32, i32, i32, i32)> = records
        .iter()
        .map(|r| {
            (
                r.layer,
                r.datatype,
                r.rect.min_x,
                r.rect.min_y,
                r.rect.max_x,
                r.rect.max_y,
            )
        })
        .collect();

    let bytes = build_rtla_to_vec(&header_for(world, levels.clone()), records).expect("build");

    // Parse the canonical preamble (ADR 0069) straight out of the returned Vec, exactly
    // as the frozen reader derives block offsets.
    assert_eq!(&bytes[0..8], &RTLA_MAGIC, "bad .rtla magic");
    let rd = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let header_len = rd(16);
    let dir_len = rd(24);
    let header_off = 32usize;
    let dir_off = ((header_off as u64 + header_len).div_ceil(16) * 16) as usize;

    let dir = rkyv::access::<ArchivedVec<ArchivedTileDirEntry>, rancor::Error>(
        &bytes[dir_off..dir_off + dir_len as usize],
    )
    .expect("directory validates");

    let total_tiles: usize = levels
        .iter()
        .map(|l| l.cols as usize * l.rows as usize)
        .sum();
    assert_eq!(dir.len(), total_tiles);

    // Sweep the finest level (last 16*16 tiles); every input record must be recovered.
    let finest_tiles = 16 * 16;
    let mut recovered = std::collections::BTreeSet::new();
    for tile in (total_tiles - finest_tiles)..total_tiles {
        let entry: &ArchivedTileDirEntry = &dir[tile];
        let offset = u64::from(entry.offset) as usize;
        let len = u64::from(entry.len) as usize;
        let payload =
            rkyv::access::<ArchivedTilePayload, rancor::Error>(&bytes[offset..offset + len])
                .expect("tile validates");
        for r in payload.records.iter() {
            recovered.insert((
                u16::from(r.layer),
                u16::from(r.datatype),
                i32::from(r.rect.min_x),
                i32::from(r.rect.min_y),
                i32::from(r.rect.max_x),
                i32::from(r.rect.max_y),
            ));
        }
    }
    assert_eq!(
        recovered, expected,
        "finest level recovers every input record"
    );
}
