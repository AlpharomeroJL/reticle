//! Out-of-core streaming demonstration and measurement.
//!
//! Builds a LARGE tile-organized archive on disk (tens of millions of entries),
//! reports the on-disk file size, memory-maps it with [`StreamingIndex`], runs a
//! small-viewport `query_region`, and prints the query time plus the number of tiles
//! and entries the query touched versus the totals.
//!
//! The honest headline: the whole file is never read into a `Vec`. Only the mapped
//! pages the query touches are faulted in by the OS. Where the platform exposes it,
//! we also print the process working-set (resident) memory after the query to show it
//! stays far below the file size.
//!
//! Run with `cargo run -p reticle-index --example stream_demo --release`. Override the
//! entry count with the first CLI argument, for example
//! `cargo run -p reticle-index --example stream_demo --release -- 20000000`.

use std::path::PathBuf;
use std::time::Instant;

use reticle_geometry::{Point, Rect};
use reticle_index::streaming::{StreamingIndex, TiledPayload};

/// A tiny deterministic xorshift PRNG so the demo is reproducible without a `rand`
/// dependency (matching the crate's benches).
struct XorShift(u64);

impl XorShift {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// The world spans `[-HALF, HALF)` in each axis (DBU).
const HALF: i32 = 5_000_000;

/// Grid side: `GRID x GRID` tiles. With tens of millions of entries this keeps a few
/// thousand entries per tile, so a small viewport touches only a handful of tiles.
const GRID: u32 = 512;

fn world() -> Rect {
    Rect::new(Point::new(-HALF, -HALF), Point::new(HALF, HALF))
}

/// A deterministic lazy generator of `count` random small rectangles spread across
/// the world. Being an iterator (rather than a collected `Vec`) matters for the
/// demo's honesty: the source entries are never held in memory, so the working-set
/// readout at the end reflects the memory-mapped query path, not builder leftovers.
struct Entries {
    rng: XorShift,
    index: u32,
    remaining: usize,
}

impl Entries {
    fn new(count: usize) -> Self {
        Self {
            rng: XorShift(0x9E37_79B9_7F4A_7C15),
            index: 0,
            remaining: count,
        }
    }
}

impl Iterator for Entries {
    type Item = (Rect, u32);

    fn next(&mut self) -> Option<(Rect, u32)> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        let span = 2u64 * HALF as u64;
        let x = ((self.rng.next_u64() % span) as i64 - i64::from(HALF)) as i32;
        let y = ((self.rng.next_u64() % span) as i64 - i64::from(HALF)) as i32;
        let w = (self.rng.next_u64() % 400 + 1) as i32;
        let h = (self.rng.next_u64() % 400 + 1) as i32;
        let id = self.index;
        self.index += 1;
        Some((Rect::new(Point::new(x, y), Point::new(x + w, y + h)), id))
    }
}

/// Brute-force count of entries intersecting `region`, cross-checking the mmap
/// result against a full linear scan. Re-runs the deterministic generator lazily,
/// so the check allocates nothing.
fn brute_force_count(count: usize, region: Rect) -> usize {
    Entries::new(count)
        .filter(|(bbox, _)| bbox.intersects(&region))
        .count()
}

/// Best-effort process resident/working-set memory in bytes, or `None` if this
/// platform is not handled. On Windows it shells out to `powershell.exe` and asks for
/// our own process's `WorkingSet64`, which keeps the demo (and the whole workspace
/// apart from the library's single mmap block) free of `unsafe`; spawning one process
/// is fine for a one-shot readout in a demo.
#[cfg(windows)]
fn working_set_bytes() -> Option<u64> {
    let pid = std::process::id();
    let out = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-Process -Id {pid}).WorkingSet64"),
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

#[cfg(not(windows))]
fn working_set_bytes() -> Option<u64> {
    None
}

fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    format!("{v:.2} {}", UNITS[u])
}

