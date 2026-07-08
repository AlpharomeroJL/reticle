//! Persisting live comments into a schema-V2 document and back (ADR 0080).
//!
//! These tests exercise the full wire path a comment takes to reach disk: a
//! `reticle_sync::Comment` becomes a `v1::Comment`, rides in the additive
//! `Document.comments` field of a V2 document, survives a prost encode/decode
//! round-trip, and decodes back to the same `Comment`. They also confirm a
//! migrated V1 document (no comments) lists zero.

use reticle_proto::v1::{Document, SchemaVersion};
use reticle_proto::{decode_document, encode_document, migrate::migrate_document};
use reticle_sync::{Comment, from_proto_comments, to_proto_comments};

/// A representative V2 document carrying two anchored comments (a root and a
/// reply on the same thread).
fn document_with_comments() -> (Document, Vec<Comment>) {
    let root = Comment::root(
        "c1",
        "TOP/shape-3",
        "alice",
        "Widen this trace.",
        1_700_000_000_000,
    );
    let reply = Comment::reply_to(&root, "c2", "bob", "Agreed, doing it.", 1_700_000_100_000);
    let comments = vec![root, reply];

    let doc = Document {
        schema_version: SchemaVersion::V2 as i32,
        technology: None,
        cells: vec![],
        top_cells: vec!["TOP".to_owned()],
        comments: to_proto_comments(&comments),
    };
    (doc, comments)
}

#[test]
fn v2_document_round_trip_preserves_comments() {
    let (doc, original) = document_with_comments();

    let bytes = encode_document(&doc);
    let decoded = decode_document(&bytes).expect("V2 document with comments must decode");

    assert_eq!(
        decoded.schema_version,
        SchemaVersion::V2 as i32,
        "still a V2 document after round-trip"
    );
    assert_eq!(decoded.comments.len(), 2, "both comments survived");

    // Decoding the proto comments back yields exactly the comments we stored.
    let restored = from_proto_comments(&decoded.comments);
    assert_eq!(
        restored, original,
        "comments survived the document round-trip"
    );

    // Thread structure is intact: the reply still points at the root.
    assert!(restored[0].is_root(), "first is the thread root");
    assert_eq!(restored[1].in_reply_to, "c1", "reply still bound to root");
    assert_eq!(
        restored[0].anchor_ref, "TOP/shape-3",
        "anchor binding preserved"
    );
}

#[test]
fn a_migrated_v1_document_lists_zero_comments() {
    // A genuine V1 document has no comments; migrating it must not invent any.
    let mut v1 = Document {
        schema_version: SchemaVersion::V1 as i32,
        technology: None,
        cells: vec![],
        top_cells: vec!["TOP".to_owned()],
        comments: vec![],
    };
    migrate_document(&mut v1).expect("V1 -> V2 migration");

    assert_eq!(v1.schema_version, SchemaVersion::V2 as i32);
    assert!(
        from_proto_comments(&v1.comments).is_empty(),
        "a migrated V1 document shows zero comments"
    );
}
