//! The seal-ring generator: a continuous stacked-metal-plus-cut barrier around the
//! die edge.
//!
//! # What it emits
//!
//! A closed ring at the die outline, built from a stack of conductor frames tied
//! together by rings of cuts:
//!
//! * **Conductor frames.** For every conductor level in the chosen [`SealStack`] a
//!   closed frame (four overlapping strips, like the [guard ring](crate::GuardRing))
//!   sits flush with the die edge, `ring_width` DBU thick. Because each level is on
//!   its own GDS layer the frames stack directly on top of one another.
//! * **Cut rings.** Between each adjacent pair of conductor levels a ring of square
//!   cuts runs along all four strips: a row of cuts along the bottom and top strips
//!   and a column along the left and right strips, every cut centered in the strip
//!   so the frames above and below enclose it on all sides.
//!
//! The result is the classic guard/seal barrier: a continuous conductor wall from
//! the lowest to the highest level in the stack, stitched by cuts, encircling the
//! whole die.
//!
//! # Why it is DRC-clean by construction (SKY130 subset)
//!
//! * **Width.** Every strip is `ring_width` thick and spans a full side of the die,
//!   so each frame rectangle measures `ring_width` on its short side; `ring_width`
//!   is validated to be at least the widest conductor minimum width in the stack.
//! * **Spacing.** The two strips on each side of a frame face each other across the
//!   die interior; the interior (`die_width`/`die_height` shrunk by two ring widths)
//!   is validated to be at least the widest conductor minimum spacing in the stack,
//!   so no interior gap is sub-spacing. Cuts within a strip are pitched at their size
//!   plus a safe margin, and cuts on different layers never share a layer, so no cut
//!   pair is a spacing violation.
//! * **Enclosure.** Each cut is square at its exact drawn size and centered in the
//!   strip; `ring_width` is validated to cover a cut plus the largest enclosure the
//!   subset asks of any conductor in the stack on both sides, so every cut is
//!   enclosed on all four sides by the frames it bridges.
//! * **Area.** Each strip spans a full side of the die, so its bounding-box area is
//!   far above any conductor minimum-area rule.
//!
//! # Subset-coverage limits
//!
//! A real seal ring needs layers this subset does not carry: a dedicated
//! seal-ring/pad-protection marker, the top thick metal, and the passivation and
//! redistribution openings. This generator builds the barrier only on the digital
//! metal stack the subset checks (`li1`, `met1`, `met2`, `met3` and the
//! `mcon`/`via`/`via2` cuts), which is enough to be DRC-clean against the committed
//! deck but is not a tape-out seal ring.

use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, DrawShape, ShapeKind, Technology};
use serde::{Deserialize, Serialize};

use crate::error::GenError;
use crate::generator::{GenOutput, GenParams, Generator};
use crate::schema::{FieldSchema, ParamSchema};
use crate::sky130::{self, Conductor, Cut};

/// How tall the seal-ring stack is: which conductor levels it walls off and which
/// cuts stitch them.
///
/// Each choice is a contiguous slice of the SKY130 digital metal stack from `li1`
/// upward, tied by the cut layers the subset carries between those levels. A taller
/// stack is a more complete barrier; the tallest the subset supports is `li1`
/// through `met3`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SealStack {
    /// `li1` and `met1`, stitched by `mcon`.
    Li1Met1,
    /// `li1`, `met1`, `met2`, stitched by `mcon` and `via`.
    UpToMet2,
    /// `li1`, `met1`, `met2`, `met3`, stitched by `mcon`, `via`, `via2`. The tallest
    /// barrier the subset supports.
    #[default]
    UpToMet3,
}

/// One conductor-to-conductor step in a seal stack: the cut that stitches the two
/// levels and the enclosure margin the cut owes each frame.
#[derive(Clone, Copy, Debug)]
struct StackStep {
    /// The cut layer stitching the level below to the level above.
    cut: Cut,
    /// The enclosure margin (DBU) each frame must keep around the cut. This is the
    /// larger of the two levels' subset enclosures where both exist, or a
    /// conservative positive margin for a step the subset gives no rule (the `via2`
    /// step), so the frames cover the cut with margin to spare either way.
    enclosure: i32,
}

