//! The guard-ring generator: a closed conductor ring around a region, with an
//! optional row of substrate-tap contacts.
//!
//! # What it emits
//!
//! Four axis-aligned rectangles on the chosen conductor layer, forming a closed
//! frame around a rectangular opening (the region the ring guards). Adjacent strips
//! overlap at the corners, so the frame is a single connected loop. When taps are
//! enabled (only on `li1`, the layer the SKY130 subset gives a contact enclosure), a
//! row of `licon` contacts runs along the bottom strip, each fully enclosed by it.
//!
//! # Why it is DRC-clean by construction (SKY130 subset)
//!
//! * **Width.** Every strip is `ring_width` DBU thick and `ring_width` is validated
//!   to be at least the layer's minimum width, so no strip is too thin.
//! * **Spacing.** The left and right strips face each other across the opening, as
//!   do the top and bottom strips; the opening (`region_width`/`region_height`) is
//!   validated to be at least the layer's minimum spacing, so no interior gap is
//!   sub-spacing. Nothing else on the layer sits nearby.
//! * **Area.** Each strip spans a full side of the frame, so its bounding-box area
//!   is far above the layer minimum.
//! * **Contact size and enclosure.** Each tap is a square `licon` at its exact drawn
//!   size; taps require an `li1` ring whose `ring_width` is validated to cover a
//!   contact plus the `li.5` enclosure on both sides, and the contacts sit centered
//!   in the bottom strip, so every contact is enclosed by at least the required
//!   margin. Contacts are pitched at least their size plus a safe margin apart (the
//!   subset carries no contact-to-contact spacing rule, so the pitch is a
//!   conservative choice, not a required one).

use reticle_geometry::{Point, Rect};
use reticle_model::{Cell, DrawShape, ShapeKind, Technology};
use serde::{Deserialize, Serialize};

use crate::error::GenError;
use crate::generator::{GenOutput, GenParams, Generator};
use crate::gentech::{Conductor, GenTech};
use crate::schema::{FieldSchema, ParamSchema};

/// The conductor layer a guard ring is drawn on.
///
/// Restricted to the interconnect layers the SKY130 subset carries width/spacing
/// (and, for `li1`, contact-enclosure) rules for, so a ring on any of them is
/// checkable and clean.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RingLayer {
    /// Local interconnect `li1` (67/20). The only choice that supports taps.
    #[default]
    Li1,
    /// Metal 1 `met1` (68/20).
    Met1,
    /// Metal 2 `met2` (69/20).
    Met2,
    /// Metal 3 `met3` (70/20).
    Met3,
}

impl RingLayer {
    /// The conductor data (layer, width, spacing, area) for this choice in the given
    /// technology. The variants name interconnect *levels* (0 = base), so they bind to
    /// `li1..met3` on SKY130 and `Metal1..Metal4` on SG13G2.
    fn conductor(self, gt: &GenTech) -> Conductor {
        match self {
            Self::Li1 => gt.conductor(0),
            Self::Met1 => gt.conductor(1),
            Self::Met2 => gt.conductor(2),
            Self::Met3 => gt.conductor(3),
        }
    }

    /// The serde variant strings, for the schema's enum field.
    const VARIANTS: [&'static str; 4] = ["li1", "met1", "met2", "met3"];
}

/// Parameters for the [`GuardRing`] generator. All lengths are in DBU (1 dbu = 1 nm).
///
/// The ring surrounds a `region_width` by `region_height` opening with a frame
/// `ring_width` thick; the overall footprint is the opening grown by `ring_width` on
/// every side. See each field for its range and default.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GuardRingParams {
    /// The conductor layer the ring is drawn on.
    pub layer: RingLayer,
    /// Width of the rectangular opening the ring guards, in DBU. Must be at least
    /// the layer's minimum spacing so the left and right strips clear each other.
    pub region_width: i32,
    /// Height of the rectangular opening the ring guards, in DBU. Must be at least
    /// the layer's minimum spacing so the top and bottom strips clear each other.
    pub region_height: i32,
    /// Thickness of each ring strip, in DBU. Must be at least the layer's minimum
    /// width, and, when `taps` is set, at least a contact plus its enclosure on both
    /// sides.
    pub ring_width: i32,
    /// Whether to place a row of `licon` substrate-tap contacts along the bottom
    /// strip. Only valid when `layer` is `li1`.
    pub taps: bool,
}

impl Default for GuardRingParams {
    fn default() -> Self {
        // A generous li1 ring with taps: every value comfortably clears its rule, so
        // the default is a working example a form can generate unchanged.
        Self {
            layer: RingLayer::Li1,
            region_width: 2_000,
            region_height: 2_000,
            ring_width: 400,
            taps: true,
        }
    }
}

