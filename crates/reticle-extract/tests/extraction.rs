//! Unit tests for connectivity extraction: same-layer touching, disjoint shapes,
//! cross-layer vias (present and missing), net naming, highlighting, and compare.

use reticle_extract::{
    ConnectionRule, ConnectionRules, Extractor, Net, NetLabel, Netlist, ShapePair,
};
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, Document, DrawShape, ShapeKind};

const M1: LayerId = LayerId::new(1, 0);
const M2: LayerId = LayerId::new(2, 0);
const VIA: LayerId = LayerId::new(10, 0);

/// A rectangle draw-shape on `layer` spanning `[(x0,y0), (x1,y1)]`.
fn rect(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
    DrawShape::new(
        layer,
        ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
    )
}

/// Builds a single-cell document named `top` from a shape list.
fn doc_with(shapes: Vec<DrawShape>) -> Document {
    let mut cell = Cell::new("top");
    cell.shapes = shapes;
    let mut doc = Document::new();
    doc.insert_cell(cell);
    doc
}

// --- Same-layer connectivity -------------------------------------------------

#[test]
fn two_overlapping_rects_form_one_net() {
    let doc = doc_with(vec![
        rect(M1, 0, 0, 10, 10),
        rect(M1, 5, 5, 15, 15), // overlaps the first
    ]);
    let netlist = Extractor::new().extract(&doc, "top");
    assert_eq!(netlist.nets.len(), 1, "one connected net");
    assert_eq!(netlist.nets[0].shape_count, 2);
    assert_eq!(netlist.nets[0].shapes, vec![0, 1]);
}

#[test]
fn two_edge_touching_rects_form_one_net() {
    // Abutting end-to-end along x = 10: closed-box touch, one net.
    let doc = doc_with(vec![rect(M1, 0, 0, 10, 10), rect(M1, 10, 0, 20, 10)]);
    let netlist = Extractor::new().extract(&doc, "top");
    assert_eq!(netlist.nets.len(), 1, "edge-abutting rects are connected");
    assert_eq!(netlist.nets[0].shape_count, 2);
}

#[test]
fn two_disjoint_rects_form_two_nets() {
    let doc = doc_with(vec![
        rect(M1, 0, 0, 10, 10),
        rect(M1, 100, 100, 110, 110), // far away
    ]);
    let netlist = Extractor::new().extract(&doc, "top");
    assert_eq!(netlist.nets.len(), 2, "two separate nets");
    for net in &netlist.nets {
        assert_eq!(net.shape_count, 1);
    }
}

#[test]
fn same_layer_only_connects_within_layer() {
    // One rect on M1 and one on M2, overlapping in space, but no via rule: the
    // different layers must stay on separate nets.
    let doc = doc_with(vec![rect(M1, 0, 0, 10, 10), rect(M2, 0, 0, 10, 10)]);
    let netlist = Extractor::new().extract(&doc, "top");
    assert_eq!(
        netlist.nets.len(),
        2,
        "overlapping shapes on different layers do not connect without a via"
    );
}

#[test]
fn transitive_chain_is_one_net() {
    // A—B—C touch pairwise in a chain; all three land on one net via union-find.
    let doc = doc_with(vec![
        rect(M1, 0, 0, 10, 10),
        rect(M1, 9, 0, 19, 10),
        rect(M1, 18, 0, 28, 10),
    ]);
    let netlist = Extractor::new().extract(&doc, "top");
    assert_eq!(netlist.nets.len(), 1);
    assert_eq!(netlist.nets[0].shape_count, 3);
}

// --- Cross-layer vias --------------------------------------------------------

fn via_rules() -> ConnectionRules {
    ConnectionRules::new().with_rule(ConnectionRule::new(M1, VIA, M2))
}

