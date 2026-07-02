//! Stable element identifiers.
//!
//! Commands like `query_shapes`, `delete_shapes`, and `transform_shapes` address
//! elements by a stable [`ElementId`] rather than a positional index, because the
//! underlying [`reticle_model::Edit`] vocabulary shifts positional indices when a
//! shape is removed. The session allocates a fresh id per placed element and
//! maps it to the element's current slot, so an id keeps addressing the same
//! element across edits.

use serde::{Deserialize, Serialize};

/// A stable, session-unique identifier for a placed element: a shape, an
/// instance, an array, or a label.
///
/// Allocated monotonically by the session; never reused within a session. A
/// mutating command that creates elements returns the new ids so a caller (or an
/// agent) can address them later.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct ElementId(pub u64);

impl std::fmt::Display for ElementId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "e{}", self.0)
    }
}
