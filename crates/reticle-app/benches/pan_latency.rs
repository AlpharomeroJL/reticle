//! Pointer-latency pan benchmark (lane 3a, catalog 44).
//!
//! The fluidity contract for browsing a streamed die is that a pointer drag stays under
//! a ~16 ms budget so panning tracks the cursor at 60 fps. This measures the *CPU*
//! pointer-to-paint work one drag frame costs against a large resident scene: pan the
//! camera by a pixel delta, recompute the visible world rectangle, pick the level of
//! detail for that viewport, and select the streamed records that would be painted. It
//! deliberately touches only the pure, window-free path (`ViewCamera` +
//! `StreamedScene`), so the number reflects the layout work rather than the GPU or egui.
//!
//! Criterion produces the estimate at run time (`cargo bench -p reticle-app
//! --bench pan_latency`); the committed figure lives in `benches/history/baseline.json`
//! and `just perf-check` fails on a regression beyond that baseline's tolerance. The
//! 16 ms budget is the design target the measured value stays comfortably under.

use criterion::{Criterion, criterion_group, criterion_main};
use reticle_app::StreamedScene;
use reticle_app::camera::{ScreenRect, ViewCamera};
use reticle_geometry::{Point, Rect};
use reticle_index::streaming::ArchivableRect;
use reticle_index::{
    LevelDims, RTLA_MAGIC, RTLA_VERSION, RtlaHeader, TileCoord, TilePayload, TileRecord,
};

/// The world side length in DBU (a 1 mm die at 1000 DBU/micron).
const WORLD_DBU: i32 = 1_000_000;
/// Pyramid depth: levels 0..=4 have 1, 2, 4, 8, 16 tiles across.
const LEVELS: u32 = 5;
/// Records archived into each finest-level tile.
const RECORDS_PER_TILE: i32 = 64;

/// Builds a fully-resident streamed scene over a `LEVELS`-deep pyramid, with the finest
/// level densely populated so the record-selection work is representative of a real die.
fn resident_scene() -> StreamedScene {
    let world = Rect::new(Point::new(0, 0), Point::new(WORLD_DBU, WORLD_DBU));
    let header = RtlaHeader {
        magic: RTLA_MAGIC,
        version: RTLA_VERSION,
        world: ArchivableRect::from_rect(world),
        dbu_per_micron: 1000,
        levels: (0..LEVELS)
            .map(|l| LevelDims {
                cols: 1 << l,
                rows: 1 << l,
            })
            .collect(),
    };
    // A generous residency bound so nothing is evicted while we fill the finest level.
    let mut scene = StreamedScene::new(header, 1 << 16).expect("valid pyramid header");
    let fine = LEVELS - 1;
    let across = 1u32 << fine; // 16
    let tile_w = WORLD_DBU / across as i32;
    for col in 0..across {
        for row in 0..across {
            let base_x = col as i32 * tile_w;
            let base_y = row as i32 * tile_w;
            let mut records = Vec::with_capacity(RECORDS_PER_TILE as usize);
            for k in 0..RECORDS_PER_TILE {
                let x = base_x + (k * 37).rem_euclid(tile_w.max(1));
                let y = base_y + (k * 53).rem_euclid(tile_w.max(1));
                records.push(TileRecord {
                    layer: 68,
                    datatype: 20,
                    rect: ArchivableRect::from_rect(Rect::new(
                        Point::new(x, y),
                        Point::new(x + 50, y + 50),
                    )),
                });
            }
            scene.insert_tile(
                TileCoord {
                    level: fine,
                    col,
                    row,
                },
                TilePayload { records },
            );
        }
    }
    scene
}

/// Benchmarks one pointer-drag frame's pan-to-paint CPU work.
fn bench_pan(c: &mut Criterion) {
    let scene = resident_scene();
    // A typical desktop canvas.
    let screen = ScreenRect::new(0.0, 0.0, 1920.0, 1080.0);
    // Zoomed so a meaningful chunk of the finest level is on screen.
    let mut camera = ViewCamera::new(Point::new(WORLD_DBU / 2, WORLD_DBU / 2), 0.02);
    // Wiggle the pan so the camera stays centered on populated tiles.
    let mut dx = 4.0_f32;

    c.bench_function("canvas_pan_pointer_latency", |b| {
        b.iter(|| {
            camera.pan_pixels(dx, 0.0);
            dx = -dx;
            let viewport = camera.visible_world_rect(&screen);
            let target = reticle_app::archive::target_level_for_viewport(&scene, viewport);
            let level = scene.paint_level(viewport, target).unwrap_or(0);
            let records = scene.painted_records(viewport, level);
            std::hint::black_box(records.len())
        });
    });
}

criterion_group!(benches, bench_pan);
criterion_main!(benches);
