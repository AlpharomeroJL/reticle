//! Versioned Protobuf schema for Reticle.
//!
//! The frozen schema (a Wave 0 contract, spec Section 6) lives in
//! `proto/reticle.proto`: geometry in integer database units, the hierarchical
//! document (cells, instances, arrays, layers, technology), the CRDT update
//! envelope, and presence and comment messages.
//!
//! In Wave 1 this crate wires `prost-build` with the vendored `protoc`
//! (ADR 0008) to generate Rust types into a `v1` module, plus conversions to and
//! from the native `reticle-model` types and a [`migrate`] path keyed on
//! [`SCHEMA_VERSION`].

/// The current schema version. Bump on any breaking change to
/// `proto/reticle.proto` and add a migration keyed on the previous value.
pub const SCHEMA_VERSION: u32 = 1;

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
