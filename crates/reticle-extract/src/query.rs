//! The F3 trace-query contract: the serializable result records the trace UI consumes.
//!
//! The trace-api lane (Phase 2) adds read-only spatial queries over an already-extracted
//! [`Netlist`](crate::Netlist): net-at-point, net-extent, and a shorts/opens report. Those
//! queries are cached per document revision, so every result record carries a `revision`
//! envelope: it is the cache key, and a UI knows a result is stale when the document's
//! revision has moved past it. The records here are the query RESULTS (what the UI renders),
//! distinct from the internal [`Net`](crate::Net) / [`Short`](crate::Short) /
//! [`Open`](crate::Open) types the extractor works with.
//!
//! Coordinates are `i64` DBU in a standalone [`RectRecord`] because
//! `reticle_geometry::Rect` is not serde-serializable; the records are byte-stable so a
//! cached result hashes and diffs exactly. Fixture-first: the trace-ui lane builds against
//! the committed canned responses (`tests/fixtures/contracts/f3_trace.json`) before the
//! query API exists.

use serde::{Deserialize, Serialize};

/// A rectangle in DBU. Standalone (not `reticle_geometry::Rect`) so a query result
/// serializes; byte-stable.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RectRecord {
    /// Minimum x in DBU.
    pub min_x: i64,
    /// Minimum y in DBU.
    pub min_y: i64,
    /// Maximum x in DBU.
    pub max_x: i64,
    /// Maximum y in DBU.
    pub max_y: i64,
}

/// A net reference: the net name and the indices of the shapes on it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct NetRef {
    /// The net name (a label or the synthesized `net_<n>`).
    pub name: String,
    /// The indices (into the queried document's flattened shapes) that belong to the net.
    pub shape_indices: Vec<usize>,
}

/// The result of a net-at-point query at a document revision.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct NetAtPoint {
    /// The document revision this result was computed against (the cache key).
    pub revision: u64,
    /// The net covering the queried point, or `None` if the point is on no net.
    pub net: Option<NetRef>,
}

/// The result of a net-extent query: a net's bounding box and shape count.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct NetExtent {
    /// The document revision this result was computed against (the cache key).
    pub revision: u64,
    /// The net the extent is for.
    pub net: String,
    /// The net's bounding box in DBU.
    pub bbox: RectRecord,
    /// How many shapes the net spans.
    pub shape_count: usize,
}

/// A short: two distinct nets that are electrically connected but should not be.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ShortRecord {
    /// One shorted net.
    pub net_a: String,
    /// The other shorted net.
    pub net_b: String,
    /// A location where the two nets touch, for the navigable list to zoom to.
    pub at: RectRecord,
}

/// An open: a net that should be a single piece but is split into `pieces` pieces.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct OpenRecord {
    /// The net that is broken.
    pub net: String,
    /// How many disconnected pieces the net is in (at least 2 for an open).
    pub pieces: usize,
}

/// A shorts/opens report at a document revision: the trace UI's navigable list.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct ShortsOpensReport {
    /// The document revision this report was computed against (the cache key).
    pub revision: u64,
    /// The shorts found.
    pub shorts: Vec<ShortRecord>,
    /// The opens found.
    pub opens: Vec<OpenRecord>,
}

impl ShortsOpensReport {
    /// Whether the report is clean: no shorts and no opens.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.shorts.is_empty() && self.opens.is_empty()
    }

    /// The total number of flagged items (shorts plus opens).
    #[must_use]
    pub fn len(&self) -> usize {
        self.shorts.len() + self.opens.len()
    }

    /// Whether the report has no flagged items (the same condition as [`is_clean`](Self::is_clean)).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.is_clean()
    }
}
