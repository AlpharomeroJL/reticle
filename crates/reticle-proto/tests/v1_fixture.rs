//! Guards the frozen V1 golden fixture (`tests/fixtures/v1_document_golden.bin`).
//!
//! The fixture was captured with the pre-V2 build (see `examples/gen_v1_fixture.rs`
//! and ADR 0080). It is the byte-for-byte artifact that later, schema-V2 code must
//! still decode without loss. This test locks in what the fixture contains under
//! the CURRENT build; the migration test (`v1_migration.rs`, added with V2) proves
//! the V2 code still reads these same bytes.

use reticle_proto::decode_document;
use reticle_proto::v1::{SchemaVersion, shape::Kind};

/// The committed fixture bytes, embedded at compile time so the test never
/// depends on the process working directory.
const GOLDEN_V1: &[u8] = include_bytes!("fixtures/v1_document_golden.bin");

#[test]
fn golden_fixture_decodes_as_v1() {
    let doc = decode_document(GOLDEN_V1).expect("golden V1 fixture must decode");
    assert_eq!(
        doc.schema_version,
        SchemaVersion::V1 as i32,
        "the frozen fixture is a V1 document"
    );
}

#[test]
fn golden_fixture_geometry_is_intact() {
    let doc = decode_document(GOLDEN_V1).expect("golden V1 fixture must decode");

    let technology = doc.technology.as_ref().expect("technology present");
    assert_eq!(technology.name, "golden-tech");
    assert_eq!(technology.layers.len(), 2, "two layers");
    assert_eq!(technology.rules.len(), 1, "one rule");

    assert_eq!(doc.cells.len(), 2, "leaf + top cell");
    assert_eq!(doc.top_cells, vec!["TOP".to_owned()]);

    let leaf = doc
        .cells
        .iter()
        .find(|c| c.name == "LEAF")
        .expect("LEAF cell present");
    assert_eq!(leaf.shapes.len(), 3, "rect + polygon + path");

    // Each shape's oneof kind must decode to the variant it was written as.
    let kinds: Vec<&Kind> = leaf.shapes.iter().filter_map(|s| s.kind.as_ref()).collect();
    assert!(matches!(kinds[0], Kind::Rect(_)), "first shape is a rect");
    assert!(
        matches!(kinds[1], Kind::Polygon(_)),
        "second shape is a polygon"
    );
    assert!(matches!(kinds[2], Kind::Path(_)), "third shape is a path");

    let top = doc
        .cells
        .iter()
        .find(|c| c.name == "TOP")
        .expect("TOP cell present");
    assert_eq!(top.instances.len(), 1, "one instance");
    assert_eq!(top.arrays.len(), 1, "one array");
}
