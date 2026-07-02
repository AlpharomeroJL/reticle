//! The minimap: a small overview panel with the current viewport rectangle.
//!
//! The minimap shows the whole document's bounds in a fixed-size panel anchored to
//! the canvas's top-right corner, draws the camera's visible world rectangle over
//! it, and recenters the camera when the user clicks or drags inside the panel.
//!
//! This module owns all the coordinate math: anchoring the panel inside the canvas,
//! the aspect-preserving world-to-panel transform (with the same `+y` flip as the
//! main camera), the clamped viewport rectangle, and the inverse mapping a click
//! uses to pick a new camera center. It is egui-free so every mapping is
//! unit-tested; the app module only paints the computed rectangles and feeds
//! pointer positions back in.

use crate::camera::ScreenRect;
use reticle_geometry::{Point, Rect};

/// The minimap panel width, in screen pixels.
pub const PANEL_WIDTH: f32 = 180.0;
/// The minimap panel height, in screen pixels.
pub const PANEL_HEIGHT: f32 = 135.0;
/// The gap between the panel and the canvas edges, in screen pixels.
const MARGIN: f32 = 12.0;
/// Inner padding between the panel border and the drawn world, in screen pixels.
const PADDING: f32 = 6.0;

/// The minimap's per-frame geometry: panel placement plus the world transform.
///
/// Built by [`MinimapLayout::compute`] from the canvas rectangle and the document
/// bounds; all queries are pure functions of the stored fields.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MinimapLayout {
    /// The panel rectangle, in canvas screen pixels.
    pub panel: ScreenRect,
    /// The world bounds the panel displays.
    world: Rect,
    /// Panel pixels per DBU (identical on both axes; aspect is preserved).
    scale: f64,
    /// Panel x of `world.min.x` (the content is centered, so this includes padding).
    origin_x: f32,
    /// Panel y of `world.max.y` (world `+y` is up, panel `+y` is down).
    origin_y: f32,
}

impl MinimapLayout {
    /// Computes the minimap geometry for a canvas and the document bounds.
    ///
    /// Returns `None` when there is nothing sensible to show: degenerate world
    /// bounds (zero width or height) or a canvas too small to host the panel plus
    /// its margins. The panel anchors to the canvas's top-right corner and the
    /// world is fit inside it with equal scale on both axes, centered.
    #[must_use]
    pub fn compute(canvas: &ScreenRect, world: Rect) -> Option<Self> {
        if world.width() <= 0 || world.height() <= 0 {
            return None;
        }
        let left = canvas.left + canvas.width - MARGIN - PANEL_WIDTH;
        let top = canvas.top + MARGIN;
        if left < canvas.left + MARGIN || canvas.height < PANEL_HEIGHT + 2.0 * MARGIN {
            return None;
        }
        let panel = ScreenRect::new(left, top, PANEL_WIDTH, PANEL_HEIGHT);
        let avail_w = f64::from(PANEL_WIDTH - 2.0 * PADDING);
        let avail_h = f64::from(PANEL_HEIGHT - 2.0 * PADDING);
        let scale = (avail_w / world.width() as f64).min(avail_h / world.height() as f64);
        if !(scale.is_finite() && scale > 0.0) {
            return None;
        }
        let content_w = world.width() as f64 * scale;
        let content_h = world.height() as f64 * scale;
        let origin_x = f64::from(panel.center_x()) - content_w / 2.0;
        let origin_y = f64::from(panel.center_y()) - content_h / 2.0;
        Some(Self {
            panel,
            world,
            scale,
            origin_x: origin_x as f32,
            origin_y: origin_y as f32,
        })
    }

    /// Whether a screen position lies inside the minimap panel.
    #[must_use]
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.panel.left
            && x <= self.panel.left + self.panel.width
            && y >= self.panel.top
            && y <= self.panel.top + self.panel.height
    }

    /// Maps a world point to panel pixels (world `+y` up maps to panel `+y` down).
    #[must_use]
    pub fn world_to_panel(&self, p: Point) -> (f32, f32) {
        let x =
            f64::from(self.origin_x) + (f64::from(p.x) - f64::from(self.world.min.x)) * self.scale;
        let y =
            f64::from(self.origin_y) + (f64::from(self.world.max.y) - f64::from(p.y)) * self.scale;
        (x as f32, y as f32)
    }

    /// Maps a panel pixel back to a world point, clamped into the world bounds.
    ///
    /// The clamp means a click in the panel's padding recenters to the nearest
    /// content edge instead of jumping outside the document.
    #[must_use]
    pub fn panel_to_world(&self, x: f32, y: f32) -> Point {
        let wx =
            f64::from(self.world.min.x) + (f64::from(x) - f64::from(self.origin_x)) / self.scale;
        let wy =
            f64::from(self.world.max.y) - (f64::from(y) - f64::from(self.origin_y)) / self.scale;
        Point::new(
            round_clamped(wx, self.world.min.x, self.world.max.x),
            round_clamped(wy, self.world.min.y, self.world.max.y),
        )
    }

    /// Maps a world rectangle to a panel-space `(left, top, width, height)`,
    /// clamped to the panel so a viewport larger than the document cannot spill
    /// past the border.
    #[must_use]
    pub fn world_rect_to_panel(&self, r: Rect) -> (f32, f32, f32, f32) {
        // World min is the bottom-left corner on screen, world max the top-right.
        let (x0, y1) = self.world_to_panel(r.min);
        let (x1, y0) = self.world_to_panel(r.max);
        let left = x0.max(self.panel.left);
        let top = y0.max(self.panel.top);
        let right = x1.min(self.panel.left + self.panel.width);
        let bottom = y1.min(self.panel.top + self.panel.height);
        (left, top, (right - left).max(0.0), (bottom - top).max(0.0))
    }
}

