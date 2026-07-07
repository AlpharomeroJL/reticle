//! Browsing a served `.rtla` archive in the browser (`?archive=<url>`, Wave 2 lane v8-2e).
//!
//! A page opened with `?archive=<url>` (parsed by [`crate::share::archive_url_from_query`])
//! streams the served archive at `<url>` over the HTTP-range
//! [`TileSource`](reticle_index::TileSource) (lane 2B) into a read-only
//! [`DocHost::Streamed`] scene (lane 2C) and browses it
//! with progressive residency: as the camera moves, the tiles covering the viewport are
//! fetched, and the coarsest resident level keeps painting until the fine tiles land
//! (ADR 0062). This module owns the *app-side* state for that browse mode.
//!
//! # What is pure here, and what is glue
//!
//! Following the same discipline as [`crate::streamed`] and [`crate::webopen`], the
//! decision logic is pure and unit-tested with no browser, GPU, or network:
//!
//! * **Pure (tested here):** the HUD counter arithmetic ([`ArchiveStats`]: mean tile
//!   size, working-set estimate, fetched fraction, byte formatting), the target-level
//!   choice for a viewport ([`target_level_for_viewport`]), and the not-yet-resident,
//!   not-yet-in-flight fetch list, coarse level first ([`wanted_tiles`]).
//! * **Glue (`#[cfg(target_arch = "wasm32")]`):** opening an `HttpRangeTileSource`
//!   (`reticle_index::tile_source`), probing the archive's total size with one ranged
//!   GET, and handing each missing tile to
//!   `spawn_fetch`. These need a browser and are proven
//!   by the served-archive Playwright spec, not a headless unit test.
//!
//! # Read-only by construction
//!
//! [`ArchiveBrowse`] holds a [`DocHost`] that is *always* the
//! [`Streamed`](crate::dochost::DocHost::Streamed) arm, which exposes no `&mut History`
//! and no mutation API, so an edit to the streamed die is a compile error rather than a
//! runtime check (ADR 0062/0071). The browse mode reuses the editor camera for pan/zoom,
//! measure, and query, all of which only read.

use std::collections::HashSet;

use reticle_geometry::Rect;
use reticle_index::TileCoord;

use crate::dochost::DocHost;
use crate::streamed::{StreamedScene, TileInbox};

/// Roughly how many tiles should span the larger viewport dimension at the chosen level:
/// the knob trading tile count (and so fetch traffic) for on-screen detail.
const TILES_ACROSS: i64 = 4;

/// The in-memory tile-cache byte budget an `HttpRangeTileSource` keeps in front of the
/// network, so panning back to a recent tile does not re-fetch it. 16 MiB is generous for
/// the handful of tiles a viewport covers while staying well within a browser tab.
#[cfg(target_arch = "wasm32")]
const CACHE_BUDGET_BYTES: usize = 16 * 1024 * 1024;

/// The most tiles the streamed scene keeps resident at once before evicting the
/// least-recently-used. Bounds the working set regardless of how far the camera roams.
#[cfg(target_arch = "wasm32")]
const MAX_RESIDENT_TILES: usize = 256;

/// The most tiles a single residency pass spawns fetches for, so a sudden zoom-out that
/// exposes a large fine level trickles in over a few frames rather than opening hundreds
/// of concurrent requests at once.
#[cfg(target_arch = "wasm32")]
const MAX_FETCH_PER_PASS: usize = 32;

/// The HUD / PERF counters for an archive browse: how much has been fetched, how many
/// tiles are resident, and the derived working-set and mean-tile estimates.
///
/// `bytes_fetched` and `tiles_fetched` are cumulative over the session (a re-fetch after
/// an eviction counts again, honest *network traffic* rather than a snapshot of resident
/// bytes), read straight off the metered [`TileInbox`]. `file_size` comes from a one-shot
/// ranged probe of the archive's `Content-Range` total (`0` when the server did not
/// report one). `working_set_bytes` is an *estimate*: the resident tile count times the
/// mean fetched tile size, because eviction happens inside the scene's LRU where the exact
/// resident byte total is not observable through the frozen [`StreamedScene`] API.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArchiveStats {
    /// The archive's total size in bytes, or `0` when unknown.
    pub file_size: u64,
    /// Cumulative raw transport bytes fetched over the network so far.
    pub bytes_fetched: u64,
    /// The number of tile fetches that have completed.
    pub tiles_fetched: u64,
    /// The tiles currently resident in RAM.
    pub tiles_resident: usize,
    /// The records painted on the most recent streamed frame (drives the HUD's
    /// "painting" readout and the e2e canvas-paint assertion).
    pub records_painted: usize,
}

