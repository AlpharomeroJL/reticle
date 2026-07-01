#![no_main]
//! Fuzz the polygon boolean engine: building two polygons from arbitrary bytes and
//! running every operation must never panic or hang.

use libfuzzer_sys::fuzz_target;
use reticle_geometry::{BooleanOp, Point, Polygon, polygon_boolean};

/// Interprets a byte slice as a sequence of `i16` coordinate pairs (kept small so
/// products stay well within range).
fn points(bytes: &[u8]) -> Vec<Point> {
    bytes
        .chunks_exact(4)
        .map(|c| {
            let x = i16::from_le_bytes([c[0], c[1]]);
            let y = i16::from_le_bytes([c[2], c[3]]);
            Point::new(i32::from(x), i32::from(y))
        })
        .collect()
}

fuzz_target!(|data: &[u8]| {
    let mid = data.len() / 2;
    let a = vec![Polygon::new(points(&data[..mid]))];
    let b = vec![Polygon::new(points(&data[mid..]))];
    for op in [
        BooleanOp::Union,
        BooleanOp::Intersection,
        BooleanOp::Difference,
        BooleanOp::Xor,
    ] {
        let _ = polygon_boolean(op, &a, &b);
    }
});
