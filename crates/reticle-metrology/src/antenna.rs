//! A simplified per-net antenna-ratio screen over a SKY130 layer subset.
//!
//! During fabrication, conductor connected to a transistor gate collects charge
//! that can damage the thin gate oxide. An antenna rule bounds the ratio of
//! connected conductor area to gate area. This module implements a deliberately
//! small screen of that idea.
//!
//! # Semantics and limits
//!
//! For each extracted net this check computes
//!
//! ```text
//! ratio = connected_metal_area / gate_area
//! ```
//!
//! where, over the SKY130 layer subset:
//!
//! - `gate_area` is the union area of polysilicon (`poly`, 66/20) shapes on the
//!   net. It is approximated as all poly on the net, **not** poly intersected
//!   with diffusion, so a poly routing wire counts as gate here.
//! - `connected_metal_area` is the union area of the interconnect conductors on
//!   the net: `li1` (67/20) and `met1`..`met4` (68/20, 69/20, 70/20, 71/20).
//!   Contact and via layers connect nets but are not counted as metal.
//!
//! A net with no poly has no gate and is never flagged. A net whose ratio exceeds
//! `threshold` is reported as an [`AntennaViolation`].
//!
//! This is a screening heuristic, not a sign-off antenna check. It does **not**
//! model per-metal-layer cumulative ratios across fabrication steps, sidewall or
//! perimeter terms, diffusion-diode protection, or partial-route (as-built) area.
//! It reduces the whole net to a single total-metal-to-gate-area ratio. Treat a
//! flag as "worth a closer look", not as a rule violation.

use reticle_extract::{Extractor, sky130_connection_rules};
use reticle_geometry::{LayerId, Polygon};
use reticle_model::{Document, DrawShape};

use crate::area::union_area_of;
use crate::polyize::shape_polygons;

/// The SKY130 layers this screen classifies. Every other layer is ignored for
/// area (contacts and vias still join nets during extraction).
mod layers {
    use reticle_geometry::LayerId;

    /// Polysilicon (gate conductor).
    pub const POLY: LayerId = LayerId::new(66, 20);
    /// Local interconnect (metal-0).
    pub const LI1: LayerId = LayerId::new(67, 20);
    /// Metal 1 through metal 4.
    pub const MET1: LayerId = LayerId::new(68, 20);
    pub const MET2: LayerId = LayerId::new(69, 20);
    pub const MET3: LayerId = LayerId::new(70, 20);
    pub const MET4: LayerId = LayerId::new(71, 20);
}

fn is_gate(layer: LayerId) -> bool {
    layer == layers::POLY
}

fn is_metal(layer: LayerId) -> bool {
    matches!(
        layer,
        layers::LI1 | layers::MET1 | layers::MET2 | layers::MET3 | layers::MET4
    )
}

/// One net whose antenna ratio exceeds the threshold.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct AntennaViolation {
    /// Net name from extraction (`net_<n>` when unlabelled).
    pub net: String,
    /// Union area of poly (gate) on the net, in DBU squared.
    pub gate_area: f64,
    /// Union area of connected metal on the net, in DBU squared.
    pub metal_area: f64,
    /// `metal_area / gate_area`.
    pub ratio: f64,
}

/// The result of an antenna screen: the threshold used and the flagged nets.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct AntennaCheck {
    /// The ratio above which a net is flagged.
    pub threshold: f64,
    /// Flagged nets, ordered by descending ratio.
    pub violations: Vec<AntennaViolation>,
}

/// Runs the antenna screen over a document's flattened top cell.
///
/// A document with no declared top cell yields an empty result.
#[must_use]
pub fn check(doc: &Document, threshold: f64) -> AntennaCheck {
    let Some(top) = crate::top_cell(doc) else {
        return AntennaCheck {
            threshold,
            violations: Vec::new(),
        };
    };
    let flat = doc.flatten(top);
    let netlist = Extractor::new()
        .with_rules(sky130_connection_rules())
        .extract_shapes(&flat);

    let mut violations = Vec::new();
    for net in &netlist.nets {
        let mut gate: Vec<Polygon> = Vec::new();
        let mut metal: Vec<Polygon> = Vec::new();
        for &idx in &net.shapes {
            let shape: &DrawShape = &flat[idx];
            if is_gate(shape.layer) {
                gate.extend(shape_polygons(shape));
            } else if is_metal(shape.layer) {
                metal.extend(shape_polygons(shape));
            }
        }
        let gate_area = union_area_of(&gate);
        if gate_area <= 0.0 {
            continue; // No gate on this net: no antenna concern.
        }
        let metal_area = union_area_of(&metal);
        let ratio = metal_area / gate_area;
        if ratio > threshold {
            violations.push(AntennaViolation {
                net: net.name.clone(),
                gate_area,
                metal_area,
                ratio,
            });
        }
    }
    violations.sort_by(|a, b| {
        b.ratio
            .partial_cmp(&a.ratio)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    AntennaCheck {
        threshold,
        violations,
    }
}

#[cfg(test)]
mod tests {
    // Areas here are integer-valued, so exact f64 equality is the right assertion.
    #![allow(clippy::float_cmp)]

    use super::check;
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{Cell, Document, DrawShape, ShapeKind};

    const POLY: LayerId = LayerId::new(66, 20);
    const LICON1: LayerId = LayerId::new(66, 44);
    const LI1: LayerId = LayerId::new(67, 20);

    fn rect(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    }

    fn doc_of(shapes: Vec<DrawShape>) -> Document {
        let mut cell = Cell::new("top");
        cell.shapes = shapes;
        let mut doc = Document::new();
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["top".into()]);
        doc
    }

    #[test]
    fn high_ratio_net_is_flagged() {
        // Tiny poly gate (area 4) connected through licon1 to a large li1 sheet
        // (area 10000): ratio 2500, well over the threshold.
        let doc = doc_of(vec![
            rect(POLY, 0, 0, 2, 2),
            rect(LICON1, 0, 0, 1, 1),
            rect(LI1, 0, 0, 100, 100),
        ]);
        let result = check(&doc, 400.0);
        assert_eq!(result.violations.len(), 1);
        let v = &result.violations[0];
        assert_eq!(v.gate_area, 4.0);
        assert_eq!(v.metal_area, 10000.0);
        assert_eq!(v.ratio, 2500.0);
    }

    #[test]
    fn normal_ratio_net_is_not_flagged() {
        // Poly gate (area 100) with a matching li1 sheet (area 100): ratio 1.
        let doc = doc_of(vec![
            rect(POLY, 0, 0, 10, 10),
            rect(LICON1, 0, 0, 1, 1),
            rect(LI1, 0, 0, 10, 10),
        ]);
        let result = check(&doc, 400.0);
        assert!(result.violations.is_empty());
    }

    #[test]
    fn net_without_gate_is_never_flagged() {
        // A large li1 sheet with no poly on the net: no gate, no antenna concern.
        let doc = doc_of(vec![rect(LI1, 0, 0, 1000, 1000)]);
        let result = check(&doc, 1.0);
        assert!(result.violations.is_empty());
    }
}
