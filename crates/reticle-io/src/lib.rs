//! Import and export for Reticle.
//!
//! Supports GDSII (via [`gds21`], Wave 1), an in-house OASIS-inspired subset
//! (Wave 1, ADR 0004), and a technology-file parser. Each format implements the
//! `reticle-model` [`Importer`](reticle_model::Importer)/[`Exporter`](reticle_model::Exporter)
//! traits so the CLI and app treat them uniformly. Parsers are fuzzed (see
//! `fuzz/`) and the GDSII importer is hardened against panics on malformed input.
//!
//! # Formats
//!
//! - [`Gds`] — GDSII binary. Rectangles/polygons ↔ boundaries, paths ↔ paths,
//!   instances ↔ struct refs, arrays ↔ array refs. See [`mod@gds`].
//! - [`Oasis`] — a compact, self-describing binary subset that round-trips
//!   rectangles and polygons on `(layer, datatype)`. It is **not** conformant
//!   OASIS; see [`mod@oasis`] for the honest scope and gaps.
//! - [`parse_technology`] — a line-oriented technology-file format (resolution,
//!   layer table, DRC rules). See [`mod@technology`].
//!
//! # Errors
//!
//! The trait methods return [`reticle_model::Result`]. Internally this crate uses
//! the richer [`IoError`], which lowers onto [`reticle_model::ModelError`] at the
//! trait boundary while preserving diagnostic detail up to that point.

pub mod error;
pub mod gds;
pub mod oasis;
pub mod technology;

pub use error::IoError;
pub use gds::Gds;
pub use oasis::Oasis;
pub use technology::parse_technology;
