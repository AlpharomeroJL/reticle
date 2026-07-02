//! Benchmarks for `cell_bbox`: memoized access via [`EditableDocument`] versus
//! recomputing the recursive bounding box uncached on every call.
//!
//! The document is a deep chain of cells where each level places the one below it
//! both as a single instance and as an array, so the effective (flattened) leaf
//! count grows multiplicatively into the thousands while the recursive `cell_bbox`
//! walk visits every level on each uncached call.

use criterion::{Criterion, criterion_group, criterion_main};
use reticle_geometry::{LayerId, Point, Rect, Transform};
use reticle_model::{
    ArrayInstance, Cell, Document, DrawShape, EditableDocument, Instance, ShapeKind,
};
use std::hint::black_box;

fn rect_shape(x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
    DrawShape::new(
        LayerId::new(1, 0),
        ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
    )
}

/// Builds a `levels`-deep hierarchy named `cell_0` (leaf) .. `cell_{levels-1}`
/// (top). Each non-leaf cell owns a small shape, a single instance of the level
/// below (offset), and a `cols x rows` array of it. Returns the document and the
/// name of the top cell.
///
/// With `cols*rows` placements plus one instance per level, the flattened leaf
/// count is roughly `(cols*rows + 1) ^ (levels - 1)`: a handful of levels is
/// already many thousands of effective leaves.
fn deep_hierarchy(levels: u32, cols: u32, rows: u32) -> (Document, String) {
    let mut doc = Document::new();

    // Leaf cell: a single unit square.
    doc.insert_cell({
        let mut leaf = Cell::new("cell_0");
        leaf.shapes.push(rect_shape(0, 0, 8, 8));
        leaf
    });

    for level in 1..levels {
        let name = format!("cell_{level}");
        let child = format!("cell_{}", level - 1);
        let mut cell = Cell::new(&name);
        // A small own shape near the origin so every level contributes geometry.
        cell.shapes.push(rect_shape(-4, -4, 0, 0));
        // One plain instance, offset so it is not subsumed by the array.
        cell.instances.push(Instance {
            cell: child.clone(),
            transform: Transform::translate(-40, -40),
        });
        // An array of the level below; pitch scales with depth to keep boxes distinct.
        let pitch = 32 * i32::try_from(level).unwrap_or(1);
        cell.arrays.push(ArrayInstance {
            cell: child,
            transform: Transform::IDENTITY,
            columns: cols,
            rows,
            column_pitch: pitch,
            row_pitch: pitch,
        });
        doc.insert_cell(cell);
    }

    let top = format!("cell_{}", levels - 1);
    doc.set_top_cells(vec![top.clone()]);
    (doc, top)
}

fn bench_cell_bbox(c: &mut Criterion) {
    // 6 levels of a 3x3 array + instance => (9+1)^5 = 100_000 effective leaves.
    let (doc, top) = deep_hierarchy(6, 3, 3);

    // Sanity: the recursive box is a real, non-empty rectangle.
    assert!(doc.cell_bbox(&top).is_some());

    let mut group = c.benchmark_group("cell_bbox");

    // Uncached: every iteration walks the full hierarchy from scratch.
    group.bench_function("uncached_recompute", |b| {
        b.iter(|| black_box(doc.cell_bbox(black_box(&top))));
    });

    // Cached warm: the EditableDocument memoizes after the first call, so every
    // subsequent access is a hash-map lookup. This is the steady-state read cost.
    let editor = EditableDocument::new(doc.clone());
    let _ = editor.cell_bbox(&top); // warm the cache once, outside the timed loop
    group.bench_function("cached_warm", |b| {
        b.iter(|| black_box(editor.cell_bbox(black_box(&top))));
    });

    group.finish();
}

criterion_group!(benches, bench_cell_bbox);
criterion_main!(benches);
