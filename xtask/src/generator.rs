//! A deterministic, chip-like hierarchical layout generator.
//!
//! The output is fully deterministic (no randomness), so a given
//! `(target_shapes, layers, depth)` always yields the same document, which makes it
//! ideal for reproducible benchmarks and media capture.

use reticle_geometry::{LayerId, Point, Rect, Transform};
use reticle_model::{ArrayInstance, Cell, Document, DrawShape, LayerInfo, ShapeKind, Technology};

/// Side length of the leaf cell's square shape grid.
const LEAF_GRID: i32 = 4;
/// Pitch between leaf shapes, in DBU.
const LEAF_PITCH: i32 = 100;
/// Feature size of a leaf shape, in DBU.
const FEATURE: i32 = 50;

/// Generates a chip-like hierarchical document whose flattened leaf-shape count is
/// approximately `target_shapes`, spread across `layers` layers and `depth` levels
/// of arrayed hierarchy. The hierarchy is never flattened, so the document stays
/// small on disk even for billions of effective leaf shapes.
#[must_use]
pub fn generate_layout(target_shapes: usize, layers: usize, depth: usize) -> Document {
    let layers = layers.max(1);
    let depth = depth.max(1);
    let mut doc = Document::new();

    // Technology: `layers` display layers with distinct colors.
    let mut tech = Technology {
        name: "generated".to_owned(),
        dbu_per_micron: 1000,
        layers: Vec::with_capacity(layers),
        rules: Vec::new(),
        stack: Vec::new(),
    };
    for layer_idx in 0..layers {
        tech.layers.push(LayerInfo {
            id: LayerId::new(layer_idx as u16, 0),
            name: format!("M{layer_idx}"),
            color_rgba: palette(layer_idx),
            visible: true,
        });
    }
    doc.set_technology(tech);

    // Leaf cell: a grid of small rects cycling through the layers.
    let leaf_shapes = (LEAF_GRID * LEAF_GRID) as usize;
    let mut leaf = Cell::new("leaf");
    for idx in 0..LEAF_GRID * LEAF_GRID {
        let px = (idx % LEAF_GRID) * LEAF_PITCH;
        let py = (idx / LEAF_GRID) * LEAF_PITCH;
        let layer = (idx as usize % layers) as u16;
        leaf.shapes.push(DrawShape::new(
            LayerId::new(layer, 0),
            ShapeKind::Rect(Rect::new(
                Point::new(px, py),
                Point::new(px + FEATURE, py + FEATURE),
            )),
        ));
    }
    doc.insert_cell(leaf);

    // Pick a per-level square array side so leaf_shapes * side^(2*depth) ~= target.
    let arrays_total = (target_shapes as f64 / leaf_shapes as f64).max(1.0);
    let side = arrays_total
        .powf(1.0 / (2.0 * depth as f64))
        .ceil()
        .max(1.0) as u32;

    // Wrap the previous level in a square array, `depth` times.
    let mut child = "leaf".to_owned();
    let mut extent = LEAF_GRID * LEAF_PITCH;
    for level in 1..=depth {
        let name = format!("level{level}");
        let pitch = extent + LEAF_PITCH; // leave a margin between array elements
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

    doc.set_top_cells(vec![child]);
    doc
}

/// The approximate flattened leaf-shape count of the document `generate_layout`
/// would produce for these parameters.
#[must_use]
pub fn approximate_shape_count(target_shapes: usize, depth: usize) -> u64 {
    let depth = depth.max(1);
    let leaf_shapes = (LEAF_GRID * LEAF_GRID) as usize;
    let arrays_total = (target_shapes as f64 / leaf_shapes as f64).max(1.0);
    let side = arrays_total
        .powf(1.0 / (2.0 * depth as f64))
        .ceil()
        .max(1.0) as u64;
    (leaf_shapes as u64) * side.pow(2 * depth as u32)
}

/// A distinct display color (`0xRRGGBBAA`) for layer index `layer_idx`.
fn palette(layer_idx: usize) -> u32 {
    const COLORS: [u32; 8] = [
        0x4F_9D_FF_FF,
        0xFF_6B_6B_FF,
        0x5B_E5_84_FF,
        0xFF_C8_4F_FF,
        0xB0_7B_FF_FF,
        0x4F_E5_E5_FF,
        0xFF_8F_D0_FF,
        0xC8_D0_58_FF,
    ];
    COLORS[layer_idx % COLORS.len()]
}
