//! Benchmark for [`Document::flatten`]: expanding a deeply nested array hierarchy
//! into its flat leaf geometry, the exact CPU work the headless render/DRC/extract
//! pipeline and the interactive open path pay to materialize a design.
//!
//! The document mirrors what `xtask gen-layout` produces: a small leaf cell wrapped
//! in a `side x side` array `depth` times, so the flattened leaf count is
//! `leaf_shapes * side^(2*depth)`. This is the structure Lane 5B measured as the
//! dominant open-path cost (about 230 ms for 4.19M leaves before the array-loop
//! factoring), so the bench pins that number and guards against regressing it.

use criterion::{Criterion, criterion_group, criterion_main};
use reticle_geometry::{LayerId, Magnification, Orientation, Point, Rect, Transform};
use reticle_model::{ArrayInstance, Cell, Document, DrawShape, ShapeKind};
use std::hint::black_box;

/// Small feature size (DBU) for the leaf rects.
const FEATURE: i32 = 4;
/// Leaf grid pitch (DBU).
const LEAF_PITCH: i32 = 10;

/// Builds a generator-shaped document: a `grid x grid` leaf cell of unit rects,
/// wrapped in a `side x side` array `depth` times. Returns the document and the top
/// cell name. The flattened leaf count is `grid^2 * side^(2*depth)`.
fn nested_array_doc(grid: i32, side: u32, depth: u32) -> (Document, String) {
    let mut doc = Document::new();

    let mut leaf = Cell::new("leaf");
    for idx in 0..grid * grid {
        let px = (idx % grid) * LEAF_PITCH;
        let py = (idx / grid) * LEAF_PITCH;
        let layer = (idx as u16) % 8;
        leaf.shapes.push(DrawShape::new(
            LayerId::new(layer, 0),
            ShapeKind::Rect(Rect::new(
                Point::new(px, py),
                Point::new(px + FEATURE, py + FEATURE),
            )),
        ));
    }
    doc.insert_cell(leaf);

    let mut child = "leaf".to_owned();
    let mut extent = grid * LEAF_PITCH;
    for level in 1..=depth {
        let name = format!("level{level}");
        let pitch = extent + LEAF_PITCH;
        let mut cell = Cell::new(&name);
        cell.arrays.push(ArrayInstance {
            cell: child.clone(),
            transform: Transform::IDENTITY,
            columns: side,
            rows: side,
            column_pitch: pitch,
            row_pitch: pitch,
        });
        doc.insert_cell(cell);
        extent = pitch.saturating_mul(side as i32);
        child = name;
    }
    doc.set_top_cells(vec![child.clone()]);
    (doc, child)
}

/// Builds a hierarchy whose array placements carry a non-identity transform (a 90
/// degree rotation), so the bench also covers the real-design case where the
/// array-loop factoring must transform each child shape once and then only translate
/// per cell.
fn rotated_array_doc(grid: i32, side: u32, depth: u32) -> (Document, String) {
    let (mut doc, top) = nested_array_doc(grid, side, depth);
    // Rewrite each level's array transform to a rotation; the leaf geometry and the
    // per-level structure are otherwise identical.
    let names: Vec<String> = (1..=depth).map(|l| format!("level{l}")).collect();
    let rot90 = Transform {
        translation: Point::ORIGIN,
        orientation: Orientation::R90,
        magnification: Magnification::UNITY,
    };
    for name in names {
        if let Some(cell) = doc.cell_mut(&name) {
            for array in &mut cell.arrays {
                array.transform = rot90;
            }
        }
    }
    (doc, top)
}

fn bench_flatten(c: &mut Criterion) {
    // grid 4 (16 leaf shapes), side 16, depth 2 => 16 * 16^4 = 1,048,576 leaves.
    // A million-leaf flatten runs in a few tens of ms, small enough for criterion's
    // sampling while still exercising the multi-reallocation, per-cell-transform path.
    let (doc, top) = nested_array_doc(4, 16, 2);
    let leaves = doc.flatten(&top).len();
    assert_eq!(leaves, 1_048_576, "expected ~1M flattened leaves");

    let mut group = c.benchmark_group("flatten");
    group.sample_size(20);

    group.bench_function("nested_array_1M_identity", |b| {
        b.iter(|| black_box(doc.flatten(black_box(&top)).len()));
    });

    let (rdoc, rtop) = rotated_array_doc(4, 16, 2);
    assert_eq!(rdoc.flatten(&rtop).len(), 1_048_576);
    group.bench_function("nested_array_1M_rotated", |b| {
        b.iter(|| black_box(rdoc.flatten(black_box(&rtop)).len()));
    });

    group.finish();
}

criterion_group!(benches, bench_flatten);
criterion_main!(benches);
