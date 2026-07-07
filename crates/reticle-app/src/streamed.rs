//! The streamed document scene: async tile residency with coarse-then-fine paint.
//!
//! A `.rtla` archive (ADR 0062) is a network transport for renderable silicon at
//! many zoom levels. This module drives the *client* side of that transport: as the
//! camera moves, it works out which tiles cover the viewport at the level the zoom
//! calls for, fetches the ones that are not already resident over a
//! [`TileSource`], and, crucially, keeps painting the
//! coarsest resident level that still covers the viewport until the fine tiles land.
//! That is *progressive refinement*: the screen is never blank while a fetch is in
//! flight, it just shows less detail for a moment.
//!
//! # Read-mostly by construction
//!
//! A [`StreamedScene`] holds a header, a [`TileSource`], and a working set of resident
//! tiles. It has **no** mutation path: there is no `apply`, no `&mut` into a document,
//! nothing an editing tool could grab. Editing stays on the in-RAM
//! [`History`](crate::history::History) document, reached only through the
//! [`Edited`](crate::dochost::DocHost::Edited) arm of a
//! [`DocHost`](crate::dochost::DocHost). Trying to edit a streamed document is therefore
//! a compile error, not a runtime check (ADR 0062, ADR 0071).
//!
//! # What is pure here, and what is glue
//!
//! Following the same split as [`crate::webopen`], the interesting behaviour is pure
//! and unit-tested with no GPU, window, or network:
//!
//! * **Pure (tested here and in `tests/residency.rs`):** the viewport → tiles mapping
//!   ([`StreamedScene::tiles_at`]), the target-level choice
//!   ([`StreamedScene::target_level`]), the progressive paint-level selection
//!   ([`StreamedScene::paint_level`]), the painted record set
//!   ([`StreamedScene::painted_records`]), residency with an LRU working-set bound
//!   ([`StreamedScene::insert_tile`]), and the async fetch → [`TileInbox`] → drain
//!   pipeline driven against a latency-injecting `MemSource`.
//! * **Glue (compiled, exercised in the app, not unit-tested):** the wasm
//!   `spawn_fetch` that hands a fetch to `wasm_bindgen_futures::spawn_local`, and the
//!   [`upload_tile_bytes`] passthrough to the render crate's
//!   [`BufferPages`](reticle_render::BufferPages): both need a browser or a GPU device
//!   that a headless test does not have (lane 1E owns the GPU this batch).

use std::collections::HashMap;

use reticle_geometry::Rect;
use reticle_index::{
    LodPyramid, RtlaHeader, TileCoord, TileId, TilePayload, TileRecord, TileSource,
};

/// Why a [`StreamedScene`] could not be built from a header.
///
/// Every field of a fetched header is untrusted until checked (the contract's standing
/// hardening lesson), so construction validates the magic, the version, and that the
/// per-level grid is the power-of-two square pyramid the coordinate mapper assumes,
/// refusing with a clear error rather than incorrect mapping a viewport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneError {
    /// The header magic did not match [`reticle_index::RTLA_MAGIC`].
    BadMagic,
    /// The header named a format version this build does not read.
    UnsupportedVersion(u32),
    /// The header carried no levels, or more than the mapper supports (`1..=31`).
    LevelCount(usize),
    /// The world bounding box has zero or negative area, so it cannot be tiled.
    DegenerateWorld,
    /// Level `level` was not the `2^level x 2^level` square grid this reader maps
    /// against; the v1 archives this build reads are power-of-two pyramids.
    NonPyramidLevel {
        /// The offending level index.
        level: usize,
        /// The `2^level` side count both `cols` and `rows` were expected to equal.
        expected: u32,
        /// The `cols` the header actually carried at this level.
        cols: u32,
        /// The `rows` the header actually carried at this level.
        rows: u32,
    },
}

impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMagic => write!(f, "rtla header magic mismatch"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported rtla version {v}"),
            Self::LevelCount(n) => write!(f, "rtla header has an unusable level count: {n}"),
            Self::DegenerateWorld => write!(f, "rtla world bounding box has no area"),
            Self::NonPyramidLevel {
                level,
                expected,
                cols,
                rows,
            } => write!(
                f,
                "rtla level {level} is not a {expected}x{expected} grid (got {cols}x{rows})"
            ),
        }
    }
}

impl std::error::Error for SceneError {}

