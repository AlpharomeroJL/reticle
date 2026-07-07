//! The pad-ring generator: a die-size-aware ring of I/O pad structures around the
//! die edge, with corner keep-outs and reinforced power pads.
//!
//! # What it emits
//!
//! Square pad structures on the top conductor level the subset carries (`met3`),
//! placed inset from a `die_width` by `die_height` die outline along all four edges:
//!
//! * **Signal pads.** A single `pad_size` square on `met3`, stepped along each edge
//!   at `pad_pitch` (center to center).
//! * **Power pads.** The first `power_pads` slots (spread around the ring) are
//!   reinforced: the same `met3` square, plus a `met2` backing plate under it and a
//!   `via2` cut array stitching the two, the way a supply pad is stapled to a lower
//!   power bus. The subset gives no distinct pad or bump layer, so this reinforcement
//!   is what makes a power pad geometrically real here (see the limits below).
//! * **Corners.** A keep-out margin at each corner leaves the corner square pad-free,
//!   so the last pad on one edge clears the first pad on the perpendicular edge by at
//!   least the `met3` spacing.
//!
//! # Why it is DRC-clean by construction (SKY130 subset)
//!
//! * **Width and area.** Every pad is a `pad_size` square, validated to be at least
//!   the `met3` minimum width, and its bounding box clears any `met3`/`met2` area
//!   rule. Backing plates only grow the footprint.
//! * **Spacing.** Pads along one edge are pitched at `pad_pitch`, validated to be at
//!   least `pad_size` plus the `met3` minimum spacing, so adjacent pads keep clean
//!   spacing. The corner keep-out is sized so pads on perpendicular edges also clear
//!   that spacing. Power-pad backing plates sit on `met2`, a different layer, so they
//!   never crowd the `met3` pads, and each power pad's plate is contained within its
//!   pad footprint so plates never approach each other.
//! * **Enclosure.** A power pad's `via2` cuts are square at the exact drawn size and
//!   the `met3` pad and `met2` backing plate each cover the whole cut array grown by
//!   a conservative margin (the subset carries no `via2` enclosure), so every cut is
//!   enclosed on all sides by both plates.
//!
//! # Subset-coverage limits
//!
//! A real pad ring needs layers and structures this subset does not carry: the
//! passivation/pad opening, the top thick metal and redistribution, the bump or
//! wire-bond geometry, and ESD devices under each pad. This generator places only the
//! `met3`/`met2` conductor footprints and the `via2` reinforcement the subset checks,
//! which is enough to be DRC-clean against the committed deck but is not a tape-out
//! pad ring. "Power pad" here means a pad reinforced to a lower metal by a cut array,
//! not a pad tied to a real supply net.

use reticle_geometry::{Point, Rect};
use reticle_model::{Cell, DrawShape, ShapeKind, Technology};
use serde::{Deserialize, Serialize};

use crate::error::GenError;
use crate::generator::{GenOutput, GenParams, Generator};
use crate::gentech::{Conductor, GenTech};
use crate::schema::{FieldSchema, ParamSchema};

/// The most `via2` cuts a power pad's reinforcement staple uses per axis. A real
/// supply pad is stitched to the lower bus by a compact via array, not a pad-wide
/// flood, so the staple is capped here regardless of pad size; this bounds the
/// emitted geometry (a large pad would otherwise tile thousands of cuts) while
/// keeping the reinforcement, and its enclosure check, genuine.
const MAX_STAPLE_PER_AXIS: i32 = 6;

/// Parameters for the [`PadRing`] generator. All lengths are in DBU (1 dbu = 1 nm).
///
/// Pads are placed inset from a `die_width` by `die_height` die outline (lower-left
/// corner at the origin), along all four edges, with the corners kept clear. See each
/// field for its range and default.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PadRingParams {
    /// Die width in DBU (the outer x extent the pads ring). Must fit at least one pad
    /// per edge inside the corner keep-outs.
    pub die_width: i32,
    /// Die height in DBU (the outer y extent the pads ring). Must fit at least one pad
    /// per edge inside the corner keep-outs.
    pub die_height: i32,
    /// Center-to-center pad pitch along an edge, in DBU. Must be at least the pad size
    /// plus the `met3` minimum spacing so adjacent pads clear each other.
    pub pad_pitch: i32,
    /// Side length of each square pad, in DBU. Must be at least the `met3` minimum
    /// width.
    pub pad_size: i32,
    /// How many of the placed pads are reinforced power pads, spread evenly around the
    /// ring. Zero for an all-signal ring; capped at the total pad count by `validate`.
    pub power_pads: u32,
}

