//! The world<->screen camera: a pure, window-free view transform.
//!
//! The canvas works in three coordinate spaces:
//!
//! * **World / DBU space**, integer database units ([`reticle_geometry::Dbu`]),
//!   `+y` pointing up, matching the document and the GPU renderer.
//! * **Screen space**, `f32` pixels inside the canvas rectangle, `+y` pointing
//!   *down*, matching egui.
//!
//! [`ViewCamera`] owns the center (in world space) and the zoom (screen pixels per
//! DBU) and converts between the two spaces. It is deliberately free of any egui
//! type so the transform can be unit-tested without a window; the egui layer feeds
//! it a screen rectangle each frame.

use reticle_geometry::{Point, Rect};
use reticle_model::Camera;

/// A 2D pan/zoom camera mapping world DBU coordinates to canvas pixels.
///
/// The camera is defined by the world point shown at the center of the canvas and
/// the zoom factor in pixels per DBU. Screen space has `+y` down (egui); world
/// space has `+y` up (the document), so the vertical axis is flipped on
/// conversion.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ViewCamera {
    /// World-space point displayed at the center of the canvas, in DBU (kept as
    /// `f64` so panning is smooth below one-DBU granularity).
    center_x: f64,
    /// World-space `y` displayed at the canvas center, in DBU.
    center_y: f64,
    /// Zoom: screen pixels per DBU. Always strictly positive.
    pixels_per_dbu: f64,
}

impl Default for ViewCamera {
    fn default() -> Self {
        Self {
            center_x: 0.0,
            center_y: 0.0,
            pixels_per_dbu: 1.0,
        }
    }
}

/// The smallest zoom the camera will accept, in pixels per DBU.
const MIN_ZOOM: f64 = 1.0e-6;
/// The largest zoom the camera will accept, in pixels per DBU.
const MAX_ZOOM: f64 = 1.0e6;

impl ViewCamera {
    /// Creates a camera centered on `center` at `pixels_per_dbu` zoom.
    ///
    /// The zoom is clamped into the supported `[1e-6, 1e6]` range so the transform
    /// never divides by zero or overflows.
    #[must_use]
    pub fn new(center: Point, pixels_per_dbu: f64) -> Self {
        Self {
            center_x: f64::from(center.x),
            center_y: f64::from(center.y),
            pixels_per_dbu: pixels_per_dbu.clamp(MIN_ZOOM, MAX_ZOOM),
        }
    }

    /// The current zoom, in screen pixels per DBU.
    #[must_use]
    pub fn pixels_per_dbu(&self) -> f64 {
        self.pixels_per_dbu
    }

    /// The world-space point at the center of the canvas, rounded to DBU.
    #[must_use]
    pub fn center(&self) -> Point {
        Point::new(round_dbu(self.center_x), round_dbu(self.center_y))
    }

    /// Converts a world-space point to screen pixels within `screen`.
    ///
    /// `screen` is the canvas rectangle in egui screen coordinates. The world
    /// center maps to the rectangle's center; `+y` in world maps to `-y` on screen.
    #[must_use]
    pub fn world_to_screen(&self, screen: &ScreenRect, world: Point) -> (f32, f32) {
        let dx = (f64::from(world.x) - self.center_x) * self.pixels_per_dbu;
        let dy = (f64::from(world.y) - self.center_y) * self.pixels_per_dbu;
        let sx = f64::from(screen.center_x()) + dx;
        // World +y is up, screen +y is down: negate.
        let sy = f64::from(screen.center_y()) - dy;
        (sx as f32, sy as f32)
    }

    /// Converts a screen-pixel position within `screen` back to a world point.
    ///
    /// This is the inverse of [`ViewCamera::world_to_screen`]; feeding a screen
    /// point through both round-trips to (approximately, subject to integer
    /// rounding) the original DBU coordinate.
    #[must_use]
    pub fn screen_to_world(&self, screen: &ScreenRect, sx: f32, sy: f32) -> Point {
        let dx = (f64::from(sx) - f64::from(screen.center_x())) / self.pixels_per_dbu;
        // Invert the +y flip.
        let dy = (f64::from(screen.center_y()) - f64::from(sy)) / self.pixels_per_dbu;
        Point::new(round_dbu(self.center_x + dx), round_dbu(self.center_y + dy))
    }