impl SealStack {
    /// The conductor levels in the stack, lowest first.
    fn conductors(self) -> &'static [Conductor] {
        match self {
            Self::Li1Met1 => &[sky130::LI1, sky130::MET1],
            Self::UpToMet2 => &[sky130::LI1, sky130::MET1, sky130::MET2],
            Self::UpToMet3 => &[sky130::LI1, sky130::MET1, sky130::MET2, sky130::MET3],
        }
    }

    /// The cut steps stitching adjacent conductor levels, lowest first. There is one
    /// fewer step than there are conductors.
    fn steps(self) -> &'static [StackStep] {
        // mcon (li1<->met1): m1.4 asks met1 to enclose it by 30; li1 has no subset
        // enclosure for mcon, so 30 is the binding margin, applied to both frames.
        const MCON_STEP: StackStep = StackStep {
            cut: sky130::MCON,
            enclosure: 30,
        };
        // via (met1<->met2): m2.4 asks met2 to enclose it by 55.
        const VIA_STEP: StackStep = StackStep {
            cut: sky130::VIA,
            enclosure: 55,
        };
        // via2 (met2<->met3): the subset carries no via2 enclosure, so a conservative
        // positive margin keeps both frames comfortably over the cut.
        const VIA2_STEP: StackStep = StackStep {
            cut: sky130::VIA2,
            enclosure: 65,
        };
        match self {
            Self::Li1Met1 => &[MCON_STEP],
            Self::UpToMet2 => &[MCON_STEP, VIA_STEP],
            Self::UpToMet3 => &[MCON_STEP, VIA_STEP, VIA2_STEP],
        }
    }

    /// The widest conductor minimum width across the stack (drives the `ring_width`
    /// floor).
    fn max_min_width(self) -> i32 {
        self.conductors()
            .iter()
            .map(|c| c.min_width)
            .max()
            .expect("a stack has at least two conductors")
    }

    /// The widest conductor minimum spacing across the stack (drives the die-interior
    /// floor).
    fn max_min_spacing(self) -> i32 {
        self.conductors()
            .iter()
            .map(|c| c.min_spacing)
            .max()
            .expect("a stack has at least two conductors")
    }

    /// The largest enclosure any step asks of a frame (drives the `ring_width` floor
    /// when cuts are present).
    fn max_enclosure(self) -> i32 {
        self.steps()
            .iter()
            .map(|s| s.enclosure)
            .max()
            .expect("a stack has at least one step")
    }

    /// The serde variant strings, for the schema's enum field.
    const VARIANTS: [&'static str; 3] = ["li1_met1", "up_to_met2", "up_to_met3"];
}

/// Parameters for the [`SealRing`] generator. All lengths are in DBU (1 dbu = 1 nm).
///
/// The ring is drawn flush with a `die_width` by `die_height` die outline (lower-left
/// corner at the origin), as a frame `ring_width` thick on every conductor level in
/// `stack`. See each field for its range and default.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SealRingParams {
    /// Which conductor levels the barrier walls off and which cuts stitch them.
    pub stack: SealStack,
    /// Die width in DBU (the ring's outer x extent). Must leave a die interior at
    /// least the stack's widest minimum spacing after two ring widths are removed.
    pub die_width: i32,
    /// Die height in DBU (the ring's outer y extent). Must leave a die interior at
    /// least the stack's widest minimum spacing after two ring widths are removed.
    pub die_height: i32,
    /// Thickness of each frame strip in DBU. Must be at least the stack's widest
    /// conductor minimum width, and at least a cut plus the largest step enclosure on
    /// both sides so every cut stays enclosed.
    pub ring_width: i32,
}

impl Default for SealRingParams {
    fn default() -> Self {
        // A generous full-stack ring on a 100 um square die: every value comfortably
        // clears its rule, so the default is a working example a form can generate
        // unchanged.
        Self {
            stack: SealStack::UpToMet3,
            die_width: 100_000,
            die_height: 100_000,
            ring_width: 900,
        }
    }
}