impl ArchiveStats {
    /// The mean fetched tile size in bytes, or `0` before any tile has been fetched.
    #[must_use]
    pub fn mean_tile_bytes(&self) -> u64 {
        if self.tiles_fetched == 0 {
            0
        } else {
            self.bytes_fetched / self.tiles_fetched
        }
    }

    /// The working-set estimate in bytes: resident tiles times the mean fetched tile
    /// size. An estimate, not an exact resident-byte total (see the type docs).
    #[must_use]
    pub fn working_set_bytes(&self) -> u64 {
        self.tiles_resident as u64 * self.mean_tile_bytes()
    }

    /// The fraction of the archive fetched so far in `0.0..=1.0`, or `None` when the
    /// total size is unknown. Clamped so a re-fetching session cannot report over `1.0`.
    #[must_use]
    pub fn fetched_fraction(&self) -> Option<f32> {
        if self.file_size == 0 {
            return None;
        }
        #[allow(clippy::cast_precision_loss)]
        Some((self.bytes_fetched as f32 / self.file_size as f32).clamp(0.0, 1.0))
    }

    /// The HUD overlay lines: bytes fetched vs archive size, tiles resident, the
    /// working-set estimate, and the frame rate. Pure, so the exact wording is
    /// unit-tested without a window.
    #[must_use]
    pub fn hud_lines(&self, fps: f64) -> Vec<String> {
        let fetched = if self.file_size > 0 {
            let pct = self.fetched_fraction().unwrap_or(0.0) * 100.0;
            format!(
                "fetched {} / {} ({pct:.0}%)",
                fmt_bytes(self.bytes_fetched),
                fmt_bytes(self.file_size)
            )
        } else {
            format!("fetched {}", fmt_bytes(self.bytes_fetched))
        };
        vec![
            "streaming .rtla".to_owned(),
            fetched,
            format!(
                "{} tiles resident · {} records",
                self.tiles_resident, self.records_painted
            ),
            format!("working set ~{}", fmt_bytes(self.working_set_bytes())),
            format!("{fps:.0} fps"),
        ]
    }
}

/// Formats a byte count as a compact human string (`"512 B"`, `"12.3 KiB"`, `"4.0 MiB"`).
#[must_use]
pub fn fmt_bytes(n: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * 1024 * 1024;
    #[allow(clippy::cast_precision_loss)]
    match n {
        0..KIB => format!("{n} B"),
        KIB..MIB => format!("{:.1} KiB", n as f64 / KIB as f64),
        MIB..GIB => format!("{:.1} MiB", n as f64 / MIB as f64),
        _ => format!("{:.1} GiB", n as f64 / GIB as f64),
    }
}

/// The finest level appropriate for `viewport` against `scene`, aiming for roughly
/// `TILES_ACROSS` tiles spanning the larger viewport dimension.
///
/// A zoomed-in (small) viewport yields a small target tile size and so a finer level; a
/// zoomed-out one a coarser level. This is the level [`wanted_tiles`] fetches toward and
/// [`StreamedScene::paint_level`] refines up to.
#[must_use]
pub fn target_level_for_viewport(scene: &StreamedScene, viewport: Rect) -> u32 {
    let extent = viewport.width().max(viewport.height()).max(1);
    let target_tile_dbu = (extent / TILES_ACROSS).max(1);
    scene.target_level(target_tile_dbu)
}

/// The tiles that should be fetched this pass: every tile covering `viewport` from the
/// coarsest level up to `target` that is neither resident nor already in flight, coarse
/// level first.
///
/// Returning the coarse levels first is what makes the coarse-then-fine refinement work:
/// the one or few coarse tiles land quickly and paint immediately (via
/// [`StreamedScene::paint_level`]) while the many fine tiles are still streaming.
#[must_use]
pub fn wanted_tiles<S: std::hash::BuildHasher>(
    scene: &StreamedScene,
    in_flight: &HashSet<TileCoord, S>,
    viewport: Rect,
    target: u32,
) -> Vec<TileCoord> {
    let mut out = Vec::new();
    for level in 0..=target {
        for coord in scene.missing_tiles(viewport, level) {
            if !in_flight.contains(&coord) {
                out.push(coord);
            }
        }
    }
    out
}