#[test]
fn via_connecting_two_layers_merges_their_nets() {
    // M1 pad and M2 pad overlap in space; a VIA square sits on both. With the via
    // rule, all three shapes become one net.
    let doc = doc_with(vec![
        rect(M1, 0, 0, 20, 20),  // 0: bottom conductor
        rect(M2, 0, 0, 20, 20),  // 1: top conductor
        rect(VIA, 5, 5, 15, 15), // 2: via landing on both
    ]);
    let netlist = Extractor::new()
        .with_rules(via_rules())
        .extract(&doc, "top");
    assert_eq!(netlist.nets.len(), 1, "via merges the two conductor nets");
    assert_eq!(netlist.nets[0].shape_count, 3);
}

#[test]
fn missing_via_leaves_layers_separate() {
    // Same two conductors overlapping in space, but no via shape at all: the rule
    // exists yet nothing bridges them, so two nets remain.
    let doc = doc_with(vec![rect(M1, 0, 0, 20, 20), rect(M2, 0, 0, 20, 20)]);
    let netlist = Extractor::new()
        .with_rules(via_rules())
        .extract(&doc, "top");
    assert_eq!(
        netlist.nets.len(),
        2,
        "no via shape means the conductors stay on separate nets"
    );
}

#[test]
fn via_touching_only_bottom_does_not_bridge() {
    // The via overlaps the M1 pad but not the M2 pad (which is placed away). A via
    // must land on BOTH conductors to bridge; here it does not, so M2 is separate.
    let doc = doc_with(vec![
        rect(M1, 0, 0, 20, 20),       // 0: bottom
        rect(M2, 100, 100, 120, 120), // 1: top, far away
        rect(VIA, 5, 5, 15, 15),      // 2: via on M1 only
    ]);
    let netlist = Extractor::new()
        .with_rules(via_rules())
        .extract(&doc, "top");
    // Net{M1, via} and net{M2}.
    assert_eq!(netlist.nets.len(), 2);
    let m2_net = netlist.net_of(1).expect("shape 1 belongs to some net");
    assert_eq!(m2_net.shape_count, 1, "the far M2 pad is alone");
}

#[test]
fn via_layer_shapes_do_not_self_connect_across_gaps() {
    // Two separate via squares over two independent conductor stacks must yield two
    // nets, not one — vias only connect the conductors they physically overlap.
    let doc = doc_with(vec![
        rect(M1, 0, 0, 10, 10),    // stack A bottom
        rect(M2, 0, 0, 10, 10),    // stack A top
        rect(VIA, 2, 2, 8, 8),     // stack A via
        rect(M1, 50, 50, 60, 60),  // stack B bottom
        rect(M2, 50, 50, 60, 60),  // stack B top
        rect(VIA, 52, 52, 58, 58), // stack B via
    ]);
    let netlist = Extractor::new()
        .with_rules(via_rules())
        .extract(&doc, "top");
    assert_eq!(netlist.nets.len(), 2, "two independent via stacks");
    for net in &netlist.nets {
        assert_eq!(net.shape_count, 3);
    }
}

// --- Naming, highlighting, and stability ------------------------------------

#[test]
fn nets_get_stable_autonames() {
    let doc = doc_with(vec![rect(M1, 0, 0, 10, 10), rect(M1, 100, 0, 110, 10)]);
    let netlist = Extractor::new().extract(&doc, "top");
    // Deterministic order by lowest member index → net_0 then net_1.
    assert_eq!(netlist.nets[0].name, "net_0");
    assert_eq!(netlist.nets[1].name, "net_1");
}

#[test]
fn label_names_the_covering_net() {
    let doc = doc_with(vec![
        rect(M1, 0, 0, 10, 10), // net covered by the VDD label
        rect(M1, 100, 0, 110, 10),
    ]);
    let labels = vec![NetLabel::new("VDD", Point::new(5, 5), M1)];
    let netlist = Extractor::new().with_labels(labels).extract(&doc, "top");
    let vdd = netlist.shapes_of("VDD").expect("a net named VDD exists");
    assert_eq!(vdd, &[0]);
    // The unlabelled net still gets an auto name.
    assert!(
        netlist
            .nets
            .iter()
            .any(|n| n.name == "net_0" || n.name == "net_1")
    );
}

