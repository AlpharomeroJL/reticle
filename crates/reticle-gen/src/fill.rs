//! The density-aware fill generator: a regular grid of fill tiles over a region,
//! honoring keep-outs and a target coverage density.
//!
//! # What it emits
//!
//! A regular grid of square fill tiles on the chosen conductor layer, packed into a
//! `region_width` by `region_height` rectangle whose lower-left corner sits at the
//! origin. Each tile is a fixed square; the grid pitch is chosen so the covered
//! fraction approaches the requested `target_density`. A tile is emitted only when it
//! lies wholly inside the region and clears every keep-out rectangle, so fill never
//! lands under a blockage.
//!
//! # Honest density, not an exact knob
//!
//! Fill is a *coverage* generator: `target_density` sets the grid pitch, and the
//! achieved density is whatever the resulting tile grid actually covers once
//! edge-clipped tiles and keep-outs are accounted for. The generator never claims it
//! hit the target exactly, it *approaches* it and reports what it actually got:
//!
//! * The pitch is `round(tile / sqrt(target))`, clamped up to at least
//!   `tile + min_spacing` so two tiles are never closer than the layer minimum
//!   spacing. That clamp caps the achievable density at
//!   `tile² / (tile + min_spacing)²`; asking for more yields that ceiling, not the
//!   impossible target.
//! * The pitch is a whole number of DBU and the region is not generally an exact
//!   multiple of it, so the count of whole tiles that fit does not divide the target
//!   evenly: the achieved density can land a little **above or below** the target
//!   (a small region packs a slightly denser or sparser grid than the ideal ratio).
//!   It is never claimed to be exact.
//! * Only whole tiles are placed (partial tiles at the top/right edge are dropped),
//!   and every tile overlapping a keep-out is dropped, each lowering the achieved
//!   density relative to an unobstructed infinite grid.
//!
//! [`FillGen::achieved_density_permille`] computes the true achieved density (placed
//! tile area over region area, in per-mille) from the same parameters, so a caller or
//! a test can read what fill actually covered rather than trusting a claim. The
//! cleanliness property test asserts the achieved density stays within a tolerance
//! band around the target (capped by the min-spacing ceiling) across the valid
//! parameter space, and that keep-outs only ever reduce it.
//!
//! # Why it is DRC-clean by construction (SKY130 subset)
//!
//! Every tile is an axis-aligned [`Rect`], which the bounding-box DRC engine checks
//! exactly, and the grid is built so each rule the subset carries for the fill layer
//! is met:
//!
//! * **Width.** Each tile is a `tile` DBU square and `tile` is validated to be at
//!   least the layer's minimum width, so no tile is too thin.
//! * **Spacing.** Tiles sit on a pitch of at least `tile + min_spacing`, so the gap
//!   between any two adjacent tiles is at least the layer's minimum spacing; no two
//!   tiles touch or come sub-spacing close.
//! * **Area.** `tile` is validated to be at least the side that makes a square meet
//!   the layer's minimum area (where the subset carries one), so every tile's area
//!   clears the minimum.
//!
//! The subset carries **no maximum-density rule** for any layer (it is a
//! min-width/spacing/area/enclosure subset), so `target_density` is a fill objective,
//! not a rule the fill must respect; there is nothing to over-fill against. This is
//! stated plainly rather than pretending a density rule bounds the output.

use reticle_geometry::{Point, Rect};
use reticle_model::{Cell, DrawShape, ShapeKind, Technology};
use serde::{Deserialize, Serialize};

use crate::error::GenError;
use crate::generator::{GenOutput, GenParams, Generator};
use crate::schema::{FieldSchema, ParamSchema};
use crate::sky130::{self, Conductor};

/// The conductor layer fill tiles are drawn on.
///
/// Restricted to the interconnect layers the SKY130 subset carries width, spacing,
/// and (where present) area rules for, so a tile grid on any of them is checkable and
/// clean.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillLayer {
    /// Local interconnect `li1` (67/20): width 170, spacing 170, area 56100.
    #[default]
    Li1,
    /// Metal 1 `met1` (68/20): width 140, spacing 140, area 83000.
    Met1,
    /// Metal 2 `met2` (69/20): width 140, spacing 140, no area rule.
    Met2,
    /// Metal 3 `met3` (70/20): width 300, spacing 300, no area rule.
    Met3,
}