/// The app-side state of an open archive browse: the read-only streamed document host,
/// the fetch inbox, the set of tiles whose fetch is in flight (so a tile is not
/// re-requested every frame while its fetch is outstanding), and the HUD counters.
///
/// The [`DocHost`] is always the [`Streamed`](DocHost::Streamed) arm; see the
/// [module docs](self) for the read-only-by-construction guarantee. On wasm it also owns
/// the shared `HttpRangeTileSource` every in-flight fetch reads from.
#[derive(Debug)]
pub struct ArchiveBrowse {
    /// The document host, always [`DocHost::Streamed`].
    host: DocHost,
    /// The mailbox async tile fetches post decoded tiles into; drained each frame.
    inbox: TileInbox,
    /// Tiles whose fetch is outstanding, to avoid re-requesting them every frame.
    in_flight: HashSet<TileCoord>,
    /// The HUD / PERF counters.
    stats: ArchiveStats,
    /// The shared HTTP-range source every in-flight fetch reads from (wasm only).
    #[cfg(target_arch = "wasm32")]
    source: std::rc::Rc<reticle_index::tile_source::HttpRangeTileSource>,
}

impl ArchiveBrowse {
    /// Builds an archive browse over `scene`, streaming from `source`, whose total size
    /// is `file_size` bytes (`0` if unknown). wasm only: the source is the browser's
    /// HTTP-range fetcher.
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn new(
        scene: StreamedScene,
        source: std::rc::Rc<reticle_index::tile_source::HttpRangeTileSource>,
        file_size: u64,
    ) -> Self {
        Self {
            host: DocHost::streamed(scene),
            inbox: TileInbox::new(),
            in_flight: HashSet::new(),
            stats: ArchiveStats {
                file_size,
                ..ArchiveStats::default()
            },
            source,
        }
    }

    /// Builds an archive browse over `scene` with no network source, for native unit
    /// tests of the drain/residency bookkeeping (the fetch glue is wasm-only).
    #[cfg(test)]
    #[must_use]
    fn for_test(scene: StreamedScene, file_size: u64) -> Self {
        Self {
            host: DocHost::streamed(scene),
            inbox: TileInbox::new(),
            in_flight: HashSet::new(),
            stats: ArchiveStats {
                file_size,
                ..ArchiveStats::default()
            },
        }
    }

    /// The read-only streamed scene being browsed.
    #[must_use]
    pub fn scene(&self) -> &StreamedScene {
        // The host is constructed as `Streamed` and never reassigned, so this holds.
        self.host.scene().expect("archive host is always Streamed")
    }

    /// The current HUD / PERF counters.
    #[must_use]
    pub fn stats(&self) -> &ArchiveStats {
        &self.stats
    }

    /// The fetch inbox, exposed so a native driver (a test) can post fetched tiles.
    #[must_use]
    pub fn inbox(&self) -> &TileInbox {
        &self.inbox
    }

    /// Adopts every tile fetched since the last call, drops those coordinates from the
    /// in-flight set, and refreshes the resident / fetched counters.
    ///
    /// Call once per frame before painting so freshly-arrived tiles are resident for
    /// this frame's paint. Returns how many tiles were adopted.
    pub fn drain(&mut self) -> usize {
        let scene = self
            .host
            .scene_mut()
            .expect("archive host is always Streamed");
        let adopted = self.inbox.drain_into(scene);
        // A tile that became resident is no longer in flight. (A fetch that errored never
        // becomes resident; such a coordinate stays in flight, which for a well-formed
        // archive does not happen; every covering tile exists.)
        // Borrow the scene through the `host` field directly (not the `scene()` method,
        // which borrows all of `self`) so `in_flight` stays independently mutable.
        let scene = self.host.scene().expect("archive host is always Streamed");
        self.in_flight.retain(|c| !scene.is_resident(*c));
        self.stats.tiles_resident = self.scene().resident_count();
        self.stats.bytes_fetched = self.inbox.fetched_bytes();
        self.stats.tiles_fetched = self.inbox.fetch_count();
        adopted
    }

    /// Records how many records were painted on the current streamed frame (for the HUD
    /// and the e2e canvas-paint assertion).
    pub fn set_records_painted(&mut self, n: usize) {
        self.stats.records_painted = n;
    }