    /// The world rectangle currently visible in `screen`, in DBU.
    ///
    /// This is the culling region: only geometry intersecting it needs to be drawn.
    #[must_use]
    pub fn visible_world_rect(&self, screen: &ScreenRect) -> Rect {
        let half_w = f64::from(screen.width) / (2.0 * self.pixels_per_dbu);
        let half_h = f64::from(screen.height) / (2.0 * self.pixels_per_dbu);
        let min = Point::new(
            round_dbu(self.center_x - half_w),
            round_dbu(self.center_y - half_h),
        );
        let max = Point::new(
            round_dbu(self.center_x + half_w),
            round_dbu(self.center_y + half_h),
        );
        Rect::new(min, max)
    }

    /// Pans the view by a screen-pixel delta (e.g. a drag), keeping zoom fixed.
    ///
    /// Dragging the mouse right by `dx` pixels moves the world under the cursor
    /// right, i.e. the camera center moves left; the vertical axis is flipped to
    /// match screen coordinates.
    pub fn pan_pixels(&mut self, dx: f32, dy: f32) {
        self.center_x -= f64::from(dx) / self.pixels_per_dbu;
        // Screen +y down -> world +y up.
        self.center_y += f64::from(dy) / self.pixels_per_dbu;
    }

    /// Zooms by `factor` about the fixed screen point `(sx, sy)` within `screen`.
    ///
    /// A `factor > 1` zooms in, `< 1` zooms out. The world point under `(sx, sy)`
    /// stays anchored to that pixel: this is the "zoom to cursor" behavior. Zoom is
    /// clamped to the supported range.
    pub fn zoom_about(&mut self, screen: &ScreenRect, factor: f64, sx: f32, sy: f32) {
        let anchor_world = self.screen_to_world(screen, sx, sy);
        let new_zoom = (self.pixels_per_dbu * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pixels_per_dbu = new_zoom;
        // Re-solve the center so that `anchor_world` maps back to `(sx, sy)`.
        let off_x = (f64::from(sx) - f64::from(screen.center_x())) / self.pixels_per_dbu;
        let off_y = (f64::from(screen.center_y()) - f64::from(sy)) / self.pixels_per_dbu;
        self.center_x = f64::from(anchor_world.x) - off_x;
        self.center_y = f64::from(anchor_world.y) - off_y;
    }

    /// Fits `world` to `screen` with a small margin, centering the content.
    ///
    /// If `world` is degenerate (zero width or height) the zoom is left unchanged
    /// and only the center moves, so an empty or line-like design still shows.
    pub fn zoom_to_fit(&mut self, screen: &ScreenRect, world: Rect) {
        self.center_x = midpoint(world.min.x, world.max.x);
        self.center_y = midpoint(world.min.y, world.max.y);
        let w = world.width().max(1) as f64;
        let h = world.height().max(1) as f64;
        // 5% margin on each side.
        let margin = 0.9;
        let zoom_x = f64::from(screen.width) * margin / w;
        let zoom_y = f64::from(screen.height) * margin / h;
        let zoom = zoom_x.min(zoom_y);
        if zoom.is_finite() && zoom > 0.0 {
            self.pixels_per_dbu = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        }
    }

    /// Centers on `target` and zooms so the rectangle fills `screen` with a margin.
    ///
    /// This is the "zoom to a feature" navigation used by the DRC panel to jump to a
    /// violation: the target is centered and framed so it occupies most of the
    /// canvas, leaving a `margin` fraction of empty space on each side. `margin` is
    /// clamped to `[0.0, 0.45)` so the target always stays strictly inside the view.
    /// A degenerate `target` (zero width or height) keeps the current zoom and only
    /// recenters, so a point-like location still lands in the middle of the canvas.
    pub fn zoom_to_rect(&mut self, screen: &ScreenRect, target: Rect, margin: f64) {
        self.center_x = midpoint(target.min.x, target.max.x);
        self.center_y = midpoint(target.min.y, target.max.y);
        // A degenerate target has no extent to frame: recenter but keep the zoom.
        if target.width() == 0 || target.height() == 0 {
            return;
        }
        let m = margin.clamp(0.0, 0.45);
        // Fraction of the viewport the target should span (1.0 means edge to edge).
        let fill = 1.0 - 2.0 * m;
        let w = target.width() as f64;
        let h = target.height() as f64;
        let zoom_x = f64::from(screen.width) * fill / w;
        let zoom_y = f64::from(screen.height) * fill / h;
        let zoom = zoom_x.min(zoom_y);
        if zoom.is_finite() && zoom > 0.0 {
            self.pixels_per_dbu = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        }
    }

    /// Builds the model [`Camera`] for this view at the given pixel size.
    ///
    /// Used by the native offscreen export path so the exported PNG frames exactly
    /// the region shown on the canvas. The [`Camera::viewport`] is the visible world
    /// rectangle; `pixels_per_dbu` is narrowed to `f32` for the GPU projection.
    #[must_use]
    pub fn to_model_camera(&self, screen: &ScreenRect) -> Camera {
        Camera {
            center: self.center(),
            pixels_per_dbu: self.pixels_per_dbu as f32,
            viewport: self.visible_world_rect(screen),
        }
    }
}

/// Applies a pinch-zoom gesture to `camera`, returning the updated camera.
///
/// `centroid` is the two-finger gesture centroid in screen pixels and `zoom_delta` is
/// egui's multiplicative pinch factor (`ctx.input(|i| i.zoom_delta())`): `1.0` is no
/// change, `> 1.0` zooms in, `< 1.0` zooms out. The world point under the centroid stays
/// pinned to that pixel across the zoom - the pinch-anchoring invariant - by deferring to
/// [`ViewCamera::zoom_about`], and the zoom is clamped to the camera's supported range.
///
/// A `zoom_delta` of exactly `1.0` (egui's rest value when no pinch is active) or a
/// non-positive factor leaves the camera untouched, so calling this every frame is a
/// no-op until the user actually pinches. Pure and window-free, so the invariant is
/// unit-tested without a touch device.
#[must_use]
pub fn apply_pinch(
    mut camera: ViewCamera,
    screen: &ScreenRect,
    centroid: (f32, f32),
    zoom_delta: f64,
) -> ViewCamera {
    // Ignore egui's rest value (exactly 1.0) and any non-positive factor, so an idle
    // frame or a degenerate gesture never disturbs the camera.
    if zoom_delta > 0.0 && (zoom_delta - 1.0).abs() > f64::EPSILON {
        camera.zoom_about(screen, zoom_delta, centroid.0, centroid.1);
    }
    camera
}

/// A canvas rectangle in egui screen pixels, with `+y` down.
///
/// This mirrors the parts of an `egui::Rect` the camera needs without depending on
/// egui, so [`ViewCamera`] stays testable. The egui layer constructs one per frame
/// from the allocated canvas response.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ScreenRect {
    /// Left edge in screen pixels.
    pub left: f32,
    /// Top edge in screen pixels.
    pub top: f32,
    /// Width in screen pixels.
    pub width: f32,
    /// Height in screen pixels.
    pub height: f32,
}

