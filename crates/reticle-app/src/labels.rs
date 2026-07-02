//! Canvas text-label layout and formatting: cell names, layer captions, and live
//! dimension text.
//!
//! The GPU paint callback draws only geometry; egui composites painter text on top
//! of it, so labels are drawn as an egui overlay (zero extra dependencies, wasm-safe).
//! This module owns everything about that overlay that is not literally an egui
//! call: which boxes are big enough to carry a centered name, where a caption is
//! anchored so it stays inside the canvas, and how dimensions are formatted in DBU
//! and microns. The app module only converts world rectangles to screen space and
//! hands the results to `egui::Painter::text`.
//!
//! Everything here is pure arithmetic and string formatting, so the layout rules
//! are unit-tested without a window.

use reticle_geometry::Rect;

/// The label font size, in logical pixels, used for canvas text.
pub const LABEL_FONT_PX: f32 = 11.0;

/// Approximate advance width of one monospace glyph, as a fraction of font size.
///
/// egui's default monospace face advances close to `0.6 em` per glyph; a slightly
/// larger factor keeps the fit test conservative so text never spills its box.
const GLYPH_ASPECT: f32 = 0.62;

/// Fraction of a box's width a centered label may occupy before it is dropped.
const MAX_FILL: f32 = 0.9;

/// A screen-space box that may carry a centered text label (e.g. a cell outline).
#[derive(Clone, PartialEq, Debug)]
pub struct LabelBox {
    /// The text to center in the box.
    pub text: String,
    /// Left edge of the box, in screen pixels.
    pub left: f32,
    /// Top edge of the box, in screen pixels.
    pub top: f32,
    /// Box width, in screen pixels.
    pub width: f32,
    /// Box height, in screen pixels.
    pub height: f32,
}

/// A label the overlay should draw: text centered at a screen position.
#[derive(Clone, PartialEq, Debug)]
pub struct PlacedLabel {
    /// The text to draw.
    pub text: String,
    /// Screen x of the text center, in pixels.
    pub x: f32,
    /// Screen y of the text center, in pixels.
    pub y: f32,
}

/// Estimates the rendered width of `text` at `font_px`, in pixels.
///
/// Uses the monospace glyph-advance approximation; good enough to decide whether a
/// label fits, which only needs to err on the side of dropping it.
#[must_use]
pub fn estimated_text_width(text: &str, font_px: f32) -> f32 {
    let chars = text.chars().count();
    font_px * GLYPH_ASPECT * chars as f32
}

/// Whether `text` fits comfortably inside a `width x height` pixel box.
///
/// The text may fill at most `MAX_FILL` of the width, and the box must be at
/// least one line tall, so labels never spill outside the outline they annotate.
#[must_use]
pub fn fits(text: &str, width: f32, height: f32, font_px: f32) -> bool {
    estimated_text_width(text, font_px) <= width * MAX_FILL && height >= font_px * 1.2
}

/// Places a centered label in every box big enough to carry its text.
///
/// Boxes whose text does not [`fits`] are skipped entirely; an overview crowded
/// with clipped names is worse than one with fewer, readable ones.
#[must_use]
pub fn place_box_labels(boxes: &[LabelBox], font_px: f32) -> Vec<PlacedLabel> {
    boxes
        .iter()
        .filter(|b| fits(&b.text, b.width, b.height, font_px))
        .map(|b| PlacedLabel {
            text: b.text.clone(),
            x: b.left + b.width * 0.5,
            y: b.top + b.height * 0.5,
        })
        .collect()
}

/// The screen anchor for a caption attached to a box spanning `left..right` at
/// `top`, kept below `canvas_top` so it never leaves the canvas.
///
/// The caption sits centered above the box with a small gap; when the box touches
/// the top of the canvas the caption flips to just below the box's top edge, so it
/// stays readable instead of being clipped.
#[must_use]
pub fn caption_anchor(
    left: f32,
    right: f32,
    top: f32,
    canvas_top: f32,
    font_px: f32,
) -> (f32, f32) {
    let x = f32::midpoint(left, right);
    let above = top - font_px * 0.9;
    if above - font_px * 0.5 < canvas_top {
        (x, top + font_px * 0.9)
    } else {
        (x, above)
    }
}

/// Converts a DBU span to microns, treating a non-positive resolution as `1`.
#[must_use]
pub fn dbu_to_microns(dbu: i64, dbu_per_micron: i64) -> f64 {
    dbu as f64 / dbu_per_micron.max(1) as f64
}

/// Formats a `width x height` extent in DBU with the micron equivalent.
///
/// Example: `800 x 600 DBU (0.80 x 0.60 um)`.
#[must_use]
pub fn dimension_text(width_dbu: i64, height_dbu: i64, dbu_per_micron: i64) -> String {
    format!(
        "{width_dbu} x {height_dbu} DBU ({:.2} x {:.2} um)",
        dbu_to_microns(width_dbu, dbu_per_micron),
        dbu_to_microns(height_dbu, dbu_per_micron)
    )
}

