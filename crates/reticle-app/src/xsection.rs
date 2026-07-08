//! Cut-line cross-sections: pure 2D interval math plus the egui panel.
//!
//! Given a cut segment in plan view, the flattened shapes, and the layer z
//! slabs from [`reticle_render::layer_spans`], this module computes, per layer,
//! the intervals of the cut line covered by geometry, and draws them as
//! rectangles in elevation view: x is the distance along the cut line (DBU), y
//! is the slab `z_bottom..z_top`.
//!
//! All the math is CPU-side and unit-tested: rectangles are clipped with
//! Liang-Barsky, polygons with an even-odd edge-crossing walk (concave shapes
//! produce multiple intervals), and paths (wires) are approximated by one
//! oriented quad per centerline segment (round end caps are treated as square,
//! a conservative approximation). Overlapping intervals on the same layer are
//! merged, so abutting shapes read as one solid bar.

use eframe::egui;
use egui::{Align2, Color32, Pos2, Sense, Stroke, StrokeKind, Vec2};

use reticle_geometry::{Endcap, LayerId, Point, Rect};
use reticle_model::{DrawShape, ShapeKind, Technology};
use reticle_render::{LayerSpan, layer_spans};
use std::collections::HashMap;

use crate::layers::{self, LayerState};
use crate::theme::{
    self,
    tokens::{CANVAS, DARK},
};

/// One covered stretch of the cut line on one layer, in elevation coordinates.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SectionInterval {
    /// The layer the geometry lives on.
    pub layer: LayerId,
    /// Distance along the cut line where coverage starts, world units (DBU).
    pub start: f32,
    /// Distance along the cut line where coverage ends.
    pub end: f32,
    /// Bottom of the layer slab (same units as [`LayerSpan`]).
    pub z_bottom: f32,
    /// Top of the layer slab.
    pub z_top: f32,
}

/// Computes the per-layer cross-section of `shapes` along the segment `a -> b`.
///
/// Every shape whose layer has a slab in `spans` contributes the intervals
/// where the segment passes through its footprint; intervals on the same layer
/// are merged when they touch or overlap. The result is sorted by slab bottom,
/// then start distance. A degenerate (zero-length) segment yields nothing.
#[must_use]
pub fn cross_section(
    shapes: &[DrawShape],
    spans: &[LayerSpan],
    a: Point,
    b: Point,
) -> Vec<SectionInterval> {
    let length = (a.distance_squared(b) as f64).sqrt();
    if length <= 0.0 {
        return Vec::new();
    }
    let by_layer: HashMap<LayerId, (f32, f32)> = spans
        .iter()
        .map(|s| (s.layer, (s.z_bottom, s.z_top)))
        .collect();

    // Gather raw t-intervals per layer.
    let mut per_layer: HashMap<LayerId, Vec<(f64, f64)>> = HashMap::new();
    for shape in shapes {
        if !by_layer.contains_key(&shape.layer) {
            continue;
        }
        let crossings = shape_crossings(shape, a, b);
        if !crossings.is_empty() {
            per_layer.entry(shape.layer).or_default().extend(crossings);
        }
    }

    let mut out = Vec::new();
    for (layer, mut intervals) in per_layer {
        let (z_bottom, z_top) = by_layer[&layer];
        merge_intervals(&mut intervals);
        for (t0, t1) in intervals {
            out.push(SectionInterval {
                layer,
                start: (t0 * length) as f32,
                end: (t1 * length) as f32,
                z_bottom,
                z_top,
            });
        }
    }
    out.sort_by(|p, q| {
        p.z_bottom
            .total_cmp(&q.z_bottom)
            .then(p.start.total_cmp(&q.start))
            .then(p.layer.cmp(&q.layer))
    });
    out
}

