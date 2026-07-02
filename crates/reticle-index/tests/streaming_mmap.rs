//! Tests for the memory-mapped, tile-organized [`StreamingIndex`].
//!
//! These are gated on `#[cfg(not(miri))]`: Miri cannot memory-map a real file. Each
//! test writes a tiled archive to a unique temp file (via `std::env::temp_dir`),
//! opens it through `mmap`, and asserts that `query_region` returns exactly the same
//! set of entries a brute-force linear scan (the [`LinearIndex`] oracle) returns, for
//! several random regions plus the empty and whole-world edge cases.

#![cfg(not(miri))]

use std::collections::BTreeSet;
use std::path::PathBuf;

use reticle_geometry::{Point, Rect, SpatialIndex};
use reticle_index::LinearIndex;
use reticle_index::streaming::{StreamingIndex, TiledPayload};

/// A tiny deterministic xorshift PRNG (matches the crate's benches).
struct XorShift(u64);

impl XorShift {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A coordinate in `[-bound, bound)`.
    fn coord(&mut self, bound: i32) -> i32 {
        (self.next_u64() % (2 * bound as u64)) as i32 - bound
    }
}

const BOUND: i32 = 10_000;

fn world() -> Rect {
    Rect::new(Point::new(-BOUND, -BOUND), Point::new(BOUND, BOUND))
}

/// `count` random small rectangles within `[-BOUND, BOUND)`.
fn random_entries(seed: u64, count: usize) -> Vec<(Rect, u32)> {
    let mut rng = XorShift::new(seed);
    (0..count)
        .map(|i| {
            let x = rng.coord(BOUND);
            let y = rng.coord(BOUND);
            let w = (rng.next_u64() % 60 + 1) as i32;
            let h = (rng.next_u64() % 60 + 1) as i32;
            (
                Rect::new(Point::new(x, y), Point::new(x + w, y + h)),
                i as u32,
            )
        })
        .collect()
}

/// A self-deleting temp file path, unique per (pid, name). Removed on drop so tests
/// leave nothing behind even on failure.
struct TempPath(PathBuf);