/// A read-only streamed scene: an `.rtla` header, the tiles currently resident in RAM,
/// and the coordinate mapping that turns a viewport into the tiles that cover it.
///
/// The scene never fetches on its own; a caller drives residency by asking it which
/// tiles are [missing](StreamedScene::missing_tiles) for the current viewport, fetching
/// those over a [`TileSource`] (see [`fetch_tile`]), and handing the results back with
/// [`insert_tile`](StreamedScene::insert_tile). Meanwhile the caller paints whatever
/// [`paint_level`](StreamedScene::paint_level) reports (the finest resident level that
/// still fully covers the viewport) so the view degrades to coarse detail rather than
/// to nothing while fine tiles stream in.
#[derive(Debug)]
pub struct StreamedScene {
    header: RtlaHeader,
    /// A shape-free [`LodPyramid`] built over the world bounds purely as a viewport →
    /// tile-index mapper; its tile contents come from the archive, not from here.
    mapper: LodPyramid,
    /// The decoded tiles held in RAM, keyed by address.
    resident: HashMap<TileCoord, TilePayload>,
    /// Residency order, least-recently-used at the front, so the working set can be
    /// trimmed to [`Self::max_resident`] without unbounded growth.
    lru: Vec<TileCoord>,
    /// The most tiles kept resident at once; the least-recently-used are evicted past
    /// it (and simply re-fetched if the camera returns to them).
    max_resident: usize,
}

impl StreamedScene {
    /// Builds a scene from a validated header, keeping at most `max_resident` tiles
    /// resident at once.
    ///
    /// # Errors
    ///
    /// Returns a [`SceneError`] if the header magic or version is wrong, the level
    /// count is outside `1..=31`, the world has no area, or any level is not the
    /// `2^level x 2^level` square grid this reader maps against.
    pub fn new(header: RtlaHeader, max_resident: usize) -> Result<Self, SceneError> {
        if header.magic != reticle_index::RTLA_MAGIC {
            return Err(SceneError::BadMagic);
        }
        if header.version != reticle_index::RTLA_VERSION {
            return Err(SceneError::UnsupportedVersion(header.version));
        }
        let level_count = header.level_count();
        if level_count == 0 || level_count > 31 {
            return Err(SceneError::LevelCount(level_count));
        }
        let world = header.world_rect();
        if world.width() <= 0 || world.height() <= 0 {
            return Err(SceneError::DegenerateWorld);
        }
        for (level, dims) in header.levels.iter().enumerate() {
            // level_count <= 31, so `level <= 30` and the shift cannot overflow.
            let expected = 1u32 << level;
            if dims.cols != expected || dims.rows != expected {
                return Err(SceneError::NonPyramidLevel {
                    level,
                    expected,
                    cols: dims.cols,
                    rows: dims.rows,
                });
            }
        }
        // The mapper is a pyramid over the same world with the same level count and no
        // shapes: `visible_tiles`/`level_for_viewport` depend only on world+level grid.
        let mapper = LodPyramid::build(world, level_count as u32, &[]);
        Ok(Self {
            header,
            mapper,
            resident: HashMap::new(),
            lru: Vec::new(),
            max_resident: max_resident.max(1),
        })
    }

    /// The archive header.
    #[must_use]
    pub fn header(&self) -> &RtlaHeader {
        &self.header
    }

    /// The world bounding box every level tiles.
    #[must_use]
    pub fn world(&self) -> Rect {
        self.header.world_rect()
    }

    /// The number of pyramid levels (`0` is coarsest, `level_count - 1` is finest).
    #[must_use]
    pub fn level_count(&self) -> u32 {
        self.header.level_count() as u32
    }

    /// The number of tiles currently resident in RAM.
    #[must_use]
    pub fn resident_count(&self) -> usize {
        self.resident.len()
    }

    /// The working-set bound: the most tiles kept resident at once.
    #[must_use]
    pub fn max_resident(&self) -> usize {
        self.max_resident
    }

    /// Whether the tile at `coord` is resident.
    #[must_use]
    pub fn is_resident(&self, coord: TileCoord) -> bool {
        self.resident.contains_key(&coord)
    }

    /// The finest level appropriate for a viewport whose tiles should be about
    /// `target_tile_dbu` across, clamped to the available level range.
    ///
    /// A caller typically passes the viewport extent divided by the number of screen
    /// tiles it wants, trading tile count for detail. This is the level the residency
    /// pass fetches toward; [`paint_level`](Self::paint_level) may report a coarser one
    /// until those fine tiles arrive.
    #[must_use]
    pub fn target_level(&self, target_tile_dbu: i64) -> u32 {
        self.mapper.level_for_viewport(target_tile_dbu)
    }

    /// The tiles at `level` whose bounds intersect `viewport`, in row-major order.
    #[must_use]
    pub fn tiles_at(&self, viewport: Rect, level: u32) -> Vec<TileCoord> {
        self.mapper
            .visible_tiles(viewport, level)
            .into_iter()
            .map(tile_id_to_coord)
            .collect()
    }

