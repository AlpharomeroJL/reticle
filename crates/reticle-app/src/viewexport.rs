//! View and export polish: theme switching, camera bookmarks, and export of the
//! current view or selection to SVG and PNG, including a print-style monochrome
//! render mode.
//!
//! Everything that can be pure is pure. The [`ViewExport`] state owns the chosen
//! theme, the per-document bookmarks, the export format and scope, and the
//! monochrome toggle; it holds no egui or GPU types, so it is unit-tested without a
//! window. The egui and GPU wiring lives in [`crate::app`], which owns one
//! [`ViewExport`] field, applies the theme each frame, and draws the panel at the
//! end of the right-hand column.
//!
//! ## Coordinate spaces
//!
//! Export works in world (DBU) space, matching the document. A shape's world
//! coordinates are mapped into the output image by an affine fit of the export
//! bounds rectangle onto the pixel canvas, with world `+y` up flipped to image
//! `+y` down (see [`Projection`]). This is the same axis convention the on-screen
//! [`crate::camera::ViewCamera`] uses, so an exported picture matches what the
//! canvas shows.
//!
//! ## Export paths
//!
//! * **SVG** is generated directly from the shape list as text ([`shapes_to_svg`]),
//!   a pure function that is the primary, fully testable export.
//! * **PNG** has two paths. The interactive *view* export reuses the native
//!   offscreen GPU renderer for a pixel-accurate, full-colour frame (native only;
//!   see [`crate::app`]). The *selection* and *monochrome* PNG export uses the pure
//!   [`rasterize`] scanline filler feeding the crate's dependency-free PNG encoder,
//!   so it works without a GPU and is unit-tested. Neither path adds a dependency.

use reticle_geometry::{LayerId, Point, Rect, Shape};
use reticle_model::{DrawShape, ShapeKind};

/// The color theme, now owned by the theme module (ADR 0095). Re-exported here
/// so existing call sites and the persisted session tag keep compiling
/// unchanged after the move; v8.1 renders one dark theme, so the enum only
/// carries the persisted tag forward (see [`crate::theme::Theme`]).
pub use crate::theme::Theme;

/// A saved camera position: a world center and a zoom, under a user-given name.
///
/// A bookmark is a pure record of where the camera looked, reconstructed into a
/// live [`crate::camera::ViewCamera`] via [`Bookmark::center`] and
/// [`Bookmark::pixels_per_dbu`]. It stays free of the camera type so it can be
/// created and round-tripped in tests without a window.
#[derive(Clone, PartialEq, Debug)]
pub struct Bookmark {
    /// The bookmark's display name.
    pub name: String,
    /// The camera center x, in DBU.
    pub center_x: i32,
    /// The camera center y, in DBU.
    pub center_y: i32,
    /// The zoom, in screen pixels per DBU.
    pub pixels_per_dbu: f64,
}

impl Bookmark {
    /// Creates a bookmark from a name, a world center, and a zoom.
    #[must_use]
    pub fn new(name: impl Into<String>, center: Point, pixels_per_dbu: f64) -> Self {
        Self {
            name: name.into(),
            center_x: center.x,
            center_y: center.y,
            pixels_per_dbu,
        }
    }

    /// The world center this bookmark restores.
    #[must_use]
    pub fn center(&self) -> Point {
        Point::new(self.center_x, self.center_y)
    }

    /// The zoom this bookmark restores, in pixels per DBU.
    #[must_use]
    pub fn pixels_per_dbu(&self) -> f64 {
        self.pixels_per_dbu
    }
}

/// Which geometry an export covers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ExportScope {
    /// Every shape in the scene (the whole document view).
    #[default]
    View,
    /// Only the current selection.
    Selection,
}

impl ExportScope {
    /// A short human label for the panel radio.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ExportScope::View => "View",
            ExportScope::Selection => "Selection",
        }
    }
}

/// Which file format an export produces.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ExportFormat {
    /// Scalable vector graphics (the primary, pure export).
    #[default]
    Svg,
    /// A raster PNG image.
    Png,
}

