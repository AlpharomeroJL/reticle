//! Connectivity extraction for Reticle.
//!
//! Wave 2 implements geometric connectivity per net across same-layer contact and
//! cross-layer vias, net highlighting, and a lightweight compare against an
//! expected netlist.
//!
//! The Wave 0 contract is [`Extractor`] and the [`Netlist`] it produces.

use reticle_model::Document;

/// A single extracted net.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Net {
    /// Net name (assigned or inferred from a label).
    pub name: String,
    /// Number of connected shapes on the net.
    pub shape_count: usize,
}

/// The result of extraction: the set of connected nets.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Netlist {
    /// The extracted nets.
    pub nets: Vec<Net>,
}

/// The connectivity extractor (Wave 2).
#[derive(Debug, Default, Clone)]
pub struct Extractor;

impl Extractor {
    /// Creates an extractor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Extracts connectivity for a cell of `doc`.
    #[must_use]
    pub fn extract(&self, _doc: &Document, _cell: &str) -> Netlist {
        // Wave 2: union-find over touching/overlapping shapes and vias.
        Netlist::default()
    }
}