impl FillLayer {
    /// The SKY130 subset data (layer, width, spacing, area) for this choice.
    fn conductor(self) -> Conductor {
        match self {
            Self::Li1 => sky130::LI1,
            Self::Met1 => sky130::MET1,
            Self::Met2 => sky130::MET2,
            Self::Met3 => sky130::MET3,
        }
    }

    /// The serde variant strings, for the schema's enum field.
    const VARIANTS: [&'static str; 4] = ["li1", "met1", "met2", "met3"];
}

/// One keep-out rectangle: an axis-aligned region, in DBU relative to the fill
/// region's origin, that no fill tile may overlap.
///
/// A tile is dropped when it intersects (positive-area) or merely touches a keep-out,
/// so fill leaves a clean margin around every blockage rather than abutting it. Given
/// as a lower-left corner plus a size so a form or a model supplies four plain
/// integers per keep-out.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeepOut {
    /// Lower-left x of the keep-out, in DBU relative to the region origin.
    pub x: i32,
    /// Lower-left y of the keep-out, in DBU relative to the region origin.
    pub y: i32,
    /// Width of the keep-out, in DBU. Must be positive.
    pub width: i32,
    /// Height of the keep-out, in DBU. Must be positive.
    pub height: i32,
}

impl KeepOut {
    /// The keep-out as a [`Rect`], grown by `margin` DBU on every side so a tile that
    /// merely touches the raw keep-out is still treated as blocked.
    fn blocked_rect(self, margin: i32) -> Rect {
        Rect::new(
            Point::new(self.x, self.y),
            Point::new(self.x + self.width, self.y + self.height),
        )
        .expanded(margin)
    }
}

/// Parameters for the [`FillGen`] generator. All lengths are in DBU (1 dbu = 1 nm).
///
/// The fill covers a `region_width` by `region_height` rectangle at the origin with a
/// grid of `tile`-sized squares whose pitch targets `target_density_permille`, never
/// placing a tile inside a [keep-out](KeepOut). See each field for its range and
/// default. The keep-out list has no fixed-width form widget, so it is not in the
/// [schema](GenParams::schema) fields (which describe the scalar inputs); it round-
/// trips through the JSON parameter path and defaults to empty.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FillParams {
    /// The conductor layer the fill tiles are drawn on.
    pub layer: FillLayer,
    /// Width of the fill region, in DBU. Must hold at least one tile plus its
    /// spacing.
    pub region_width: i32,
    /// Height of the fill region, in DBU. Must hold at least one tile plus its
    /// spacing.
    pub region_height: i32,
    /// Side length of each square fill tile, in DBU. Must be at least the layer's
    /// minimum width and, where the layer has an area rule, large enough for a square
    /// to meet it.
    pub tile: i32,
    /// Target coverage density, in per-mille (‰) of the region area. The grid pitch
    /// is chosen to approach it; the achieved density is reported honestly by
    /// [`FillGen::achieved_density_permille`] and may fall short (edge clipping,
    /// keep-outs) or be capped (the min-spacing pitch ceiling).
    pub target_density_permille: i32,
    /// Rectangles no fill tile may overlap, in DBU relative to the region origin.
    pub keepouts: Vec<KeepOut>,
}

impl Default for FillParams {
    fn default() -> Self {
        // A mid-coverage li1 fill over a 10x10 um region with a single central
        // keep-out: a working, non-trivial example a form can generate unchanged.
        Self {
            layer: FillLayer::Li1,
            region_width: 10_000,
            region_height: 10_000,
            tile: 400,
            target_density_permille: 400,
            keepouts: vec![KeepOut {
                x: 4_000,
                y: 4_000,
                width: 2_000,
                height: 2_000,
            }],
        }
    }
}