impl ExportFormat {
    /// A short human label for the panel radio.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Svg => "SVG",
            ExportFormat::Png => "PNG",
        }
    }

    /// The lowercase file extension for this format (no dot).
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Svg => "svg",
            ExportFormat::Png => "png",
        }
    }
}

/// The persisted view/export UI state: theme, bookmarks, and export options.
///
/// The app owns exactly one of these. It carries no egui or GPU types, so it is
/// serialized (via the sibling `session` text format) and unit-tested on its own.
/// The `name_input` field is transient panel scratch (the bookmark-name text box)
/// and is not persisted.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct ViewExport {
    /// The active egui theme.
    pub theme: Theme,
    /// Saved camera bookmarks, in creation order.
    pub bookmarks: Vec<Bookmark>,
    /// The bookmark-name text box contents (transient panel scratch).
    pub name_input: String,
    /// The chosen export scope.
    pub scope: ExportScope,
    /// The chosen export format.
    pub format: ExportFormat,
    /// Whether the print-style monochrome render mode is on.
    pub monochrome: bool,
    /// The most recent canvas rectangle in screen pixels, refreshed at the end of
    /// each frame. The export panel draws inside the right column, before the
    /// canvas is measured this frame, so it frames the current view using the last
    /// frame's rect. Transient (not persisted).
    pub last_canvas: Option<crate::camera::ScreenRect>,
}

impl ViewExport {
    /// Creates the default view/export state (dark theme, no bookmarks).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a bookmark with the given name at `center`/`pixels_per_dbu`.
    ///
    /// A blank name is replaced with an auto-generated `View N` label so the list
    /// entry is always clickable. Returns the stored name.
    pub fn add_bookmark(&mut self, name: &str, center: Point, pixels_per_dbu: f64) -> String {
        let name = if name.trim().is_empty() {
            format!("View {}", self.bookmarks.len() + 1)
        } else {
            name.trim().to_owned()
        };
        self.bookmarks
            .push(Bookmark::new(name.clone(), center, pixels_per_dbu));
        name
    }

    /// Removes the bookmark at `index`, if it exists.
    pub fn remove_bookmark(&mut self, index: usize) {
        if index < self.bookmarks.len() {
            self.bookmarks.remove(index);
        }
    }
}

/// An affine world-to-pixel fit: maps an export bounds rectangle onto a pixel
/// canvas, preserving aspect ratio and flipping world `+y` up to image `+y` down.
///
/// The bounds are centered in the canvas; the scale is the smaller of the two axis
/// fits so nothing is clipped, and the content is padded by `MARGIN` on the tight
/// axis. This is the export analogue of the on-screen camera's `zoom_to_fit`.
#[derive(Clone, Copy, Debug)]
pub struct Projection {
    /// Pixels per DBU (uniform on both axes).
    scale: f64,
    /// World x mapped to pixel x=0 after scaling.
    origin_x: f64,
    /// World y mapped to pixel y=0 after scaling (before the vertical flip).
    origin_y: f64,
    /// Canvas height in pixels, for the vertical flip.
    height: f64,
}

/// Fraction of the canvas left as a border on the tight axis when fitting.
const MARGIN: f64 = 0.05;

impl Projection {
    /// Fits `bounds` onto a `width`x`height` pixel canvas.
    ///
    /// A degenerate bounds (zero width or height) is fitted as if it were one DBU
    /// wide/tall so a single shape still lands centered rather than dividing by
    /// zero.
    #[must_use]
    pub fn fit(bounds: Rect, width: u32, height: u32) -> Self {
        let w = f64::from(width.max(1));
        let h = f64::from(height.max(1));
        let bw = bounds.width().max(1) as f64;
        let bh = bounds.height().max(1) as f64;
        let fill = 1.0 - 2.0 * MARGIN;
        let scale = ((w * fill / bw).min(h * fill / bh)).max(f64::MIN_POSITIVE);
        // Center the bounds in the canvas.
        let cx = f64::midpoint(f64::from(bounds.min.x), f64::from(bounds.max.x));
        let cy = f64::midpoint(f64::from(bounds.min.y), f64::from(bounds.max.y));
        let origin_x = w / 2.0 - cx * scale;
        let origin_y = h / 2.0 - cy * scale;
        Self {
            scale,
            origin_x,
            origin_y,
            height: h,
        }
    }

