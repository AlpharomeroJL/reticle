//! Integration tests for `reticle-proto`: document encode/decode round-trips,
//! the migration support window, and the schema-version constant.

use reticle_proto::v1::{
    Array, Cell, Document, Instance, Layer, LayerId, Orientation, Point, Polygon, Rect,
    SchemaVersion, Shape, Technology, Transform,
};
use reticle_proto::{SCHEMA_VERSION, decode_document, encode_document, migrate};

/// Builds a representative document exercising the technology, a cell with a
/// rect shape and a polygon shape (via the `Shape::kind` oneof), an instance,
/// and an array.
fn sample_document() -> Document {
    let metal1 = LayerId {
        layer: 10,
        datatype: 0,
    };

    let technology = Technology {
        name: "demo-tech".to_owned(),
        dbu_per_micron: 1_000,
        layers: vec![Layer {
            id: Some(metal1),
            name: "metal1".to_owned(),
            color_rgba: 0xFF_00_00_FF,
            visible: true,
            fill_opacity: 0.5,
        }],
        rules: vec![],
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

    let transform = Transform {
        translation: Some(Point { x: 500, y: 500 }),
        orientation: Orientation::R90 as i32,
        mag_num: 1,
        mag_den: 1,
    };

    let leaf_cell = Cell {
        name: "LEAF".to_owned(),
        shapes: vec![rect_shape, polygon_shape],
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
        comments: vec![],
    }
}

#[test]
fn document_round_trips_through_bytes() {
    let original = sample_document();

    let bytes = encode_document(&original);
    assert!(!bytes.is_empty(), "encoding produced no bytes");

    let decoded = decode_document(&bytes).expect("decode of a valid document must succeed");

    assert_eq!(decoded, original);
}

#[test]
fn decode_rejects_truncated_input() {
    let bytes = encode_document(&sample_document());
    // Chop the buffer mid-message; the trailing length-delimited field is now
    // incomplete and must fail to decode.
    let truncated = &bytes[..bytes.len() / 2];
    assert!(decode_document(truncated).is_err());
}

#[test]
fn migrate_supports_the_current_and_no_other_versions() {
    // Version 0 is the "unspecified" sentinel and is not readable.
    assert!(!migrate::is_supported(0));
    // Every version from 1 through the current one is supported.
    assert!(migrate::is_supported(1));
    assert!(migrate::is_supported(SCHEMA_VERSION));
    // A future, unknown version is not supported.
    assert!(!migrate::is_supported(SCHEMA_VERSION + 1));
}

#[test]
fn schema_version_constant_matches_the_proto_enum() {
    assert_eq!(SCHEMA_VERSION, 2);
    // The Rust constant and the wire enum must agree on the current version.
    assert_eq!(SCHEMA_VERSION as i32, SchemaVersion::V2 as i32);
}
