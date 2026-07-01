//! Comparing an extracted netlist against an expected one — the geometric half of
//! an LVS (layout-versus-schematic) check.
//!
//! The two netlists are compared purely by *which shapes share a net*, not by net
//! name or order (extraction assigns arbitrary `net_<n>` names, so names cannot be
//! the key). Concretely, for every unordered pair of shapes we ask: does the
//! extracted netlist put them on the same net, and does the expected one? A
//! disagreement is reported as either a [`missing connection`](NetlistDiff::missing)
//! (expected together, extracted apart — an *open*) or an
//! [`extra connection`](NetlistDiff::extra) (extracted together, expected apart —
//! a *short*).
//!
//! Pairs are summarised as [`ShapePair`]s (always stored with `a < b`). Reporting
//! representative pairs rather than whole nets keeps the diff small and points
//! directly at the offending shapes for highlighting.

use std::collections::BTreeSet;

use crate::netlist::Netlist;
use crate::union_find::DisjointSet;

/// An unordered pair of shape indices, normalised so `a < b`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShapePair {
    /// The lower shape index.
    pub a: usize,
    /// The higher shape index.
    pub b: usize,
}

impl ShapePair {
    /// Creates a normalised pair (`a < b`). Panics-free: equal or reversed inputs
    /// are ordered.
    #[must_use]
    pub fn new(x: usize, y: usize) -> Self {
        if x <= y {
            Self { a: x, b: y }
        } else {
            Self { a: y, b: x }
        }
    }
}

/// The differences between an extracted and an expected netlist.
///
/// Empty (both lists agree on every shape pair) means the layouts are
/// connectivity-equivalent over the shapes they mention.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetlistDiff {
    /// Pairs that should be connected (same net in `expected`) but are *not* in the
    /// extracted netlist — opens.
    pub missing: Vec<ShapePair>,
    /// Pairs that are connected in the extracted netlist but should *not* be
    /// (different nets in `expected`) — shorts.
    pub extra: Vec<ShapePair>,
}

impl NetlistDiff {
    /// Returns `true` if the netlists agree on every shape pair.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.extra.is_empty()
    }

    /// The total number of disagreeing pairs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.missing.len() + self.extra.len()
    }
}

/// Rebuilds a disjoint set from a netlist's per-net shape membership over
/// `shape_count` shapes.
///
/// Every net unions its members; shapes not mentioned by any net stay singletons.
fn dsu_from_netlist(netlist: &Netlist, shape_count: usize) -> DisjointSet {
    let mut dsu = DisjointSet::new(shape_count);
    for net in &netlist.nets {
        let mut members = net.shapes.iter().copied();
        if let Some(first) = members.next() {
            for m in members {
                if first < shape_count && m < shape_count {
                    dsu.union(first, m);
                }
            }
        }
    }
    dsu
}

/// The set of shape indices referenced by either netlist, plus a shared upper
/// bound so both disjoint sets are sized identically.
fn shape_universe(a: &Netlist, b: &Netlist) -> (BTreeSet<usize>, usize) {
    let mut universe = BTreeSet::new();
    for net in a.nets.iter().chain(&b.nets) {
        universe.extend(net.shapes.iter().copied());
    }
    let bound = universe.iter().copied().max().map_or(0, |m| m + 1);
    (universe, bound)
}

/// Compares `extracted` against `expected`, returning every shape pair the two
/// disagree on.
///
/// Both netlists are reduced to disjoint sets over the union of the shape indices
/// they mention; then every unordered pair of referenced shapes is classified.
/// The comparison is symmetric in the two representations and independent of net
/// names and ordering.
#[must_use]
pub fn compare_netlists(extracted: &Netlist, expected: &Netlist) -> NetlistDiff {
    let (universe, bound) = shape_universe(extracted, expected);
    let mut got = dsu_from_netlist(extracted, bound);
    let mut want = dsu_from_netlist(expected, bound);

    let ids: Vec<usize> = universe.into_iter().collect();
    let mut diff = NetlistDiff::default();
    for (idx, &i) in ids.iter().enumerate() {
        for &j in &ids[idx + 1..] {
            let same_expected = want.connected(i, j);
            let same_extracted = got.connected(i, j);
            match (same_expected, same_extracted) {
                (true, false) => diff.missing.push(ShapePair::new(i, j)),
                (false, true) => diff.extra.push(ShapePair::new(i, j)),
                _ => {}
            }
        }
    }
    diff.missing.sort_unstable();
    diff.extra.sort_unstable();
    diff
}