    /// Maps a world point to `(x, y)` pixel coordinates, flipping the vertical axis.
    #[must_use]
    pub fn map(&self, p: Point) -> (f64, f64) {
        let px = f64::from(p.x) * self.scale + self.origin_x;
        // World +y up, image +y down: reflect about the canvas mid-line.
        let py = self.height - (f64::from(p.y) * self.scale + self.origin_y);
        (px, py)
    }

    /// The pixel length of a world-space distance `d` (DBU) at this scale.
    #[must_use]
    pub fn scale_len(&self, d: i64) -> f64 {
        d as f64 * self.scale
    }
}

/// The bounding box of `shapes`, or a unit box at the origin when the list is empty.
///
/// Used as the export fit region so an export with no geometry still produces a
/// valid, if blank, image rather than an invalid zero-size canvas.
#[must_use]
pub fn shapes_bounds(shapes: &[DrawShape]) -> Rect {
    shapes
        .iter()
        .map(Shape::bounding_box)
        .reduce(|a, b| {
            Rect::new(
                Point::new(a.min.x.min(b.min.x), a.min.y.min(b.min.y)),
                Point::new(a.max.x.max(b.max.x), a.max.y.max(b.max.y)),
            )
        })
        .unwrap_or_else(|| Rect::new(Point::ORIGIN, Point::new(1, 1)))
}

/// A colour source for export: maps a layer to its `(r, g, b, a)` fill colour.
///
/// The app implements this over its live layer table; tests use a fixed mapping.
pub trait LayerPaint {
    /// The `(r, g, b, a)` colour for `layer`.
    fn color(&self, layer: LayerId) -> (u8, u8, u8, u8);
}

impl<F: Fn(LayerId) -> (u8, u8, u8, u8)> LayerPaint for F {
    fn color(&self, layer: LayerId) -> (u8, u8, u8, u8) {
        self(layer)
    }
}

/// The colour a shape draws with, honouring the monochrome print mode.
///
/// In monochrome mode every shape is pure black regardless of layer, which is what
/// the print-style outline render wants; otherwise the layer colour from `paint` is
/// used. The returned alpha is dropped to fully opaque for a crisp print.
fn shape_rgb(paint: &impl LayerPaint, layer: LayerId, monochrome: bool) -> (u8, u8, u8) {
    if monochrome {
        (0, 0, 0)
    } else {
        let (r, g, b, _) = paint.color(layer);
        (r, g, b)
    }
}

/// Formats an `(r, g, b)` triple as a `#rrggbb` CSS colour.
fn hex(rgb: (u8, u8, u8)) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb.0, rgb.1, rgb.2)
}

