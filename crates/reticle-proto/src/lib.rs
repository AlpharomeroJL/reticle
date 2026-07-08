//! Versioned Protobuf schema for Reticle.
//!
//! The frozen schema (a Wave 0 contract, spec Section 6) lives in
//! `proto/reticle.proto`: geometry in integer database units, the hierarchical
//! document (cells, instances, arrays, layers, technology), the CRDT update
//! envelope, and presence and comment messages.
//!
//! In Wave 1 this crate wires `prost-build` with the vendored `protoc`
//! (ADR 0008) to generate Rust types into the [`v1`] module and exposes
//! [`encode_document`] / [`decode_document`] helpers, plus a [`migrate`] path
//! keyed on [`SCHEMA_VERSION`]. Conversions to and from the native
//! `reticle-model` types live in a separate crate.

use prost::Message;

/// Generated Protobuf types for schema package `reticle.v1`.
///
/// The contents are produced at build time by `prost-build` from
/// `proto/reticle.proto` and included verbatim. Generated code carries no
/// rustdoc and can trip pedantic lints, so documentation and the relevant
/// Clippy lints are relaxed for this module only.
pub mod v1 {
    #![allow(missing_docs)]
    #![allow(clippy::pedantic)]
    include!(concat!(env!("OUT_DIR"), "/reticle.v1.rs"));
}

/// The current schema version. Bump on any breaking change to
/// `proto/reticle.proto` and add a migration keyed on the previous value.
///
/// V2 (ADR 0080) adds the additive `Document.comments` field. V1 documents
/// remain readable and upgrade losslessly through [`migrate::migrate_document`].
pub const SCHEMA_VERSION: u32 = 2;

/// Encodes a [`v1::Document`] into a freshly allocated Protobuf byte buffer.
///
/// The encoding is the canonical prost length-prefixed field wire format and is
/// the inverse of [`decode_document`].
#[must_use]
pub fn encode_document(document: &v1::Document) -> Vec<u8> {
    document.encode_to_vec()
}

/// Decodes a [`v1::Document`] from a Protobuf byte buffer.
///
/// # Errors
///
/// Returns [`prost::DecodeError`] if `bytes` is not a valid encoding of a
/// [`v1::Document`] (for example, truncated input or a malformed field).
pub fn decode_document(bytes: &[u8]) -> Result<v1::Document, prost::DecodeError> {
    v1::Document::decode(bytes)
}

/// Schema migration between versions.
///
/// The migration contract (ADR 0080): a document tagged with any supported
/// version upgrades in place to the current [`SCHEMA_VERSION`] losslessly. The
/// V1 to V2 step is purely additive - it stamps the version to V2 and leaves the
/// (already empty) `comments` list untouched, so every V1 geometry byte is
/// preserved. The frozen V1 golden fixture proves this against real pre-V2 bytes.
pub mod migrate {
    use super::{SCHEMA_VERSION, v1};

    /// Reason a [`v1::Document`] could not be migrated to the current schema.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum MigrationError {
        /// The `schema_version` field was `SCHEMA_VERSION_UNSPECIFIED` (0), which
        /// carries no version and cannot be interpreted.
        Unspecified,
        /// The document was tagged with a version this build does not know how to
        /// read (newer than [`SCHEMA_VERSION`]). The wrapped value is that tag.
        Unsupported(u32),
    }

    impl core::fmt::Display for MigrationError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match self {
                Self::Unspecified => f.write_str("document has an unspecified schema version"),
                Self::Unsupported(v) => {
                    write!(
                        f,
                        "document schema version {v} is newer than supported {SCHEMA_VERSION}"
                    )
                }
            }
        }
    }

    impl core::error::Error for MigrationError {}

    /// Returns `true` if a document tagged `version` can be read by this build,
    /// possibly after migration.
    #[must_use]
    pub fn is_supported(version: u32) -> bool {
        (1..=SCHEMA_VERSION).contains(&version)
    }

    /// Upgrades `document` in place to the current [`SCHEMA_VERSION`], preserving
    /// all geometry losslessly.
    ///
    /// A V1 document is upgraded to V2 by stamping `schema_version` to V2; the
    /// additive `comments` field is left as it decoded (empty for genuine V1
    /// bytes). A document already at the current version is left unchanged. The
    /// technology, cells and top-cell list are never touched, so the geometry is
    /// byte-for-byte identical before and after (verified by the golden-fixture
    /// migration test).
    ///
    /// # Errors
    ///
    /// Returns [`MigrationError::Unspecified`] if `schema_version` is the 0
    /// sentinel, or [`MigrationError::Unsupported`] if it is newer than this
    /// build understands.
    pub fn migrate_document(document: &mut v1::Document) -> Result<(), MigrationError> {
        // `schema_version` is stored as the enum's i32; treat unknown negatives as
        // out of range too by going through u32.
        let version = u32::try_from(document.schema_version)
            .map_err(|_| MigrationError::Unsupported(SCHEMA_VERSION + 1))?;

        match version {
            0 => return Err(MigrationError::Unspecified),
            v if v > SCHEMA_VERSION => return Err(MigrationError::Unsupported(v)),
            _ => {}
        }

        // Every supported version reads forward to the current one. The only
        // versioned change from V1 to V2 is the additive `comments` field, which
        // needs no data movement; stamp the current version and return.
        document.schema_version = v1::SchemaVersion::V2 as i32;
        Ok(())
    }
}

