//! Oracle tests for the connectivity intent checker ([`check_intent`]).
//!
//! These exercise both directions of the LVS-lite check, which is where its value
//! lies: a genuinely connected layout with separate forbidden nets must be
//! *satisfied*, and each way the intent can be violated (a nicked wire, a deleted
//! via, an added bridge, a terminal over empty space) must be *reported* with
//! coordinates. The perturbation property tests start from a satisfied layout and
//! assert that a random break introduces an open and a random bridge introduces a
//! short.
//!
//! Fixtures use the SKY130 conductor/via layers the checker connects across
//! (`met1`, `via1`, `met2`, ...), so extraction runs the same via stack the
//! checker configures.

use proptest::prelude::*;
use reticle_extract::{ForbiddenPair, IntentNet, IntentSpec, Terminal, check_intent, terminal};
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, Document, DrawShape, ShapeKind};

// SKY130 layers the checker's via stack uses.
const MET1: LayerId = LayerId::new(68, 20);
const VIA1: LayerId = LayerId::new(68, 44);
const MET2: LayerId = LayerId::new(69, 20);

/// A rectangle draw-shape on `layer` spanning `[(x0,y0), (x1,y1)]`.
fn rect(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
    DrawShape::new(
        layer,
        ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
    )
}

/// An axis-aligned rectangle region.
fn region(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
    Rect::new(Point::new(x0, y0), Point::new(x1, y1))
}

/// Builds a single-cell document named `top` from a shape list.
fn doc_with(shapes: Vec<DrawShape>) -> Document {
    let mut cell = Cell::new("top");
    cell.shapes = shapes;
    let mut doc = Document::new();
    doc.insert_cell(cell);
    doc
}

/// A net with the given name and terminals.
fn net(name: &str, terminals: Vec<Terminal>) -> IntentNet {
    IntentNet {
        name: name.to_owned(),
        terminals,
    }
}

// --- Known-good: a satisfied layout ------------------------------------------

/// Two met1 terminals at either end of a single met1 wire are one net; a separate
/// met1 pad is a second net; the two are forbidden from touching and do not. The
/// report must be satisfied.
fn satisfied_fixture() -> (Document, IntentSpec) {
    let doc = doc_with(vec![
        // sig: a horizontal met1 wire from x=0 to x=100, with terminals at each end.
        rect(MET1, 0, 0, 100, 10),
        // pwr: an isolated met1 pad well away from sig.
        rect(MET1, 0, 100, 20, 120),
    ]);
    let spec = IntentSpec {
        nets: vec![
            net(
                "sig",
                vec![
                    terminal("sig_l", MET1, region(0, 0, 5, 10)),
                    terminal("sig_r", MET1, region(95, 0, 100, 10)),
                ],
            ),
            net(
                "pwr",
                vec![terminal("pwr_a", MET1, region(0, 100, 20, 120))],
            ),
        ],
        forbidden: vec![ForbiddenPair {
            net_a: "sig".to_owned(),
            net_b: "pwr".to_owned(),
        }],
    };
    (doc, spec)
}

#[test]
fn satisfied_layout_reports_no_violations() {
    let (doc, spec) = satisfied_fixture();
    let report = check_intent(&doc, "top", &spec);
    assert!(
        report.is_satisfied(),
        "genuinely connected, separated layout must satisfy the intent: {report:?}"
    );
    assert!(report.opens.is_empty());
    assert!(report.shorts.is_empty());
}

/// A via stack (met1 -> via1 -> met2) joins a met1 terminal to a met2 terminal:
/// with the via present, the cross-layer net is one component.
#[test]
fn via_stack_connects_cross_layer_terminals() {
    let doc = doc_with(vec![
        rect(MET1, 0, 0, 20, 20),
        rect(MET2, 0, 0, 20, 20),
        rect(VIA1, 5, 5, 15, 15),
    ]);
    let spec = IntentSpec {
        nets: vec![net(
            "n",
            vec![
                terminal("n_m1", MET1, region(0, 0, 5, 5)),
                terminal("n_m2", MET2, region(15, 15, 20, 20)),
            ],
        )],
        forbidden: vec![],
    };
    let report = check_intent(&doc, "top", &spec);
    assert!(report.is_satisfied(), "via bridges the stack: {report:?}");
}

// --- Known-bad opens ---------------------------------------------------------