/// Renders `shapes` to standalone SVG text, fitting `bounds` into `width`x`height`.
///
/// Each [`ShapeKind`] maps to one SVG element: a rectangle to `<rect>`, a polygon
/// to `<polygon>`, and a path (wire) to a stroked `<polyline>` whose stroke width
/// is the path width scaled to output pixels. Colours come from `paint` (the layer
/// table) unless `monochrome` is set, in which case shapes are black outlines on a
/// white page: filled shapes become unfilled, black-stroked outlines and wires stay
/// black strokes. This is a pure function of its inputs and is the primary,
/// fully-tested export path.
#[must_use]
pub fn shapes_to_svg(
    shapes: &[DrawShape],
    bounds: Rect,
    width: u32,
    height: u32,
    paint: &impl LayerPaint,
    monochrome: bool,
) -> String {
    use std::fmt::Write as _;
    let proj = Projection::fit(bounds, width, height);
    let mut out = String::new();
    // Writing to a String is infallible, so the `write!` results are discarded.
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\">"
    );
    // A page background: white always (the print sheet); it also gives the colour
    // export a neutral backing so semi-transparent layer fills read consistently.
    let _ = writeln!(
        out,
        "  <rect width=\"{width}\" height=\"{height}\" fill=\"#ffffff\"/>"
    );
    for s in shapes {
        let rgb = shape_rgb(paint, s.layer, monochrome);
        let color = hex(rgb);
        match &s.kind {
            ShapeKind::Rect(r) => {
                let (x0, y0) = proj.map(Point::new(r.min.x, r.max.y));
                let w = proj.scale_len(r.width());
                let h = proj.scale_len(r.height());
                if monochrome {
                    let _ = writeln!(
                        out,
                        "  <rect x=\"{x0:.3}\" y=\"{y0:.3}\" width=\"{w:.3}\" height=\"{h:.3}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"1\"/>"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "  <rect x=\"{x0:.3}\" y=\"{y0:.3}\" width=\"{w:.3}\" height=\"{h:.3}\" fill=\"{color}\"/>"
                    );
                }
            }
            ShapeKind::Polygon(p) => {
                let pts = points_attr(&proj, p.vertices());
                if monochrome {
                    let _ = writeln!(
                        out,
                        "  <polygon points=\"{pts}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"1\"/>"
                    );
                } else {
                    let _ = writeln!(out, "  <polygon points=\"{pts}\" fill=\"{color}\"/>");
                }
            }
            ShapeKind::Path(p) => {
                let pts = points_attr(&proj, p.points());
                // A wire has no fill; its width scales into pixels, floored to a
                // hairline so a thin wire is still visible.
                let sw = proj.scale_len(i64::from(p.width())).max(1.0);
                let _ = writeln!(
                    out,
                    "  <polyline points=\"{pts}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{sw:.3}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"/>"
                );
            }
        }
    }
    out.push_str("</svg>\n");
    out
}