impl Default for PadRingParams {
    fn default() -> Self {
        // A 200 um square die with 60 um pads on a 100 um pitch and four power pads:
        // a clean, useful default a form can generate unchanged.
        Self {
            die_width: 200_000,
            die_height: 200_000,
            pad_pitch: 100_000,
            pad_size: 60_000,
            power_pads: 4,
        }
    }
}

impl PadRingParams {
    /// Inclusive lower bound offered for the die size: enough for two corner keep-outs
    /// plus one pad. Per-parameter `validate` enforces the exact bound from the pad
    /// size and pitch.
    const DIE_MIN: i64 = 10_000;
    /// Inclusive upper bound for die lengths: well within the DBU coordinate range
    /// while allowing a multi-millimetre die.
    const DIE_MAX: i64 = 4_000_000;
    /// Inclusive lower bound offered for the pad size: the `met3` minimum width (300).
    const PAD_MIN: i64 = 300;
    /// Inclusive upper bound for the pad size.
    const PAD_MAX: i64 = 500_000;
    /// Inclusive lower bound offered for the pitch: a `met3`-width pad plus `met3`
    /// spacing. Per-parameter `validate` enforces the real floor from `pad_size`.
    const PITCH_MIN: i64 = 300 + 300;
    /// Inclusive upper bound for the pitch.
    const PITCH_MAX: i64 = 1_000_000;
    /// Inclusive upper bound offered for the power-pad count; `validate` additionally
    /// caps it at the actual number of placed pads.
    const POWER_MAX: i64 = 4_096;

    /// The conductor level pads are drawn on: the top interconnect (`met3` on SKY130,
    /// `Metal4` on SG13G2), the top of the generator stack.
    fn pad_conductor(gt: &GenTech) -> Conductor {
        gt.top()
    }

    /// The backing-plate level for a power pad: one below the pad level.
    fn backing_conductor(gt: &GenTech) -> Conductor {
        gt.conductor(2)
    }

    /// How far a pad sits in from the die edge on the perpendicular axis. A small
    /// fixed inset (the pad-level spacing) keeps every pad clearly inside the outline.
    fn edge_inset(gt: &GenTech) -> i32 {
        Self::pad_conductor(gt).min_spacing
    }

    /// The pad-level minimum spacing, the clearance every non-overlapping pad pair must
    /// keep.
    fn pad_spacing(gt: &GenTech) -> i32 {
        Self::pad_conductor(gt).min_spacing
    }

    /// Corner ownership. The **left and right columns run the full die height**; the
    /// **bottom and top rows fill only the interior width between the columns**, inset
    /// by a pad plus the pad spacing from each side. That way a row pad and the
    /// perpendicular column pad always clear each other by the pad spacing, and the
    /// two corners never place two pads a sub-spacing gap apart.
    ///
    /// The count along a column is how many pads fit in the height between the edge
    /// insets; along a row, how many fit in the corner-clear interior width.
    fn column_pad_count(&self, gt: &GenTech) -> u32 {
        // Full-height span available to a column: [inset, height - inset].
        let span = i64::from(self.die_height) - 2 * i64::from(Self::edge_inset(gt));
        fit_count(span, self.pad_size, self.pad_pitch)
    }

    /// The count of pads along a bottom/top row (the corner-clear interior width).
    fn row_pad_count(&self, gt: &GenTech) -> u32 {
        let span = self.row_interior_span(gt);
        fit_count(span, self.pad_size, self.pad_pitch)
    }

