//! Connectivity intent checking (LVS-lite): does a laid-out cell join what an
//! [`IntentSpec`] says it must, and keep apart what it says it must not?
//!
//! [`check_intent`] flattens a cell to leaf shapes, extracts geometric
//! connectivity ([`Extractor`]) into connected components, then judges the spec
//! against those components:
//!
//! - Each [`Terminal`] is mapped to the component containing a shape on the
//!   terminal's layer that overlaps the terminal region. A terminal that matches
//!   no shape is *unmatched*.
//! - An [`IntentNet`] is satisfied when all of its terminals land on **one**
//!   component. Terminals spanning several components, or any unmatched terminal,
//!   produce an [`Open`].
//! - A [`ForbiddenPair`] is satisfied when the two nets' terminals occupy
//!   **different** components. Any terminal of `net_a` sharing a component with any
//!   terminal of `net_b` produces a [`Short`].
//!
//! The result is an [`IntentReport`]; an empty report ([`IntentReport::is_satisfied`])
//! means the layout matches the intent.
//!
//! # Connectivity model
//!
//! Extraction runs over the flattened geometry with same-layer touch plus the
//! SKY130 via/contact stack ([`sky130_connection_rules`]). A caller whose
//! technology differs can extract with its own rules; [`check_intent`] uses the
//! SKY130 stack, which is the technology this crate's fixtures target.

use reticle_geometry::{LayerId, Rect};
use reticle_model::{Document, DrawShape};

use crate::Extractor;
use crate::connectivity::rects_touch;
use crate::intent::{ForbiddenPair, IntentNet, IntentReport, IntentSpec, Open, Short, Terminal};
use crate::netlist::Netlist;
use crate::rules::ConnectionRules;

/// The SKY130 conductor and via/contact layers this checker connects across.
///
/// Numbers are the process's GDSII `(layer, datatype)` pairs. Only the layers the
/// via stack needs are listed; the checker treats every other layer as
/// same-layer-only.
mod layers {
    use reticle_geometry::LayerId;

    /// Diffusion (active) conductor.
    pub const DIFF: LayerId = LayerId::new(65, 20);
    /// Polysilicon conductor.
    pub const POLY: LayerId = LayerId::new(66, 20);
    /// Local-interconnect contact: joins poly/diff to `li1`.
    pub const LICON1: LayerId = LayerId::new(66, 44);
    /// Local interconnect (metal-0) conductor.
    pub const LI1: LayerId = LayerId::new(67, 20);
    /// Metal contact: joins `li1` to `met1`.
    pub const MCON: LayerId = LayerId::new(67, 44);
    /// Metal-1 conductor.
    pub const MET1: LayerId = LayerId::new(68, 20);
    /// Via1: joins `met1` to `met2`.
    pub const VIA1: LayerId = LayerId::new(68, 44);
    /// Metal-2 conductor.
    pub const MET2: LayerId = LayerId::new(69, 20);
    /// Via2: joins `met2` to `met3`.
    pub const VIA2: LayerId = LayerId::new(69, 44);
    /// Metal-3 conductor.
    pub const MET3: LayerId = LayerId::new(70, 20);
    /// Via3: joins `met3` to `met4`.
    pub const VIA3: LayerId = LayerId::new(70, 44);
    /// Metal-4 conductor.
    pub const MET4: LayerId = LayerId::new(71, 20);
}

/// The SKY130 via/contact [`ConnectionRules`]: the contact/via layers and the two
/// conductor layers each one bridges, bottom to top.
///
/// This is the rule set [`check_intent`] extracts with. It is exposed so a caller
/// can reuse the same stack when driving [`Extractor`] directly.
#[must_use]
pub fn sky130_connection_rules() -> ConnectionRules {
    use layers::{DIFF, LI1, LICON1, MCON, MET1, MET2, MET3, MET4, POLY, VIA1, VIA2, VIA3};
    ConnectionRules::new()
        // licon1 lands on either poly or diff below and li1 above.
        .connect(POLY, LICON1, LI1)
        .connect(DIFF, LICON1, LI1)
        .connect(LI1, MCON, MET1)
        .connect(MET1, VIA1, MET2)
        .connect(MET2, VIA2, MET3)
        .connect(MET3, VIA3, MET4)
}

/// Checks the connectivity intent of `cell` in `doc` against `spec`.
///
/// Flattens the cell to leaf shapes, extracts connected components over the
/// SKY130 via stack, and reports every net whose terminals are not all on one
/// component ([`Open`]) and every forbidden pair whose nets share a component
/// ([`Short`]). The `reticle_extract` module documentation states the full rule.
///
/// An unknown `cell` flattens to no shapes, so every terminal is unmatched and
/// every multi-terminal net opens.
#[must_use]
pub fn check_intent(doc: &Document, cell: &str, spec: &IntentSpec) -> IntentReport {
    let shapes = doc.flatten(cell);
    let netlist = Extractor::new()
        .with_rules(sky130_connection_rules())
        .extract_shapes(&shapes);

    let mut report = IntentReport::default();
    check_opens(&shapes, &netlist, &spec.nets, &mut report);
    check_shorts(&shapes, &netlist, spec, &mut report);
    report
}

/// A component identifier: an index into the extracted [`Netlist`]'s nets. Two
/// terminals are on the same component iff they map to the same [`ComponentId`].
type ComponentId = usize;

/// Where a terminal landed: either the component that owns a matching shape, or
/// nothing (no shape on the terminal's layer overlaps its region).
#[derive(Clone, Copy)]
enum TerminalHit {
    /// The terminal matched a shape belonging to this component.
    Component(ComponentId),
    /// No shape matched the terminal.
    Unmatched,
}

