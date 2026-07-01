//! The netlist output types: [`Net`], [`Netlist`], and net [`NetLabel`] seeds.
//!
//! [`Net`] and [`Netlist`] are the Wave 0 frozen contract, enriched additively: a
//! net keeps its [`name`](Net::name) and [`shape_count`](Net::shape_count) and now
//! also records the [`shapes`](Net::shapes) that belong to it, so the renderer can
//! highlight a whole net from a single shape hit.

use reticle_geometry::{LayerId, Point};

/// A single extracted net: a maximal set of shapes that are electrically
/// connected.
///
/// The frozen fields [`name`](Self::name) and [`shape_count`](Self::shape_count)
/// are preserved; [`shapes`](Self::shapes) is an additive enrichment listing the
/// member shape indices (into the extracted cell's shape list) for highlighting.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Net {
    /// Net name (assigned or inferred from a label).
    pub name: String,
    /// Number of connected shapes on the net.
    pub shape_count: usize,
    /// Indices of the shapes on this net, sorted ascending. Length equals
    /// [`shape_count`](Self::shape_count).
    pub shapes: Vec<usize>,
}

impl Net {
    /// Creates a net with `name` and the given member shape indices. `shape_count`
    /// is derived from the members; the indices are sorted and de-duplicated.
    #[must_use]
    pub fn new(name: impl Into<String>, mut shapes: Vec<usize>) -> Self {
        shapes.sort_unstable();
        shapes.dedup();
        Self {
            name: name.into(),
            shape_count: shapes.len(),
            shapes,
        }
    }

    /// Returns `true` if `shape` is a member of this net.
    #[must_use]
    pub fn contains(&self, shape: usize) -> bool {
        self.shapes.binary_search(&shape).is_ok()
    }
}

/// The result of extraction: the set of connected nets.
///
/// Nets are emitted in a stable order (by their lowest member shape index), so the
/// same layout always yields the same netlist.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Netlist {
    /// The extracted nets.
    pub nets: Vec<Net>,
}

impl Netlist {
    /// Creates a netlist from its nets.
    #[must_use]
    pub fn new(nets: Vec<Net>) -> Self {
        Self { nets }
    }

    /// The number of nets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nets.len()
    }

    /// Returns `true` if there are no nets.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nets.is_empty()
    }

    /// Returns the net containing `shape`, if any.
    #[must_use]
    pub fn net_of(&self, shape: usize) -> Option<&Net> {
        self.nets.iter().find(|n| n.contains(shape))
    }

    /// The member shape indices of the net named `name`, if present. Convenience
    /// for the renderer's net-highlight path.
    #[must_use]
    pub fn shapes_of(&self, name: &str) -> Option<&[usize]> {
        self.nets
            .iter()
            .find(|n| n.name == name)
            .map(|n| n.shapes.as_slice())
    }
}

/// A seed that names a net: whichever net covers a shape touching
/// [`point`](Self::point) on [`layer`](Self::layer) takes [`name`](Self::name).
///
/// The model has no text/label primitive, so net names are supplied to the
/// extractor out-of-band through these seeds (mirroring how a real flow attaches
/// pin/label geometry). A net with no matching label falls back to `net_<n>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetLabel {
    /// The name to assign to the covering net.
    pub name: String,
    /// A point that must lie on (or touch) a shape of the target net.
    pub point: Point,
    /// The layer the labelled shape lives on.
    pub layer: LayerId,
}

impl NetLabel {
    /// Creates a label placing `name` at `point` on `layer`.
    #[must_use]
    pub fn new(name: impl Into<String>, point: Point, layer: LayerId) -> Self {
        Self {
            name: name.into(),
            point,
            layer,
        }
    }
}
