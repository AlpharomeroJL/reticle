//! The via-farm generator: an array of cuts between two conductor layers, covered
//! by enclosing plates.
//!
//! # What it emits
//!
//! A `rows` by `cols` array of square cuts on a contact/via layer, plus two
//! conductor plates (a lower and an upper layer) that each cover the whole cut
//! array with enclosure to spare. This is the standard way to carry current between
//! two layers over an area: many small cuts in parallel under shared metal.
//!
//! # Why it is DRC-clean by construction (SKY130 subset)
//!
//! * **Cut size.** Every cut is a square at the layer's exact drawn size, so the cut
//!   width rule is met exactly.
//! * **Cut spacing.** The subset carries no cut-to-cut spacing rule for the contact
//!   and via layers, so any non-overlapping pitch is clean; the generator pitches
//!   cuts at the cut size plus a safe margin, comfortably clear.
//! * **Enclosure.** Each plate is the bounding box of the whole array grown by the
//!   enclosure margin on every side, so every cut, including the corner cuts, is
//!   enclosed by at least the required margin. The subset's enclosure rule names one
//!   plate (for example `m1.4`: `mcon` enclosed by `met1`); the other plate is grown
//!   by the same margin, which only ever exceeds any rule it has.
//! * **Plate width.** A plate spans at least one cut plus enclosure on both sides,
//!   which already clears every conductor's minimum width.
//! * **Plate area.** Where a plate's layer has a minimum-area rule (`li1`, `met1`),
//!   the plate is grown symmetrically until its area meets the minimum, so even a
//!   1x1 farm's plates are large enough.

use reticle_geometry::{Point, Rect};
use reticle_model::{Cell, DrawShape, ShapeKind, Technology};
use serde::{Deserialize, Serialize};

use crate::error::GenError;
use crate::generator::{GenOutput, GenParams, Generator};
use crate::gentech::{Conductor, Cut, GenTech};
use crate::schema::{FieldSchema, ParamSchema};

/// The cut layer a via farm arrays, which fixes the two conductor layers it bridges.
///
/// Each choice bridges an adjacent pair in the SKY130 digital metal stack, so the
/// plates and the cut's enclosure are exactly the rules the subset carries for that
/// level.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CutKind {
    /// Local-interconnect contact `mcon` (67/44), bridging `li1` and `met1`.
    #[default]
    Mcon,
    /// Via 1 `via` (68/44), bridging `met1` and `met2`.
    Via,
    /// Via 2 `via2` (69/44), bridging `met2` and `met3`.
    Via2,
}

impl CutKind {
    /// The stack cut level this kind arrays: `mcon`/`via`/`via2` are the base-up cut
    /// levels 0/1/2, which bind to the process's own cut layers via [`GenTech`].
    fn level(self) -> usize {
        match self {
            Self::Mcon => 0,
            Self::Via => 1,
            Self::Via2 => 2,
        }
    }

    /// The cut, the lower plate layer, the upper plate layer, and the enclosure
    /// margin (DBU) the plates grow the cut array by, resolved against `gt`.
    ///
    /// The margin is the deck's enclosure value for the cut (grown on both plates);
    /// where the deck gives no enclosure rule, [`GenTech`] carries a conservative
    /// positive margin so the plates still fully cover the cuts.
    fn spec(self, gt: &GenTech) -> CutSpec {
        let level = self.level();
        let cut = gt.cut(level);
        let (_, enclosure) = cut.enclosure.expect("via-farm cut has an enclosure margin");
        CutSpec {
            cut,
            lower: gt.cut_lower(level),
            upper: gt.cut_upper(level),
            enclosure,
        }
    }

    /// The serde variant strings, for the schema's enum field.
    const VARIANTS: [&'static str; 3] = ["mcon", "via", "via2"];
}

/// The resolved geometry data behind a [`CutKind`].
#[derive(Clone, Copy, Debug)]
struct CutSpec {
    cut: Cut,
    lower: Conductor,
    upper: Conductor,
    enclosure: i32,
}

/// Parameters for the [`ViaFarm`] generator.
///
/// The farm is a `rows` by `cols` grid of cuts at a fixed pitch, covered by a lower
/// and an upper plate. The cut kind fixes which two layers are bridged and the cut
/// size; only the array shape is free.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ViaFarmParams {
    /// The cut layer to array, which fixes the two bridged conductor layers.
    pub cut: CutKind,
    /// Number of cut rows (y count). At least 1.
    pub rows: u32,
    /// Number of cut columns (x count). At least 1.
    pub cols: u32,
}