impl SealRingParams {
    /// Inclusive lower bound offered for the die size across every stack: enough for
    /// two `met3` ring widths plus a `met3`-spacing interior. Per-stack `validate`
    /// still enforces the exact bound from `ring_width`.
    const DIE_MIN: i64 = 2 * 300 + 300;
    /// Inclusive upper bound for die lengths: well within the DBU coordinate range
    /// while allowing a multi-millimetre die.
    const DIE_MAX: i64 = 4_000_000;
    /// Inclusive lower bound offered for `ring_width`: the widest conductor minimum
    /// width in the subset (`met3`, 300). Per-stack `validate` enforces the real
    /// floor, which is higher whenever cuts must stay enclosed.
    const RING_MIN: i64 = 300;
    /// Inclusive upper bound for `ring_width`.
    const RING_MAX: i64 = 100_000;

    /// The smallest `ring_width` that keeps every cut in the stack enclosed: the
    /// largest cut size plus the largest step enclosure on both sides.
    fn ring_floor_for_cuts(stack: SealStack) -> i32 {
        let max_cut = stack
            .steps()
            .iter()
            .map(|s| s.cut.size)
            .max()
            .expect("a stack has at least one step");
        max_cut + 2 * stack.max_enclosure()
    }

    /// The smallest die side that leaves an interior of at least `min_interior` after
    /// two ring widths are removed.
    fn die_floor(&self, min_interior: i32) -> i64 {
        i64::from(self.ring_width) * 2 + i64::from(min_interior)
    }
}

impl GenParams for SealRingParams {
    fn schema() -> ParamSchema {
        // Placeholders; `Generator::schema` stamps the real id/title/description over
        // these before the schema is handed out.
        ParamSchema {
            generator_id: String::new(),
            title: String::new(),
            description: String::new(),
            fields: vec![
                FieldSchema::enumerated(
                    "stack",
                    "Conductor levels the barrier walls off and the cuts that stitch them.",
                    &SealStack::VARIANTS,
                    "up_to_met3",
                ),
                FieldSchema::int(
                    "die_width",
                    "Die width; must leave a die interior >= the stack min spacing.",
                    100_000,
                    Self::DIE_MIN,
                    Self::DIE_MAX,
                    "dbu",
                ),
                FieldSchema::int(
                    "die_height",
                    "Die height; must leave a die interior >= the stack min spacing.",
                    100_000,
                    Self::DIE_MIN,
                    Self::DIE_MAX,
                    "dbu",
                ),
                FieldSchema::int(
                    "ring_width",
                    "Thickness of each frame strip; at least a cut plus enclosure on both sides.",
                    900,
                    Self::RING_MIN,
                    Self::RING_MAX,
                    "dbu",
                ),
            ],
        }
    }

    fn validate(&self) -> Result<(), GenError> {
        check_range(
            "ring_width",
            self.ring_width,
            Self::RING_MIN,
            Self::RING_MAX,
        )?;

        // `ring_width` must clear both the widest conductor width and, because the
        // stack always has cuts, room to enclose the largest cut on both sides.
        let ring_floor = i64::from(self.stack.max_min_width())
            .max(i64::from(Self::ring_floor_for_cuts(self.stack)));
        if i64::from(self.ring_width) < ring_floor {
            return Err(GenError::Invalid {
                field: "ring_width",
                reason: "too thin to span the stack min width and enclose the stack's cuts",
            });
        }

        // The die must be large enough to leave a spacing-clean interior after two
        // ring widths. Check the field range first, then the ring-dependent floor.
        check_range("die_width", self.die_width, Self::DIE_MIN, Self::DIE_MAX)?;
        check_range("die_height", self.die_height, Self::DIE_MIN, Self::DIE_MAX)?;
        let interior = self.stack.max_min_spacing();
        if i64::from(self.die_width) < self.die_floor(interior) {
            return Err(GenError::Invalid {
                field: "die_width",
                reason: "too small to leave a spacing-clean interior after two ring widths",
            });
        }
        if i64::from(self.die_height) < self.die_floor(interior) {
            return Err(GenError::Invalid {
                field: "die_height",
                reason: "too small to leave a spacing-clean interior after two ring widths",
            });
        }
        Ok(())
    }
}

/// The seal-ring generator.
///
/// Emits a continuous stacked-metal-plus-cut barrier around the die edge; see
/// [`SealRingParams`] for the parameters and the [crate overview](crate) for the
/// DRC-clean-by-construction argument.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SealRing;