/// The entry count from the first CLI argument (default 30M), capped so the archive
/// stays under rkyv's roughly 2 GiB 32-bit offset limit. Prints a note when capping.
fn requested_count() -> usize {
    // rkyv's default 32-bit relative pointers cap a single archive near 2 GiB, and the
    // in-memory builder is bounded by RAM regardless. Cap so a large count prints a
    // clear note rather than a deep rkyv panic; the out-of-core READ mechanism is
    // identical at any size, only the touched tiles are faulted in (see ADR 0016).
    const MAX_ENTRIES: usize = 90_000_000;
    let requested: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(30_000_000);
    let count = requested.min(MAX_ENTRIES);
    if count < requested {
        println!(
            "note: capped {requested} to {MAX_ENTRIES} entries so the archive stays under\n\
             \x20     rkyv's ~2 GiB 32-bit offset limit (a 64-bit-offset build is a follow-up)."
        );
    }
    count
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let count = requested_count();

    println!("== Reticle out-of-core streaming demo ==");
    println!("entries          : {count}");
    println!("grid             : {GRID} x {GRID} = {} tiles", GRID * GRID);
    println!(
        "world            : [{}, {}) in each axis (DBU)",
        -HALF, HALF
    );

    // Build the tiled archive from the lazy generator, then write it to disk. The
    // source entries are never collected into a Vec, so nothing from this phase
    // lingers in the working set measured after the query below.
    let t = Instant::now();
    let payload = TiledPayload::build(world(), GRID, Entries::new(count));
    println!("generated and tiled {count} entries in {:?}", t.elapsed());

    let t = Instant::now();
    let bytes = payload.serialize()?;
    let file_size = bytes.len() as u64;
    println!(
        "serialized in {:?}, archive size {} ({file_size} bytes)",
        t.elapsed(),
        human_bytes(file_size)
    );

    let mut path: PathBuf = std::env::temp_dir();
    path.push(format!(
        "reticle_stream_demo_{}_{}.rkyv",
        std::process::id(),
        count
    ));
    std::fs::write(&path, &bytes)?;
    println!("wrote archive to {}", path.display());

    // Drop the in-memory archive so resident memory below reflects the mmap path,
    // not the builder. Correctness is cross-checked later by re-running the
    // deterministic generator, which allocates nothing.
    drop(bytes);
    drop(payload);

    // Map the file and query a small viewport.
    let index = StreamingIndex::open(&path)?;
    let total = index.total_entries();
    let header = index.header()?;
    println!(
        "mapped file: total_entries {total}, grid {} x {}",
        header.grid_n, header.grid_n
    );

    // A small viewport: ~1/1000 of the world width on each side, near the origin.
    let vp_half = HALF / 1000;
    let viewport = Rect::new(Point::new(-vp_half, -vp_half), Point::new(vp_half, vp_half));
    println!(
        "viewport         : [{}, {}) in each axis ({:.4}% of world area)",
        -vp_half,
        vp_half,
        100.0 * (viewport.area() as f64) / (world().area() as f64)
    );

    // Warm nothing on purpose: time the first (cold) query so the numbers reflect
    // demand paging from disk.
    let t = Instant::now();
    let (results, tiles_touched, entries_scanned) = index.query_region_counted(viewport);
    let elapsed = t.elapsed();

    let total_tiles = (header.grid_n as usize) * (header.grid_n as usize);
    println!("---- query ----");
    println!("query time       : {elapsed:?}");
    println!("results          : {}", results.len());
    println!(
        "tiles touched    : {tiles_touched} / {total_tiles} ({:.4}%)",
        100.0 * tiles_touched as f64 / total_tiles as f64
    );
    println!(
        "entries scanned  : {entries_scanned} / {total} ({:.6}%)",
        100.0 * entries_scanned as f64 / total as f64
    );

    // Read the working set now, straight after the query, before the cross-check
    // walks the generator again, so the number is the residency of the query path.
    let rss_after_query = working_set_bytes();

    // Cross-check against a brute-force linear scan of the regenerated entries.
    let expected = brute_force_count(count, viewport);
    assert_eq!(
        results.len(),
        expected,
        "mmap query must match brute-force scan"
    );
    println!("cross-check      : matches brute-force scan ({expected} entries)");

    match rss_after_query {
        Some(rss) => {
            println!(
                "working set      : {} (file on disk is {})",
                human_bytes(rss),
                human_bytes(file_size)
            );
            println!(
                "                   the mapped file is never copied into a Vec; only the\n\
                 \x20                  tiles the query touched were faulted in by the OS."
            );
        }
        None => {
            println!(
                "working set      : not measured on this platform. Memory is demand-paged\n\
                 \x20                  by the OS: the code never materializes the full entries\n\
                 \x20                  array, and only the touched tiles' pages are resident."
            );
        }
    }

    // Clean up the temp file.
    let _ = std::fs::remove_file(&path);
    println!("removed {}", path.display());
    Ok(())
}