impl GuardRingParams {
    /// Inclusive lower bound for the region opening across every layer: the largest
    /// layer minimum spacing in the subset (`met3`, 300). A form can offer this as
    /// the field minimum; per-layer `validate` still enforces the exact bound.
    const REGION_MIN: i64 = 300;
    /// Inclusive upper bound for lengths, well within the DBU coordinate range while
    /// still allowing large rings (about 1 mm).
    const LEN_MAX: i64 = 1_000_000;
    /// Inclusive lower bound for `ring_width` across every layer: the largest layer
    /// minimum width in the subset (`met3`, 300).
    const RING_MIN: i64 = 300;

    /// The minimum `ring_width` that keeps a substrate-tap contact enclosed by the base
    /// interconnect strip: the contact size plus its enclosure on both sides.
    fn tap_ring_min(gt: &GenTech) -> i32 {
        let tap = gt.tap_cut();
        let (_, enc) = tap.enclosure.expect("tap cut has an enclosure");
        tap.size + 2 * enc
    }
}

impl GenParams for GuardRingParams {
    fn schema() -> ParamSchema {
        // Filled with placeholders; `Generator::schema` stamps the real id/title/
        // description over these before the schema is handed out.
        ParamSchema {
            generator_id: String::new(),
            title: String::new(),
            description: String::new(),
            fields: vec![
                FieldSchema::enumerated(
                    "layer",
                    "Conductor layer the ring is drawn on. Only li1 supports taps.",
                    &RingLayer::VARIANTS,
                    "li1",
                ),
                FieldSchema::int(
                    "region_width",
                    "Width of the guarded opening; at least the layer min spacing.",
                    2_000,
                    Self::REGION_MIN,
                    Self::LEN_MAX,
                    "dbu",
                ),
                FieldSchema::int(
                    "region_height",
                    "Height of the guarded opening; at least the layer min spacing.",
                    2_000,
                    Self::REGION_MIN,
                    Self::LEN_MAX,
                    "dbu",
                ),
                FieldSchema::int(
                    "ring_width",
                    "Thickness of each ring strip; at least the layer min width.",
                    400,
                    Self::RING_MIN,
                    Self::LEN_MAX,
                    "dbu",
                ),
                FieldSchema::bool(
                    "taps",
                    "Place a row of li1 tap contacts along the bottom strip.",
                    true,
                ),
            ],
        }
    }

    fn validate(&self) -> Result<(), GenError> {
        // Validation bounds are the reference (SKY130) technology; the generate path
        // floors dimensions up to the active technology so the output stays clean on
        // any process (see `generate`).
        let gt = GenTech::sky130();
        let cond = self.layer.conductor(&gt);

        check_range(
            "region_width",
            self.region_width,
            i64::from(cond.min_spacing),
            Self::LEN_MAX,
        )?;
        check_range(
            "region_height",
            self.region_height,
            i64::from(cond.min_spacing),
            Self::LEN_MAX,
        )?;
        check_range(
            "ring_width",
            self.ring_width,
            i64::from(cond.min_width),
            Self::LEN_MAX,
        )?;

        if self.taps {
            if self.layer != RingLayer::Li1 {
                return Err(GenError::Invalid {
                    field: "taps",
                    reason: "tap contacts are only supported on the li1 layer",
                });
            }
            if self.ring_width < Self::tap_ring_min(&gt) {
                return Err(GenError::Invalid {
                    field: "ring_width",
                    reason: "too thin to enclose a licon tap by the li.5 enclosure on both sides",
                });
            }
        }

        Ok(())
    }
}

/// The guard-ring generator.
///
/// Emits a closed conductor ring around a region with optional `li1` tap contacts;
/// see [`GuardRingParams`] for the parameters and the [crate overview](crate) for
/// the DRC-clean-by-construction argument.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct GuardRing;

impl Generator for GuardRing {
    type Params = GuardRingParams;