/// The parameter intervals `t in [0, 1]` where the segment `a -> b` passes
/// through the footprint of `shape`.
fn shape_crossings(shape: &DrawShape, a: Point, b: Point) -> Vec<(f64, f64)> {
    match &shape.kind {
        ShapeKind::Rect(rect) => rect_crossing(*rect, a, b).into_iter().collect(),
        ShapeKind::Polygon(poly) => {
            let vertices: Vec<(f64, f64)> = poly
                .vertices()
                .iter()
                .map(|p| (f64::from(p.x), f64::from(p.y)))
                .collect();
            polygon_crossings(&vertices, a, b)
        }
        ShapeKind::Path(path) => path_crossings(path.points(), path.width(), path.endcap(), a, b),
    }
}

/// Liang-Barsky clip of the segment `a -> b` against an axis-aligned rectangle.
/// Returns the covered `t` interval, or `None` when the segment misses.
fn rect_crossing(rect: Rect, a: Point, b: Point) -> Option<(f64, f64)> {
    let ax = f64::from(a.x);
    let ay = f64::from(a.y);
    let dx = f64::from(b.x) - ax;
    let dy = f64::from(b.y) - ay;
    let (x0, y0) = (f64::from(rect.min.x), f64::from(rect.min.y));
    let (x1, y1) = (f64::from(rect.max.x), f64::from(rect.max.y));

    let mut t0 = 0.0f64;
    let mut t1 = 1.0f64;
    for (p, q) in [
        (-dx, ax - x0), // entering from the left edge
        (dx, x1 - ax),  // leaving through the right edge
        (-dy, ay - y0), // bottom
        (dy, y1 - ay),  // top
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None; // parallel and outside this slab
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
    }
    (t1 > t0).then_some((t0, t1))
}

/// Even-odd crossings of the segment `a -> b` through a (possibly concave)
/// polygon given as `(x, y)` vertices.
///
/// Candidate parameters are the segment's endpoints plus every proper edge
/// intersection; each gap between consecutive candidates is classified by
/// testing its midpoint against the polygon, so a segment that enters and
/// leaves several times yields several intervals and a segment that starts or
/// ends inside is handled naturally.
fn polygon_crossings(vertices: &[(f64, f64)], a: Point, b: Point) -> Vec<(f64, f64)> {
    if vertices.len() < 3 {
        return Vec::new();
    }
    let ax = f64::from(a.x);
    let ay = f64::from(a.y);
    let dx = f64::from(b.x) - ax;
    let dy = f64::from(b.y) - ay;

    let mut ts = vec![0.0f64, 1.0];
    let count = vertices.len();
    for i in 0..count {
        let (px, py) = vertices[i];
        let (qx, qy) = vertices[(i + 1) % count];
        let ex = qx - px;
        let ey = qy - py;
        let denom = dx * ey - dy * ex;
        if denom == 0.0 {
            continue; // parallel (collinear overlap contributes via midpoints)
        }
        let cut_t = ((px - ax) * ey - (py - ay) * ex) / denom;
        let edge_u = ((px - ax) * dy - (py - ay) * dx) / denom;
        if (0.0..=1.0).contains(&cut_t) && (0.0..=1.0).contains(&edge_u) {
            ts.push(cut_t);
        }
    }
    ts.sort_by(f64::total_cmp);
    ts.dedup_by(|p, q| (*p - *q).abs() < 1e-12);

    let mut out = Vec::new();
    for pair in ts.windows(2) {
        let (t0, t1) = (pair[0], pair[1]);
        if t1 - t0 <= 1e-12 {
            continue;
        }
        let tm = f64::midpoint(t0, t1);
        if point_in_polygon(ax + tm * dx, ay + tm * dy, vertices) {
            out.push((t0, t1));
        }
    }
    merge_intervals(&mut out);
    out
}