impl Default for ViaFarmParams {
    fn default() -> Self {
        // A small mcon farm: a clean, useful default a form can generate unchanged.
        Self {
            cut: CutKind::Mcon,
            rows: 3,
            cols: 3,
        }
    }
}

impl ViaFarmParams {
    /// Inclusive lower bound on each array dimension.
    const COUNT_MIN: i64 = 1;
    /// Inclusive upper bound on each array dimension: large enough for a real power
    /// via array, bounded so the emitted geometry and the coordinate range stay sane.
    const COUNT_MAX: i64 = 256;
}

impl GenParams for ViaFarmParams {
    fn schema() -> ParamSchema {
        ParamSchema {
            generator_id: String::new(),
            title: String::new(),
            description: String::new(),
            fields: vec![
                FieldSchema::enumerated(
                    "cut",
                    "Cut layer to array, which fixes the two bridged conductor layers.",
                    &CutKind::VARIANTS,
                    "mcon",
                ),
                FieldSchema::int(
                    "rows",
                    "Number of cut rows (y count).",
                    3,
                    Self::COUNT_MIN,
                    Self::COUNT_MAX,
                    "count",
                ),
                FieldSchema::int(
                    "cols",
                    "Number of cut columns (x count).",
                    3,
                    Self::COUNT_MIN,
                    Self::COUNT_MAX,
                    "count",
                ),
            ],
        }
    }

    fn validate(&self) -> Result<(), GenError> {
        check_range(
            "rows",
            i64::from(self.rows),
            Self::COUNT_MIN,
            Self::COUNT_MAX,
        )?;
        check_range(
            "cols",
            i64::from(self.cols),
            Self::COUNT_MIN,
            Self::COUNT_MAX,
        )?;
        Ok(())
    }
}

/// The via-farm generator.
///
/// Emits a cut array between two conductor layers with enclosing plates; see
/// [`ViaFarmParams`] for the parameters and the [crate overview](crate) for the
/// DRC-clean-by-construction argument.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ViaFarm;

impl Generator for ViaFarm {
    type Params = ViaFarmParams;

    fn id(&self) -> &'static str {
        "via_farm"
    }

    fn title(&self) -> &'static str {
        "Via farm"
    }

    fn description(&self) -> &'static str {
        "An array of cuts between two conductor layers, covered by enclosing lower \
         and upper plates. DRC-clean by construction against the SKY130 subset."
    }

    fn generate(
        &self,
        params: &Self::Params,
        tech: &Technology,
        cell: &mut Cell,
    ) -> Result<GenOutput, GenError> {
        let start = cell.shapes.len();
        let gt = GenTech::for_technology(tech);
        let spec = params.cut.spec(&gt);
        let size = spec.cut.size;
        let pitch = size + gt.safe_cut_margin();

        // Emit the cut grid with its lower-left cut at the origin. Track the array
        // bounding box for the plates.
        let mut array_bbox: Option<Rect> = None;
        for row in 0..params.rows {
            for col in 0..params.cols {
                let x = (col as i32) * pitch;
                let y = (row as i32) * pitch;
                let cut = Rect::new(Point::new(x, y), Point::new(x + size, y + size));
                cell.shapes
                    .push(DrawShape::new(spec.cut.layer, ShapeKind::Rect(cut)));
                array_bbox = Some(array_bbox.map_or(cut, |b| b.union(&cut)));
            }
        }
        // `rows`/`cols` are at least 1, so the array is non-empty.
        let array_bbox = array_bbox.expect("at least one cut");

        // Each plate covers the whole array grown by the enclosure margin, then bumped
        // up to the layer's minimum area where the subset has one.
        let base_plate = array_bbox.expanded(spec.enclosure);
        let lower_plate = grow_to_area(base_plate, spec.lower.min_area);
        let upper_plate = grow_to_area(base_plate, spec.upper.min_area);
        cell.shapes.push(DrawShape::new(
            spec.lower.layer,
            ShapeKind::Rect(lower_plate),
        ));
        cell.shapes.push(DrawShape::new(
            spec.upper.layer,
            ShapeKind::Rect(upper_plate),
        ));

        let added = cell.shapes.len() - start;
        let bbox = lower_plate.union(&upper_plate);
        Ok(GenOutput {
            shapes_added: added,
            bbox: Some(bbox),
        })
    }
}

