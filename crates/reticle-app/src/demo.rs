//! The built-in demo document shown on startup.
//!
//! Reticle is about *large hierarchical* layouts, so the demo is hierarchical
//! rather than a flat pile of rectangles: a small leaf standard cell is placed as
//! single instances and as a repeated array inside a top cell, and a couple of
//! routing wires and a labeled polygon are drawn directly in the top cell. This
//! gives the canvas immediate content on both native and web with no file IO.

use reticle_geometry::{Endcap, LayerId, Path, Point, Polygon, Rect, Transform};
use reticle_model::{
    ArrayInstance, Cell, Document, DrawShape, Instance, LayerInfo, ShapeKind, Technology,
};

/// The name of the demo's top cell.
pub const TOP_CELL: &str = "CHIP_TOP";

/// The name of the reusable leaf cell.
pub const LEAF_CELL: &str = "STD_CELL";

/// Database units per micron for the demo technology.
const DBU_PER_MICRON: i64 = 1000;

/// Builds the demo technology: a handful of named, colored layers.
///
/// Colors are packed `0xRRGGBBAA`. The layer ids double as the demo's drawing
/// layers and drive the layer-manager panel and per-layer culling.
#[must_use]
pub fn demo_technology() -> Technology {
    Technology {
        name: "reticle-demo".to_owned(),
        dbu_per_micron: DBU_PER_MICRON,
        layers: vec![
            LayerInfo {
                id: LayerId::new(1, 0),
                name: "NWELL".to_owned(),
                color_rgba: 0x5A_8F_C0_FF,
                visible: true,
            },
            LayerInfo {
                id: LayerId::new(2, 0),
                name: "ACTIVE".to_owned(),
                color_rgba: 0x4C_B0_54_FF,
                visible: true,
            },
            LayerInfo {
                id: LayerId::new(3, 0),
                name: "POLY".to_owned(),
                color_rgba: 0xC0_3A_3A_FF,
                visible: true,
            },
            LayerInfo {
                id: LayerId::new(4, 0),
                name: "METAL1".to_owned(),
                color_rgba: 0x3A_74_C0_FF,
                visible: true,
            },
            LayerInfo {
                id: LayerId::new(5, 0),
                name: "METAL2".to_owned(),
                color_rgba: 0xC9_9A_2E_FF,
                visible: true,
            },
            LayerInfo {
                id: LayerId::new(6, 0),
                name: "TEXT".to_owned(),
                color_rgba: 0xD8_D8_D8_FF,
                visible: true,
            },
        ],
        rules: Vec::new(),
    }
}

/// Layer id helpers for the demo geometry.
mod layer {
    use reticle_geometry::LayerId;

    /// The n-well layer.
    pub const NWELL: LayerId = LayerId::new(1, 0);
    /// The active/diffusion layer.
    pub const ACTIVE: LayerId = LayerId::new(2, 0);
    /// The polysilicon gate layer.
    pub const POLY: LayerId = LayerId::new(3, 0);
    /// The first metal layer.
    pub const METAL1: LayerId = LayerId::new(4, 0);
    /// The second metal layer.
    pub const METAL2: LayerId = LayerId::new(5, 0);
}

/// Builds the reusable leaf standard cell: a well, two diffusion regions, two poly
/// gates, and a metal-1 strap. All geometry is in the cell's local coordinates.
#[must_use]
fn leaf_cell() -> Cell {
    let mut cell = Cell::new(LEAF_CELL);
    let rect = |layer, x0, y0, x1, y1| {
        DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    };
    cell.shapes.push(rect(layer::NWELL, 0, 0, 2000, 2800));
    cell.shapes.push(rect(layer::ACTIVE, 300, 400, 1700, 1100));
    cell.shapes.push(rect(layer::ACTIVE, 300, 1700, 1700, 2400));
    cell.shapes.push(rect(layer::POLY, 700, 200, 900, 2600));
    cell.shapes.push(rect(layer::POLY, 1200, 200, 1400, 2600));
    cell.shapes.push(rect(layer::METAL1, 200, 1250, 1800, 1550));
    cell
}

/// Builds the complete demo document: technology, leaf cell, and a top cell that
/// instances the leaf a few times, arrays it into a block, and adds routing.
///
/// The returned [`Document`] has [`TOP_CELL`] registered as its single top cell.
#[must_use]
pub fn demo_document() -> Document {
    let mut doc = Document::new();
    doc.set_technology(demo_technology());
    doc.insert_cell(leaf_cell());

    let mut top = Cell::new(TOP_CELL);

    // A row of three individually-placed instances near the origin.
    for i in 0..3 {
        top.instances.push(Instance {
            cell: LEAF_CELL.to_owned(),
            transform: Transform::translate(i * 2500, 0),
        });
    }

    // A dense array block above the row: this is where the "hierarchical scale"
    // comes from -- one small cell repeated into a grid.
    top.arrays.push(ArrayInstance {
        cell: LEAF_CELL.to_owned(),
        transform: Transform::translate(0, 4000),
        columns: 8,
        rows: 6,
        column_pitch: 2500,
        row_pitch: 3200,
    });

    // Two metal-2 routing wires spanning the block.
    top.shapes.push(DrawShape::new(
        layer::METAL2,
        ShapeKind::Path(Path::new(
            vec![
                Point::new(-500, 3200),
                Point::new(21000, 3200),
                Point::new(21000, 24000),
            ],
            220,
            Endcap::Square,
        )),
    ));
    top.shapes.push(DrawShape::new(
        layer::METAL2,
        ShapeKind::Path(Path::new(
            vec![Point::new(-500, 3600), Point::new(19500, 3600)],
            220,
            Endcap::Flat,
        )),
    ));

    // A metal-1 guard-ring polygon around the individual-instance row.
    top.shapes.push(DrawShape::new(
        layer::METAL1,
        ShapeKind::Polygon(Polygon::new(vec![
            Point::new(-400, -400),
            Point::new(7400, -400),
            Point::new(7400, 3000),
            Point::new(6900, 3000),
            Point::new(6900, 100),
            Point::new(-400, 100),
        ])),
    ));

    doc.insert_cell(top);
    doc.set_top_cells(vec![TOP_CELL.to_owned()]);
    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::Shape;

    #[test]
    fn demo_has_top_cell() {
        let doc = demo_document();
        assert_eq!(doc.top_cells(), &[TOP_CELL.to_owned()]);
        assert!(doc.cell(TOP_CELL).is_some());
        assert!(doc.cell(LEAF_CELL).is_some());
    }

    #[test]
    fn demo_flattens_to_many_shapes() {
        let doc = demo_document();
        let flat = doc.flatten(TOP_CELL);
        // Leaf has 6 shapes; 3 instances + an 8x6 array = 51 placements, plus 3
        // top-level shapes.
        let placements = 3 + 8 * 6;
        assert_eq!(flat.len(), placements * 6 + 3);
    }

    #[test]
    fn demo_bbox_is_nonempty() {
        let doc = demo_document();
        let bbox = doc.cell_bbox(TOP_CELL).expect("top cell has a bbox");
        assert!(bbox.width() > 0 && bbox.height() > 0);
    }

    #[test]
    fn every_demo_layer_is_in_technology() {
        let doc = demo_document();
        let tech_ids: std::collections::HashSet<_> =
            doc.technology().layers.iter().map(|l| l.id).collect();
        for shape in doc.flatten(TOP_CELL) {
            assert!(
                tech_ids.contains(&shape.layer()),
                "layer {:?} missing from technology",
                shape.layer()
            );
        }
    }
}
