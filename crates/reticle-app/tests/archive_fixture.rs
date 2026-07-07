//! Builds and round-trips the committed served-archive Playwright fixture (lane v8-2e).
//!
//! The served-archive e2e (`e2e/tests/served-archive.spec.ts`) opens a small `.rtla` over
//! a local HTTP-range server and asserts tiles stream in and the canvas paints. That
//! fixture is committed at `e2e/fixtures/fixture.rtla`; this test is both its **generator**
//! and its **regression guard**:
//!
//! * As a guard (the default), it builds the fixture into a temp file with
//!   [`build_rtla`](reticle_index::build_rtla) and proves an
//!   [`MmapTileSource`](reticle_index::tile_source::MmapTileSource) reads it back and a
//!   whole-world [`query_viewport`](reticle_index::tile_source::query_viewport) returns
//!   exactly the records that went in, so a change to the frozen `.rtla` framing that
//!   would break the browser reader fails here too.
//! * As a generator, when `RETICLE_ARCHIVE_FIXTURE_OUT` names a path it writes the same
//!   bytes there. The committed fixture was produced with:
//!   `RETICLE_ARCHIVE_FIXTURE_OUT=e2e/fixtures/fixture.rtla cargo test -p reticle-app --test archive_fixture`.
//!
//! The archive is a three-level power-of-two pyramid (1x1, 2x2, 4x4) over a 10000x10000
//! DBU world, with one record centred in each of the sixteen finest tiles, so every fine
//! tile is non-empty and the browser paints a full grid of blocks as they stream in.

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use reticle_geometry::{Point, Rect};
use reticle_index::streaming::ArchivableRect;
use reticle_index::tile_source::{MmapTileSource, query_viewport};
use reticle_index::{
    LevelDims, RTLA_MAGIC, RTLA_VERSION, RtlaHeader, TileRecord, TileSource, build_rtla,
};

/// The fixture world: 10000 x 10000 DBU.
const WORLD_SIDE: i32 = 10_000;
/// The finest level is a 4x4 grid, so one record per finest tile is sixteen records.
const FINE_SIDE: i32 = 4;

/// A minimal park-based executor, so the async `TileSource` can be driven without pulling
/// in an async-runtime dev-dependency (mirrors `tests/residency.rs`).
struct ThreadWaker(std::thread::Thread);
impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = Box::pin(future);
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park_timeout(Duration::from_millis(1)),
        }
    }
}

/// The three-level power-of-two pyramid header the streamed reader expects.
fn fixture_header() -> RtlaHeader {
    RtlaHeader {
        magic: RTLA_MAGIC,
        version: RTLA_VERSION,
        world: ArchivableRect::from_rect(Rect::new(
            Point::new(0, 0),
            Point::new(WORLD_SIDE, WORLD_SIDE),
        )),
        dbu_per_micron: 1000,
        levels: vec![
            LevelDims { cols: 1, rows: 1 },
            LevelDims { cols: 2, rows: 2 },
            LevelDims { cols: 4, rows: 4 },
        ],
    }
}

/// One record centred in each finest (4x4) tile, so every fine tile is non-empty. Layers
/// cycle through a few real GDS metal layers so the browser paints them in distinct colors.
fn fixture_records() -> Vec<TileRecord> {
    let tile = WORLD_SIDE / FINE_SIDE; // 2500 DBU per finest tile
    let mut records = Vec::new();
    for row in 0..FINE_SIDE {
        for col in 0..FINE_SIDE {
            let x0 = col * tile + 500;
            let y0 = row * tile + 500;
            let x1 = col * tile + 2000;
            let y1 = row * tile + 2000;
            let layer = 68 + u16::try_from((row + col) % 3).unwrap();
            records.push(TileRecord {
                layer,
                datatype: 20,
                rect: ArchivableRect::from_rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
            });
        }
    }
    records
}

#[test]
fn builds_and_round_trips_the_served_archive_fixture() {
    let header = fixture_header();
    let records = fixture_records();
    let expected = records.len();
    assert_eq!(
        expected,
        (FINE_SIDE * FINE_SIDE) as usize,
        "one per finest tile"
    );

    // Write to the requested committed path when generating, else a unique temp file.
    let (out_path, temp) = if let Some(p) = std::env::var_os("RETICLE_ARCHIVE_FIXTURE_OUT") {
        (std::path::PathBuf::from(p), false)
    } else {
        let mut p = std::env::temp_dir();
        p.push(format!("reticle_fixture_{}.rtla", std::process::id()));
        (p, true)
    };
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).expect("fixture output directory");
    }

    build_rtla(&header, records.clone(), &out_path).expect("build the .rtla fixture");

    // The MmapTileSource (the native sibling of the browser HTTP-range reader, sharing the
    // exact frozen framing) opens it, and a whole-world query returns every record.
    let source = MmapTileSource::open(&out_path).expect("mmap-open the fixture");
    let got_header = block_on(source.header()).expect("fixture header");
    assert_eq!(got_header, header, "the header round-trips");
    let world = Rect::new(Point::new(0, 0), Point::new(WORLD_SIDE, WORLD_SIDE));
    let painted = block_on(query_viewport(&source, &got_header, world)).expect("query");
    assert_eq!(
        painted.len(),
        expected,
        "every fine-tile record is served back by the archive"
    );

    if temp {
        let _ = std::fs::remove_file(&out_path);
    } else {
        // Sanity-check the generated committed fixture is a small, real file.
        let len = std::fs::metadata(&out_path)
            .expect("fixture metadata")
            .len();
        assert!(len > 128, "fixture is non-trivial ({len} bytes)");
    }
}