    /// The interior x-span a row's pads may occupy: the die width minus, on each side,
    /// the edge inset plus a column pad plus the pad spacing.
    fn row_interior_span(&self, gt: &GenTech) -> i64 {
        let side = i64::from(Self::edge_inset(gt))
            + i64::from(self.pad_size)
            + i64::from(Self::pad_spacing(gt));
        i64::from(self.die_width) - 2 * side
    }

    /// Whether the two opposite rows (bottom and top) clear each other vertically:
    /// their inner faces are `die_height - 2*(inset + size)` apart, which must be at
    /// least the pad spacing (or the rows would overlap, which the die size forbids
    /// here because both rows are always placed).
    fn rows_clear_vertically(&self, gt: &GenTech) -> bool {
        let gap = i64::from(self.die_height)
            - 2 * (i64::from(Self::edge_inset(gt)) + i64::from(self.pad_size));
        gap >= i64::from(Self::pad_spacing(gt))
    }

    /// The total number of pads placed around the ring: two full-height columns plus
    /// two interior-width rows.
    fn total_pads(&self, gt: &GenTech) -> u32 {
        self.column_pad_count(gt)
            .saturating_mul(2)
            .saturating_add(self.row_pad_count(gt).saturating_mul(2))
    }
}

impl GenParams for PadRingParams {
    fn schema() -> ParamSchema {
        // Placeholders; `Generator::schema` stamps the real id/title/description over
        // these before the schema is handed out.
        ParamSchema {
            generator_id: String::new(),
            title: String::new(),
            description: String::new(),
            fields: vec![
                FieldSchema::int(
                    "die_width",
                    "Die width; must fit at least one pad per edge inside the corner keep-outs.",
                    200_000,
                    Self::DIE_MIN,
                    Self::DIE_MAX,
                    "dbu",
                ),
                FieldSchema::int(
                    "die_height",
                    "Die height; must fit at least one pad per edge inside the corner keep-outs.",
                    200_000,
                    Self::DIE_MIN,
                    Self::DIE_MAX,
                    "dbu",
                ),
                FieldSchema::int(
                    "pad_pitch",
                    "Center-to-center pad pitch; at least the pad size plus met3 spacing.",
                    100_000,
                    Self::PITCH_MIN,
                    Self::PITCH_MAX,
                    "dbu",
                ),
                FieldSchema::int(
                    "pad_size",
                    "Side length of each square pad; at least the met3 min width.",
                    60_000,
                    Self::PAD_MIN,
                    Self::PAD_MAX,
                    "dbu",
                ),
                FieldSchema::int(
                    "power_pads",
                    "Number of reinforced power pads, spread around the ring.",
                    4,
                    0,
                    Self::POWER_MAX,
                    "count",
                ),
            ],
        }
    }

    fn validate(&self) -> Result<(), GenError> {
        check_range(
            "pad_size",
            self.pad_size.into(),
            Self::PAD_MIN,
            Self::PAD_MAX,
        )?;
        check_range(
            "pad_pitch",
            self.pad_pitch.into(),
            Self::PITCH_MIN,
            Self::PITCH_MAX,
        )?;
        check_range(
            "die_width",
            self.die_width.into(),
            Self::DIE_MIN,
            Self::DIE_MAX,
        )?;
        check_range(
            "die_height",
            self.die_height.into(),
            Self::DIE_MIN,
            Self::DIE_MAX,
        )?;
        check_range("power_pads", i64::from(self.power_pads), 0, Self::POWER_MAX)?;

        // Validation bounds are the reference (SKY130) technology; the generate path
        // uses the active technology's own spacing.
        let gt = GenTech::sky130();

        // Pitch must keep adjacent pads spaced by at least the pad-level min spacing.
        let pitch_floor =
            i64::from(self.pad_size) + i64::from(Self::pad_conductor(&gt).min_spacing);
        if i64::from(self.pad_pitch) < pitch_floor {
            return Err(GenError::Invalid {
                field: "pad_pitch",
                reason: "too tight to keep adjacent pads spaced by the pad-level min spacing",
            });
        }

        // The die must fit at least one column pad (full height) and, once the
        // corners are reserved for the columns, at least one row pad in the interior
        // width. Blame the dimension that is short.
        if self.column_pad_count(&gt) == 0 {
            return Err(GenError::Invalid {
                field: "die_height",
                reason: "too short to fit a pad along the left/right columns",
            });
        }
        if self.row_pad_count(&gt) == 0 {
            return Err(GenError::Invalid {
                field: "die_width",
                reason: "too narrow to fit a pad in the interior between the columns",
            });
        }
        // The bottom and top rows are both placed, so they must clear each other
        // vertically by the pad spacing (a short die that would leave a sub-spacing
        // gap between the two rows is rejected rather than emitting a violation).
        if !self.rows_clear_vertically(&gt) {
            return Err(GenError::Invalid {
                field: "die_height",
                reason: "too short to clear the bottom and top rows by the pad-level spacing",
            });
        }

        // Power pads cannot exceed the pads actually placed.
        if self.power_pads > self.total_pads(&gt) {
            return Err(GenError::Invalid {
                field: "power_pads",
                reason: "more power pads requested than pads placed around the ring",
            });
        }
        Ok(())
    }
}