impl FillParams {
    /// Inclusive lower bound for the region across every layer: enough for one tile
    /// plus a min-spacing pitch on the coarsest layer (`met3`: tile floor 300 plus
    /// spacing 300).
    const REGION_MIN: i64 = 600;
    /// Inclusive upper bound for region lengths, well within the DBU coordinate range
    /// while allowing a large fill area (about 1 mm).
    const REGION_MAX: i64 = 1_000_000;
    /// Inclusive lower bound for `tile` across every layer: the largest layer minimum
    /// width in the subset (`met3`, 300). Per-layer `validate` still enforces the
    /// exact width and area floors.
    const TILE_MIN: i64 = 300;
    /// Inclusive upper bound for `tile`: a coarse fill cell, bounded so a tile always
    /// fits several times over inside the largest region.
    const TILE_MAX: i64 = 100_000;
    /// Inclusive density bounds, in per-mille. At least 1 (some coverage) and at most
    /// 900 (a dense fill); the actual ceiling is the min-spacing pitch, computed per
    /// tile.
    const DENSITY_MIN: i64 = 1;
    /// Inclusive upper density bound in per-mille.
    const DENSITY_MAX: i64 = 900;

    /// The smallest `tile` whose square meets the layer's minimum area, or the
    /// layer's minimum width when it carries no area rule: the true per-layer floor
    /// for `tile`.
    fn tile_floor(cond: Conductor) -> i32 {
        let width_floor = cond.min_width;
        let area_floor = cond.min_area.map_or(0, isqrt_ceil_i32);
        width_floor.max(area_floor)
    }
}

impl GenParams for FillParams {
    fn schema() -> ParamSchema {
        // Placeholders for id/title/description; `Generator::schema` stamps the real
        // values. The keepouts list has no scalar-field widget and is intentionally
        // absent from the schema fields (see the struct docs); it is still accepted on
        // the JSON parameter path.
        ParamSchema {
            generator_id: String::new(),
            title: String::new(),
            description: String::new(),
            fields: vec![
                FieldSchema::enumerated(
                    "layer",
                    "Conductor layer the fill tiles are drawn on.",
                    &FillLayer::VARIANTS,
                    "li1",
                ),
                FieldSchema::int(
                    "region_width",
                    "Width of the fill region; at least one tile plus its spacing.",
                    10_000,
                    Self::REGION_MIN,
                    Self::REGION_MAX,
                    "dbu",
                ),
                FieldSchema::int(
                    "region_height",
                    "Height of the fill region; at least one tile plus its spacing.",
                    10_000,
                    Self::REGION_MIN,
                    Self::REGION_MAX,
                    "dbu",
                ),
                FieldSchema::int(
                    "tile",
                    "Side of each square fill tile; at least the layer width/area floor.",
                    400,
                    Self::TILE_MIN,
                    Self::TILE_MAX,
                    "dbu",
                ),
                FieldSchema::int(
                    "target_density_permille",
                    "Target coverage in per-mille; the achieved value is reported honestly.",
                    400,
                    Self::DENSITY_MIN,
                    Self::DENSITY_MAX,
                    "permille",
                ),
            ],
        }
    }

    fn validate(&self) -> Result<(), GenError> {
        let cond = self.layer.conductor();
        let tile_floor = Self::tile_floor(cond);

        check_range("tile", self.tile, i64::from(tile_floor), Self::TILE_MAX)?;
        check_range(
            "target_density_permille",
            self.target_density_permille,
            Self::DENSITY_MIN,
            Self::DENSITY_MAX,
        )?;

        // A region must hold at least one whole tile plus the min-spacing pitch, so
        // the grid is never empty and the first tile clears the far edge cleanly.
        let region_floor = i64::from(self.tile) + i64::from(cond.min_spacing);
        check_range(
            "region_width",
            self.region_width,
            region_floor,
            Self::REGION_MAX,
        )?;
        check_range(
            "region_height",
            self.region_height,
            region_floor,
            Self::REGION_MAX,
        )?;

        for (i, k) in self.keepouts.iter().enumerate() {
            if k.width <= 0 || k.height <= 0 {
                // The offending field is the keep-out list; the reason names which one.
                return Err(GenError::Invalid {
                    field: "keepouts",
                    reason: keepout_reason(i),
                });
            }
        }

        Ok(())
    }
}