/// The first byte of a length-delimited [`v1::SyncMessage`] frame carrying a
/// [`v1::CrdtUpdate`] (`payload` field 1). This is the protobuf tag byte
/// `(1 << 3) | 2` (field 1, wire type 2 = length-delimited).
pub const SYNC_TAG_UPDATE: u8 = 0x0A;

/// The first byte of a length-delimited [`v1::SyncMessage`] frame carrying a
/// [`v1::Presence`] (`payload` field 2): tag `(2 << 3) | 2`.
pub const SYNC_TAG_PRESENCE: u8 = 0x12;

/// The first byte of a length-delimited [`v1::SyncMessage`] frame carrying a
/// [`v1::Comment`] (`payload` field 3): tag `(3 << 3) | 2`.
pub const SYNC_TAG_COMMENT: u8 = 0x1A;

#[cfg(test)]
mod wire_invariant {
    //! Freezes the `SyncMessage` first-byte wire invariant (ADR 0061).
    //!
    //! A relay classifies a live frame by its first byte alone: 0x0A update,
    //! 0x12 presence, 0x1A comment. This is a direct consequence of `payload`
    //! being a protobuf `oneof` (each variant is a length-delimited message
    //! field, so the first emitted byte is the field's tag). The Cloudflare
    //! Durable Object relay depends on it; this test fails loudly if the field
    //! numbers ever move.

    use super::{SYNC_TAG_COMMENT, SYNC_TAG_PRESENCE, SYNC_TAG_UPDATE, v1};
    use prost::Message as _;

    /// Wraps `payload` in a `SyncMessage` and returns its prost encoding.
    fn encode(payload: v1::sync_message::Payload) -> Vec<u8> {
        v1::SyncMessage {
            payload: Some(payload),
        }
        .encode_to_vec()
    }

    #[test]
    fn update_frame_first_byte_is_the_update_tag() {
        let bytes = encode(v1::sync_message::Payload::Update(v1::CrdtUpdate {
            schema_version: v1::SchemaVersion::V1 as i32,
            doc_id: "doc".to_owned(),
            actor: "alice".to_owned(),
            update: vec![1, 2, 3],
        }));
        assert_eq!(bytes[0], SYNC_TAG_UPDATE, "update frames start with 0x0A");
        assert_eq!(SYNC_TAG_UPDATE, 0x0A);
    }

    #[test]
    fn presence_frame_first_byte_is_the_presence_tag() {
        let bytes = encode(v1::sync_message::Payload::Presence(v1::Presence {
            actor: "alice".to_owned(),
            ..Default::default()
        }));
        assert_eq!(
            bytes[0], SYNC_TAG_PRESENCE,
            "presence frames start with 0x12"
        );
        assert_eq!(SYNC_TAG_PRESENCE, 0x12);
    }

    #[test]
    fn comment_frame_first_byte_is_the_comment_tag() {
        let bytes = encode(v1::sync_message::Payload::Comment(v1::Comment {
            id: "c1".to_owned(),
            body: "hi".to_owned(),
            ..Default::default()
        }));
        assert_eq!(bytes[0], SYNC_TAG_COMMENT, "comment frames start with 0x1A");
        assert_eq!(SYNC_TAG_COMMENT, 0x1A);
    }

    #[test]
    fn the_three_tags_are_distinct() {
        // The relay's whole classification scheme rests on these being different.
        let tags = [SYNC_TAG_UPDATE, SYNC_TAG_PRESENCE, SYNC_TAG_COMMENT];
        assert_eq!(
            tags.iter().collect::<std::collections::HashSet<_>>().len(),
            3,
            "each variant must have a distinct first byte"
        );
    }

    #[test]
    fn an_empty_variant_still_leads_with_its_tag() {
        // Even a payload sub-message with no populated fields is length-delimited,
        // so the tag byte is emitted regardless of contents. This is what lets the
        // relay key off the byte without ever decoding the body.
        let bytes = encode(v1::sync_message::Payload::Update(v1::CrdtUpdate::default()));
        assert_eq!(bytes[0], SYNC_TAG_UPDATE);
    }
}