impl ScreenRect {
    /// Creates a screen rectangle from its top-left corner and size.
    #[must_use]
    pub fn new(left: f32, top: f32, width: f32, height: f32) -> Self {
        Self {
            left,
            top,
            width,
            height,
        }
    }

    /// The horizontal center of the rectangle, in screen pixels.
    #[must_use]
    pub fn center_x(&self) -> f32 {
        self.left + self.width * 0.5
    }

    /// The vertical center of the rectangle, in screen pixels.
    #[must_use]
    pub fn center_y(&self) -> f32 {
        self.top + self.height * 0.5
    }
}

/// Rounds an `f64` DBU coordinate to the nearest integer, saturating into range.
fn round_dbu(v: f64) -> i32 {
    let r = v.round();
    if r >= f64::from(i32::MAX) {
        i32::MAX
    } else if r <= f64::from(i32::MIN) {
        i32::MIN
    } else {
        r as i32
    }
}

/// The midpoint of two DBU coordinates as an `f64`.
fn midpoint(a: i32, b: i32) -> f64 {
    f64::midpoint(f64::from(a), f64::from(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> ScreenRect {
        ScreenRect::new(0.0, 0.0, 800.0, 600.0)
    }

    #[test]
    fn world_screen_round_trips() {
        let cam = ViewCamera::new(Point::new(1000, -500), 0.25);
        let s = screen();
        for &world in &[
            Point::new(0, 0),
            Point::new(1000, -500),
            Point::new(4000, 2000),
            Point::new(-3000, 6000),
        ] {
            let (sx, sy) = cam.world_to_screen(&s, world);
            let back = cam.screen_to_world(&s, sx, sy);
            // Integer rounding tolerance of a couple of DBU.
            assert!((back.x - world.x).abs() <= 2, "x: {back:?} vs {world:?}");
            assert!((back.y - world.y).abs() <= 2, "y: {back:?} vs {world:?}");
        }
    }

    #[test]
    fn center_maps_to_screen_center() {
        let cam = ViewCamera::new(Point::new(42, 17), 3.0);
        let s = screen();
        let (sx, sy) = cam.world_to_screen(&s, Point::new(42, 17));
        assert!((sx - s.center_x()).abs() < 0.01);
        assert!((sy - s.center_y()).abs() < 0.01);
    }

    #[test]
    fn world_y_up_is_screen_y_down() {
        let cam = ViewCamera::new(Point::ORIGIN, 1.0);
        let s = screen();
        let (_, sy_high) = cam.world_to_screen(&s, Point::new(0, 100));
        let (_, sy_low) = cam.world_to_screen(&s, Point::new(0, -100));
        // Larger world y should be higher on screen (smaller screen y).
        assert!(sy_high < sy_low);
    }

    #[test]
    fn zoom_to_cursor_keeps_point_fixed() {
        let mut cam = ViewCamera::new(Point::new(0, 0), 1.0);
        let s = screen();
        // Pick an off-center cursor and the world point currently under it.
        let (cursor_x, cursor_y) = (620.0_f32, 130.0_f32);
        let before = cam.screen_to_world(&s, cursor_x, cursor_y);
        cam.zoom_about(&s, 2.5, cursor_x, cursor_y);
        let after = cam.screen_to_world(&s, cursor_x, cursor_y);
        assert!(
            (before.x - after.x).abs() <= 2 && (before.y - after.y).abs() <= 2,
            "cursor world point moved: {before:?} -> {after:?}"
        );
        // Zoom actually changed.
        assert!((cam.pixels_per_dbu() - 2.5).abs() < 1e-9);
    }

    #[test]
    fn zoom_is_clamped() {
        let mut cam = ViewCamera::new(Point::ORIGIN, 1.0);
        let s = screen();
        cam.zoom_about(&s, 1e12, 0.0, 0.0);
        assert!(cam.pixels_per_dbu() <= MAX_ZOOM);
        cam.zoom_about(&s, 1e-24, 0.0, 0.0);
        assert!(cam.pixels_per_dbu() >= MIN_ZOOM);
    }

    #[test]
    fn pan_moves_world_under_cursor() {
        let mut cam = ViewCamera::new(Point::ORIGIN, 2.0);
        let before = cam.center();
        cam.pan_pixels(20.0, 0.0);
        // Dragging right moves the camera center left in world space.
        assert!(cam.center().x < before.x);
    }

    #[test]
    fn zoom_to_fit_centers_content() {
        let mut cam = ViewCamera::new(Point::ORIGIN, 1.0);
        let s = screen();
        let content = Rect::new(Point::new(1000, 2000), Point::new(5000, 4000));
        cam.zoom_to_fit(&s, content);
        let c = cam.center();
        assert_eq!(c.x, 3000);
        assert_eq!(c.y, 3000);
        // Content should now fit inside the visible rect.
        let vis = cam.visible_world_rect(&s);
        assert!(vis.min.x <= content.min.x && vis.max.x >= content.max.x);
        assert!(vis.min.y <= content.min.y && vis.max.y >= content.max.y);
    }

    #[test]
    fn zoom_to_rect_frames_target_inside_viewport() {
        let mut cam = ViewCamera::new(Point::ORIGIN, 1.0);
        let s = screen();
        let target = Rect::new(Point::new(9000, -4000), Point::new(9600, -3400));
        cam.zoom_to_rect(&s, target, 0.1);
        // Centered on the target.
        let c = cam.center();
        assert_eq!(c.x, 9300);
        assert_eq!(c.y, -3700);
        // The visible rectangle fully contains the target, with room to spare.
        let vis = cam.visible_world_rect(&s);
        assert!(
            vis.min.x <= target.min.x && vis.max.x >= target.max.x,
            "x not framed: {vis:?} vs {target:?}"
        );
        assert!(
            vis.min.y <= target.min.y && vis.max.y >= target.max.y,
            "y not framed: {vis:?} vs {target:?}"
        );
        // The target must not fill the whole viewport (the margin left a border).
        assert!(vis.width() > target.width());
        assert!(vis.height() > target.height());
    }

    #[test]
    fn zoom_to_rect_recenters_on_degenerate_target() {
        let mut cam = ViewCamera::new(Point::new(1, 1), 4.0);
        let s = screen();
        let before_zoom = cam.pixels_per_dbu();
        // A zero-area point location: only the center should move.
        let point = Rect::new(Point::new(500, 700), Point::new(500, 700));
        cam.zoom_to_rect(&s, point, 0.1);
        assert_eq!(cam.center(), Point::new(500, 700));
        assert!((cam.pixels_per_dbu() - before_zoom).abs() < 1e-9);
    }

    #[test]
    fn pinch_keeps_world_point_under_the_centroid_fixed() {
        // The anchoring invariant: the world point under the pinch centroid stays fixed
        // to that pixel while the zoom changes.
        let cam = ViewCamera::new(Point::new(0, 0), 1.0);
        let s = screen();
        let centroid = (610.0_f32, 155.0_f32);
        let before = cam.screen_to_world(&s, centroid.0, centroid.1);
        let zoomed = apply_pinch(cam, &s, centroid, 1.8);
        let after = zoomed.screen_to_world(&s, centroid.0, centroid.1);
        assert!(
            (before.x - after.x).abs() <= 2 && (before.y - after.y).abs() <= 2,
            "centroid world point moved: {before:?} -> {after:?}"
        );
        // A pinch-out (delta > 1) zooms in.
        assert!(zoomed.pixels_per_dbu() > cam.pixels_per_dbu());
    }

    #[test]
    fn pinch_of_unity_delta_is_a_noop() {
        // egui reports a zoom_delta of exactly 1.0 when there is no active pinch;
        // applying it must leave the camera bit-for-bit unchanged.
        let cam = ViewCamera::new(Point::new(100, -50), 3.0);
        let s = screen();
        assert_eq!(apply_pinch(cam, &s, (10.0, 20.0), 1.0), cam);
        // A non-positive factor is ignored rather than corrupting the zoom.
        assert_eq!(apply_pinch(cam, &s, (10.0, 20.0), 0.0), cam);
    }

    #[test]
    fn pinch_zoom_is_clamped() {
        let cam = ViewCamera::new(Point::ORIGIN, 1.0);
        let s = screen();
        let hard_in = apply_pinch(cam, &s, (0.0, 0.0), 1e12);
        assert!(hard_in.pixels_per_dbu() <= MAX_ZOOM);
        let hard_out = apply_pinch(cam, &s, (0.0, 0.0), 1e-24);
        assert!(hard_out.pixels_per_dbu() >= MIN_ZOOM);
    }

    #[test]
    fn visible_rect_grows_as_zoom_shrinks() {
        let s = screen();
        let wide = ViewCamera::new(Point::ORIGIN, 0.1).visible_world_rect(&s);
        let tight = ViewCamera::new(Point::ORIGIN, 10.0).visible_world_rect(&s);
        assert!(wide.width() > tight.width());
        assert!(wide.height() > tight.height());
    }
}
