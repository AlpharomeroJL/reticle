//! The per-task connectivity intent spec and its structural report.
//!
//! An [`IntentSpec`] states what a laid-out cell must connect: named nets, each a
//! set of terminals that must be electrically joined, and pairs of nets that must
//! not be. Checking a document against it yields an [`IntentReport`] of the opens
//! (a net whose terminals are not all connected) and shorts (two nets that are
//! connected but must not be), with coordinates. These types live here, next to
//! the connectivity extraction the checker builds on; the checker itself is
//! implemented in a later wave. `reticle-agent-api` re-exports them.

use reticle_geometry::{LayerId, Rect};
use serde::{Deserialize, Serialize};

/// A named connection point on a layer: a labeled region a net attaches to.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Terminal {
    /// The terminal name (often the net or port name).
    pub name: String,
    /// The layer the terminal is on.
    pub layer: LayerId,
    /// The terminal region, in database units.
    pub region: Rect,
}

/// A net: a set of terminals that must be electrically connected.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct IntentNet {
    /// The net name.
    pub name: String,
    /// The terminals that must all be joined.
    pub terminals: Vec<Terminal>,
}

/// A pair of nets that must not be connected to each other.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ForbiddenPair {
    /// The first net name.
    pub net_a: String,
    /// The second net name.
    pub net_b: String,
}

/// The connectivity intent for a task.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct IntentSpec {
    /// The nets that must each be fully connected.
    pub nets: Vec<IntentNet>,
    /// Net pairs that must stay electrically separate.
    #[serde(default)]
    pub forbidden: Vec<ForbiddenPair>,
}

/// A net whose terminals are not all on one connected component.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Open {
    /// The affected net.
    pub net: String,
    /// A location near the break.
    pub at: Rect,
    /// A human-readable description of the disconnection.
    pub detail: String,
}

/// Two nets that are electrically connected but must not be.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Short {
    /// The first net.
    pub net_a: String,
    /// The second net.
    pub net_b: String,
    /// A location where they touch.
    pub at: Rect,
}

/// The result of checking a document against an [`IntentSpec`].
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct IntentReport {
    /// Nets that are not fully connected.
    pub opens: Vec<Open>,
    /// Net pairs that are connected but must not be.
    pub shorts: Vec<Short>,
}

impl IntentReport {
    /// True when there are no opens and no shorts.
    #[must_use]
    pub fn is_satisfied(&self) -> bool {
        self.opens.is_empty() && self.shorts.is_empty()
    }
}