    /// Spawns background fetches for every tile covering `viewport` up to `target` that is
    /// not yet resident or in flight, coarse level first, capped at
    /// [`MAX_FETCH_PER_PASS`] per call (wasm only).
    ///
    /// Each fetch runs on the browser microtask queue and posts its decoded tile back to
    /// the inbox for [`drain`](Self::drain) to adopt next frame.
    #[cfg(target_arch = "wasm32")]
    pub fn spawn_missing(&mut self, viewport: Rect, target: u32) {
        let wanted = {
            let scene = self.scene();
            wanted_tiles(scene, &self.in_flight, viewport, target)
        };
        for coord in wanted.into_iter().take(MAX_FETCH_PER_PASS) {
            self.in_flight.insert(coord);
            crate::streamed::spawn_fetch(
                std::rc::Rc::clone(&self.source),
                coord,
                self.inbox.clone(),
            );
        }
    }
}

/// A single-slot mailbox the async archive-open task posts its finished
/// [`ArchiveBrowse`] (or a failure message) into, for the egui loop to install.
///
/// Mirrors [`WebOpenInbox`](crate::webopen::WebOpenInbox): the open runs on
/// `wasm_bindgen_futures::spawn_local` (it fetches the header and probes the size), so it
/// cannot borrow the App; it assembles the whole [`ArchiveBrowse`] and posts it here, and
/// the App adopts it on the next frame. Cheaply cloneable; the type is uniform across
/// targets (nothing is ever posted on native).
#[derive(Clone, Default, Debug)]
pub struct ArchiveOpenInbox {
    #[cfg(target_arch = "wasm32")]
    inner: std::rc::Rc<std::cell::RefCell<Option<Result<ArchiveBrowse, String>>>>,
}

impl ArchiveOpenInbox {
    /// A new, empty inbox.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Posts the finished open result for the App to install next frame (wasm only).
    #[cfg(target_arch = "wasm32")]
    pub fn post(&self, result: Result<ArchiveBrowse, String>) {
        *self.inner.borrow_mut() = Some(result);
    }

    /// Takes the posted open result, if one has arrived (wasm only).
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn take(&self) -> Option<Result<ArchiveBrowse, String>> {
        self.inner.borrow_mut().take()
    }

    /// Takes the posted open result (native: nothing is ever posted, always `None`).
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn take(&self) -> Option<Result<ArchiveBrowse, String>> {
        None
    }
}

/// Kicks off opening the served archive at `url` on the browser microtask queue: opens
/// the HTTP-range source, probes the archive's total size, fetches and validates the
/// header, builds the streamed scene, and posts the finished [`ArchiveBrowse`] (or a
/// human-readable failure) into `inbox` for the egui loop to install (wasm only).
#[cfg(target_arch = "wasm32")]
pub fn start_archive_open(url: String, inbox: ArchiveOpenInbox, repaint: eframe::egui::Context) {
    // The frozen HttpRangeTileSource caches tiles in OPFS, and OPFS's synchronous
    // FileSystemSyncAccessHandle exists ONLY in a dedicated Web Worker. We browse on the
    // main thread (ADR 0062 wires the source into the main-thread DocHost), so that
    // caching path must be disabled before the first fetch or it aborts every tile.
    disable_main_thread_opfs();
    wasm_bindgen_futures::spawn_local(async move {
        // The total size is best-effort: an archive still streams without it (the HUD
        // just shows bytes fetched with no denominator).
        let file_size = archive_total_size(&url).await.unwrap_or(0);
        let result = open_streamed(&url, file_size).await;
        inbox.post(result);
        repaint.request_repaint();
    });
}

/// Makes OPFS report unavailable on the main thread, so the frozen `HttpRangeTileSource`'s
/// cache path falls back cleanly to the network plus its in-memory LRU (wasm only).
///
/// The source's `opfs_dir()` starts with `navigator.storage.getDirectory()`; overriding
/// that to return a *rejected* promise makes it return `None`, so both the OPFS read and
/// write short-circuit before reaching the worker-only
/// `FileSystemFileHandle.createSyncAccessHandle()` (which throws a synchronous
/// `TypeError` the source's async guards cannot catch). Losing the persistent cross-reload
/// cache is expected here: it genuinely needs a worker, which lane 2D-alpha owns; the
/// in-memory LRU still spares a re-fetch while panning. The override is CSP-safe (no
/// `eval`/`new Function`) and idempotent: re-running it just reinstalls the rejecter.
#[cfg(target_arch = "wasm32")]
fn disable_main_thread_opfs() {
    use wasm_bindgen::JsValue;
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(navigator) = js_sys::Reflect::get(window.as_ref(), &JsValue::from_str("navigator"))
    else {
        return;
    };
    let Ok(storage) = js_sys::Reflect::get(&navigator, &JsValue::from_str("storage")) else {
        return;
    };
    if storage.is_undefined() || storage.is_null() {
        return;
    }
    let reject = wasm_bindgen::closure::Closure::<dyn FnMut() -> js_sys::Promise>::new(|| {
        js_sys::Promise::reject(&JsValue::from_str(
            "OPFS is unavailable on the main thread (needs a Web Worker)",
        ))
    });
    let _ = js_sys::Reflect::set(
        &storage,
        &JsValue::from_str("getDirectory"),
        reject.as_ref(),
    );
    // The rejecter must outlive this call (it is invoked on every later tile fetch).
    reject.forget();
}