impl Generator for SealRing {
    type Params = SealRingParams;

    fn id(&self) -> &'static str {
        "seal_ring"
    }

    fn title(&self) -> &'static str {
        "Seal ring"
    }

    fn description(&self) -> &'static str {
        "A continuous stacked-metal-plus-cut barrier around the die edge: a closed \
         conductor frame on each level of the chosen stack, stitched by rings of \
         cuts. DRC-clean by construction against the SKY130 subset."
    }

    fn generate(
        &self,
        params: &Self::Params,
        _tech: &Technology,
        cell: &mut Cell,
    ) -> Result<GenOutput, GenError> {
        let start = cell.shapes.len();
        let rw = params.ring_width;
        let die_w = params.die_width;
        let die_h = params.die_height;

        // A closed frame on every conductor level, flush with the die outline.
        for cond in params.stack.conductors() {
            for strip in frame_strips(die_w, die_h, rw) {
                cell.shapes
                    .push(DrawShape::new(cond.layer, ShapeKind::Rect(strip)));
            }
        }

        // A ring of cuts for every stitch step, centered in the strips.
        for step in params.stack.steps() {
            emit_cut_ring(cell, step.cut, die_w, die_h, rw);
        }

        let added = cell.shapes.len() - start;
        let bbox = Rect::from_points([Point::new(0, 0), Point::new(die_w, die_h)]);
        Ok(GenOutput {
            shapes_added: added,
            bbox,
        })
    }
}

/// The four overlapping strips of a closed frame flush with a `die_w` by `die_h`
/// outline, each `rw` thick. Left/right span the full height; bottom/top span the
/// full width; they share the corner squares, so the frame is one connected loop.
fn frame_strips(die_w: i32, die_h: i32, rw: i32) -> [Rect; 4] {
    [
        Rect::new(Point::new(0, 0), Point::new(die_w, rw)), // bottom
        Rect::new(Point::new(0, die_h - rw), Point::new(die_w, die_h)), // top
        Rect::new(Point::new(0, 0), Point::new(rw, die_h)), // left
        Rect::new(Point::new(die_w - rw, 0), Point::new(die_w, die_h)), // right
    ]
}

/// Places a ring of square cuts along all four strips of the frame: a row centered
/// in the bottom and top strips and a column centered in the left and right strips.
///
/// Each cut is square at its exact drawn size and centered across the `rw`-thick
/// strip, so the frames above and below (each `rw` thick and validated to leave room
/// for the enclosure) enclose it on all sides. Cuts step at the cut size plus a safe
/// margin; the corners are left cut-free so no two cuts on the layer overlap or crowd
/// (the subset carries no cut-to-cut spacing rule, so the pitch is a conservative
/// choice).
fn emit_cut_ring(cell: &mut Cell, cut: Cut, die_w: i32, die_h: i32, rw: i32) {
    let size = cut.size;
    let pitch = size + sky130::SAFE_CUT_MARGIN;

    // Cut centers stay inside the corner squares of the frame, so a horizontal run
    // spans x in [rw, die_w - rw) and a vertical run spans y in [rw, die_h - rw).
    // Offsets that center the cut across the strip thickness.
    let off = (rw - size) / 2;

    // Horizontal runs: bottom strip (y near 0) and top strip (y near die_h - rw).
    let bottom_y = off;
    let top_y = die_h - rw + off;
    let first_x = rw + off;
    let last_x = die_w - rw - off - size;
    let mut x = first_x;
    while x <= last_x {
        push_cut(cell, cut.layer, x, bottom_y, size);
        push_cut(cell, cut.layer, x, top_y, size);
        x += pitch;
    }

    // Vertical runs: left strip (x near 0) and right strip (x near die_w - rw). Skip
    // the first and last row so a corner cut is not emitted twice (the horizontal
    // runs already cover the corner span start/end); step strictly inside the corners.
    let left_x = off;
    let right_x = die_w - rw + off;
    let first_y = rw + off + pitch;
    let last_y = die_h - rw - off - size - pitch;
    let mut y = first_y;
    while y <= last_y {
        push_cut(cell, cut.layer, left_x, y, size);
        push_cut(cell, cut.layer, right_x, y, size);
        y += pitch;
    }
}