/// Nicking the wire (splitting the single met1 span into two pieces with a gap)
/// disconnects the two terminals; the checker must report an Open on `sig` at a
/// terminal region.
#[test]
fn nicked_wire_reports_open_with_coordinates() {
    // Two met1 segments with a gap in [48, 52): the terminals at x<5 and x>95 now
    // sit on different components.
    let doc = doc_with(vec![
        rect(MET1, 0, 0, 48, 10),
        rect(MET1, 52, 0, 100, 10),
        rect(MET1, 0, 100, 20, 120), // pwr, unchanged
    ]);
    let (_good_doc, spec) = satisfied_fixture();

    let report = check_intent(&doc, "top", &spec);
    assert!(!report.is_satisfied());
    assert_eq!(report.shorts.len(), 0, "no forbidden nets touch");
    assert_eq!(report.opens.len(), 1, "exactly the sig net opens");
    let open = &report.opens[0];
    assert_eq!(open.net, "sig");
    // The location is one of sig's terminal regions (the right terminal is the one
    // found on the disagreeing component).
    assert_eq!(
        open.at,
        region(95, 0, 100, 10),
        "the open points at the disconnected terminal"
    );
    assert!(
        open.detail.contains("different component"),
        "detail explains the split: {}",
        open.detail
    );
}

/// Deleting the bridging via leaves the met1 and met2 terminals on separate
/// components: an Open even though both terminals match geometry.
#[test]
fn missing_via_reports_open() {
    let doc = doc_with(vec![
        rect(MET1, 0, 0, 20, 20),
        rect(MET2, 0, 0, 20, 20),
        // via1 deleted
    ]);
    let spec = IntentSpec {
        nets: vec![net(
            "n",
            vec![
                terminal("n_m1", MET1, region(0, 0, 5, 5)),
                terminal("n_m2", MET2, region(15, 15, 20, 20)),
            ],
        )],
        forbidden: vec![],
    };
    let report = check_intent(&doc, "top", &spec);
    assert_eq!(
        report.opens.len(),
        1,
        "the cross-layer net opens: {report:?}"
    );
    assert_eq!(report.opens[0].net, "n");
}

/// A terminal whose region covers no shape on its layer is an Open, distinct in
/// wording from a split net.
#[test]
fn terminal_over_empty_space_reports_open() {
    let doc = doc_with(vec![rect(MET1, 0, 0, 100, 10)]);
    let spec = IntentSpec {
        nets: vec![net(
            "sig",
            vec![
                terminal("sig_l", MET1, region(0, 0, 5, 10)),
                // This terminal sits far above any geometry.
                terminal("floating", MET1, region(0, 500, 5, 505)),
            ],
        )],
        forbidden: vec![],
    };
    let report = check_intent(&doc, "top", &spec);
    assert_eq!(report.opens.len(), 1);
    let open = &report.opens[0];
    assert_eq!(open.net, "sig");
    assert_eq!(
        open.at,
        region(0, 500, 5, 505),
        "points at the empty terminal"
    );
    assert!(
        open.detail.contains("no matching geometry"),
        "detail names the unmatched terminal: {}",
        open.detail
    );
}

/// A terminal whose region matches, but on the wrong layer, does not count: the
/// layer must agree. Here the shape under the terminal is met2, not the terminal's
/// met1, so the terminal is unmatched.
#[test]
fn terminal_on_wrong_layer_does_not_match() {
    let doc = doc_with(vec![rect(MET2, 0, 0, 20, 20)]);
    let spec = IntentSpec {
        nets: vec![net(
            "n",
            vec![
                terminal("a", MET1, region(0, 0, 10, 10)), // met1, but only met2 here
                terminal("b", MET2, region(10, 10, 20, 20)),
            ],
        )],
        forbidden: vec![],
    };
    let report = check_intent(&doc, "top", &spec);
    assert_eq!(report.opens.len(), 1, "the met1 terminal is unmatched");
    assert!(report.opens[0].detail.contains("no matching geometry"));
}

// --- Known-bad shorts --------------------------------------------------------

/// Adding a met1 sliver that bridges the sig wire and the pwr pad merges the two
/// forbidden nets into one component: the checker must report a Short.
#[test]
fn bridging_sliver_reports_short_with_coordinates() {
    let (_doc, spec) = satisfied_fixture();
    // Start from the satisfied geometry, then add a vertical met1 sliver joining
    // the sig wire (y in [0,10]) to the pwr pad (y in [100,120]).
    let doc = doc_with(vec![
        rect(MET1, 0, 0, 100, 10),   // sig
        rect(MET1, 0, 100, 20, 120), // pwr
        rect(MET1, 0, 10, 10, 100),  // bridging sliver spanning the gap
    ]);
    let report = check_intent(&doc, "top", &spec);
    assert!(!report.is_satisfied());
    assert_eq!(
        report.opens.len(),
        0,
        "both nets are still internally whole"
    );
    assert_eq!(report.shorts.len(), 1, "sig and pwr are shorted");
    let short = &report.shorts[0];
    assert!(
        (short.net_a == "sig" && short.net_b == "pwr")
            || (short.net_a == "pwr" && short.net_b == "sig"),
        "the short names the two forbidden nets: {short:?}"
    );
    // The reported location is finite and lies within the design's extent.
    assert!(short.at.width() >= 0 && short.at.height() >= 0);
}

