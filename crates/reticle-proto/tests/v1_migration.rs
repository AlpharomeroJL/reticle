//! The review target (ADR 0080): proves the V2 build still reads the frozen
//! pre-V2 golden fixture and migrates it losslessly.
//!
//! The bytes in `fixtures/v1_document_golden.bin` were captured with the V1-only
//! build (commit that precedes the schema edit). Here we decode them with the
//! current V2 code, confirm the geometry is intact and `comments` is empty, then
//! run `migrate_document` and assert the version becomes V2 while every geometry
//! byte is preserved.

use reticle_proto::migrate::{MigrationError, migrate_document};
use reticle_proto::v1::{Document, SchemaVersion};
use reticle_proto::{decode_document, encode_document};

/// The committed pre-V2 fixture, embedded at compile time.
const GOLDEN_V1: &[u8] = include_bytes!("fixtures/v1_document_golden.bin");

/// Re-encodes only the geometry-bearing fields of `doc` (technology, cells,
/// top-cell list) into a canonical buffer, zeroing the version and dropping
/// comments. Two documents with equal geometry bytes are byte-for-byte identical
/// in everything the migration must preserve.
fn geometry_bytes(doc: &Document) -> Vec<u8> {
    let geometry = Document {
        schema_version: SchemaVersion::Unspecified as i32,
        technology: doc.technology.clone(),
        cells: doc.cells.clone(),
        top_cells: doc.top_cells.clone(),
        comments: vec![],
    };
    encode_document(&geometry)
}

#[test]
fn v2_build_decodes_the_pre_v2_fixture_with_empty_comments() {
    let doc = decode_document(GOLDEN_V1).expect("V2 build must decode the frozen V1 bytes");

    assert_eq!(
        doc.schema_version,
        SchemaVersion::V1 as i32,
        "the fixture is still tagged V1 before migration"
    );
    assert!(
        doc.comments.is_empty(),
        "a genuine V1 document carries no comments"
    );
    // Geometry survived the additive schema change.
    assert!(doc.technology.is_some(), "technology intact");
    assert_eq!(doc.cells.len(), 2, "both cells intact");
}

#[test]
fn migrating_the_golden_fixture_is_lossless() {
    let before = decode_document(GOLDEN_V1).expect("decode V1 fixture");
    let geometry_before = geometry_bytes(&before);

    let mut migrated = before.clone();
    migrate_document(&mut migrated).expect("V1 -> V2 migration must succeed");

    // Version was stamped to V2...
    assert_eq!(
        migrated.schema_version,
        SchemaVersion::V2 as i32,
        "migration upgrades the version to V2"
    );
    // ...comments remain empty (nothing to synthesize for a pre-V2 doc)...
    assert!(
        migrated.comments.is_empty(),
        "migration does not invent comments"
    );
    // ...and every geometry byte is preserved.
    assert_eq!(
        geometry_bytes(&migrated),
        geometry_before,
        "geometry is byte-for-byte identical across migration"
    );
    // The geometry fields themselves are also structurally unchanged.
    assert_eq!(migrated.technology, before.technology);
    assert_eq!(migrated.cells, before.cells);
    assert_eq!(migrated.top_cells, before.top_cells);
}

#[test]
fn migrating_an_already_v2_document_is_idempotent() {
    let mut doc = decode_document(GOLDEN_V1).expect("decode V1 fixture");
    migrate_document(&mut doc).expect("first migration");
    let once = doc.clone();
    migrate_document(&mut doc).expect("second migration is a no-op");
    assert_eq!(doc, once, "migrating a V2 document changes nothing");
}

#[test]
fn migration_rejects_unspecified_and_future_versions() {
    let mut unspecified = decode_document(GOLDEN_V1).expect("decode");
    unspecified.schema_version = SchemaVersion::Unspecified as i32;
    assert_eq!(
        migrate_document(&mut unspecified),
        Err(MigrationError::Unspecified),
        "the 0 sentinel cannot be migrated"
    );

    let mut future = decode_document(GOLDEN_V1).expect("decode");
    future.schema_version = 999;
    assert_eq!(
        migrate_document(&mut future),
        Err(MigrationError::Unsupported(999)),
        "a version newer than this build is refused"
    );
}
