//! Connectivity extraction for Reticle.
//!
//! Wave 2 implements geometric connectivity per net across same-layer contact and
//! cross-layer vias, net highlighting, and a lightweight compare against an
//! expected netlist (the geometric half of an LVS check).
//!
//! # Contract
//!
//! The Wave 0 contract is [`Extractor`] and the [`Netlist`] it produces. Both are
//! preserved and enriched additively: [`Net`] now also records its member shape
//! indices, and [`Extractor`] gains configurable [`ConnectionRules`] and
//! [`NetLabel`] seeds.
//!
//! # How it works
//!
//! 1. Build a bulk-loaded R-tree over every shape's bounding box.
//! 2. Union same-layer touching/overlapping shapes and, per
//!    [`ConnectionRule`], the conductors each via shape bridges, all in a
//!    disjoint set (union-find). Candidate pairs come from spatial-index queries,
//!    not an `O(n²)` scan.
//! 3. Emit one [`Net`] per connected component, named from a matching
//!    [`NetLabel`] or else `net_<n>`.
//!
//! ```
//! use reticle_extract::Extractor;
//! use reticle_geometry::{LayerId, Point, Rect};
//! use reticle_model::{Cell, Document, DrawShape, ShapeKind};
//!
//! let metal = LayerId::new(1, 0);
//! let mut cell = Cell::new("top");
//! // Two overlapping rectangles on one layer → a single net.
//! cell.shapes.push(DrawShape::new(metal, ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(10, 10)))));
//! cell.shapes.push(DrawShape::new(metal, ShapeKind::Rect(Rect::new(Point::new(5, 5), Point::new(15, 15)))));
//! let mut doc = Document::new();
//! doc.insert_cell(cell);
//!
//! let netlist = Extractor::new().extract(&doc, "top");
//! assert_eq!(netlist.nets.len(), 1);
//! assert_eq!(netlist.nets[0].shape_count, 2);
//! ```

mod compare;
mod connectivity;
pub mod device;
mod intent;
mod intent_check;
mod netlist;
pub mod query;
mod rules;
mod union_find;

pub use compare::{NetlistDiff, ShapePair, compare_netlists};
pub use connectivity::{build_components, rects_touch, shape_covers_point, shapes_touch};
// Device recognition (Wave 6): a new sibling layer over the frozen connectivity
// types above. Additive re-exports only; nothing above is changed or shadowed.
pub use device::{
    Device, DeviceDiff, DeviceKind, DeviceNetlist, DeviceSummary, DeviceTech, compare_devices,
    extract_devices, extract_devices_labeled,
};
pub use intent::{ForbiddenPair, IntentNet, IntentReport, IntentSpec, Open, Short, Terminal};
pub use intent_check::{check_intent, sky130_connection_rules, terminal};
pub use netlist::{Net, NetLabel, Netlist};
pub use query::{
    NetAtPoint, NetExtent, NetRef, OpenRecord, RectRecord, ShortRecord, ShortsOpensReport,
    net_at_point, net_extent, shorts_opens,
};
pub use rules::{ConnectionRule, ConnectionRules};
pub use union_find::DisjointSet;

// --- lane netlist: spice writer ---
pub mod spice;
pub use spice::{
    SpiceDevice, SpiceParseError, SpiceSubckt, SpiceTech, UNBOUND_NODE, format_spice, parse_spice,
    to_spice_subckt, write_spice,
};

use reticle_model::{Document, DrawShape};

/// The connectivity extractor.
///
/// A default extractor connects only same-layer touching/overlapping shapes. Add
/// via/contact behaviour with [`with_rules`](Self::with_rules) and net names with
/// [`with_labels`](Self::with_labels); both are additive and preserve the Wave 0
/// [`extract`](Self::extract) signature.
#[derive(Debug, Default, Clone)]
pub struct Extractor {
    rules: ConnectionRules,
    labels: Vec<NetLabel>,
}

impl Extractor {
    /// Creates an extractor with no via rules and no labels (same-layer
    /// connectivity, `net_<n>` names).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the extractor configured with `rules` for cross-layer via/contact
    /// connectivity.
    #[must_use]
    pub fn with_rules(mut self, rules: ConnectionRules) -> Self {
        self.rules = rules;
        self
    }

    /// Returns the extractor configured with net-naming `labels`.
    #[must_use]
    pub fn with_labels(mut self, labels: Vec<NetLabel>) -> Self {
        self.labels = labels;
        self
    }

    /// The configured connection rules.
    #[must_use]
    pub fn rules(&self) -> &ConnectionRules {
        &self.rules
    }

    /// Extracts connectivity for the named cell of `doc`.
    ///
    /// The cell's *own* shapes are used (hierarchy is not flattened here; call
    /// [`extract_shapes`](Self::extract_shapes) with `doc.flatten(cell)` to extract
    /// a flattened cell). An unknown cell yields an empty netlist.
    #[must_use]
    pub fn extract(&self, doc: &Document, cell: &str) -> Netlist {
        let Some(cell) = doc.cell(cell) else {
            return Netlist::default();
        };
        self.extract_shapes(&cell.shapes)
    }

    /// Extracts connectivity over an explicit flat shape list.
    ///
    /// Net member indices in the result index into `shapes`. This is the entry
    /// point for extracting flattened geometry (e.g. `doc.flatten(top)`).
    #[must_use]
    pub fn extract_shapes(&self, shapes: &[DrawShape]) -> Netlist {
        let mut dsu = connectivity::build_components(shapes, &self.rules);

        // Group shape indices by their disjoint-set root, preserving ascending
        // order within each group. Track first-seen order of roots so nets are
        // emitted deterministically by their lowest member index.
        let mut root_order: Vec<usize> = Vec::new();
        let mut groups: std::collections::HashMap<usize, Vec<usize>> =
            std::collections::HashMap::new();
        for i in 0..shapes.len() {
            let root = dsu.find(i);
            let entry = groups.entry(root).or_insert_with(|| {
                root_order.push(root);
                Vec::new()
            });
            entry.push(i);
        }

        // Assign names: a label matches a net if any of the net's shapes is on the
        // label's layer and covers the label point. The first matching label wins.
        let mut nets = Vec::with_capacity(root_order.len());
        let mut auto = 0usize;
        for root in root_order {
            let members = groups.remove(&root).unwrap_or_default();
            let name = self.name_for(shapes, &members).unwrap_or_else(|| {
                let n = format!("net_{auto}");
                auto += 1;
                n
            });
            nets.push(Net::new(name, members));
        }
        Netlist::new(nets)
    }

    /// Finds a label naming the net whose `members` include a shape the label
    /// covers, returning that label's name.
    fn name_for(&self, shapes: &[DrawShape], members: &[usize]) -> Option<String> {
        self.labels.iter().find_map(|label| {
            members
                .iter()
                .any(|&i| {
                    let s = &shapes[i];
                    s.layer == label.layer && connectivity::shape_covers_point(s, label.point)
                })
                .then(|| label.name.clone())
        })
    }

    /// Compares an `extracted` netlist against an `expected` one, returning the
    /// shape pairs they disagree on (opens and shorts), the geometric half of an
    /// LVS check. See [`compare_netlists`].
    #[must_use]
    pub fn compare(&self, extracted: &Netlist, expected: &Netlist) -> NetlistDiff {
        compare_netlists(extracted, expected)
    }
}