/// Even-odd point-in-polygon test.
fn point_in_polygon(x: f64, y: f64, vertices: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let n = vertices.len();
    for i in 0..n {
        let (xi, yi) = vertices[i];
        let (xj, yj) = vertices[(i + 1) % n];
        if (yi > y) != (yj > y) {
            let x_cross = xi + (y - yi) * (xj - xi) / (yj - yi);
            if x < x_cross {
                inside = !inside;
            }
        }
    }
    inside
}

/// Crossings through a path (wire) footprint: one oriented quad per centerline
/// segment, `width / 2` on each side, with square/custom caps extending the
/// run's free ends (round caps are approximated as square).
fn path_crossings(
    points: &[Point],
    width: i32,
    endcap: Endcap,
    a: Point,
    b: Point,
) -> Vec<(f64, f64)> {
    if points.len() < 2 || width <= 0 {
        return Vec::new();
    }
    let half = f64::from(width) / 2.0;
    let extension = match endcap {
        Endcap::Flat => 0.0,
        Endcap::Square | Endcap::Round => half,
        Endcap::Custom(e) => f64::from(e),
    };

    let mut out = Vec::new();
    let last_segment = points.len() - 2;
    for (i, pair) in points.windows(2).enumerate() {
        let (p0, p1) = (pair[0], pair[1]);
        let mut x0 = f64::from(p0.x);
        let mut y0 = f64::from(p0.y);
        let mut x1 = f64::from(p1.x);
        let mut y1 = f64::from(p1.y);
        let len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
        if len <= 0.0 {
            continue;
        }
        let ux = (x1 - x0) / len;
        let uy = (y1 - y0) / len;
        // Extend the free ends by the cap.
        if i == 0 {
            x0 -= ux * extension;
            y0 -= uy * extension;
        }
        if i == last_segment {
            x1 += ux * extension;
            y1 += uy * extension;
        }
        // Perpendicular half-width offset.
        let nx = -uy * half;
        let ny = ux * half;
        let quad = [
            (x0 + nx, y0 + ny),
            (x1 + nx, y1 + ny),
            (x1 - nx, y1 - ny),
            (x0 - nx, y0 - ny),
        ];
        out.extend(polygon_crossings(&quad, a, b));
    }
    merge_intervals(&mut out);
    out
}