#[test]
fn shapes_of_supports_highlighting() {
    // Net highlighting: from a net name, recover its member shape indices.
    let doc = doc_with(vec![
        rect(M1, 0, 0, 10, 10),
        rect(M1, 9, 0, 19, 10),
        rect(M1, 100, 0, 110, 10),
    ]);
    let netlist = Extractor::new().extract(&doc, "top");
    let net = netlist.net_of(0).expect("shape 0 on a net");
    assert_eq!(net.shapes, vec![0, 1]);
    assert!(net.contains(1));
    assert!(!net.contains(2));
}

#[test]
fn unknown_cell_is_empty() {
    let doc = doc_with(vec![rect(M1, 0, 0, 10, 10)]);
    let netlist = Extractor::new().extract(&doc, "nope");
    assert!(netlist.is_empty());
}

#[test]
fn empty_cell_is_empty_netlist() {
    let doc = doc_with(vec![]);
    let netlist = Extractor::new().extract(&doc, "top");
    assert!(netlist.nets.is_empty());
}

// --- Compare (LVS geometric half) -------------------------------------------

#[test]
fn compare_identical_netlists_has_no_diff() {
    let extracted = Netlist::new(vec![Net::new("a", vec![0, 1]), Net::new("b", vec![2])]);
    let expected = Netlist::new(vec![Net::new("x", vec![0, 1]), Net::new("y", vec![2])]);
    let diff = Extractor::new().compare(&extracted, &expected);
    assert!(diff.is_empty(), "same partitions regardless of names");
}

#[test]
fn compare_reports_missing_connection_open() {
    // Expected: 0,1,2 all one net. Extracted: 2 split off. Missing pairs (0,2),(1,2).
    let extracted = Netlist::new(vec![Net::new("a", vec![0, 1]), Net::new("b", vec![2])]);
    let expected = Netlist::new(vec![Net::new("n", vec![0, 1, 2])]);
    let diff = Extractor::new().compare(&extracted, &expected);
    assert!(diff.extra.is_empty());
    assert_eq!(
        diff.missing,
        vec![ShapePair::new(0, 2), ShapePair::new(1, 2)]
    );
}

#[test]
fn compare_reports_extra_connection_short() {
    // Expected: two nets. Extracted: shorted into one. Extra pairs (0,2),(0,3),...
    let extracted = Netlist::new(vec![Net::new("s", vec![0, 1, 2, 3])]);
    let expected = Netlist::new(vec![Net::new("a", vec![0, 1]), Net::new("b", vec![2, 3])]);
    let diff = Extractor::new().compare(&extracted, &expected);
    assert!(diff.missing.is_empty());
    assert_eq!(
        diff.extra,
        vec![
            ShapePair::new(0, 2),
            ShapePair::new(0, 3),
            ShapePair::new(1, 2),
            ShapePair::new(1, 3),
        ]
    );
}

#[test]
fn compare_end_to_end_against_extraction() {
    // Extract a real layout, then compare it to the intended connectivity.
    let doc = doc_with(vec![
        rect(M1, 0, 0, 10, 10),
        rect(M1, 9, 0, 19, 10), // touches shape 0
        rect(M1, 100, 0, 110, 10),
    ]);
    let extracted = Extractor::new().extract(&doc, "top");
    // Intended: {0,1} and {2} — matches extraction.
    let expected = Netlist::new(vec![Net::new("sig", vec![0, 1]), Net::new("iso", vec![2])]);
    let diff = Extractor::new().compare(&extracted, &expected);
    assert!(
        diff.is_empty(),
        "extraction matches the intended netlist: {diff:?}"
    );
}
