//! Builds a large deterministic `.rtla` archive via the external two-pass builder
//! (lane 2A `build_rtla`), for the flagship streaming demo. Not a test; a one-shot
//! generator that streams records through the bounded-memory builder so it can emit a
//! multi-hundred-MB archive without holding it all in RAM.
//!
//! Run:
//! `cargo run -p reticle-index --example build_anchor --release -- <records> <out.rtla>`
//! The record layout matches the crate's build tests (deterministic, spread across a
//! 1e6 x 1e6 DBU world), so the archive reads back through the same `MmapTileSource`
//! path the browser streams over HTTP Range.

use std::path::PathBuf;

use reticle_geometry::{Point, Rect};
use reticle_index::archive::{LevelDims, RTLA_MAGIC, RTLA_VERSION, RtlaHeader, TileRecord};
use reticle_index::build_rtla;
use reticle_index::streaming::ArchivableRect;

/// Target finest-tile size in DBU (mirrors `reticle convert`'s `plan_levels`).
const TARGET_FINEST_TILE_DBU: i64 = 1024;
/// Maximum pyramid depth (mirrors `reticle convert`'s `plan_levels`).
const MAX_LEVELS: u32 = 12;

/// The `i`th deterministic small record spread across `world` (mirrors the crate's
/// `rtla_build` test generator; `i64` arithmetic is safe past 30M records).
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

fn main() {
    let mut args = std::env::args().skip(1);
    let n: u32 = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000_000);
    let out = PathBuf::from(
        args.next()
            .unwrap_or_else(|| "scratch/flagship/anchor.rtla".to_owned()),
    );
    let world = Rect::new(Point::new(0, 0), Point::new(1_000_000, 1_000_000));
    // Replicate `reticle convert`'s plan_levels EXACTLY: consecutive `2^i x 2^i` grids,
    // depth chosen so a finest tile is ~1024 DBU across, clamped to [1, 12]. The browser
    // maps a viewport to tiles assuming this canonical power-of-two pyramid, so an archive
    // with non-consecutive levels (e.g. 1,8,64) parses but never fetches a tile.
    let span = i64::from(world.max.x - world.min.x)
        .max(i64::from(world.max.y - world.min.y))
        .max(1);
    let ideal = (span / TARGET_FINEST_TILE_DBU).max(1) as u64;
    let count = (64 - ideal.leading_zeros()).clamp(1, MAX_LEVELS);
    let levels: Vec<LevelDims> = (0..count)
        .map(|i| {
            let n = 1u32 << i;
            LevelDims { cols: n, rows: n }
        })
        .collect();
    let header = RtlaHeader {
        magic: RTLA_MAGIC,
        version: RTLA_VERSION,
        world: ArchivableRect::from_rect(world),
        dbu_per_micron: 1000,
        levels,
    };
    let records = (0..n).map(|i| record_at(i, world));
    build_rtla(&header, records, &out).expect("build_rtla should succeed");
    let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    println!(
        "built {n} records -> {} ({:.1} MiB)",
        out.display(),
        size as f64 / 1_048_576.0
    );
}
