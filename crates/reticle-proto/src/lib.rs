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
pub const SCHEMA_VERSION: u32 = 1;

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
/// Wave 1 fills this in as generated types land. It exists in Wave 0 so the
/// versioning contract is visible to dependents.
pub mod migrate {
    use super::SCHEMA_VERSION;

    /// Returns `true` if a document tagged `version` can be read by this build,
    /// possibly after migration.
    #[must_use]
    pub fn is_supported(version: u32) -> bool {
        (1..=SCHEMA_VERSION).contains(&version)
    }
}
