//! End-to-end writer/reader agreement (Wave 2 gate regression guard).
//!
//! An archive built by lane 2A's [`build_rtla`] must be readable by lane 2B's real
//! [`MmapTileSource`]. The two lanes defined the physical byte framing in parallel and
//! diverged: 2A wrote an offset-first preamble, 2B read a magic-first preamble, so 2B
//! rejected every archive 2A produced as "bad magic". Neither lane's own tests caught
//! it (2A read back with a local reader, 2B's proptest streamed from an in-memory
//! double). Both now share ADR 0069's canonical framing; this test proves it and keeps
//! it from regressing.

#![cfg(not(target_arch = "wasm32"))]

use pollster::block_on;
use reticle_geometry::{Point, Rect};
use reticle_index::archive::ArchivedTilePayload;
use reticle_index::streaming::ArchivableRect;
use reticle_index::tile_source::MmapTileSource;
use reticle_index::{
    LevelDims, RTLA_MAGIC, RTLA_VERSION, RtlaHeader, TileCoord, TileRecord, TileSource,
};

#[test]
fn an_archive_built_by_the_builder_is_read_by_the_mmap_source() {
    let world = Rect::new(Point::new(0, 0), Point::new(100, 100));
    let header = RtlaHeader {
        magic: RTLA_MAGIC,
        version: RTLA_VERSION,
        world: ArchivableRect::from_rect(world),
        dbu_per_micron: 1000,
        levels: vec![
            LevelDims { cols: 1, rows: 1 },
            LevelDims { cols: 2, rows: 2 },
        ],
    };
    let rec = |layer: u16, x: i32, y: i32| TileRecord {
        layer,
        datatype: 20,
        rect: ArchivableRect::from_rect(Rect::new(Point::new(x, y), Point::new(x + 5, y + 5))),
    };
    let records = vec![rec(68, 10, 10), rec(69, 60, 60)];

    let path = std::env::temp_dir().join(format!("rtla_xtest_{}.rtla", std::process::id()));
    reticle_index::build_rtla(&header, records, &path).expect("the builder writes the archive");

    // The core proof: 2B's real reader opens 2A's archive. If the framing diverged, this
    // fails on the magic check (the exact bug this guards).
    let src = MmapTileSource::open(&path).expect("the mmap source opens the built archive");

    let h = block_on(src.header()).expect("the reader parses the header");
    assert_eq!(h.dbu_per_micron, 1000);
    assert_eq!(h.levels.len(), 2);
    assert_eq!(h.world_rect(), world);

    // A tile fetch returns bytes that validate as an archived payload, proving the
    // directory offsets and tile framing are readable too, not just the preamble.
    let bytes = block_on(src.tile_bytes(TileCoord {
        level: 1,
        col: 1,
        row: 1,
    }))
    .expect("the reader fetches a finest-level tile");
    let _ = rkyv::access::<ArchivedTilePayload, rkyv::rancor::Error>(&bytes)
        .expect("the fetched tile validates as an ArchivedTilePayload");

    let _ = std::fs::remove_file(&path);
}