/// Sorts intervals and merges any that touch or overlap, in place.
fn merge_intervals(intervals: &mut Vec<(f64, f64)>) {
    if intervals.len() <= 1 {
        return;
    }
    intervals.sort_by(|p, q| p.0.total_cmp(&q.0));
    let mut merged: Vec<(f64, f64)> = Vec::with_capacity(intervals.len());
    for &(start, end) in intervals.iter() {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    *intervals = merged;
}

/// Renders the "Cross-section" elevation view into `ui`: the managed panel body
/// (ADR 0096; a floating `egui::Window` before lane 2c re-hosted it).
///
/// With no completed cut line it shows a hint; otherwise it computes the layer
/// slabs and intervals for the current scene (cheap, per frame) and paints the
/// elevation view. Layers hidden in the layer panel are omitted.
pub fn panel(
    ui: &mut egui::Ui,
    cut: Option<(Point, Point)>,
    shapes: &[DrawShape],
    tech: &Technology,
    layers: &LayerState,
) {
    let Some((a, b)) = cut else {
        ui.label("Pick two points with the Cut tool to slice the layout.");
        return;
    };
    ui.label(format!("Cut ({}, {}) -> ({}, {}) DBU", a.x, a.y, b.x, b.y));
    let spans = layer_spans(tech, shapes);
    let mut intervals = cross_section(shapes, &spans, a, b);
    intervals.retain(|i| layers.is_visible(i.layer));
    if intervals.is_empty() {
        ui.label("The cut line crosses no visible geometry.");
        return;
    }
    let length = (a.distance_squared(b) as f64).sqrt() as f32;
    draw_section(ui, &intervals, length, layers);
}

/// Paints the elevation view: one filled rectangle per interval, with distance
/// and z axis labels.
fn draw_section(
    ui: &mut egui::Ui,
    intervals: &[SectionInterval],
    length: f32,
    layers: &LayerState,
) {
    let size = ui.available_size().max(Vec2::new(240.0, 140.0));
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, DARK.bg_canvas);

    // Elevation extents, padded so slabs never touch the frame.
    let z_min = intervals
        .iter()
        .map(|i| i.z_bottom)
        .fold(f32::INFINITY, f32::min);
    let z_max = intervals
        .iter()
        .map(|i| i.z_top)
        .fold(f32::NEG_INFINITY, f32::max);
    let z_pad = ((z_max - z_min) * 0.08).max(1.0);
    let (z_min, z_max) = (z_min - z_pad, z_max + z_pad);

    // Plot area inside the label margins.
    let left = rect.min.x + 8.0;
    let right = rect.max.x - 8.0;
    let top = rect.min.y + 6.0;
    let bottom = rect.max.y - 18.0;
    let plot_w = (right - left).max(1.0);
    let plot_h = (bottom - top).max(1.0);

    let sx = |distance: f32| left + (distance / length.max(1.0)) * plot_w;
    let sy = |z: f32| bottom - ((z - z_min) / (z_max - z_min)) * plot_h;

    for interval in intervals {
        let color = layer_color(layers, interval.layer);
        let x0 = sx(interval.start);
        let x1 = sx(interval.end).max(x0 + 1.0); // thin crossings stay visible
        let y0 = sy(interval.z_top);
        let y1 = sy(interval.z_bottom).max(y0 + 1.0);
        let bar = egui::Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1));
        painter.rect_filled(bar, 0.0, color);
        painter.rect_stroke(
            bar,
            0.0,
            Stroke::new(1.0, color.gamma_multiply(1.4)),
            StrokeKind::Middle,
        );
    }

    // Axis labels: distance along the cut and the z range.
    let label = CANVAS.hud_label;
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    painter.text(
        Pos2::new(left, rect.max.y - 2.0),
        Align2::LEFT_BOTTOM,
        "0",
        font.clone(),
        label,
    );
    painter.text(
        Pos2::new(right, rect.max.y - 2.0),
        Align2::RIGHT_BOTTOM,
        format!("{length:.0} DBU"),
        font.clone(),
        label,
    );
    painter.text(
        Pos2::new(left, top),
        Align2::LEFT_TOP,
        format!("z {z_max:.0}"),
        font.clone(),
        label,
    );
    painter.text(
        Pos2::new(left, bottom),
        Align2::LEFT_BOTTOM,
        format!("z {z_min:.0}"),
        font,
        label,
    );
}