/// Rounds a panel-derived world coordinate to DBU, clamped into `[lo, hi]`.
fn round_clamped(v: f64, lo: i32, hi: i32) -> i32 {
    let r = v.round().clamp(f64::from(lo), f64::from(hi));
    r as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas() -> ScreenRect {
        ScreenRect::new(100.0, 50.0, 800.0, 600.0)
    }

    fn world() -> Rect {
        Rect::new(Point::new(0, 0), Point::new(10_000, 5_000))
    }

    #[test]
    fn panel_anchors_to_top_right_inside_canvas() {
        let m = MinimapLayout::compute(&canvas(), world()).expect("fits");
        let c = canvas();
        assert!((m.panel.left + m.panel.width) < c.left + c.width);
        assert!(m.panel.top > c.top);
        assert!(m.panel.left > c.left);
        assert!((m.panel.width - PANEL_WIDTH).abs() < f32::EPSILON);
        assert!((m.panel.height - PANEL_HEIGHT).abs() < f32::EPSILON);
    }

    #[test]
    fn degenerate_world_or_tiny_canvas_yields_none() {
        let flat = Rect::new(Point::new(0, 0), Point::new(1000, 0));
        assert!(MinimapLayout::compute(&canvas(), flat).is_none());
        let tiny = ScreenRect::new(0.0, 0.0, 120.0, 90.0);
        assert!(MinimapLayout::compute(&tiny, world()).is_none());
    }

    #[test]
    fn world_corners_map_inside_the_panel() {
        let m = MinimapLayout::compute(&canvas(), world()).expect("fits");
        for p in [
            world().min,
            world().max,
            Point::new(world().min.x, world().max.y),
            Point::new(world().max.x, world().min.y),
        ] {
            let (x, y) = m.world_to_panel(p);
            assert!(m.contains(x, y), "corner {p:?} mapped outside: ({x}, {y})");
        }
    }

    #[test]
    fn y_axis_is_flipped_like_the_camera() {
        let m = MinimapLayout::compute(&canvas(), world()).expect("fits");
        let (_, y_high) = m.world_to_panel(Point::new(0, 5_000));
        let (_, y_low) = m.world_to_panel(Point::new(0, 0));
        assert!(
            y_high < y_low,
            "larger world y must sit higher in the panel"
        );
    }

    #[test]
    fn panel_world_round_trips_within_a_dbu_scale() {
        let m = MinimapLayout::compute(&canvas(), world()).expect("fits");
        // One panel pixel covers 1/scale DBU; the round trip must stay within it.
        let tolerance = (1.0 / m.scale).ceil() as i64;
        for p in [
            Point::new(0, 0),
            Point::new(10_000, 5_000),
            Point::new(2_500, 4_000),
            Point::new(9_999, 1),
        ] {
            let (x, y) = m.world_to_panel(p);
            let back = m.panel_to_world(x, y);
            assert!(
                (i64::from(back.x) - i64::from(p.x)).abs() <= tolerance,
                "x: {back:?} vs {p:?}"
            );
            assert!(
                (i64::from(back.y) - i64::from(p.y)).abs() <= tolerance,
                "y: {back:?} vs {p:?}"
            );
        }
    }

    #[test]
    fn clicks_clamp_into_the_world_bounds() {
        let m = MinimapLayout::compute(&canvas(), world()).expect("fits");
        // The panel's top-left corner lies in the padding, outside the content.
        let p = m.panel_to_world(m.panel.left, m.panel.top);
        assert!(p.x >= world().min.x && p.x <= world().max.x);
        assert!(p.y >= world().min.y && p.y <= world().max.y);
    }

    #[test]
    fn contains_matches_the_panel_rect() {
        let m = MinimapLayout::compute(&canvas(), world()).expect("fits");
        assert!(m.contains(m.panel.center_x(), m.panel.center_y()));
        assert!(!m.contains(m.panel.left - 1.0, m.panel.top));
        assert!(!m.contains(m.panel.left, m.panel.top + m.panel.height + 1.0));
    }

    #[test]
    fn viewport_rect_is_clamped_to_the_panel() {
        let m = MinimapLayout::compute(&canvas(), world()).expect("fits");
        // A viewport far larger than the document: the drawn rect must not spill.
        let huge = Rect::new(Point::new(-100_000, -100_000), Point::new(100_000, 100_000));
        let (left, top, w, h) = m.world_rect_to_panel(huge);
        assert!(left >= m.panel.left);
        assert!(top >= m.panel.top);
        assert!(left + w <= m.panel.left + m.panel.width + 0.001);
        assert!(top + h <= m.panel.top + m.panel.height + 0.001);
        // A small centered viewport maps to a strictly smaller rect.
        let small = Rect::new(Point::new(4_000, 2_000), Point::new(6_000, 3_000));
        let (_, _, sw, sh) = m.world_rect_to_panel(small);
        assert!(sw > 0.0 && sw < w);
        assert!(sh > 0.0 && sh < h);
    }

    #[test]
    fn aspect_ratio_is_preserved() {
        // A wide world: width should limit the scale, leaving vertical padding.
        let m = MinimapLayout::compute(&canvas(), world()).expect("fits");
        let (x0, y0) = m.world_to_panel(Point::new(0, 5_000));
        let (x1, y1) = m.world_to_panel(Point::new(10_000, 0));
        let drawn_w = f64::from(x1 - x0);
        let drawn_h = f64::from(y1 - y0);
        let world_aspect = 10_000.0 / 5_000.0;
        assert!(
            (drawn_w / drawn_h - world_aspect).abs() < 1e-3,
            "drawn {drawn_w} x {drawn_h} should keep aspect {world_aspect}"
        );
    }
}
