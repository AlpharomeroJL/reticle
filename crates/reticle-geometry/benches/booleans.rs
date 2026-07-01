//! Benchmarks for polygon boolean operations.

use criterion::{Criterion, criterion_group, criterion_main};
use reticle_geometry::{BooleanOp, Point, Polygon, Rect, polygon_boolean};
use std::hint::black_box;

/// A `side` x `side` grid of overlapping square polygons (pitch smaller than size),
/// the kind of dense, overlapping input a DRC merge produces.
fn overlapping_grid(side: i32, size: i32, pitch: i32) -> Vec<Polygon> {
    let mut polys = Vec::with_capacity((side * side) as usize);
    for col in 0..side {
        for row in 0..side {
            let px = col * pitch;
            let py = row * pitch;
            polys.push(Polygon::from_rect(Rect::new(
                Point::new(px, py),
                Point::new(px + size, py + size),
            )));
        }
    }
    polys
}

fn bench_union(c: &mut Criterion) {
    let mut group = c.benchmark_group("boolean");
    for &side in &[16i32, 32] {
        let polys = overlapping_grid(side, 10, 8);
        let count = polys.len();
        group.bench_function(format!("self_union_{count}_squares"), |b| {
            b.iter(|| {
                polygon_boolean(BooleanOp::Union, black_box(&polys), black_box(&[])).unwrap()
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_union);
criterion_main!(benches);