/// Pushes one square cut of side `size` with lower-left corner `(x, y)`.
fn push_cut(cell: &mut Cell, layer: LayerId, x: i32, y: i32, size: i32) {
    let r = Rect::new(Point::new(x, y), Point::new(x + size, y + size));
    cell.shapes.push(DrawShape::new(layer, ShapeKind::Rect(r)));
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

    fn build(params: &SealRingParams) -> Cell {
        let mut cell = Cell::new("top");
        SealRing
            .generate(params, &Technology::default(), &mut cell)
            .expect("valid params generate");
        cell
    }

    #[test]
    fn default_params_validate_and_generate() {
        let p = SealRingParams::default();
        p.validate().expect("default is valid");
        let cell = build(&p);
        // Four conductor levels * 4 strips = 16 frame rects, plus three cut rings.
        assert!(cell.shapes.len() > 16, "frames plus cut rings");
        for cond in p.stack.conductors() {
            assert!(
                cell.shapes.iter().any(|s| s.layer == cond.layer),
                "frame present on conductor layer {:?}",
                cond.layer
            );
        }
        for step in p.stack.steps() {
            assert!(
                cell.shapes.iter().any(|s| s.layer == step.cut.layer),
                "cut ring present on layer {:?}",
                step.cut.layer
            );
        }
    }

    #[test]
    fn frame_count_matches_stack_depth() {
        let p = SealRingParams {
            stack: SealStack::Li1Met1,
            die_width: 20_000,
            die_height: 20_000,
            ring_width: 900,
        };
        p.validate().expect("valid");
        let cell = build(&p);
        // Two conductors * 4 strips = 8 frame rects on li1/met1.
        let frames = cell
            .shapes
            .iter()
            .filter(|s| s.layer == sky130::LI1.layer || s.layer == sky130::MET1.layer)
            .count();
        assert_eq!(frames, 8, "one four-strip frame per conductor level");
        // Only the mcon cut ring stitches this two-level stack.
        assert!(cell.shapes.iter().any(|s| s.layer == sky130::MCON.layer));
        assert!(!cell.shapes.iter().any(|s| s.layer == sky130::VIA.layer));
    }

    #[test]
    fn thin_ring_rejected() {
        let p = SealRingParams {
            ring_width: 300, // clears met3 width but far too thin to enclose a cut
            ..SealRingParams::default()
        };
        assert!(matches!(
            p.validate(),
            Err(GenError::Invalid {
                field: "ring_width",
                ..
            })
        ));
    }

    #[test]
    fn tiny_die_rejected() {
        let p = SealRingParams {
            die_width: 1_000, // smaller than two ring widths plus interior
            ..SealRingParams::default()
        };
        assert!(matches!(
            p.validate(),
            Err(GenError::OutOfRange {
                field: "die_width",
                ..
            } | GenError::Invalid {
                field: "die_width",
                ..
            })
        ));
    }

    #[test]
    fn every_cut_is_enclosed_by_its_frames() {
        // The load-bearing invariant: each cut is fully inside the strips of the
        // conductor levels it stitches, with margin on all four sides.
        let p = SealRingParams::default();
        let cell = build(&p);
        let rw = p.ring_width;
        let strips = frame_strips(p.die_width, p.die_height, rw);
        for step in p.stack.steps() {
            let enc = i64::from(step.enclosure);
            for s in cell.shapes.iter().filter(|s| s.layer == step.cut.layer) {
                let ShapeKind::Rect(cut) = s.kind else {
                    panic!("cut is a rect")
                };
                // Some strip must contain the cut with at least the step enclosure.
                let ok = strips.iter().any(|strip| {
                    i64::from(strip.min.x) <= i64::from(cut.min.x) - enc
                        && i64::from(strip.min.y) <= i64::from(cut.min.y) - enc
                        && i64::from(cut.max.x) + enc <= i64::from(strip.max.x)
                        && i64::from(cut.max.y) + enc <= i64::from(strip.max.y)
                });
                assert!(ok, "cut {cut:?} enclosed by a strip with margin {enc}");
            }
        }
    }
}