/// Maps `terminal` to the component of a shape on its layer overlapping its
/// region, or [`TerminalHit::Unmatched`].
///
/// A shape matches when it is on the terminal's [`layer`](Terminal::layer) and its
/// bounding box overlaps the terminal [`region`](Terminal::region) as closed boxes
/// (edge/corner contact counts, matching the connectivity engine's touch rule).
/// The component id is the index of the owning net in `netlist`; a matched shape
/// always belongs to exactly one net, so the mapping is well defined.
fn locate_terminal(shapes: &[DrawShape], netlist: &Netlist, terminal: &Terminal) -> TerminalHit {
    for (idx, shape) in shapes.iter().enumerate() {
        if shape.layer == terminal.layer && overlaps(shape, terminal.region) {
            // The matched shape's net index is the component identifier.
            if let Some(component) = component_of(netlist, idx) {
                return TerminalHit::Component(component);
            }
        }
    }
    TerminalHit::Unmatched
}

/// Returns the index of the net in `netlist` that owns shape `idx`, if any.
fn component_of(netlist: &Netlist, idx: usize) -> Option<ComponentId> {
    netlist.nets.iter().position(|net| net.contains(idx))
}

/// Returns `true` if `shape`'s bounding box overlaps `region` as closed boxes.
fn overlaps(shape: &DrawShape, region: Rect) -> bool {
    use reticle_geometry::Shape as _;
    rects_touch(&shape.bounding_box(), &region)
}

/// Appends an [`Open`] for every intent net whose terminals are not all on one
/// component (including any unmatched terminal).
fn check_opens(
    shapes: &[DrawShape],
    netlist: &Netlist,
    nets: &[IntentNet],
    report: &mut IntentReport,
) {
    for net in nets {
        if let Some(open) = net_open(shapes, netlist, net) {
            report.opens.push(open);
        }
    }
}

/// Returns an [`Open`] describing the disconnection of `net`, or `None` if all its
/// terminals share one component.
///
/// A net with zero or one terminal is trivially connected. Otherwise the first
/// unmatched terminal opens the net; failing that, terminals landing on more than
/// one distinct component open it. The reported location is a terminal region near
/// the break.
fn net_open(shapes: &[DrawShape], netlist: &Netlist, net: &IntentNet) -> Option<Open> {
    // The component every matched terminal must agree on, plus the location to
    // report if one disagrees.
    let mut agreed: Option<ComponentId> = None;
    for terminal in &net.terminals {
        match locate_terminal(shapes, netlist, terminal) {
            TerminalHit::Unmatched => {
                return Some(Open {
                    net: net.name.clone(),
                    at: terminal.region,
                    detail: format!(
                        "terminal '{}' on layer {}/{} has no matching geometry in its region",
                        terminal.name, terminal.layer.layer, terminal.layer.datatype
                    ),
                });
            }
            TerminalHit::Component(component) => match agreed {
                None => agreed = Some(component),
                Some(first) if first != component => {
                    return Some(Open {
                        net: net.name.clone(),
                        at: terminal.region,
                        detail: format!(
                            "terminal '{}' is on a different component ({}) from the net's \
                             other terminals ({}); the net is split",
                            terminal.name, component, first
                        ),
                    });
                }
                Some(_) => {}
            },
        }
    }
    None
}

/// Appends a [`Short`] for every forbidden pair whose two nets share a component.
fn check_shorts(
    shapes: &[DrawShape],
    netlist: &Netlist,
    spec: &IntentSpec,
    report: &mut IntentReport,
) {
    for pair in &spec.forbidden {
        if let Some(short) = pair_short(shapes, netlist, spec, pair) {
            report.shorts.push(short);
        }
    }
}

/// Returns a [`Short`] if any terminal of `pair.net_a` shares a component with any
/// terminal of `pair.net_b`, or `None` if the two nets stay separate.
///
/// The reported location is the overlap of the two colliding terminal regions when
/// they intersect, else the bounding box spanning them, a location on the offending
/// component.
fn pair_short(
    shapes: &[DrawShape],
    netlist: &Netlist,
    spec: &IntentSpec,
    pair: &ForbiddenPair,
) -> Option<Short> {
    let a_terms = terminals_of(spec, &pair.net_a);
    let b_terms = terminals_of(spec, &pair.net_b);

    for ta in a_terms {
        let TerminalHit::Component(ca) = locate_terminal(shapes, netlist, ta) else {
            continue;
        };
        for tb in b_terms {
            let TerminalHit::Component(cb) = locate_terminal(shapes, netlist, tb) else {
                continue;
            };
            if ca == cb {
                let at = ta
                    .region
                    .intersection(&tb.region)
                    .unwrap_or_else(|| ta.region.union(&tb.region));
                return Some(Short {
                    net_a: pair.net_a.clone(),
                    net_b: pair.net_b.clone(),
                    at,
                });
            }
        }
    }
    None
}

/// The terminals of the intent net named `name`, or an empty slice if the spec has
/// no such net.
fn terminals_of<'a>(spec: &'a IntentSpec, name: &str) -> &'a [Terminal] {
    spec.nets
        .iter()
        .find(|n| n.name == name)
        .map_or(&[][..], |n| n.terminals.as_slice())
}

/// A convenience: build a [`Terminal`] from its parts. Not used by the checker
/// itself but handy for callers and tests assembling specs.
#[must_use]
pub fn terminal(name: impl Into<String>, layer: LayerId, region: Rect) -> Terminal {
    Terminal {
        name: name.into(),
        layer,
        region,
    }
}