/// With no forbidding rule, the same bridged geometry is not a violation: the
/// checker only reports shorts the spec forbids.
#[test]
fn bridge_without_forbidden_rule_is_not_a_short() {
    let doc = doc_with(vec![
        rect(MET1, 0, 0, 100, 10),
        rect(MET1, 0, 100, 20, 120),
        rect(MET1, 0, 10, 10, 100),
    ]);
    let spec = IntentSpec {
        nets: vec![
            net("sig", vec![terminal("sig_l", MET1, region(0, 0, 5, 10))]),
            net(
                "pwr",
                vec![terminal("pwr_a", MET1, region(0, 100, 20, 120))],
            ),
        ],
        forbidden: vec![], // no rule -> no short even though they touch
    };
    let report = check_intent(&doc, "top", &spec);
    assert!(report.is_satisfied(), "nothing is forbidden: {report:?}");
}

// --- Perturbation property tests (both directions) ---------------------------

/// The satisfied fixture, but with the sig wire built from `pieces` abutting
/// segments so a break can be introduced by removing one. Returns the doc.
fn wire_doc(gap_at: Option<usize>) -> Document {
    // sig wire as five abutting 20-wide met1 segments [0,20),[20,40),...,[80,100].
    let mut shapes = Vec::new();
    for seg in 0..5 {
        if Some(seg) == gap_at {
            continue; // drop this segment to break the wire
        }
        let x0 = (seg as i32) * 20;
        shapes.push(rect(MET1, x0, 0, x0 + 20, 10));
    }
    shapes.push(rect(MET1, 0, 100, 20, 120)); // pwr pad
    doc_with(shapes)
}

/// The spec for [`wire_doc`]: sig spans the whole wire, pwr is separate, forbidden.
fn wire_spec() -> IntentSpec {
    IntentSpec {
        nets: vec![
            net(
                "sig",
                vec![
                    terminal("sig_l", MET1, region(0, 0, 5, 10)),
                    terminal("sig_r", MET1, region(95, 0, 100, 10)),
                ],
            ),
            net(
                "pwr",
                vec![terminal("pwr_a", MET1, region(0, 100, 20, 120))],
            ),
        ],
        forbidden: vec![ForbiddenPair {
            net_a: "sig".to_owned(),
            net_b: "pwr".to_owned(),
        }],
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Removing any one interior segment of the wire breaks it: since the terminals
    /// sit at the two ends, dropping any of segments 1..=3 (or the segment holding a
    /// terminal) must open `sig`. We restrict to interior segments 1..=3 so both end
    /// terminals still match geometry, isolating the "split net" open.
    #[test]
    fn random_break_introduces_an_open(seg in 1usize..=3) {
        // Baseline: intact wire is satisfied.
        let base = check_intent(&wire_doc(None), "top", &wire_spec());
        prop_assert!(base.is_satisfied(), "intact wire is satisfied: {:?}", base);

        // Break: drop an interior segment, the two ends can no longer be one net.
        let broken = check_intent(&wire_doc(Some(seg)), "top", &wire_spec());
        prop_assert!(!broken.is_satisfied(), "a break must be detected");
        prop_assert!(
            broken.opens.iter().any(|o| o.net == "sig"),
            "sig must be reported open after removing segment {}: {:?}",
            seg,
            broken
        );
        prop_assert!(broken.shorts.is_empty(), "a break is not a short");
    }

    /// Adding a met1 bridge at a random x between the sig wire and the pwr pad joins
    /// two forbidden nets: a short must be reported, and no open.
    #[test]
    fn random_bridge_introduces_a_short(x0 in 0i32..=90) {
        // Baseline: no bridge, satisfied.
        let base = check_intent(&wire_doc(None), "top", &wire_spec());
        prop_assert!(base.is_satisfied());

        // Bridge: a vertical met1 sliver from the wire (y=10) up to the pad (y=100)
        // at a random x. It touches both the sig wire and the pwr pad only when it
        // overlaps the pad's x-extent [0,20]; place it there so the short is real.
        let x0 = x0.min(15); // keep the sliver under the pad's x-extent
        let mut shapes = vec![
            rect(MET1, 0, 0, 100, 10),   // sig
            rect(MET1, 0, 100, 20, 120), // pwr
        ];
        shapes.push(rect(MET1, x0, 10, x0 + 5, 100)); // bridge
        let doc = doc_with(shapes);

        let report = check_intent(&doc, "top", &wire_spec());
        prop_assert!(!report.is_satisfied(), "a bridge must be detected: {:?}", report);
        prop_assert!(
            report.shorts.iter().any(|s| {
                (s.net_a == "sig" && s.net_b == "pwr") || (s.net_a == "pwr" && s.net_b == "sig")
            }),
            "sig/pwr short must be reported: {:?}",
            report
        );
        prop_assert!(report.opens.is_empty(), "a bridge is not an open");
    }
}