/// The fill color for a layer's section bar, from the layer table (or gray).
fn layer_color(layers: &LayerState, id: LayerId) -> Color32 {
    layers.rows().iter().find(|r| r.id == id).map_or(
        theme::tokens::with_alpha(CANVAS.layer_fallback, 220),
        |r| {
            let (red, green, blue, _) = layers::rgba_components(r.color_rgba);
            theme::tokens::layer_rgba(red, green, blue, 220)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::{Path, Polygon};

    const LAYER_A: LayerId = LayerId::new(1, 0);
    const LAYER_B: LayerId = LayerId::new(2, 0);

    fn span(layer: LayerId, z_bottom: f32, z_top: f32) -> LayerSpan {
        LayerSpan {
            layer,
            z_bottom,
            z_top,
        }
    }

    fn rect_shape(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    }

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn rect_crossing_projects_onto_the_cut_axis() {
        let shapes = [rect_shape(LAYER_A, 10, 0, 20, 10)];
        let spans = [span(LAYER_A, 5.0, 8.0)];
        // Horizontal cut through the middle of the rect.
        let got = cross_section(&shapes, &spans, Point::new(0, 5), Point::new(30, 5));
        assert_eq!(got.len(), 1);
        let i = got[0];
        assert_eq!(i.layer, LAYER_A);
        assert!(approx(i.start, 10.0), "start {}", i.start);
        assert!(approx(i.end, 20.0), "end {}", i.end);
        assert!(approx(i.z_bottom, 5.0) && approx(i.z_top, 8.0));
    }

    #[test]
    fn miss_and_degenerate_cut_produce_nothing() {
        let shapes = [rect_shape(LAYER_A, 10, 0, 20, 10)];
        let spans = [span(LAYER_A, 0.0, 1.0)];
        // Passes above the rectangle.
        assert!(cross_section(&shapes, &spans, Point::new(0, 50), Point::new(30, 50)).is_empty());
        // Zero-length cut.
        assert!(cross_section(&shapes, &spans, Point::new(5, 5), Point::new(5, 5)).is_empty());
        // Layer without a span is skipped.
        assert!(cross_section(&shapes, &[], Point::new(0, 5), Point::new(30, 5)).is_empty());
    }

    #[test]
    fn cut_starting_inside_a_shape_begins_at_zero() {
        let shapes = [rect_shape(LAYER_A, 10, 0, 20, 10)];
        let spans = [span(LAYER_A, 0.0, 1.0)];
        // Starts at (15, 5), inside the rect, and leaves through x = 20.
        let got = cross_section(&shapes, &spans, Point::new(15, 5), Point::new(30, 5));
        assert_eq!(got.len(), 1);
        assert!(approx(got[0].start, 0.0), "starts covered");
        assert!(approx(got[0].end, 5.0), "leaves after 5 DBU");
    }

    #[test]
    fn concave_polygon_yields_two_intervals() {
        // A U shape: two 10-wide towers joined by a base below y = 10; a cut at
        // y = 15 crosses both towers but not the gap between them.
        let u = Polygon::new(vec![
            Point::new(0, 0),
            Point::new(40, 0),
            Point::new(40, 30),
            Point::new(30, 30),
            Point::new(30, 10),
            Point::new(10, 10),
            Point::new(10, 30),
            Point::new(0, 30),
        ]);
        let shapes = [DrawShape::new(LAYER_A, ShapeKind::Polygon(u))];
        let spans = [span(LAYER_A, 0.0, 2.0)];
        let got = cross_section(&shapes, &spans, Point::new(-5, 15), Point::new(45, 15));
        assert_eq!(got.len(), 2, "the cut crosses the two towers: {got:?}");
        assert!(approx(got[0].start, 5.0) && approx(got[0].end, 15.0));
        assert!(approx(got[1].start, 35.0) && approx(got[1].end, 45.0));
    }

    #[test]
    fn path_footprint_is_the_widened_centerline() {
        // A horizontal wire 4 wide from (0,0) to (20,0); a vertical cut at
        // x = 10 crosses its footprint y in [-2, 2].
        let wire = Path::new(vec![Point::new(0, 0), Point::new(20, 0)], 4, Endcap::Flat);
        let shapes = [DrawShape::new(LAYER_A, ShapeKind::Path(wire))];
        let spans = [span(LAYER_A, 0.0, 1.0)];
        let got = cross_section(&shapes, &spans, Point::new(10, -10), Point::new(10, 10));
        assert_eq!(got.len(), 1);
        assert!(approx(got[0].start, 8.0), "start {}", got[0].start);
        assert!(approx(got[0].end, 12.0), "end {}", got[0].end);
    }

    #[test]
    fn multiple_layers_carry_their_own_stack_z() {
        let shapes = [
            rect_shape(LAYER_B, 0, 0, 30, 10), // upper slab
            rect_shape(LAYER_A, 5, 0, 25, 10), // lower slab
        ];
        let spans = [span(LAYER_A, 0.0, 10.0), span(LAYER_B, 20.0, 35.0)];
        let got = cross_section(&shapes, &spans, Point::new(0, 5), Point::new(30, 5));
        assert_eq!(got.len(), 2);
        // Sorted by slab bottom: A first.
        assert_eq!(got[0].layer, LAYER_A);
        assert!(approx(got[0].z_bottom, 0.0) && approx(got[0].z_top, 10.0));
        assert!(approx(got[0].start, 5.0) && approx(got[0].end, 25.0));
        assert_eq!(got[1].layer, LAYER_B);
        assert!(approx(got[1].z_bottom, 20.0) && approx(got[1].z_top, 35.0));
        assert!(approx(got[1].start, 0.0) && approx(got[1].end, 30.0));
    }

    #[test]
    fn abutting_shapes_merge_into_one_interval() {
        let shapes = [
            rect_shape(LAYER_A, 0, 0, 10, 10),
            rect_shape(LAYER_A, 10, 0, 20, 10), // shares the x = 10 edge
        ];
        let spans = [span(LAYER_A, 0.0, 1.0)];
        let got = cross_section(&shapes, &spans, Point::new(0, 5), Point::new(20, 5));
        assert_eq!(got.len(), 1, "touching intervals merge: {got:?}");
        assert!(approx(got[0].start, 0.0) && approx(got[0].end, 20.0));
    }

    #[test]
    fn diagonal_cut_reports_distances_not_x_coordinates() {
        // Cut along the diagonal of a square: the crossing length is the
        // diagonal length, not the axis span.
        let shapes = [rect_shape(LAYER_A, 0, 0, 10, 10)];
        let spans = [span(LAYER_A, 0.0, 1.0)];
        let got = cross_section(&shapes, &spans, Point::new(0, 0), Point::new(10, 10));
        assert_eq!(got.len(), 1);
        let diag = (200.0f32).sqrt();
        assert!(approx(got[0].start, 0.0));
        assert!((got[0].end - diag).abs() < 1e-2, "end {}", got[0].end);
    }

    /// The committed SKY130 technology file: 1000 DBU per micron, so every
    /// stack nanometer is exactly one world unit.
    const SKY130: &str = include_str!("../../../tech/sky130.tech");

    #[test]
    fn sky130_cut_across_met1_and_met2_lands_in_their_z_bands() {
        let tech = reticle_io::parse_technology(SKY130).expect("committed sky130.tech parses");
        let met1 = LayerId::new(68, 20);
        let met2 = LayerId::new(69, 20);
        // Two overlapping straps: met1 runs x 0..3000 DBU, met2 x 2000..5000.
        let shapes = [
            rect_shape(met1, 0, -200, 3000, 200),
            rect_shape(met2, 2000, -200, 5000, 200),
        ];
        let spans = layer_spans(&tech, &shapes);
        // A cut from x = -1000 to x = 6000 along y = 0 crosses both straps.
        let got = cross_section(&shapes, &spans, Point::new(-1000, 0), Point::new(6000, 0));
        assert_eq!(got.len(), 2, "one interval per metal: {got:?}");

        // Sorted by slab bottom: met1 first, at its physical band from the
        // `stack 68 20 1376 360` directive; met2 above from `stack 69 20 2006
        // 360`. Distances are measured from the cut start, not x coordinates.
        assert_eq!(got[0].layer, met1);
        assert!(approx(got[0].z_bottom, 1376.0) && approx(got[0].z_top, 1736.0));
        assert!(approx(got[0].start, 1000.0) && approx(got[0].end, 4000.0));
        assert_eq!(got[1].layer, met2);
        assert!(approx(got[1].z_bottom, 2006.0) && approx(got[1].z_top, 2366.0));
        assert!(approx(got[1].start, 3000.0) && approx(got[1].end, 6000.0));

        // Where the straps overlap in plan view (x 2000..3000) the metals stay
        // separated in elevation: met1's top sits below met2's bottom.
        assert!(got[0].z_top < got[1].z_bottom, "real stack keeps a gap");
    }
}