/// Opens the source at `url`, reads its header, and builds the streamed scene, returning
/// the assembled browse or a human-readable failure (wasm only).
#[cfg(target_arch = "wasm32")]
async fn open_streamed(url: &str, file_size: u64) -> Result<ArchiveBrowse, String> {
    use reticle_index::TileSource as _;
    use reticle_index::tile_source::HttpRangeTileSource;

    let source = HttpRangeTileSource::open(url, CACHE_BUDGET_BYTES)
        .await
        .map_err(|e| format!("could not open the archive: {e}"))?;
    let header = source
        .header()
        .await
        .map_err(|e| format!("could not read the archive header: {e}"))?;
    let scene = StreamedScene::new(header, MAX_RESIDENT_TILES)
        .map_err(|e| format!("the archive header is not a streamable pyramid: {e}"))?;
    Ok(ArchiveBrowse::new(
        scene,
        std::rc::Rc::new(source),
        file_size,
    ))
}

/// Probes the archive's total byte size with a single `bytes=0-0` ranged GET, reading the
/// total after the slash of the `Content-Range` response header, or `None` on any failure
/// (wasm only). Reuses exactly the `web_sys` surface the tile fetcher already uses, so no
/// new browser feature is needed.
#[cfg(target_arch = "wasm32")]
async fn archive_total_size(url: &str) -> Option<u64> {
    use wasm_bindgen::JsCast as _;
    let window = web_sys::window()?;
    let request = web_sys::Request::new_with_str(url).ok()?;
    request.headers().set("Range", "bytes=0-0").ok()?;
    let resp_value = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request))
        .await
        .ok()?;
    let response: web_sys::Response = resp_value.dyn_into().ok()?;
    // "bytes 0-0/12345" -> "12345".
    let content_range = response.headers().get("Content-Range").ok().flatten()?;
    content_range.rsplit('/').next()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::Point;
    use reticle_index::streaming::ArchivableRect;
    use reticle_index::{LevelDims, RTLA_MAGIC, RTLA_VERSION, RtlaHeader, TilePayload};

    fn world() -> Rect {
        Rect::new(Point::new(0, 0), Point::new(1000, 1000))
    }

    /// A `levels`-deep power-of-two pyramid header over a 1000x1000 world.
    fn header(levels: u32) -> RtlaHeader {
        RtlaHeader {
            magic: RTLA_MAGIC,
            version: RTLA_VERSION,
            world: ArchivableRect::from_rect(world()),
            dbu_per_micron: 1000,
            levels: (0..levels)
                .map(|l| LevelDims {
                    cols: 1 << l,
                    rows: 1 << l,
                })
                .collect(),
        }
    }

    fn scene(levels: u32) -> StreamedScene {
        StreamedScene::new(header(levels), 64).expect("valid header")
    }

    #[test]
    fn fmt_bytes_scales_by_unit() {
        assert_eq!(fmt_bytes(0), "0 B");
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(1024), "1.0 KiB");
        assert_eq!(fmt_bytes(1536), "1.5 KiB");
        assert_eq!(fmt_bytes(2 * 1024 * 1024), "2.0 MiB");
        assert_eq!(fmt_bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }

    #[test]
    fn stats_derive_mean_working_set_and_fraction() {
        let stats = ArchiveStats {
            file_size: 1000,
            bytes_fetched: 400,
            tiles_fetched: 4,
            tiles_resident: 3,
            records_painted: 12,
        };
        // Mean tile = 400 / 4 = 100; working set estimate = 3 resident * 100.
        assert_eq!(stats.mean_tile_bytes(), 100);
        assert_eq!(stats.working_set_bytes(), 300);
        assert_eq!(stats.fetched_fraction(), Some(0.4));
    }

    #[test]
    fn stats_before_any_fetch_and_without_a_size_are_graceful() {
        let empty = ArchiveStats::default();
        assert_eq!(empty.mean_tile_bytes(), 0);
        assert_eq!(empty.working_set_bytes(), 0);
        assert_eq!(
            empty.fetched_fraction(),
            None,
            "unknown size => no fraction"
        );
        // The HUD still renders without a denominator, never dividing by zero.
        let lines = empty.hud_lines(60.0);
        assert!(lines.iter().any(|l| l.contains("streaming .rtla")));
        assert!(lines.iter().any(|l| l.starts_with("fetched 0 B")));
        assert!(lines.iter().any(|l| l.contains("60 fps")));
    }

    #[test]
    fn fetched_fraction_clamps_over_report() {
        let stats = ArchiveStats {
            file_size: 100,
            bytes_fetched: 250,
            ..ArchiveStats::default()
        };
        assert_eq!(
            stats.fetched_fraction(),
            Some(1.0),
            "a re-fetch cannot exceed 1"
        );
    }

    #[test]
    fn target_level_gets_finer_as_the_viewport_shrinks() {
        let scene = scene(4); // levels 0..=3 (1,2,4,8 across)
        let whole = target_level_for_viewport(&scene, world());
        let quarter =
            target_level_for_viewport(&scene, Rect::new(Point::new(0, 0), Point::new(250, 250)));
        let tiny =
            target_level_for_viewport(&scene, Rect::new(Point::new(0, 0), Point::new(30, 30)));
        assert!(
            quarter >= whole && tiny >= quarter,
            "a smaller viewport calls for an equal-or-finer level: {whole} {quarter} {tiny}"
        );
        assert!(
            tiny > whole,
            "a tiny viewport is strictly finer than the whole world"
        );
    }

    #[test]
    fn wanted_tiles_returns_coarse_first_and_skips_resident_and_in_flight() {
        let mut scene = scene(3); // levels 0 (1x1), 1 (2x2), 2 (4x4)
        let view = world();
        // Nothing resident, nothing in flight: every covering tile from level 0 up is
        // wanted, coarsest first, so the first entry is the single level-0 tile.
        let in_flight = HashSet::new();
        let wanted = wanted_tiles(&scene, &in_flight, view, 2);
        assert_eq!(
            wanted.first().unwrap().level,
            0,
            "coarse level requested first"
        );
        assert!(
            wanted.iter().any(|c| c.level == 2),
            "fine tiles are wanted too"
        );

        // Make the coarse tile resident and mark one fine tile in flight: neither appears.
        let coarse = TileCoord {
            level: 0,
            col: 0,
            row: 0,
        };
        scene.insert_tile(coarse, TilePayload::default());
        let a_fine = TileCoord {
            level: 2,
            col: 0,
            row: 0,
        };
        let mut in_flight = HashSet::new();
        in_flight.insert(a_fine);
        let wanted = wanted_tiles(&scene, &in_flight, view, 2);
        assert!(
            !wanted.contains(&coarse),
            "a resident tile is not re-wanted"
        );
        assert!(
            !wanted.contains(&a_fine),
            "an in-flight tile is not re-wanted"
        );
    }

    #[test]
    fn browse_drain_adopts_tiles_and_clears_them_from_in_flight() {
        let mut browse = ArchiveBrowse::for_test(scene(3), 4096);
        let coord = TileCoord {
            level: 0,
            col: 0,
            row: 0,
        };
        // Simulate a fetch: a metered post, and the coordinate marked in flight.
        browse.in_flight.insert(coord);
        browse.inbox().post_metered(
            coord,
            TilePayload {
                records: Vec::new(),
            },
            256,
        );
        assert_eq!(
            browse.stats().tiles_resident,
            0,
            "not adopted until drained"
        );

        let adopted = browse.drain();
        assert_eq!(adopted, 1);
        assert_eq!(browse.stats().tiles_resident, 1);
        assert_eq!(browse.stats().bytes_fetched, 256);
        assert_eq!(browse.stats().tiles_fetched, 1);
        assert!(
            browse.scene().is_resident(coord),
            "the fetched tile is resident"
        );
        assert!(
            !browse.in_flight.contains(&coord),
            "an adopted tile leaves the in-flight set"
        );

        browse.set_records_painted(7);
        assert_eq!(browse.stats().records_painted, 7);
    }
}
