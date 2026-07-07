//! The residency proof: coarse-then-fine progressive refinement (Wave 2 lane 2C).
//!
//! This is the wave's headline test. It stands up an in-memory `.rtla` archive behind a
//! `MemSource` [`TileSource`] that injects a per-tile fetch latency, then proves the
//! progressive-refinement contract of [`StreamedScene`](reticle_app::StreamedScene):
//!
//! 1. A coarse level is resident from before a camera move.
//! 2. The camera moves to a zoomed-in view whose detail level is the finest level.
//!    *Immediately*, before the injected latency elapses, the scene still paints from
//!    the coarse resident level, and no fine tile is resident yet.
//! 3. After the injected fetch latency elapses and the arrived tiles are drained in, the
//!    resident set has transitioned coarse → fine, the painted level is the fine level,
//!    and the painted record set exactly matches the fine-level query.
//!
//! The async fetches are driven by a tiny park-based executor ([`block_on`]) so the test
//! needs no async runtime dependency; on native the app would drive them on a runtime
//! task and on wasm through `wasm_bindgen_futures::spawn_local` (see
//! `reticle_app::streamed::spawn_fetch`). The latency is real wall-clock time, and the
//! test asserts the elapsed fetch time was at least the injected latency, so the "the
//! delay actually happened" claim is measured, not assumed.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

use reticle_app::{StreamedScene, TileInbox, fetch_tile};
use reticle_geometry::{Point, Rect};
use reticle_index::streaming::ArchivableRect;
use reticle_index::{
    LevelDims, RTLA_MAGIC, RTLA_VERSION, RtlaHeader, TileCoord, TilePayload, TileRecord,
    TileSource, TileSourceError,
};

// ---------------------------------------------------------------------------
// A minimal park-based executor, so the async TileSource can be driven without an
// async-runtime dependency (the workspace deliberately pulls in none for the app).
// ---------------------------------------------------------------------------