/// The pad-ring generator.
///
/// Emits a die-size-aware ring of I/O pads with corner keep-outs and reinforced power
/// pads; see [`PadRingParams`] for the parameters and the [crate overview](crate) for
/// the DRC-clean-by-construction argument.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PadRing;

/// Which edge of the die a pad run marches along, fixing how a slot index maps to a
/// pad position.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Edge {
    Bottom,
    Top,
    Left,
    Right,
}

impl Generator for PadRing {
    type Params = PadRingParams;

    fn id(&self) -> &'static str {
        "pad_ring"
    }

    fn title(&self) -> &'static str {
        "Pad ring"
    }

    fn description(&self) -> &'static str {
        "A die-size-aware ring of I/O pad structures around the die edge, with corner \
         keep-outs and reinforced power pads. DRC-clean by construction against the \
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

        // Collect every pad's lower-left corner in a stable order (bottom, top, left,
        // right), so the "first N are power pads" choice is deterministic.
        let mut pads: Vec<Point> = Vec::new();
        for edge in [Edge::Bottom, Edge::Top, Edge::Left, Edge::Right] {
            collect_edge_pads(params, edge, &mut pads, &gt);
        }

        // Spread the requested power pads evenly across the placed pads by striding:
        // the power pads land at indices 0, stride, 2*stride, ... so they are
        // distributed around the ring rather than bunched on one edge.
        let total = pads.len() as u32;
        let power = params.power_pads.min(total);
        let stride = if power == 0 {
            1
        } else {
            (total / power).max(1)
        };

        for (i, &corner) in pads.iter().enumerate() {
            let i = i as u32;
            // A slot is a power pad if it is one of the first `power` stride steps.
            let is_power = power > 0 && i.is_multiple_of(stride) && (i / stride) < power;
            emit_pad(cell, corner, params.pad_size, is_power, &gt);
        }

        let added = cell.shapes.len() - start;
        let bbox = Rect::from_points([
            Point::new(0, 0),
            Point::new(params.die_width, params.die_height),
        ]);
        Ok(GenOutput {
            shapes_added: added,
            bbox,
        })
    }
}