    /// The tiles at `level` covering `viewport` that are not yet resident: the fetch
    /// list for a residency pass.
    #[must_use]
    pub fn missing_tiles(&self, viewport: Rect, level: u32) -> Vec<TileCoord> {
        self.tiles_at(viewport, level)
            .into_iter()
            .filter(|coord| !self.resident.contains_key(coord))
            .collect()
    }

    /// The finest level, no finer than `target`, whose every viewport-covering tile is
    /// resident, the level to paint right now, or `None` if not even the coarsest
    /// level is fully covered yet.
    ///
    /// Coarser levels have fewer, larger tiles, so they are far likelier to be fully
    /// resident; this walks from `target` down to `0` and returns the first fully-covered
    /// level. That is the mechanism behind coarse-then-fine: immediately after a camera
    /// move only a coarse level is complete, and this returns it; once the fine tiles
    /// arrive it returns `target`.
    #[must_use]
    pub fn paint_level(&self, viewport: Rect, target: u32) -> Option<u32> {
        let target = target.min(self.level_count().saturating_sub(1));
        for level in (0..=target).rev() {
            let tiles = self.tiles_at(viewport, level);
            if !tiles.is_empty() && tiles.iter().all(|c| self.resident.contains_key(c)) {
                return Some(level);
            }
        }
        None
    }

    /// The records painted for `viewport` from the resident tiles at `level`: every
    /// resident record at that level whose rectangle intersects the viewport.
    ///
    /// Pass the level [`paint_level`](Self::paint_level) chose. A record that straddles
    /// a tile border and so appears in more than one tile is returned once per tile it
    /// was archived into; a caller that needs a strict set can dedupe, but the residency
    /// proof compares this against the same-shaped reference query, so any such
    /// multiplicity matches on both sides.
    #[must_use]
    pub fn painted_records(&self, viewport: Rect, level: u32) -> Vec<TileRecord> {
        let mut out = Vec::new();
        for coord in self.tiles_at(viewport, level) {
            if let Some(payload) = self.resident.get(&coord) {
                for record in &payload.records {
                    if record.rect.to_rect().intersects(&viewport) {
                        out.push(*record);
                    }
                }
            }
        }
        out
    }

    /// Marks the tile at `coord` resident with `payload`, refreshing its recency and
    /// evicting the least-recently-used tiles if the working set now exceeds the bound.
    pub fn insert_tile(&mut self, coord: TileCoord, payload: TilePayload) {
        self.resident.insert(coord, payload);
        self.touch(coord);
        self.evict_to_bound();
    }

    /// Empties the resident set (e.g. on opening a different archive).
    pub fn clear(&mut self) {
        self.resident.clear();
        self.lru.clear();
    }

    /// Moves `coord` to the most-recently-used end of the recency list.
    fn touch(&mut self, coord: TileCoord) {
        self.lru.retain(|c| *c != coord);
        self.lru.push(coord);
    }

    /// Evicts least-recently-used tiles until the resident set fits the bound.
    fn evict_to_bound(&mut self) {
        while self.resident.len() > self.max_resident && !self.lru.is_empty() {
            let victim = self.lru.remove(0);
            self.resident.remove(&victim);
        }
    }
}

/// Converts a mapper [`TileId`] into an archive [`TileCoord`] (identical grid address,
/// different type: the mapper and the archive were designed in separate lanes).
#[must_use]
fn tile_id_to_coord(id: TileId) -> TileCoord {
    TileCoord {
        level: id.level,
        col: id.x,
        row: id.y,
    }
}

/// A single-slot mailbox that async tile fetches post decoded tiles into and the scene
/// drains on the next frame.
///
/// This is the one seam between the async fetch world (a `spawn_local` task on wasm, a
/// driven future in a native test) and the synchronous egui loop, exactly like
/// [`WebOpenInbox`](crate::webopen::WebOpenInbox): a spawned fetch cannot borrow the
/// scene, so it posts here and the loop applies the result with
/// [`drain_into`](TileInbox::drain_into). Cheaply cloneable (a shared handle), so every
/// in-flight fetch holds its own clone.
#[derive(Clone, Default, Debug)]
pub struct TileInbox {
    inner: std::rc::Rc<std::cell::RefCell<std::collections::VecDeque<(TileCoord, TilePayload)>>>,
}

impl TileInbox {
    /// A new, empty inbox.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Posts a decoded tile for the scene to adopt next frame.
    pub fn post(&self, coord: TileCoord, payload: TilePayload) {
        self.inner.borrow_mut().push_back((coord, payload));
    }