/// The density-aware fill generator.
///
/// Emits a regular grid of DRC-clean fill tiles over a region, honoring keep-outs and
/// approaching a target coverage density; see [`FillParams`] for the parameters,
/// [`FillGen::achieved_density_permille`] for the honest achieved coverage, and the
/// [crate overview](crate) for the DRC-clean-by-construction argument.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FillGen;

impl FillGen {
    /// The grid pitch (DBU) the given tile and target density resolve to: the ideal
    /// `tile / sqrt(target)` rounded to a whole DBU, clamped up to `tile + min_spacing`
    /// so tiles never come sub-spacing close.
    ///
    /// This is the single source of the pitch, shared by [`generate`](Generator::generate)
    /// and [`achieved_density_permille`](Self::achieved_density_permille), so the
    /// reported density is derived from the same grid that is drawn.
    fn pitch(params: &FillParams) -> i32 {
        let cond = params.layer.conductor();
        let min_pitch = params.tile.saturating_add(cond.min_spacing);
        // p = tile / sqrt(density_fraction) = tile * sqrt(1000 / permille).
        let ratio = 1000.0_f64 / f64::from(params.target_density_permille);
        let ideal = f64::from(params.tile) * ratio.sqrt();
        // Round to the nearest DBU, then never allow a pitch below the min-spacing
        // floor (which would over-fill past the spacing rule).
        let rounded = ideal.round();
        let rounded = if rounded.is_finite() {
            rounded.clamp(0.0, f64::from(i32::MAX)) as i32
        } else {
            min_pitch
        };
        rounded.max(min_pitch)
    }

    /// The x/y positions (lower-left corners) of every whole tile that fits inside a
    /// `region` of the given extent at the given pitch, before keep-out filtering.
    ///
    /// A tile at `k * pitch` is kept only if its far edge `k * pitch + tile` is within
    /// the region, so partial edge tiles are never placed.
    fn tile_origins(region_extent: i32, tile: i32, pitch: i32) -> Vec<i32> {
        let mut xs = Vec::new();
        let mut x: i64 = 0;
        while x + i64::from(tile) <= i64::from(region_extent) {
            xs.push(x as i32);
            x += i64::from(pitch);
        }
        xs
    }

    /// The number of whole tiles this generator will place for `params`, after edge
    /// clipping and keep-out filtering. Pure and side-effect free; the honest density
    /// and the drawn geometry both derive from it.
    fn placed_tile_count(params: &FillParams) -> u64 {
        let pitch = Self::pitch(params);
        let xs = Self::tile_origins(params.region_width, params.tile, pitch);
        let ys = Self::tile_origins(params.region_height, params.tile, pitch);
        // Block any tile that touches a keep-out: expand each keep-out by the tile
        // size less one so a tile whose lower-left is within reach is caught, then
        // test the actual tile rectangle for intersection/touch.
        let blocks: Vec<Rect> = params.keepouts.iter().map(|k| k.blocked_rect(0)).collect();
        let mut count: u64 = 0;
        for &ty in &ys {
            for &tx in &xs {
                let tile_rect = Rect::new(
                    Point::new(tx, ty),
                    Point::new(tx + params.tile, ty + params.tile),
                );
                if blocks.iter().any(|b| touches_or_overlaps(&tile_rect, b)) {
                    continue;
                }
                count += 1;
            }
        }
        count
    }

    /// The honestly achieved coverage density for `params`, in per-mille (‰) of the
    /// region area: the total area of the tiles actually placed (whole tiles only,
    /// keep-outs excluded) divided by the region's area.
    ///
    /// This is the number to compare against `target_density_permille`: it reflects
    /// the real drawn geometry, so it accounts for the min-spacing pitch ceiling, the
    /// dropped partial edge tiles, and every keep-out. It approaches the target but is
    /// not exact: whole-DBU pitch rounding over a finite region can put it a little
    /// above or below the target, and edge clipping or keep-outs pull it further down.
    #[must_use]
    pub fn achieved_density_permille(params: &FillParams) -> i64 {
        let placed = Self::placed_tile_count(params);
        let tile_area = i64::from(params.tile) * i64::from(params.tile);
        let covered = (placed as i64).saturating_mul(tile_area);
        let region_area = i64::from(params.region_width) * i64::from(params.region_height);
        if region_area <= 0 {
            return 0;
        }
        (covered.saturating_mul(1000)) / region_area
    }
}