impl TempPath {
    fn new(name: &str) -> Self {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "reticle_mmap_test_{}_{}_{name}.rkyv",
            std::process::id(),
            // A per-instance nonce so parallel tests never collide.
            NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        Self(p)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Writes a tiled archive of `entries` to a fresh temp file and returns the handle.
fn write_archive(name: &str, grid_n: u32, entries: &[(Rect, u32)]) -> TempPath {
    let payload = TiledPayload::build(world(), grid_n, entries.iter().copied());
    let bytes = payload.serialize().expect("serialize tiled payload");
    let temp = TempPath::new(name);
    std::fs::write(temp.path(), &bytes).expect("write archive to temp file");
    temp
}

/// Sorted id set from a mmap query result.
fn mmap_ids(index: &StreamingIndex, region: Rect) -> BTreeSet<u32> {
    index
        .query_region(region)
        .into_iter()
        .map(|(_, id)| id)
        .collect()
}

/// Sorted id set from the brute-force [`LinearIndex`] oracle.
fn oracle_ids(entries: &[(Rect, u32)], region: Rect) -> BTreeSet<u32> {
    let oracle = LinearIndex::bulk_load(entries.iter().map(|(r, id)| (*r, *id)));
    oracle.query_rect(region).into_iter().copied().collect()
}

#[test]
fn query_matches_oracle_over_random_regions() {
    let entries = random_entries(0xA11CE, 40_000);
    let temp = write_archive("random", 32, &entries);
    let index = StreamingIndex::open(temp.path()).expect("open mmap index");

    assert_eq!(index.total_entries(), entries.len());

    let mut rng = XorShift::new(0xBEEF);
    for _ in 0..64 {
        // A random query rect of positive area, sometimes reaching outside the world.
        let x = rng.coord(BOUND + 2_000);
        let y = rng.coord(BOUND + 2_000);
        let w = (rng.next_u64() % 4_000 + 1) as i32;
        let h = (rng.next_u64() % 4_000 + 1) as i32;
        let region = Rect::new(Point::new(x, y), Point::new(x + w, y + h));

        assert_eq!(
            mmap_ids(&index, region),
            oracle_ids(&entries, region),
            "mismatch for region {region:?}"
        );
    }
}

#[test]
fn whole_world_region_returns_all_entries() {
    let entries = random_entries(0xF00D, 20_000);
    let temp = write_archive("wholeworld", 16, &entries);
    let index = StreamingIndex::open(temp.path()).expect("open mmap index");

    // A region covering (and slightly exceeding) the world must return every entry.
    let big = Rect::new(
        Point::new(-BOUND - 100, -BOUND - 100),
        Point::new(BOUND + 100, BOUND + 100),
    );
    let got = mmap_ids(&index, big);
    let expected = oracle_ids(&entries, big);
    assert_eq!(got, expected);
    assert_eq!(
        got.len(),
        entries.len(),
        "every entry lies within the world"
    );
}

#[test]
fn empty_region_returns_nothing_and_touches_nothing() {
    let entries = random_entries(0x1234, 10_000);
    let temp = write_archive("empty", 16, &entries);
    let index = StreamingIndex::open(temp.path()).expect("open mmap index");

    // A degenerate (zero-area) region: `intersects` requires positive-area overlap,
    // so nothing matches, and the query should not scan any tile's entries.
    let degenerate = Rect::new(Point::new(0, 0), Point::new(0, 0));
    let (results, tiles, scanned) = index.query_region_counted(degenerate);
    assert!(results.is_empty());
    assert_eq!(scanned, 0, "empty region must scan no entries");
    let _ = tiles; // tiles may be reported as the single collapsed tile or zero.

    // A region entirely outside the world touches no tiles at all.
    let outside = Rect::new(
        Point::new(BOUND + 1_000, BOUND + 1_000),
        Point::new(BOUND + 2_000, BOUND + 2_000),
    );
    let (results, tiles, scanned) = index.query_region_counted(outside);
    assert!(results.is_empty());
    assert_eq!(tiles, 0, "region outside the world touches no tiles");
    assert_eq!(scanned, 0);
}

#[test]
fn small_viewport_touches_few_tiles_not_all() {
    // The whole point of the tile layout: a small viewport reads only a few tiles.
    let entries = random_entries(0x5EED, 50_000);
    let grid: u32 = 64;
    let temp = write_archive("smallvp", grid, &entries);
    let index = StreamingIndex::open(temp.path()).expect("open mmap index");

    // A viewport ~1/20 of the world width, near the origin.
    let vp = BOUND / 20;
    let region = Rect::new(Point::new(-vp, -vp), Point::new(vp, vp));
    let (results, tiles, scanned) = index.query_region_counted(region);

    let total_tiles = (grid as usize) * (grid as usize);
    assert!(
        tiles < total_tiles / 4,
        "small viewport touched {tiles} of {total_tiles} tiles, expected a small fraction"
    );
    assert!(
        scanned < index.total_entries() / 4,
        "small viewport scanned {scanned} of {} entries, expected a small fraction",
        index.total_entries()
    );
    // And it must still be exactly correct.
    let got: BTreeSet<u32> = results.into_iter().map(|(_, id)| id).collect();
    assert_eq!(got, oracle_ids(&entries, region));
}

#[test]
fn open_rejects_a_non_archive_file() {
    // A file of garbage bytes must be rejected as an error, never UB.
    let temp = TempPath::new("garbage");
    std::fs::write(temp.path(), b"this is definitely not an rkyv archive").expect("write garbage");
    assert!(StreamingIndex::open(temp.path()).is_err());
}

#[test]
fn build_with_zero_grid_is_clamped_and_still_correct() {
    // `grid_n = 0` must be raised to 1 (a single tile) and still answer correctly.
    let entries = random_entries(0x9, 2_000);
    let temp = write_archive("zerogrid", 0, &entries);
    let index = StreamingIndex::open(temp.path()).expect("open mmap index");
    assert_eq!(index.header().expect("header").grid_n, 1);

    let region = Rect::new(Point::new(-500, -500), Point::new(500, 500));
    assert_eq!(mmap_ids(&index, region), oracle_ids(&entries, region));
}
