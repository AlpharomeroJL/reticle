//! Criterion benchmarks for the R-tree index.
//!
//! Measures the two hot paths for browsing a large, mostly-static layout: bulk
//! building an R-tree over one million random rectangles, and answering point
//! nearest-neighbour queries against the built tree. Numbers are produced by
//! Criterion at run time (`cargo bench -p reticle-index`); nothing here is
//! hard-coded.

use criterion::{Criterion, criterion_group, criterion_main};
use reticle_geometry::{Point, Rect, SpatialIndex};
use reticle_index::RTreeIndex;

/// A tiny deterministic xorshift PRNG so benches are reproducible without pulling
/// in a `rand` dependency.
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

    /// A coordinate in roughly `[-500_000, 500_000)` DBU.
    fn coord(&mut self) -> i32 {
        (self.next_u64() % 1_000_000) as i32 - 500_000
    }
}

/// Builds `count` random small rectangles.
fn random_rects(count: usize) -> Vec<(Rect, u32)> {
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    (0..count)
        .map(|i| {
            let x = rng.coord();
            let y = rng.coord();
            let width = (rng.next_u64() % 200 + 1) as i32;
            let height = (rng.next_u64() % 200 + 1) as i32;
            (
                Rect::new(Point::new(x, y), Point::new(x + width, y + height)),
                i as u32,
            )
        })
        .collect()
}

fn bench_rtree(c: &mut Criterion) {
    const N: usize = 1_000_000;
    let rects = random_rects(N);

    c.bench_function("rtree_bulk_load_1m", |b| {
        b.iter(|| {
            let index = RTreeIndex::bulk_load(rects.iter().copied());
            std::hint::black_box(index.len())
        });
    });

    let index = RTreeIndex::bulk_load(rects.iter().copied());
    let mut rng = XorShift(0x1234_5678_9ABC_DEF0);
    c.bench_function("rtree_nearest_point_query", |b| {
        b.iter(|| {
            let p = Point::new(rng.coord(), rng.coord());
            std::hint::black_box(index.nearest(std::hint::black_box(p)))
        });
    });
}

criterion_group!(benches, bench_rtree);
criterion_main!(benches);