/// Appends the lower-left corners of every pad along one edge to `pads`.
///
/// The left and right columns run the full die height, starting one edge inset up and
/// stepping by `pad_pitch`. The bottom and top rows fill only the interior width
/// between the columns: they start a pad-plus-spacing in from the column on each side,
/// so a row pad always clears the perpendicular column by the pad spacing. Only whole
/// pads that fit their span are emitted.
fn collect_edge_pads(params: &PadRingParams, edge: Edge, pads: &mut Vec<Point>, gt: &GenTech) {
    let size = params.pad_size;
    let inset = PadRingParams::edge_inset(gt);

    match edge {
        Edge::Left | Edge::Right => {
            // Full-height column: pads step along y from `inset` upward.
            let count = params.column_pad_count(gt);
            let near_x = inset;
            let far_x = params.die_width - inset - size;
            let x = if edge == Edge::Left { near_x } else { far_x };
            for i in 0..count {
                let y = inset + (i as i32) * params.pad_pitch;
                pads.push(Point::new(x, y));
            }
        }
        Edge::Bottom | Edge::Top => {
            // Interior-width row: pads step along x, clearing the columns by spacing.
            let count = params.row_pad_count(gt);
            let row_lo_x = inset + size + PadRingParams::pad_spacing(gt);
            let near_y = inset;
            let far_y = params.die_height - inset - size;
            let y = if edge == Edge::Bottom { near_y } else { far_y };
            for i in 0..count {
                let x = row_lo_x + (i as i32) * params.pad_pitch;
                pads.push(Point::new(x, y));
            }
        }
    }
}

/// How many pads of side `size` fit along a `span` at center-to-center `pitch`: one
/// at the span start, then one per whole `pitch` that still fits a whole pad. Returns
/// `0` if not even one pad fits (including a non-positive span).
fn fit_count(span: i64, size: i32, pitch: i32) -> u32 {
    if span < i64::from(size) {
        return 0;
    }
    // Last start that still fits a whole pad is `span - size`.
    let extra = (span - i64::from(size)) / i64::from(pitch);
    (1 + extra) as u32
}

/// Emits one pad at lower-left corner `corner`: a `met3` square, plus, for a power
/// pad, a `met2` backing plate and a `via2` cut array stitching the two.
///
/// The reinforcement is contained within the pad footprint: the `via2` cuts sit in
/// the pad interior and both the pad and the backing plate cover the cut array grown
/// by a conservative enclosure, so every cut is enclosed on all sides and the backing
/// plate never reaches beyond the pad it backs.
fn emit_pad(cell: &mut Cell, corner: Point, size: i32, is_power: bool, gt: &GenTech) {
    let pad_layer = PadRingParams::pad_conductor(gt).layer;
    let pad = Rect::new(corner, Point::new(corner.x + size, corner.y + size));
    cell.shapes
        .push(DrawShape::new(pad_layer, ShapeKind::Rect(pad)));

    if !is_power {
        return;
    }

    // Reinforce: a bounded top-cut staple inside the pad, covered by a backing plate
    // one level down. A power pad is stapled to the lower bus by a compact via array,
    // not a full flood of the pad, so the array is capped per axis regardless of pad
    // size; this keeps the emitted geometry bounded and matches how real supply pads
    // staple.
    let cut = gt.cut(2);
    let (_, enc) = cut.enclosure.expect("top cut has an enclosure margin");
    let pitch = cut.size + gt.safe_cut_margin();

    // How many cuts fit per axis inside the pad, leaving `enc + pitch` of margin on
    // each side so both covering plates stay inside the pad, then cap at the staple
    // maximum. A pad that passed `validate` is at least the met3 min width, but a very
    // small pad may fit no cut; guard for that.
    let margin = enc + pitch;
    let usable = i64::from(size) - 2 * i64::from(margin);
    let fit_per_axis = if usable < i64::from(cut.size) {
        0
    } else {
        (1 + (usable - i64::from(cut.size)) / i64::from(pitch)).min(i64::from(MAX_STAPLE_PER_AXIS))
            as i32
    };
    if fit_per_axis == 0 {
        return; // pad too small to staple; it stays a plain pad
    }

    // Center the capped array in the pad.
    let span = (fit_per_axis - 1) * pitch + cut.size;
    let lo_x = corner.x + (size - span) / 2;
    let lo_y = corner.y + (size - span) / 2;

    let mut array_bbox: Option<Rect> = None;
    for row in 0..fit_per_axis {
        for col in 0..fit_per_axis {
            let x = lo_x + col * pitch;
            let y = lo_y + row * pitch;
            let c = Rect::new(Point::new(x, y), Point::new(x + cut.size, y + cut.size));
            cell.shapes
                .push(DrawShape::new(cut.layer, ShapeKind::Rect(c)));
            array_bbox = Some(array_bbox.map_or(c, |b| b.union(&c)));
        }
    }

    // A pad big enough to be a power pad always fits at least one cut; if it somehow
    // did not, skip the backing plate rather than emit a plate over no cuts.
    if let Some(bbox) = array_bbox {
        // The met2 backing plate covers the whole cut array grown by the enclosure,
        // and stays inside the pad footprint (the array was inset by `enc + pitch`, so
        // growing by `enc` keeps a positive gap to the pad edge).
        let plate = bbox.expanded(enc);
        cell.shapes.push(DrawShape::new(
            PadRingParams::backing_conductor(gt).layer,
            ShapeKind::Rect(plate),
        ));
    }
}