/// Formats a vertex list as an SVG `points` attribute value in pixel space.
fn points_attr(proj: &Projection, verts: &[Point]) -> String {
    verts
        .iter()
        .map(|&v| {
            let (x, y) = proj.map(v);
            format!("{x:.3},{y:.3}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A tightly-packed RGBA8 image, row 0 at the top, ready for the PNG encoder.
///
/// Produced by [`rasterize`] for the pure PNG export path.
#[derive(Clone, PartialEq, Debug)]
pub struct Raster {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes of RGBA, row-major from the top.
    pub pixels: Vec<u8>,
}

impl Raster {
    /// Reads the `(r, g, b, a)` pixel at `(x, y)`, or `None` if out of bounds.
    #[must_use]
    pub fn get(&self, x: u32, y: u32) -> Option<(u8, u8, u8, u8)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = ((y * self.width + x) * 4) as usize;
        Some((
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ))
    }
}

/// Rasterizes `shapes` into an RGBA image, fitting `bounds` into `width`x`height`.
///
/// This is the pure PNG path (no GPU): a white page with each shape's axis-aligned
/// bounding box filled with its colour (black in monochrome mode). Filling the
/// bounding box keeps the rasterizer a small, dependency-free scanline loop while
/// still giving a faithful raster for rectangles (the common export case) and a
/// legible block for polygons and wires. Alpha-blends colours over the page so
/// overlapping semi-transparent layers read like the canvas. The output feeds the
/// crate's built-in PNG encoder.
#[must_use]
pub fn rasterize(
    shapes: &[DrawShape],
    bounds: Rect,
    width: u32,
    height: u32,
    paint: &impl LayerPaint,
    monochrome: bool,
) -> Raster {
    let out_w = width.max(1);
    let out_h = height.max(1);
    let proj = Projection::fit(bounds, out_w, out_h);
    // Start from a white page.
    let mut pixels = vec![255u8; (out_w as usize) * (out_h as usize) * 4];
    let (wf, hf) = (f64::from(out_w), f64::from(out_h));
    for shape in shapes {
        let (red, green, blue) = shape_rgb(paint, shape.layer, monochrome);
        // Colour alpha: opaque in monochrome, else the layer's own alpha.
        let alpha = if monochrome {
            255
        } else {
            paint.color(shape.layer).3
        };
        let bb = shape.bounding_box();
        // Map the world bounding box to pixel extents (note the y flip swaps
        // min/max corners).
        let (x0, y1) = proj.map(Point::new(bb.min.x, bb.min.y));
        let (x1, y0) = proj.map(Point::new(bb.max.x, bb.max.y));
        let px0 = x0.floor().clamp(0.0, wf) as u32;
        let px1 = x1.ceil().clamp(0.0, wf) as u32;
        let py0 = y0.floor().clamp(0.0, hf) as u32;
        let py1 = y1.ceil().clamp(0.0, hf) as u32;
        for py in py0..py1 {
            for px in px0..px1 {
                let idx = ((py * out_w + px) * 4) as usize;
                blend(&mut pixels[idx..idx + 4], red, green, blue, alpha);
            }
        }
    }
    Raster {
        width: out_w,
        height: out_h,
        pixels,
    }
}

/// Alpha-blends `(r, g, b, a)` over the RGBA pixel in `dst` (source-over).
fn blend(dst: &mut [u8], r: u8, g: u8, b: u8, a: u8) {
    let sa = f64::from(a) / 255.0;
    let inv = 1.0 - sa;
    for (d, s) in dst[..3].iter_mut().zip([r, g, b]) {
        *d = (f64::from(s) * sa + f64::from(*d) * inv).round() as u8;
    }
    dst[3] = 255;
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::{Endcap, Path, Polygon};

    /// A fixed colour source for tests: red for layer 1, blue for layer 2, else grey.
    fn paint() -> impl LayerPaint {
        |layer: LayerId| match layer.layer {
            1 => (255, 0, 0, 255),
            2 => (0, 0, 255, 128),
            _ => (128, 128, 128, 255),
        }
    }

    fn rect_shape(layer: u16, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            LayerId::new(layer, 0),
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    }

    // ---- Theme -----------------------------------------------------------

    #[test]
    fn theme_defaults_to_dark() {
        // v8.1 renders one dark theme; the toggle is retired (ADR 0095).
        let ve = ViewExport::new();
        assert_eq!(ve.theme, Theme::Dark);
    }

    #[test]
    fn theme_tag_round_trips() {
        assert_eq!(Theme::from_tag(Theme::Light.tag()), Theme::Light);
        assert_eq!(Theme::from_tag(Theme::Dark.tag()), Theme::Dark);
        // Unknown tags fall back to the dark default.
        assert_eq!(Theme::from_tag("chartreuse"), Theme::Dark);
    }

    // ---- Bookmarks -------------------------------------------------------

    #[test]
    fn bookmark_round_trips_camera() {
        let bm = Bookmark::new("cell A", Point::new(1200, -800), 0.25);
        assert_eq!(bm.center(), Point::new(1200, -800));
        assert!((bm.pixels_per_dbu() - 0.25).abs() < 1e-12);
    }

    #[test]
    fn add_bookmark_restores_through_view_camera() {
        // The save/restore contract the app relies on: a saved (center, ppd) round
        // trips through ViewCamera exactly.
        let mut ve = ViewExport::new();
        ve.add_bookmark("origin zoom", Point::new(500, 700), 4.0);
        let bm = &ve.bookmarks[0];
        let cam = crate::camera::ViewCamera::new(bm.center(), bm.pixels_per_dbu());
        assert_eq!(cam.center(), Point::new(500, 700));
        assert!((cam.pixels_per_dbu() - 4.0).abs() < 1e-12);
    }

    #[test]
    fn blank_bookmark_name_is_autogenerated() {
        let mut ve = ViewExport::new();
        let n1 = ve.add_bookmark("   ", Point::ORIGIN, 1.0);
        let n2 = ve.add_bookmark("", Point::ORIGIN, 1.0);
        assert_eq!(n1, "View 1");
        assert_eq!(n2, "View 2");
    }

    #[test]
    fn remove_bookmark_by_index() {
        let mut ve = ViewExport::new();
        ve.add_bookmark("a", Point::ORIGIN, 1.0);
        ve.add_bookmark("b", Point::ORIGIN, 1.0);
        ve.remove_bookmark(0);
        assert_eq!(ve.bookmarks.len(), 1);
        assert_eq!(ve.bookmarks[0].name, "b");
        // Out-of-range removal is a no-op.
        ve.remove_bookmark(9);
        assert_eq!(ve.bookmarks.len(), 1);
    }

    // ---- Projection ------------------------------------------------------

    #[test]
    fn projection_centers_and_flips_y() {
        let bounds = Rect::new(Point::new(0, 0), Point::new(100, 100));
        let proj = Projection::fit(bounds, 200, 200);
        // The world center maps to the canvas center.
        let (cx, cy) = proj.map(Point::new(50, 50));
        assert!((cx - 100.0).abs() < 0.5, "cx={cx}");
        assert!((cy - 100.0).abs() < 0.5, "cy={cy}");
        // A higher world y is a smaller image y (flip).
        let (_, y_top) = proj.map(Point::new(50, 100));
        let (_, y_bot) = proj.map(Point::new(50, 0));
        assert!(y_top < y_bot, "y not flipped: {y_top} !< {y_bot}");
    }

    #[test]
    fn projection_handles_degenerate_bounds() {
        // A zero-area point bounds must not divide by zero; it maps to a finite
        // pixel near the canvas center.
        let bounds = Rect::new(Point::new(7, 7), Point::new(7, 7));
        let proj = Projection::fit(bounds, 64, 64);
        let (x, y) = proj.map(Point::new(7, 7));
        assert!(x.is_finite() && y.is_finite());
        assert!((x - 32.0).abs() < 1.0 && (y - 32.0).abs() < 1.0);
    }

    // ---- SVG generation --------------------------------------------------

    #[test]
    fn svg_has_header_and_page() {
        let shapes = [rect_shape(1, 0, 0, 100, 100)];
        let svg = shapes_to_svg(&shapes, shapes_bounds(&shapes), 256, 256, &paint(), false);
        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(svg.contains("width=\"256\""));
        assert!(svg.contains("viewBox=\"0 0 256 256\""));
        assert!(svg.contains("fill=\"#ffffff\"")); // page background
        assert!(svg.trim_end().ends_with("</svg>"));
    }

    #[test]
    fn svg_rect_uses_layer_color() {
        let shapes = [rect_shape(1, 0, 0, 100, 100)];
        let svg = shapes_to_svg(&shapes, shapes_bounds(&shapes), 256, 256, &paint(), false);
        assert!(svg.contains("<rect"), "no rect element");
        // Layer 1 is red (#ff0000) in the test paint.
        assert!(svg.contains("fill=\"#ff0000\""), "rect not red: {svg}");
    }

    #[test]
    fn svg_polygon_element_and_points() {
        let poly = DrawShape::new(
            LayerId::new(2, 0),
            ShapeKind::Polygon(Polygon::new(vec![
                Point::new(0, 0),
                Point::new(100, 0),
                Point::new(100, 100),
            ])),
        );
        let shapes = [poly];
        let svg = shapes_to_svg(&shapes, shapes_bounds(&shapes), 256, 256, &paint(), false);
        assert!(svg.contains("<polygon points=\""), "no polygon: {svg}");
        // Layer 2 is blue.
        assert!(svg.contains("fill=\"#0000ff\""));
        // Three vertices means three coordinate pairs (two spaces between).
        let line = svg.lines().find(|l| l.contains("<polygon")).unwrap();
        assert_eq!(line.matches(',').count(), 3);
    }

    #[test]
    fn svg_path_is_stroked_polyline() {
        let path = DrawShape::new(
            LayerId::new(1, 0),
            ShapeKind::Path(Path::new(
                vec![Point::new(0, 0), Point::new(200, 0)],
                40,
                Endcap::Round,
            )),
        );
        let shapes = [path];
        let svg = shapes_to_svg(&shapes, shapes_bounds(&shapes), 256, 128, &paint(), false);
        assert!(svg.contains("<polyline points=\""), "no polyline: {svg}");
        assert!(svg.contains("fill=\"none\""));
        assert!(svg.contains("stroke=\"#ff0000\""));
        assert!(svg.contains("stroke-width=\""));
    }

    #[test]
    fn svg_monochrome_is_black_outlines() {
        // Every shape kind, mixed layers: monochrome forces black with no fills.
        let shapes = [
            rect_shape(1, 0, 0, 100, 100),
            DrawShape::new(
                LayerId::new(2, 0),
                ShapeKind::Polygon(Polygon::new(vec![
                    Point::new(0, 0),
                    Point::new(50, 0),
                    Point::new(50, 50),
                ])),
            ),
        ];
        let svg = shapes_to_svg(&shapes, shapes_bounds(&shapes), 256, 256, &paint(), true);
        // No coloured fills at all; only black strokes on shapes.
        assert!(!svg.contains("#ff0000"), "colour leaked into mono: {svg}");
        assert!(!svg.contains("#0000ff"));
        assert!(svg.contains("stroke=\"#000000\""), "no black stroke: {svg}");
        // The rect and polygon are unfilled outlines.
        assert!(svg.contains("<rect x=") && svg.contains("fill=\"none\""));
    }

    #[test]
    fn empty_shapes_still_valid_svg() {
        let svg = shapes_to_svg(&[], shapes_bounds(&[]), 64, 64, &paint(), false);
        assert!(svg.starts_with("<svg"));
        assert!(svg.trim_end().ends_with("</svg>"));
    }

    // ---- Rasterize / PNG path -------------------------------------------

    #[test]
    fn raster_fills_shape_and_keeps_white_page() {
        let shapes = [rect_shape(1, 0, 0, 100, 100)];
        let ras = rasterize(&shapes, shapes_bounds(&shapes), 64, 64, &paint(), false);
        assert_eq!(ras.width, 64);
        assert_eq!(ras.pixels.len(), 64 * 64 * 4);
        // The center pixel sits inside the red rect.
        let (r, g, b, a) = ras.get(32, 32).unwrap();
        assert!(r > 200 && g < 60 && b < 60, "center not red: {r},{g},{b}");
        assert_eq!(a, 255);
        // A corner pixel is outside the fitted shape, so it stays white page.
        let (r, g, b, _) = ras.get(0, 0).unwrap();
        assert!(
            r > 240 && g > 240 && b > 240,
            "corner not white: {r},{g},{b}"
        );
    }

    #[test]
    fn raster_monochrome_center_is_black() {
        let shapes = [rect_shape(1, 0, 0, 100, 100)];
        let ras = rasterize(&shapes, shapes_bounds(&shapes), 64, 64, &paint(), true);
        let (r, g, b, _) = ras.get(32, 32).unwrap();
        assert!(
            r < 20 && g < 20 && b < 20,
            "mono center not black: {r},{g},{b}"
        );
    }

    #[test]
    fn raster_blends_semitransparent_layer() {
        // Layer 2 is blue at alpha 128 over a white page -> a light blue, not pure.
        let shapes = [rect_shape(2, 0, 0, 100, 100)];
        let ras = rasterize(&shapes, shapes_bounds(&shapes), 32, 32, &paint(), false);
        let (r, g, b, _) = ras.get(16, 16).unwrap();
        assert!(b > 200, "blue channel not raised: {b}");
        // White backing bleeds through the half alpha, so red/green stay high.
        assert!(r > 100 && g > 100, "no blend with white page: {r},{g}");
    }
}