    /// Whether no fetched tiles are waiting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().is_empty()
    }

    /// The number of fetched tiles waiting to be drained.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.borrow().len()
    }

    /// Drains every waiting tile into `scene`, returning how many were adopted.
    pub fn drain_into(&self, scene: &mut StreamedScene) -> usize {
        let drained: Vec<(TileCoord, TilePayload)> = self.inner.borrow_mut().drain(..).collect();
        let n = drained.len();
        for (coord, payload) in drained {
            scene.insert_tile(coord, payload);
        }
        n
    }
}

/// Fetches one tile's bytes over `source`, validates and decodes them into a
/// [`TilePayload`], and posts it to `inbox`.
///
/// The bytes are an independently-archived tile (ADR 0062), so they are validated with
/// `rkyv` exactly as the mmap path is: a truncated or corrupt tile yields
/// [`TileSourceError::Malformed`](reticle_index::TileSourceError) rather than undefined
/// behaviour. This is the portable core the wasm `spawn_fetch` and the native test
/// driver both call.
///
/// # Errors
///
/// Propagates a transport error from `source`, or reports a decode failure as
/// [`TileSourceError::Malformed`](reticle_index::TileSourceError).
pub async fn fetch_tile<S: TileSource>(
    source: &S,
    coord: TileCoord,
    inbox: &TileInbox,
) -> Result<(), reticle_index::TileSourceError> {
    let bytes = source.tile_bytes(coord).await?;
    let payload = rkyv::from_bytes::<TilePayload, rkyv::rancor::Error>(&bytes)
        .map_err(|e| reticle_index::TileSourceError::Malformed(e.to_string()))?;
    inbox.post(coord, payload);
    Ok(())
}

/// Spawns a background fetch of `coord` on the browser microtask queue, posting the
/// result into `inbox` for the egui loop to drain (wasm only).
///
/// `TileSource`'s wasm implementation fetches over the network, which cannot block the
/// UI thread, so each tile is handed to `wasm_bindgen_futures::spawn_local`. The source
/// is shared (`Rc`) and moved into the task so the future owns everything it needs and
/// is `'static`. A fetch error is swallowed here: the tile simply stays non-resident and
/// the coarser level keeps painting, and any user-facing surfacing is the caller's job.
#[cfg(target_arch = "wasm32")]
pub fn spawn_fetch<S>(source: std::rc::Rc<S>, coord: TileCoord, inbox: TileInbox)
where
    S: TileSource + 'static,
{
    wasm_bindgen_futures::spawn_local(async move {
        let _ = fetch_tile(source.as_ref(), coord, &inbox).await;
    });
}

