//! The per-task connectivity intent spec and its structural report.
//!
//! An [`IntentSpec`] states what a laid-out cell must connect: named nets, each a
//! set of terminals that must be electrically joined, and pairs of nets that must
//! not be. Checking a document against it yields an [`IntentReport`] of the opens
//! (a net whose terminals are not all connected) and shorts (two nets that are
//! connected but must not be), with coordinates. The checker lives in
//! `reticle-extract`; these are the frozen types it produces and consumes.

use serde::{Deserialize, Serialize};

use crate::args::{LayerArg, RectArg};

/// A named connection point on a layer: a labeled region a net attaches to.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Terminal {
    /// The terminal name (often the net or port name).
    pub name: String,
    /// The layer the terminal is on.
    pub layer: LayerArg,
    /// The terminal region, in database units.
    pub region: RectArg,
}

/// A net: a set of terminals that must be electrically connected.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
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
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct IntentSpec {
    /// The nets that must each be fully connected.
    pub nets: Vec<IntentNet>,
    /// Net pairs that must stay electrically separate.
    #[serde(default)]
    pub forbidden: Vec<ForbiddenPair>,
}

/// A net whose terminals are not all on one connected component.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Open {
    /// The affected net.
    pub net: String,
    /// A location near the break.
    pub at: RectArg,
    /// A human-readable description of the disconnection.
    pub detail: String,
}

/// Two nets that are electrically connected but must not be.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Short {
    /// The first net.
    pub net_a: String,
    /// The second net.
    pub net_b: String,
    /// A location where they touch.
    pub at: RectArg,
}

/// The result of checking a document against an [`IntentSpec`].
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
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