/// The caption for a single selected shape: its layer label plus live dimensions.
///
/// `layer_label` is the inspector-style `name (layer/datatype)` string.
#[must_use]
pub fn selection_caption(layer_label: &str, bounds: &Rect, dbu_per_micron: i64) -> String {
    format!(
        "{layer_label}  {}",
        dimension_text(bounds.width(), bounds.height(), dbu_per_micron)
    )
}

/// The caption for a multi-shape selection: the count plus the combined extent.
#[must_use]
pub fn multi_selection_caption(count: usize, bounds: &Rect, dbu_per_micron: i64) -> String {
    format!(
        "{count} shapes  {}",
        dimension_text(bounds.width(), bounds.height(), dbu_per_micron)
    )
}

/// The live caption while a measurement is in progress: deltas and distance from
/// the anchor point to the cursor, updated every frame.
#[must_use]
pub fn live_measure_caption(
    start: reticle_geometry::Point,
    cursor: reticle_geometry::Point,
    dbu_per_micron: i64,
) -> String {
    let m = crate::measure::Measurement::new(start, cursor, dbu_per_micron);
    format!(
        "dx {}, dy {}: {:.1} DBU ({:.3} um)",
        m.dx(),
        m.dy(),
        m.distance_dbu(),
        m.distance_microns()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::Point;

    fn label_box(text: &str, w: f32, h: f32) -> LabelBox {
        LabelBox {
            text: text.to_owned(),
            left: 100.0,
            top: 200.0,
            width: w,
            height: h,
        }
    }

    #[test]
    fn width_estimate_grows_with_text_and_font() {
        assert!(estimated_text_width("abcdef", 11.0) > estimated_text_width("abc", 11.0));
        assert!(estimated_text_width("abc", 22.0) > estimated_text_width("abc", 11.0));
        assert!((estimated_text_width("", 11.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn fit_rejects_narrow_and_short_boxes() {
        // A 10-char name needs roughly 68 px at 11 px; a 40 px box is too narrow.
        assert!(!fits("NAND2_CELL", 40.0, 50.0, LABEL_FONT_PX));
        // Wide enough but shorter than one line: rejected.
        assert!(!fits("NAND2_CELL", 400.0, 8.0, LABEL_FONT_PX));
        // Comfortable on both axes: accepted.
        assert!(fits("NAND2_CELL", 400.0, 50.0, LABEL_FONT_PX));
    }

    #[test]
    fn placement_centers_text_and_skips_small_boxes() {
        let boxes = vec![
            label_box("BIG_CELL", 300.0, 80.0),
            label_box("TINY", 10.0, 4.0),
        ];
        let placed = place_box_labels(&boxes, LABEL_FONT_PX);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].text, "BIG_CELL");
        assert!((placed[0].x - 250.0).abs() < 1e-3);
        assert!((placed[0].y - 240.0).abs() < 1e-3);
    }

    #[test]
    fn caption_anchor_sits_above_the_box() {
        let (x, y) = caption_anchor(100.0, 300.0, 500.0, 0.0, LABEL_FONT_PX);
        assert!((x - 200.0).abs() < 1e-3);
        assert!(y < 500.0, "caption should be above the box top");
    }

    #[test]
    fn caption_anchor_flips_below_at_the_canvas_top() {
        // Box top nearly at the canvas top: no room above, so anchor below it.
        let (_, y) = caption_anchor(0.0, 100.0, 22.0, 20.0, LABEL_FONT_PX);
        assert!(y > 22.0, "caption should flip below a clipped box top");
    }

    #[test]
    fn dimension_text_reports_both_units() {
        let s = dimension_text(800, 600, 1000);
        assert_eq!(s, "800 x 600 DBU (0.80 x 0.60 um)");
    }

    #[test]
    fn nonpositive_resolution_does_not_divide_by_zero() {
        assert!(dbu_to_microns(500, 0).is_finite());
        assert!((dbu_to_microns(500, 0) - 500.0).abs() < 1e-9);
        let s = dimension_text(10, 10, -3);
        assert!(s.contains("10 x 10 DBU"));
    }

    #[test]
    fn selection_caption_includes_layer_and_dimensions() {
        let bounds = Rect::new(Point::new(0, 0), Point::new(400, 300));
        let s = selection_caption("METAL1 (4/0)", &bounds, 1000);
        assert!(s.starts_with("METAL1 (4/0)"));
        assert!(s.contains("400 x 300 DBU"));
        assert!(s.contains("(0.40 x 0.30 um)"));
    }

    #[test]
    fn multi_selection_caption_includes_count() {
        let bounds = Rect::new(Point::new(0, 0), Point::new(1000, 2000));
        let s = multi_selection_caption(5, &bounds, 1000);
        assert!(s.starts_with("5 shapes"));
        assert!(s.contains("1000 x 2000 DBU"));
    }

    #[test]
    fn live_measure_caption_reports_deltas_and_distance() {
        let s = live_measure_caption(Point::new(0, 0), Point::new(3000, 4000), 1000);
        assert!(s.contains("dx 3000"));
        assert!(s.contains("dy 4000"));
        assert!(s.contains("5000.0 DBU"));
        assert!(s.contains("5.000 um"));
    }
}