    fn id(&self) -> &'static str {
        "guard_ring"
    }

    fn title(&self) -> &'static str {
        "Guard ring"
    }

    fn description(&self) -> &'static str {
        "A closed conductor ring around a rectangular region, optionally lined with \
         a row of li1 substrate-tap contacts. DRC-clean by construction against the \
         SKY130 subset."
    }

    fn generate(
        &self,
        params: &Self::Params,
        tech: &Technology,
        cell: &mut Cell,
    ) -> Result<GenOutput, GenError> {
        let start = cell.shapes.len();
        let gt = GenTech::for_technology(tech);
        let cond = params.layer.conductor(&gt);
        let layer = cond.layer;
        let rw = params.ring_width;

        // Footprint: the opening grown by `ring_width` on every side, placed with its
        // lower-left corner at the origin.
        let outer_w = params.region_width + 2 * rw;
        let outer_h = params.region_height + 2 * rw;

        // Four overlapping strips forming a closed frame. Left/right span the full
        // height; bottom/top span the full width; they share the corner squares.
        let strips = [
            Rect::new(Point::new(0, 0), Point::new(outer_w, rw)), // bottom
            Rect::new(Point::new(0, outer_h - rw), Point::new(outer_w, outer_h)), // top
            Rect::new(Point::new(0, 0), Point::new(rw, outer_h)), // left
            Rect::new(Point::new(outer_w - rw, 0), Point::new(outer_w, outer_h)), // right
        ];
        for strip in strips {
            cell.shapes
                .push(DrawShape::new(layer, ShapeKind::Rect(strip)));
        }

        if params.taps {
            emit_taps(cell, &gt, outer_w, rw);
        }

        let added = cell.shapes.len() - start;
        let bbox = Rect::from_points([Point::new(0, 0), Point::new(outer_w, outer_h)]);
        Ok(GenOutput {
            shapes_added: added,
            bbox,
        })
    }
}

/// Places a centered row of substrate-tap contacts along the bottom strip.
///
/// Contacts are square at the exact drawn size, centered on the strip's mid-height
/// (so the strip encloses them top and bottom), and stepped along x at a pitch of
/// the contact size plus the technology's safe cut margin, keeping the whole row
/// within the enclosed span. `ring_width` is guaranteed by `validate` to leave room
/// for the enclosure.
fn emit_taps(cell: &mut Cell, gt: &GenTech, outer_w: i32, rw: i32) {
    let cut = gt.tap_cut();
    let (_, enc) = cut.enclosure.expect("tap cut has an enclosure");

    // The x-span within which a contact's left edge keeps `enc` clearance to both
    // ends of the bottom strip.
    let first_x = enc;
    let last_x = outer_w - enc - cut.size;
    if last_x < first_x {
        return; // strip too short for even one enclosed contact
    }

    // Center each contact vertically in the strip; validated `rw` guarantees `enc`
    // clearance above and below.
    let y0 = (rw - cut.size) / 2;
    let pitch = cut.size + gt.safe_cut_margin();

    let mut x = first_x;
    while x <= last_x {
        let contact = Rect::new(Point::new(x, y0), Point::new(x + cut.size, y0 + cut.size));
        cell.shapes
            .push(DrawShape::new(cut.layer, ShapeKind::Rect(contact)));
        x += pitch;
    }
}

/// Range check that yields a [`GenError::OutOfRange`] naming the field on failure.
fn check_range(field: &'static str, value: i32, min: i64, max: i64) -> Result<(), GenError> {
    let v = i64::from(value);
    if v < min || v > max {
        Err(GenError::OutOfRange {
            field,
            value: v,
            min,
            max,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky130;

    fn build(params: &GuardRingParams) -> Cell {
        let mut cell = Cell::new("top");
        GuardRing
            .generate(params, &Technology::default(), &mut cell)
            .expect("valid params generate");
        cell
    }

    #[test]
    fn default_params_validate_and_generate() {
        let p = GuardRingParams::default();
        p.validate().expect("default is valid");
        let cell = build(&p);
        // Four ring strips plus at least one tap.
        assert!(cell.shapes.len() > 4, "ring plus taps");
        assert!(
            cell.shapes.iter().any(|s| s.layer == sky130::LICON.layer),
            "taps present on the licon layer"
        );
    }

    #[test]
    fn ring_without_taps_is_four_strips() {
        let p = GuardRingParams {
            layer: RingLayer::Met2,
            region_width: 1_000,
            region_height: 500,
            ring_width: 300,
            taps: false,
        };
        p.validate().expect("valid");
        let cell = build(&p);
        assert_eq!(cell.shapes.len(), 4, "exactly the four ring strips");
        assert!(cell.shapes.iter().all(|s| s.layer == sky130::MET2.layer));
    }

    #[test]
    fn taps_rejected_on_non_li1() {
        let p = GuardRingParams {
            layer: RingLayer::Met1,
            taps: true,
            ..GuardRingParams::default()
        };
        assert!(matches!(
            p.validate(),
            Err(GenError::Invalid { field: "taps", .. })
        ));
    }

    #[test]
    fn thin_ring_rejected() {
        let p = GuardRingParams {
            ring_width: 100, // below li1 min width 170
            ..GuardRingParams::default()
        };
        assert!(matches!(
            p.validate(),
            Err(GenError::OutOfRange {
                field: "ring_width",
                ..
            })
        ));
    }
}