struct ThreadWaker(std::thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

/// Drives `future` to completion on the current thread, re-polling on wake or after a
/// short park (so a self-waking timer future makes progress without a real reactor).
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

/// A future that resolves once a fixed duration has elapsed: the injected fetch latency.
struct Delay {
    ready_at: Instant,
}

impl Delay {
    fn new(duration: Duration) -> Self {
        Self {
            ready_at: Instant::now() + duration,
        }
    }
}

impl Future for Delay {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if Instant::now() >= self.ready_at {
            Poll::Ready(())
        } else {
            // Ask to be re-polled; block_on also parks with a timeout as a backstop.
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

// ---------------------------------------------------------------------------
// MemSource: an in-memory `.rtla` archive with an injected per-tile fetch latency.
// This is the test double lane 2B's `MemTileSource` will later supersede; until 2B is
// on the base, the contract is exercised against the frozen `TileSource` trait here.
// ---------------------------------------------------------------------------

struct MemSource {
    header: RtlaHeader,
    tiles: HashMap<TileCoord, Vec<u8>>,
    latency: Duration,
}

impl TileSource for MemSource {
    async fn header(&self) -> Result<RtlaHeader, TileSourceError> {
        Ok(self.header.clone())
    }

    async fn tile_bytes(&self, coord: TileCoord) -> Result<Vec<u8>, TileSourceError> {
        // Every fetch pays the injected latency before its bytes are available.
        Delay::new(self.latency).await;
        self.tiles
            .get(&coord)
            .cloned()
            .ok_or(TileSourceError::OutOfRange(coord))
    }
}

/// Archives a [`TilePayload`] to its independently-validatable bytes, exactly as the
/// `.rtla` builder (lane 2A) would write one tile.
fn encode(payload: &TilePayload) -> Vec<u8> {
    rkyv::to_bytes::<rkyv::rancor::Error>(payload)
        .expect("archive tile")
        .to_vec()
}

fn rec(layer: u16, rect: Rect) -> TileRecord {
    TileRecord {
        layer,
        datatype: 0,
        rect: ArchivableRect::from_rect(rect),
    }
}

fn r(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
    Rect::new(Point::new(x0, y0), Point::new(x1, y1))
}

/// The world every level tiles: 1000 x 1000 DBU.
fn world() -> Rect {
    r(0, 0, 1000, 1000)
}

/// A three-level power-of-two pyramid header (1x1, 2x2, 4x4).
fn header() -> RtlaHeader {
    RtlaHeader {
        magic: RTLA_MAGIC,
        version: RTLA_VERSION,
        world: ArchivableRect::from_rect(world()),
        dbu_per_micron: 1000,
        levels: vec![
            LevelDims { cols: 1, rows: 1 },
            LevelDims { cols: 2, rows: 2 },
            LevelDims { cols: 4, rows: 4 },
        ],
    }
}

/// The coarse (level 0) tile: one decimated block approximating the busy region.
fn coarse_tile_record() -> TileRecord {
    rec(0, r(100, 100, 400, 400))
}

/// The four fine (level 2) tiles the test viewport covers, each with one exact record
/// fully inside the viewport. Returned in the row-major order `tiles_at` yields:
/// (0,0), (1,0), (0,1), (1,1).
fn fine_tiles() -> Vec<(TileCoord, TileRecord)> {
    vec![
        (fc(0, 0), rec(1, r(120, 120, 180, 180))),
        (fc(1, 0), rec(2, r(300, 120, 360, 180))),
        (fc(0, 1), rec(3, r(120, 300, 180, 360))),
        (fc(1, 1), rec(4, r(300, 300, 360, 360))),
    ]
}

/// A level-2 (fine) tile coordinate.
fn fc(col: u32, row: u32) -> TileCoord {
    TileCoord { level: 2, col, row }
}

/// The single level-0 (coarse) tile coordinate.
fn coarse_coord() -> TileCoord {
    TileCoord {
        level: 0,
        col: 0,
        row: 0,
    }
}

/// Builds the `MemSource` archive: the coarse tile plus the four fine tiles, all behind
/// the given per-tile latency.
fn mem_source(latency: Duration) -> MemSource {
    let mut tiles = HashMap::new();
    tiles.insert(
        coarse_coord(),
        encode(&TilePayload {
            records: vec![coarse_tile_record()],
        }),
    );
    for (coord, record) in fine_tiles() {
        tiles.insert(
            coord,
            encode(&TilePayload {
                records: vec![record],
            }),
        );
    }
    MemSource {
        header: header(),
        tiles,
        latency,
    }
}

#[test]
fn camera_move_paints_coarse_then_swaps_to_fine_after_the_fetch_delay() {
    // A per-tile latency long enough to be unambiguously observable, short enough to
    // keep the test fast.
    let latency = Duration::from_millis(20);
    let source = mem_source(latency);
    let inbox = TileInbox::new();
    let mut scene = StreamedScene::new(source.header.clone(), 64).expect("valid header");

    // --- Open: the coarse level is fetched (through the real source+inbox path) and
    // made resident, standing in for the state before the zoom-in camera move. ---
    block_on(fetch_tile(&source, coarse_coord(), &inbox)).expect("coarse fetch");
    inbox.drain_into(&mut scene);
    assert!(
        scene.is_resident(coarse_coord()),
        "coarse tile resident on open"
    );

    // --- Camera move: zoom in so the detail level is the finest level (2). ---
    let view = r(100, 100, 400, 400);
    // The world is 1000 wide; a 250-DBU target tile selects the 4x4 finest level.
    let target = scene.target_level(250);
    assert_eq!(target, 2, "the zoomed-in view calls for the finest level");

    let missing = scene.missing_tiles(view, target);
    assert_eq!(missing.len(), 4, "all four fine tiles need fetching");
    for coord in &missing {
        assert!(!scene.is_resident(*coord));
    }

    // --- Immediately after the move (fetches not yet complete): paint coarse. ---
    let paint_now = scene
        .paint_level(view, target)
        .expect("something covers the view");
    assert_eq!(
        paint_now, 0,
        "before the fine tiles land, the coarse resident level paints"
    );
    let coarse_painted = scene.painted_records(view, paint_now);
    assert_eq!(
        coarse_painted,
        vec![coarse_tile_record()],
        "the coarse decimated block is what shows first"
    );
    // No fine tile is resident yet.
    for (coord, _) in fine_tiles() {
        assert!(!scene.is_resident(coord), "no fine tile resident pre-delay");
    }

    // --- Drive the fine-tile fetches to completion: each pays the injected latency. ---
    let start = Instant::now();
    block_on(async {
        for coord in &missing {
            fetch_tile(&source, *coord, &inbox)
                .await
                .expect("fine fetch");
        }
    });
    let elapsed = start.elapsed();
    assert!(
        elapsed >= latency,
        "the injected fetch delay actually elapsed: {elapsed:?} >= {latency:?}"
    );
    let drained = inbox.drain_into(&mut scene);
    assert_eq!(drained, 4, "four fine tiles arrived and were adopted");

    // --- After the delay: the resident set transitioned coarse -> fine. ---
    for (coord, _) in fine_tiles() {
        assert!(scene.is_resident(coord), "fine tile resident after delay");
    }
    let paint_after = scene
        .paint_level(view, target)
        .expect("fine level covers view");
    assert_eq!(paint_after, 2, "the fine level now paints");

    // The painted record set matches the fine-level query exactly.
    let fine_painted = scene.painted_records(view, paint_after);
    let expected: Vec<TileRecord> = fine_tiles().into_iter().map(|(_, record)| record).collect();
    assert_eq!(
        fine_painted, expected,
        "painted fine records match the fine-level query"
    );

    // And the swap genuinely changed what is drawn.
    assert_ne!(
        fine_painted, coarse_painted,
        "coarse and fine paints differ; refinement occurred"
    );
}

#[test]
fn a_missing_tile_fetch_reports_out_of_range_and_leaves_the_scene_paintable() {
    // A tile the archive does not carry must error cleanly (never a fake success), and
    // the scene must keep painting whatever coarse level is already resident.
    let source = mem_source(Duration::from_millis(0));
    let inbox = TileInbox::new();
    let mut scene = StreamedScene::new(source.header.clone(), 64).expect("valid header");

    block_on(fetch_tile(&source, coarse_coord(), &inbox)).unwrap();
    inbox.drain_into(&mut scene);

    // Level 2 tile (3,3) is a valid address but was never inserted into the source.
    let absent = fc(3, 3);
    let err = block_on(fetch_tile(&source, absent, &inbox)).unwrap_err();
    assert!(
        matches!(err, TileSourceError::OutOfRange(c) if c == absent),
        "a missing tile is an honest OutOfRange, got {err:?}"
    );
    assert!(inbox.is_empty(), "a failed fetch posts nothing");

    // The coarse level still covers and paints the view.
    let view = r(100, 100, 400, 400);
    assert_eq!(scene.paint_level(view, 2), Some(0));
}
