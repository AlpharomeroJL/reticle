//! Regenerates the frozen V1 golden fixture `tests/fixtures/v1_document_golden.bin`.
//!
//! The fixture is the byte-for-byte pre-V2 artifact that the V2 loader and the
//! migration path must still read (ADR 0080). It is produced by this example
//! with the schema pinned at `SCHEMA_VERSION_V1`, so re-running it must yield the
//! same bytes as long as the V1 fields are untouched (new V2 fields are additive
//! and default-empty, so they emit nothing for this all-V1 document).
//!
//! Run from anywhere with `cargo run -p reticle-proto --example gen_v1_fixture`;
//! the output path is resolved against `CARGO_MANIFEST_DIR`, never the CWD.

use std::path::PathBuf;

use reticle_proto::encode_document;
use reticle_proto::v1::{
    Array, Cell, Document, Endcap, Instance, Layer, LayerId, Orientation, Path, Point, Polygon,
    Rect, Rule, RuleKind, SchemaVersion, Shape, Technology, Transform,
};

/// Builds the representative V1 document captured by the golden fixture: a
/// technology with two layers and a rule, a leaf cell carrying a rect, a polygon
/// and a path across both layers, and a top cell with an instance and an array.
fn golden_v1_document() -> Document {
    let metal1 = LayerId {
        layer: 10,
        datatype: 0,
    };
    let metal2 = LayerId {
        layer: 20,
        datatype: 0,
    };

    let technology = Technology {
        name: "golden-tech".to_owned(),
        dbu_per_micron: 1_000,
        layers: vec![
            Layer {
                id: Some(metal1),
                name: "metal1".to_owned(),
                color_rgba: 0xFF_00_00_FF,
                visible: true,
                fill_opacity: 0.5,
            },
            Layer {
                id: Some(metal2),
                name: "metal2".to_owned(),
                color_rgba: 0x00_FF_00_FF,
                visible: true,
                fill_opacity: 0.35,
            },
        ],
        rules: vec![Rule {
            name: "M1.W.1".to_owned(),
            kind: RuleKind::Width as i32,
            layer: Some(metal1),
            other_layer: None,
            value: 50,
        }],
    };

    let rect_shape = Shape {
        layer: Some(metal1),
        kind: Some(reticle_proto::v1::shape::Kind::Rect(Rect {
            min: Some(Point { x: 0, y: 0 }),
            max: Some(Point { x: 100, y: 200 }),
        })),
    };

    let polygon_shape = Shape {
        layer: Some(metal1),
        kind: Some(reticle_proto::v1::shape::Kind::Polygon(Polygon {
            vertices: vec![
                Point { x: 0, y: 0 },
                Point { x: 50, y: 0 },
                Point { x: 50, y: 50 },
                Point { x: 0, y: 50 },
            ],
        })),
    };

    let path_shape = Shape {
        layer: Some(metal2),
        kind: Some(reticle_proto::v1::shape::Kind::Path(Path {
            points: vec![
                Point { x: 0, y: 100 },
                Point { x: 300, y: 100 },
                Point { x: 300, y: 400 },
            ],
            width: 40,
            endcap: Endcap::Square as i32,
            endcap_extension: 0,
        })),
    };

    let transform = Transform {
        translation: Some(Point { x: 500, y: 500 }),
        orientation: Orientation::R90 as i32,
        mag_num: 1,
        mag_den: 1,
    };

    let leaf_cell = Cell {
        name: "LEAF".to_owned(),
        shapes: vec![rect_shape, polygon_shape, path_shape],
        instances: vec![],
        arrays: vec![],
    };

    let top_cell = Cell {
        name: "TOP".to_owned(),
        shapes: vec![],
        instances: vec![Instance {
            cell: "LEAF".to_owned(),
            transform: Some(transform),
        }],
        arrays: vec![Array {
            cell: "LEAF".to_owned(),
            transform: Some(transform),
            columns: 4,
            rows: 2,
            column_pitch: 120,
            row_pitch: 220,
        }],
    };

    Document {
        schema_version: SchemaVersion::V1 as i32,
        technology: Some(technology),
        cells: vec![leaf_cell, top_cell],
        top_cells: vec!["TOP".to_owned()],
    }
}

fn main() -> std::io::Result<()> {
    let bytes = encode_document(&golden_v1_document());

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    std::fs::create_dir_all(&path)?;
    path.push("v1_document_golden.bin");
    std::fs::write(&path, &bytes)?;

    println!("wrote {} bytes to {}", bytes.len(), path.display());
    Ok(())
}
