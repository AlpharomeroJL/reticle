//! Grid spacing, snapping, and ruler tick selection, all pure DBU arithmetic.
//!
//! The canvas draws a background grid, rulers along the top and left edges, and
//! optionally snaps the cursor to grid intersections. Choosing a readable grid step
//! (one that is neither a dense blur nor a single line) and rounding a point onto it
//! are pure functions of the zoom and a base step, so they live here and are
//! unit-tested without any drawing.

use reticle_geometry::Point;

/// The grid configuration: the base step and whether snapping is enabled.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GridSettings {
    /// The smallest grid step, in DBU. The displayed step is this scaled up by a
    /// power of the subdivision base so grid lines never get denser than
    /// [`GridSettings::min_pixels`].
    pub base_step_dbu: i32,
    /// Whether the cursor snaps to grid intersections.
    pub snap_enabled: bool,
    /// Whether the grid is drawn at all.
    pub visible: bool,
}

impl Default for GridSettings {
    fn default() -> Self {
        Self {
            base_step_dbu: 100,
            snap_enabled: true,
            visible: true,
        }
    }
}

impl GridSettings {
    /// The minimum on-screen spacing, in pixels, between adjacent grid lines.
    ///
    /// [`GridSettings::display_step_dbu`] multiplies the base step until the spacing
    /// is at least this many pixels, keeping the grid legible at any zoom.
    pub const fn min_pixels() -> f64 {
        8.0
    }

    /// The step actually drawn/snapped at zoom `pixels_per_dbu`, in DBU.
    ///
    /// Starts from [`GridSettings::base_step_dbu`] and multiplies by 10 (falling
    /// back to 2× and 5× intermediates) until adjacent lines are at least
    /// [`GridSettings::min_pixels`] apart, so the grid coarsens smoothly as the user
    /// zooms out. Returns at least the base step.
    #[must_use]
    pub fn display_step_dbu(&self, pixels_per_dbu: f64) -> i64 {
        let base = i64::from(self.base_step_dbu.max(1));
        if pixels_per_dbu <= 0.0 {
            return base;
        }
        let min_px = Self::min_pixels();
        // Multipliers within each decade: 1, 2, 5, then ×10 and repeat.
        let mults = [1_i64, 2, 5];
        let mut decade = 1_i64;
        // Cap iterations so a pathological zoom cannot loop forever.
        for _ in 0..12 {
            for &m in &mults {
                let step = base.saturating_mul(decade).saturating_mul(m);
                if step as f64 * pixels_per_dbu >= min_px {
                    return step;
                }
            }
            decade = decade.saturating_mul(10);
        }
        base.saturating_mul(decade)
    }

    /// Snaps `p` to the nearest grid intersection if snapping is enabled.
    ///
    /// The grid used is the base step (the finest grid), so snapping is precise
    /// regardless of the coarser display step. When snapping is disabled `p` is
    /// returned unchanged.
    #[must_use]
    pub fn snap(&self, p: Point) -> Point {
        if !self.snap_enabled {
            return p;
        }
        let step = self.base_step_dbu.max(1);
        Point::new(snap_scalar(p.x, step), snap_scalar(p.y, step))
    }
}

/// Rounds `v` to the nearest multiple of `step` (round half away from zero),
/// saturating into the coordinate range.
fn snap_scalar(v: i32, step: i32) -> i32 {
    let step = i64::from(step.max(1));
    let v = i64::from(v);
    let half = step / 2;
    let snapped = if v >= 0 {
        ((v + half) / step) * step
    } else {
        ((v - half) / step) * step
    };
    snapped.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// The grid-line world coordinates along one axis within `[min, max]` DBU at the
/// given `step`, as an iterator of DBU positions.
///
/// The first line is the smallest multiple of `step` that is `>= min`; lines
/// continue up to and including `max`. Used to draw both the background grid and
/// ruler ticks. Returns an empty vector for a non-positive step or inverted range.
#[must_use]
pub fn grid_lines(min: i32, max: i32, step: i64) -> Vec<i32> {
    if step <= 0 || max < min {
        return Vec::new();
    }
    let min = i64::from(min);
    let max = i64::from(max);
    // Round `min` up to the next multiple of `step`.
    let first = (min.div_euclid(step)) * step;
    let first = if first < min { first + step } else { first };
    let mut out = Vec::new();
    let mut x = first;
    // Guard against an unbounded loop if the range is astronomically wide relative
    // to the step; the canvas never asks for more than a screenful of lines.
    let mut guard = 0;
    while x <= max && guard < 100_000 {
        out.push(x as i32);
        x += step;
        guard += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_rounds_to_nearest_grid() {
        let g = GridSettings {
            base_step_dbu: 100,
            snap_enabled: true,
            visible: true,
        };
        assert_eq!(g.snap(Point::new(140, 160)), Point::new(100, 200));
        assert_eq!(g.snap(Point::new(149, 151)), Point::new(100, 200));
        assert_eq!(g.snap(Point::new(50, 50)), Point::new(100, 100));
        assert_eq!(g.snap(Point::new(-140, -160)), Point::new(-100, -200));
    }

    #[test]
    fn snap_disabled_is_identity() {
        let g = GridSettings {
            base_step_dbu: 100,
            snap_enabled: false,
            visible: true,
        };
        assert_eq!(g.snap(Point::new(137, 942)), Point::new(137, 942));
    }

    #[test]
    fn display_step_coarsens_as_zoom_shrinks() {
        let g = GridSettings::default();
        let tight = g.display_step_dbu(10.0);
        let loose = g.display_step_dbu(0.001);
        assert!(loose > tight, "loose {loose} should exceed tight {tight}");
        // At every zoom the drawn spacing is at least the minimum pixel spacing.
        for &ppd in &[100.0, 10.0, 1.0, 0.1, 0.01, 0.0005] {
            let step = g.display_step_dbu(ppd) as f64;
            assert!(step * ppd >= GridSettings::min_pixels() - 1e-6);
        }
    }

    #[test]
    fn display_step_never_below_base() {
        let g = GridSettings::default();
        // Extremely high zoom: base step already exceeds min pixels.
        assert!(g.display_step_dbu(1000.0) >= i64::from(g.base_step_dbu));
    }

    #[test]
    fn grid_lines_span_range_on_step() {
        let lines = grid_lines(-250, 250, 100);
        assert_eq!(lines, vec![-200, -100, 0, 100, 200]);
    }

    #[test]
    fn grid_lines_empty_for_bad_input() {
        assert!(grid_lines(0, 100, 0).is_empty());
        assert!(grid_lines(100, 0, 10).is_empty());
    }

    #[test]
    fn grid_lines_first_at_or_after_min() {
        let lines = grid_lines(105, 500, 100);
        assert_eq!(lines.first().copied(), Some(200));
        assert!(lines.iter().all(|&x| (105..=500).contains(&x)));
    }
}