/// Uploads a resident tile's already-encoded vertex bytes into the render crate's
/// [`BufferPages`](reticle_render::BufferPages), returning the allocation.
///
/// This is the *only* way lane 2C touches the GPU: a thin passthrough to the existing
/// [`BufferPages::upload`](reticle_render::BufferPages::upload) API (2C changes no render
/// pipeline). The record → vertex encoding is the renderer's existing job, shared with
/// the in-RAM document path; this seam just moves the finished bytes onto a page. It
/// needs a live `wgpu` device and so is exercised in the running app, not in a headless
/// unit test (lane 1E holds the GPU this batch).
pub fn upload_tile_bytes(
    pages: &mut reticle_render::BufferPages,
    device: &egui_wgpu::wgpu::Device,
    queue: &egui_wgpu::wgpu::Queue,
    bytes: &[u8],
) -> Option<reticle_render::Allocation> {
    pages.upload(device, queue, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::Point;
    use reticle_index::streaming::ArchivableRect;
    use reticle_index::{LevelDims, RTLA_MAGIC, RTLA_VERSION};

    /// A `world`-sized header with `levels` power-of-two square levels.
    fn header(world: Rect, levels: u32) -> RtlaHeader {
        RtlaHeader {
            magic: RTLA_MAGIC,
            version: RTLA_VERSION,
            world: ArchivableRect::from_rect(world),
            dbu_per_micron: 1000,
            levels: (0..levels)
                .map(|l| LevelDims {
                    cols: 1 << l,
                    rows: 1 << l,
                })
                .collect(),
        }
    }

    fn record(rect: Rect) -> TileRecord {
        TileRecord {
            layer: 68,
            datatype: 20,
            rect: ArchivableRect::from_rect(rect),
        }
    }

    fn world() -> Rect {
        Rect::new(Point::new(0, 0), Point::new(1000, 1000))
    }

    #[test]
    fn new_rejects_a_bad_header() {
        let mut h = header(world(), 3);
        h.magic = *b"NOTRTLA1";
        assert_eq!(StreamedScene::new(h, 8).unwrap_err(), SceneError::BadMagic);

        let mut h = header(world(), 3);
        h.version = 999;
        assert_eq!(
            StreamedScene::new(h, 8).unwrap_err(),
            SceneError::UnsupportedVersion(999)
        );

        // A level whose grid is not 2^level square is refused rather than mapped incorrectly.
        let mut h = header(world(), 3);
        h.levels[2] = LevelDims { cols: 3, rows: 4 };
        assert_eq!(
            StreamedScene::new(h, 8).unwrap_err(),
            SceneError::NonPyramidLevel {
                level: 2,
                expected: 4,
                cols: 3,
                rows: 4,
            }
        );
    }

    #[test]
    fn tiles_at_maps_the_viewport_to_the_level_grid() {
        let scene = StreamedScene::new(header(world(), 3), 32).unwrap();
        // Level 0 is a single tile covering the whole world.
        assert_eq!(
            scene.tiles_at(world(), 0),
            vec![TileCoord {
                level: 0,
                col: 0,
                row: 0
            }]
        );
        // A viewport in the lower-left quarter hits exactly one level-1 tile.
        let quarter = Rect::new(Point::new(10, 10), Point::new(400, 400));
        assert_eq!(
            scene.tiles_at(quarter, 1),
            vec![TileCoord {
                level: 1,
                col: 0,
                row: 0
            }]
        );
    }

    #[test]
    fn paint_level_falls_back_to_the_coarsest_resident_cover() {
        let mut scene = StreamedScene::new(header(world(), 3), 32).unwrap();
        let view = Rect::new(Point::new(10, 10), Point::new(200, 200));
        // Nothing resident yet: nothing to paint.
        assert_eq!(scene.paint_level(view, 2), None);

        // Make level 0 (the single coarse tile) resident.
        scene.insert_tile(
            TileCoord {
                level: 0,
                col: 0,
                row: 0,
            },
            TilePayload::default(),
        );
        // Target is the fine level 2, but only level 0 is covered, so we paint level 0.
        assert_eq!(scene.paint_level(view, 2), Some(0));

        // Now make every level-2 tile the viewport touches resident.
        for coord in scene.missing_tiles(view, 2) {
            scene.insert_tile(coord, TilePayload::default());
        }
        // The fine level is fully covered, so it wins.
        assert_eq!(scene.paint_level(view, 2), Some(2));
    }

    #[test]
    fn painted_records_returns_resident_records_intersecting_the_viewport() {
        let mut scene = StreamedScene::new(header(world(), 2), 32).unwrap();
        let inside = record(Rect::new(Point::new(20, 20), Point::new(40, 40)));
        let outside = record(Rect::new(Point::new(900, 900), Point::new(950, 950)));
        scene.insert_tile(
            TileCoord {
                level: 0,
                col: 0,
                row: 0,
            },
            TilePayload {
                records: vec![inside, outside],
            },
        );
        let view = Rect::new(Point::new(0, 0), Point::new(100, 100));
        let painted = scene.painted_records(view, 0);
        assert_eq!(painted, vec![inside], "only the intersecting record paints");
    }

    #[test]
    fn insert_evicts_least_recently_used_past_the_bound() {
        let mut scene = StreamedScene::new(header(world(), 3), 2).unwrap();
        let a = TileCoord {
            level: 2,
            col: 0,
            row: 0,
        };
        let b = TileCoord {
            level: 2,
            col: 1,
            row: 0,
        };
        let c = TileCoord {
            level: 2,
            col: 2,
            row: 0,
        };
        scene.insert_tile(a, TilePayload::default());
        scene.insert_tile(b, TilePayload::default());
        // Touch `a` so `b` becomes the least-recently-used.
        scene.insert_tile(a, TilePayload::default());
        scene.insert_tile(c, TilePayload::default());
        assert_eq!(scene.resident_count(), 2);
        assert!(scene.is_resident(a), "a was touched, stays resident");
        assert!(scene.is_resident(c), "c is newest");
        assert!(!scene.is_resident(b), "b was the LRU and was evicted");
    }

    #[test]
    fn inbox_drains_posted_tiles_into_the_scene() {
        let mut scene = StreamedScene::new(header(world(), 2), 32).unwrap();
        let inbox = TileInbox::new();
        let coord = TileCoord {
            level: 0,
            col: 0,
            row: 0,
        };
        inbox.post(coord, TilePayload::default());
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox.drain_into(&mut scene), 1);
        assert!(inbox.is_empty());
        assert!(scene.is_resident(coord));
    }
}