impl Generator for FillGen {
    type Params = FillParams;

    fn id(&self) -> &'static str {
        "fill"
    }

    fn title(&self) -> &'static str {
        "Density fill"
    }

    fn description(&self) -> &'static str {
        "A regular grid of fill tiles over a region, honoring keep-out rectangles and \
         approaching a target coverage density. DRC-clean by construction against the \
         SKY130 subset; the achieved density is reported honestly and may fall short \
         of the target."
    }

    fn generate(
        &self,
        params: &Self::Params,
        _tech: &Technology,
        cell: &mut Cell,
    ) -> Result<GenOutput, GenError> {
        let start = cell.shapes.len();
        let layer = params.layer.conductor().layer;
        let pitch = Self::pitch(params);

        let xs = Self::tile_origins(params.region_width, params.tile, pitch);
        let ys = Self::tile_origins(params.region_height, params.tile, pitch);
        let blocks: Vec<Rect> = params.keepouts.iter().map(|k| k.blocked_rect(0)).collect();

        let mut bbox: Option<Rect> = None;
        for &ty in &ys {
            for &tx in &xs {
                let tile_rect = Rect::new(
                    Point::new(tx, ty),
                    Point::new(tx + params.tile, ty + params.tile),
                );
                if blocks.iter().any(|b| touches_or_overlaps(&tile_rect, b)) {
                    continue;
                }
                cell.shapes
                    .push(DrawShape::new(layer, ShapeKind::Rect(tile_rect)));
                bbox = Some(bbox.map_or(tile_rect, |b| b.union(&tile_rect)));
            }
        }

        let added = cell.shapes.len() - start;
        Ok(GenOutput {
            shapes_added: added,
            bbox,
        })
    }
}

/// Whether two rectangles overlap or merely touch (share any boundary point). Used to
/// block a fill tile that abuts a keep-out, not just one that overlaps it, so fill
/// keeps clear of every blockage.
fn touches_or_overlaps(a: &Rect, b: &Rect) -> bool {
    a.min.x <= b.max.x && b.min.x <= a.max.x && a.min.y <= b.max.y && b.min.y <= a.max.y
}

/// The smallest integer `s` with `s * s >= n` for `n >= 0`, clamped into `i32`.
///
/// Used to turn a layer minimum area into the smallest square-tile side that meets it.
fn isqrt_ceil_i32(n: i64) -> i32 {
    if n <= 0 {
        return 0;
    }
    let mut s = (n as f64).sqrt() as i64;
    while s * s < n {
        s += 1;
    }
    while s > 0 && (s - 1) * (s - 1) >= n {
        s -= 1;
    }
    s.clamp(0, i64::from(i32::MAX)) as i32
}