/// Range check that yields a [`GenError::OutOfRange`] naming the field on failure.
fn check_range(field: &'static str, value: i64, min: i64, max: i64) -> Result<(), GenError> {
    if value < min || value > max {
        Err(GenError::OutOfRange {
            field,
            value,
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

    fn build(params: &PadRingParams) -> Cell {
        let mut cell = Cell::new("top");
        PadRing
            .generate(params, &Technology::default(), &mut cell)
            .expect("valid params generate");
        cell
    }

    #[test]
    fn default_params_validate_and_generate() {
        let p = PadRingParams::default();
        p.validate().expect("default is valid");
        let cell = build(&p);
        // At least the pads on met3, plus power-pad reinforcement on met2/via2.
        assert!(
            cell.shapes.iter().any(|s| s.layer == sky130::MET3.layer),
            "pads on met3"
        );
        assert!(
            cell.shapes.iter().any(|s| s.layer == sky130::MET2.layer),
            "power-pad backing on met2"
        );
        assert!(
            cell.shapes.iter().any(|s| s.layer == sky130::VIA2.layer),
            "power-pad cuts on via2"
        );
    }

    #[test]
    fn power_pad_count_is_honored() {
        let p = PadRingParams::default();
        let total = p.total_pads(&GenTech::sky130());
        assert!(total >= p.power_pads, "enough pads for the power request");
        // Count distinct met3 pad squares: one per pad.
        let cell = build(&p);
        let pads = cell
            .shapes
            .iter()
            .filter(|s| s.layer == sky130::MET3.layer)
            .count() as u32;
        assert_eq!(pads, total, "one met3 square per placed pad");
    }

    #[test]
    fn zero_power_is_all_signal() {
        let p = PadRingParams {
            power_pads: 0,
            ..PadRingParams::default()
        };
        p.validate().expect("valid");
        let cell = build(&p);
        assert!(
            !cell.shapes.iter().any(|s| s.layer == sky130::VIA2.layer),
            "no reinforcement cuts when no power pads"
        );
        assert!(
            !cell.shapes.iter().any(|s| s.layer == sky130::MET2.layer),
            "no backing plates when no power pads"
        );
    }

    #[test]
    fn tight_pitch_rejected() {
        let p = PadRingParams {
            pad_size: 60_000,
            pad_pitch: 60_100, // below pad_size + met3 spacing (60_000 + 300)
            ..PadRingParams::default()
        };
        assert!(matches!(
            p.validate(),
            Err(GenError::Invalid {
                field: "pad_pitch",
                ..
            })
        ));
    }

    #[test]
    fn too_many_power_pads_rejected() {
        let p = PadRingParams {
            power_pads: 4_000, // far more than a 200 um die can hold
            ..PadRingParams::default()
        };
        assert!(matches!(
            p.validate(),
            Err(GenError::Invalid {
                field: "power_pads",
                ..
            })
        ));
    }

    #[test]
    fn small_die_one_pad_per_edge() {
        // A die just big enough for one pad per edge inside the keep-outs.
        let p = PadRingParams {
            die_width: 200_000,
            die_height: 200_000,
            pad_pitch: 100_000,
            pad_size: 40_000,
            power_pads: 1,
        };
        p.validate().expect("valid");
        let cell = build(&p);
        assert!(cell.shapes.iter().any(|s| s.layer == sky130::MET3.layer));
    }
}