/// Grows `rect` symmetrically until its area is at least `min_area`, if a minimum is
/// given; otherwise returns `rect` unchanged.
///
/// The grown rectangle still contains the original, so any enclosure the original
/// satisfied is preserved (growing a plate only ever adds margin). The growth is
/// per-axis and rounded up, so the result comfortably clears the area threshold.
fn grow_to_area(rect: Rect, min_area: Option<i64>) -> Rect {
    let Some(min_area) = min_area else {
        return rect;
    };
    if rect.area() >= min_area {
        return rect;
    }
    // Target a square of side `ceil(sqrt(min_area))`, never shrinking either axis.
    let side = isqrt_ceil(min_area);
    let cur_w = rect.width();
    let cur_h = rect.height();
    let target_w = cur_w.max(side);
    let target_h = cur_h.max(side);
    let grow_x = clamp_i32(target_w - cur_w);
    let grow_y = clamp_i32(target_h - cur_h);
    // Expand right/top so the lower-left stays put and the plate still starts at the
    // array origin corner.
    Rect::new(
        rect.min,
        Point::new(
            rect.max.x.saturating_add(grow_x),
            rect.max.y.saturating_add(grow_y),
        ),
    )
}

/// The smallest integer `s` with `s * s >= n` for `n >= 0`.
fn isqrt_ceil(n: i64) -> i64 {
    if n <= 0 {
        return 0;
    }
    let mut s = (n as f64).sqrt() as i64;
    // Correct any floating-point under/overshoot with a couple of integer steps.
    while s * s < n {
        s += 1;
    }
    while s > 0 && (s - 1) * (s - 1) >= n {
        s -= 1;
    }
    s
}

/// Clamps a non-negative widened delta into the DBU (`i32`) range.
fn clamp_i32(v: i64) -> i32 {
    v.clamp(0, i64::from(i32::MAX)) as i32
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

    fn build(params: &ViaFarmParams) -> Cell {
        let mut cell = Cell::new("top");
        ViaFarm
            .generate(params, &Technology::default(), &mut cell)
            .expect("valid params generate");
        cell
    }

    #[test]
    fn default_generates_grid_plus_two_plates() {
        let p = ViaFarmParams::default();
        p.validate().expect("default valid");
        let cell = build(&p);
        // 3x3 cuts + lower + upper plate.
        assert_eq!(cell.shapes.len(), 9 + 2);
        let cuts = cell
            .shapes
            .iter()
            .filter(|s| s.layer == sky130::MCON.layer)
            .count();
        assert_eq!(cuts, 9);
        assert!(cell.shapes.iter().any(|s| s.layer == sky130::LI1.layer));
        assert!(cell.shapes.iter().any(|s| s.layer == sky130::MET1.layer));
    }

    #[test]
    fn one_by_one_plate_meets_min_area() {
        // The trap case: a single mcon whose bare enclosure plate would be below the
        // met1 minimum area must be grown to clear it.
        let p = ViaFarmParams {
            cut: CutKind::Mcon,
            rows: 1,
            cols: 1,
        };
        let cell = build(&p);
        let met1 = cell
            .shapes
            .iter()
            .find(|s| s.layer == sky130::MET1.layer)
            .expect("upper met1 plate");
        let ShapeKind::Rect(r) = met1.kind else {
            panic!("plate is a rect")
        };
        assert!(
            r.area() >= sky130::MET1.min_area.unwrap(),
            "met1 plate area {} >= min {}",
            r.area(),
            sky130::MET1.min_area.unwrap()
        );
    }

    #[test]
    fn zero_rows_rejected() {
        let p = ViaFarmParams {
            rows: 0,
            ..ViaFarmParams::default()
        };
        assert!(matches!(
            p.validate(),
            Err(GenError::OutOfRange { field: "rows", .. })
        ));
    }

    #[test]
    fn isqrt_ceil_is_exact() {
        assert_eq!(isqrt_ceil(0), 0);
        assert_eq!(isqrt_ceil(1), 1);
        assert_eq!(isqrt_ceil(2), 2);
        assert_eq!(isqrt_ceil(4), 2);
        assert_eq!(isqrt_ceil(5), 3);
        assert_eq!(isqrt_ceil(83_000), 289); // 288^2 = 82944 < 83000 <= 289^2
    }
}
