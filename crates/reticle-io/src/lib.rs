//! Import and export for Reticle.
//!
//! Supports GDSII (via [`gds21`], Wave 1), the in-house Reticle container format
//! (OASIS-inspired, ADR 0004), a conformant-OASIS *writer* subset, and a
//! technology-file parser. Each format implements the `reticle-model`
//! [`Importer`](reticle_model::Importer)/[`Exporter`](reticle_model::Exporter)
//! traits so the CLI and app treat them uniformly. Parsers are fuzzed (see
//! `fuzz/`) and the GDSII importer is hardened against panics on malformed input.
//!
//! # Formats
//!
//! - [`Gds`], GDSII binary. Rectangles/polygons ↔ boundaries, paths ↔ paths,
//!   labels ↔ TEXT elements, instances ↔ struct refs, arrays ↔ array refs.
//!   See [`mod@gds`].
//! - [`Oasis`], the **Reticle container format (OASIS-inspired, ADR 0004)**: a
//!   compact, self-describing in-house binary that round-trips rectangles,
//!   polygons, paths, and text labels on `(layer, datatype)`, plus placements and
//!   arrays. It borrows OASIS's spirit but is **not** conformant OASIS and no
//!   third-party tool (`KLayout`, `gdstk`) can read it; see [`mod@oasis`] for the
//!   honest scope and gaps.
//! - [`OasisStd`], a genuine **conformant OASIS (SEMI P39) writer** for a practical
//!   subset - `KLayout` reads its output. Export only (no reader). See
//!   [`mod@oasis_std`] for the subset and documented gaps.
//! - [`parse_technology`] and [`write_technology`], a line-oriented
//!   technology-file format (resolution, layer table, DRC rules) and its
//!   canonical serializer. See [`mod@technology`].
//!
//! # Errors
//!
//! The trait methods return [`reticle_model::Result`]. Internally this crate uses
//! the richer [`IoError`], which lowers onto [`reticle_model::ModelError`] at the
//! trait boundary while preserving diagnostic detail up to that point.

pub mod cif;
pub mod dxf;
pub mod error;
pub mod gds;
pub mod gds_stream;
pub mod oasis;
pub mod oasis_std;
pub mod technology;

pub use error::{ImportWarning, IoError, WarningKind};
pub use gds::{Gds, GdsImport};
pub use gds_stream::{GdsEvent, GdsRecordReader};
pub use oasis::Oasis;
pub use oasis_std::OasisStd;
pub use technology::{parse_technology, write_technology};