/// A stable human-readable reason for a bad keep-out at `index`, naming which entry is
/// degenerate. Returns one of a fixed set of `&'static str`s so the message needs no
/// allocation and fits [`GenError::Invalid`].
fn keepout_reason(index: usize) -> &'static str {
    // A handful of pre-rendered messages cover the common small lists; anything beyond
    // falls back to a generic message. This keeps the reason `&'static str` without an
    // allocating error variant.
    const REASONS: [&str; 8] = [
        "keep-out 0 has a non-positive width or height",
        "keep-out 1 has a non-positive width or height",
        "keep-out 2 has a non-positive width or height",
        "keep-out 3 has a non-positive width or height",
        "keep-out 4 has a non-positive width or height",
        "keep-out 5 has a non-positive width or height",
        "keep-out 6 has a non-positive width or height",
        "keep-out 7 has a non-positive width or height",
    ];
    REASONS
        .get(index)
        .copied()
        .unwrap_or("a keep-out has a non-positive width or height")
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

    fn build(params: &FillParams) -> Cell {
        let mut cell = Cell::new("top");
        FillGen
            .generate(params, &Technology::default(), &mut cell)
            .expect("valid params generate");
        cell
    }

    #[test]
    fn default_params_validate_and_generate() {
        let p = FillParams::default();
        p.validate().expect("default is valid");
        let cell = build(&p);
        assert!(!cell.shapes.is_empty(), "fill places tiles");
        assert!(
            cell.shapes.iter().all(|s| s.layer == sky130::LI1.layer),
            "all tiles on the fill layer"
        );
    }

    #[test]
    fn no_tile_lands_in_a_keepout() {
        let p = FillParams::default();
        let cell = build(&p);
        let block = p.keepouts[0].blocked_rect(0);
        for s in &cell.shapes {
            let ShapeKind::Rect(r) = s.kind else {
                panic!("fill tiles are rects")
            };
            assert!(
                !touches_or_overlaps(&r, &block),
                "tile {r:?} overlaps the keep-out {block:?}"
            );
        }
    }

    #[test]
    fn achieved_density_tracks_target() {
        // Over a large unobstructed region the achieved density lands close to the
        // target (within a small band for whole-DBU pitch rounding and edge tiles).
        let p = FillParams {
            region_width: 20_000,
            region_height: 20_000,
            keepouts: Vec::new(),
            target_density_permille: 500,
            ..FillParams::default()
        };
        let achieved = FillGen::achieved_density_permille(&p);
        let target = i64::from(p.target_density_permille);
        assert!(achieved > 0, "some fill was placed");
        assert!(
            (achieved - target).abs() <= target / 8 + 20,
            "achieved {achieved} not within tolerance of target {target}"
        );
    }

    #[test]
    fn keepout_lowers_density() {
        let base = FillParams {
            keepouts: Vec::new(),
            ..FillParams::default()
        };
        let blocked = FillParams::default(); // has one central keep-out
        assert!(
            FillGen::achieved_density_permille(&blocked)
                < FillGen::achieved_density_permille(&base),
            "the keep-out must remove coverage"
        );
    }

    #[test]
    fn tile_below_area_floor_rejected() {
        // On li1 the tile floor is the area-derived side ceil(sqrt(56100)) = 237, above
        // the width floor 170. A tile of 200 clears the width floor but not the area
        // floor, so `validate` must reject it naming `tile`.
        let p = FillParams {
            layer: FillLayer::Li1,
            tile: 200,
            ..FillParams::default()
        };
        assert!(matches!(
            p.validate(),
            Err(GenError::OutOfRange { field: "tile", .. })
        ));
    }

    #[test]
    fn region_too_small_rejected() {
        let p = FillParams {
            region_width: 500, // below tile(400)+spacing(170) on li1
            ..FillParams::default()
        };
        assert!(matches!(
            p.validate(),
            Err(GenError::OutOfRange {
                field: "region_width",
                ..
            })
        ));
    }

    #[test]
    fn degenerate_keepout_rejected() {
        let p = FillParams {
            keepouts: vec![KeepOut {
                x: 0,
                y: 0,
                width: 0,
                height: 100,
            }],
            ..FillParams::default()
        };
        assert!(matches!(
            p.validate(),
            Err(GenError::Invalid {
                field: "keepouts",
                ..
            })
        ));
    }

    #[test]
    fn isqrt_ceil_is_exact() {
        assert_eq!(isqrt_ceil_i32(0), 0);
        assert_eq!(isqrt_ceil_i32(1), 1);
        assert_eq!(isqrt_ceil_i32(56_100), 237); // 236^2 = 55696 < 56100 <= 237^2
        assert_eq!(isqrt_ceil_i32(83_000), 289);
    }

    #[test]
    fn pitch_is_capped_at_min_spacing() {
        // A target of 900 permille on li1 would want a pitch near the tile size, but
        // the min-spacing floor caps it at tile + spacing.
        let p = FillParams {
            target_density_permille: 900,
            ..FillParams::default()
        };
        let pitch = FillGen::pitch(&p);
        assert!(
            pitch >= p.tile + sky130::LI1.min_spacing,
            "pitch {pitch} below the min-spacing floor"
        );
    }
}
