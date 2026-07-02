//! Viewport culling and level-of-detail selection.
//!
//! Drawing every shape of a large hierarchical layout every frame is wasteful, so
//! the canvas culls to the visible world rectangle. This module owns the pure
//! culling logic, decoupled from egui:
//!
//! * [`SceneIndex`] bulk-loads the flattened top-cell shapes into a spatial index
//!   ([`reticle_index::RTreeIndex`]) and answers viewport queries.
//! * [`lod_for_zoom`] picks a [`DetailLevel`]: at low zoom the canvas draws cell
//!   bounding boxes instead of individual shapes.
//! * [`visible_cell_boxes`] returns the per-instance cell bounding boxes that
//!   overlap the viewport, for the low-detail path.
//!
//! The index stores shape *indices* into the flattened list so callers can recover
//! the original [`DrawShape`] and its layer for drawing and hit-testing.

use reticle_geometry::{Rect, Shape, SpatialIndex};
use reticle_index::RTreeIndex;
use reticle_model::{Document, DrawShape};

/// The level of detail to draw the scene at, chosen from the zoom level.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DetailLevel {
    /// Draw individual shapes (high zoom): the normal path.
    Shapes,
    /// Draw only cell bounding boxes (low zoom): the level-of-detail path, where
    /// individual shapes would be smaller than a pixel.
    CellBoxes,
}

/// The zoom, in pixels per DBU, at or below which the canvas switches to the
/// cell-bounding-box level of detail.
///
/// At this zoom a 1000-DBU cell spans a few pixels, so drawing its internal shapes
/// would be indistinguishable from drawing its outline.
const LOD_THRESHOLD_PPD: f64 = 0.02;

/// Chooses the detail level for a given zoom.
///
/// Returns [`DetailLevel::CellBoxes`] when zoomed far enough out that individual
/// shapes are sub-pixel, and [`DetailLevel::Shapes`] otherwise.
#[must_use]
pub fn lod_for_zoom(pixels_per_dbu: f64) -> DetailLevel {
    if pixels_per_dbu <= LOD_THRESHOLD_PPD {
        DetailLevel::CellBoxes
    } else {
        DetailLevel::Shapes
    }
}

/// The projected extent, in pixels, at or below which a single cell placement is drawn
/// as its bounding-box quad rather than its tessellated shapes.
///
/// At roughly one pixel a cell's internal geometry cannot be told apart from a filled
/// outline, so drawing the whole tessellated chunk is wasted work; the draw list swaps
/// in the cell's bounding-box quad instead. This is the per-cell analogue of the global
/// [`lod_for_zoom`] switch.
const LOD_MIN_PROJECTED_PIXELS: f64 = 1.0;

/// Chooses the detail level for a single cell placement, given its bounding box in
/// top-cell DBU and the current zoom.
///
/// A chunk is drawn as its bounding-box quad ([`DetailLevel::CellBoxes`]) when either
/// the global zoom is already in the cell-box regime ([`lod_for_zoom`]) or this cell's
/// own projected extent is at or below one pixel; otherwise its tessellated shapes are
/// drawn ([`DetailLevel::Shapes`]). Keying on the cell's own extent means a tiny cell
/// drops to a box before a large one does at the same zoom.
///
/// The projected extent is the larger of the box's width and height scaled by
/// `pixels_per_dbu`. A degenerate (empty or non-positive-area) box or a non-positive
/// zoom is treated as sub-pixel and drawn as a box.
#[must_use]
pub fn chunk_lod(bbox: Rect, pixels_per_dbu: f64) -> DetailLevel {
    if lod_for_zoom(pixels_per_dbu) == DetailLevel::CellBoxes {
        return DetailLevel::CellBoxes;
    }
    if pixels_per_dbu <= 0.0 {
        return DetailLevel::CellBoxes;
    }
    let extent_dbu = bbox.width().max(bbox.height()) as f64;
    let projected = extent_dbu * pixels_per_dbu;
    if projected <= LOD_MIN_PROJECTED_PIXELS {
        DetailLevel::CellBoxes
    } else {
        DetailLevel::Shapes
    }
}

/// A spatial index over a flattened top cell, for viewport culling and hit-testing.
///
/// The index holds the flattened [`DrawShape`] list plus an R-tree keyed on each
/// shape's bounding box; queries return indices into the list.
#[derive(Debug)]
pub struct SceneIndex {
    shapes: Vec<DrawShape>,
    index: RTreeIndex<usize>,
    bounds: Option<Rect>,
}

impl SceneIndex {
    /// Flattens `top` of `doc` and bulk-loads its shapes into a spatial index.
    ///
    /// If `top` is missing or empty the index is empty and every query returns
    /// nothing.
    #[must_use]
    pub fn build(doc: &Document, top: &str) -> Self {
        let shapes = doc.flatten(top);
        Self::from_shapes(shapes)
    }

    /// Builds an index directly from a flattened shape list (used by tests).
    #[must_use]
    pub fn from_shapes(shapes: Vec<DrawShape>) -> Self {
        let entries: Vec<(Rect, usize)> = shapes
            .iter()
            .enumerate()
            .map(|(i, s)| (s.bounding_box(), i))
            .collect();
        let bounds = entries.iter().map(|(r, _)| *r).reduce(|a, b| a.union(&b));
        let index = RTreeIndex::bulk_load(entries);
        Self {
            shapes,
            index,
            bounds,
        }
    }

    /// All flattened shapes, indexed by the values returned from queries.
    #[must_use]
    pub fn shapes(&self) -> &[DrawShape] {
        &self.shapes
    }

    /// The number of shapes in the scene.
    #[must_use]
    pub fn len(&self) -> usize {
        self.shapes.len()
    }

    /// Whether the scene has no shapes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    /// The bounding box of the whole scene, or `None` if empty.
    #[must_use]
    pub fn bounds(&self) -> Option<Rect> {
        self.bounds
    }

    /// Returns the indices of every shape whose bounding box intersects `viewport`.
    ///
    /// This is the per-frame cull: feed it the camera's visible world rectangle and
    /// only draw the shapes it returns. The result is sorted so drawing order (and
    /// tests) are deterministic regardless of index internals.
    #[must_use]
    pub fn query(&self, viewport: Rect) -> Vec<usize> {
        let mut hits: Vec<usize> = self
            .index
            .query_rect(viewport)
            .into_iter()
            .copied()
            .collect();
        hits.sort_unstable();
        hits
    }

    /// The topmost shape whose bounding box contains `point`, if any.
    ///
    /// "Topmost" is the last shape in draw order (highest index), matching the way
    /// later shapes paint over earlier ones. Used for click-to-select.
    #[must_use]
    pub fn pick(&self, point: reticle_geometry::Point) -> Option<usize> {
        let probe = Rect::new(point, point.translate(1, 1));
        self.index
            .query_rect(probe)
            .into_iter()
            .copied()
            .filter(|&i| self.shapes[i].bounding_box().contains(point))
            .max()
    }
}

/// A single cell placement's bounding box in top-cell coordinates, tagged with the
/// cell name, for the low-detail (cell-box) draw path.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CellBox {
    /// The placed cell's name.
    pub cell: String,
    /// The placement's bounding box in top-cell DBU coordinates.
    pub bbox: Rect,
}

/// Returns the bounding boxes of the top cell's direct instances and array
/// placements that overlap `viewport`.
///
/// This is the level-of-detail geometry: instead of the (possibly millions of)
/// leaf shapes, the canvas draws one outline per placement. Array placements are
/// expanded to one box per element, but only those intersecting the viewport are
/// returned, so the cost is bounded by what is on screen.
#[must_use]
pub fn visible_cell_boxes(doc: &Document, top: &str, viewport: Rect) -> Vec<CellBox> {
    let Some(cell) = doc.cell(top) else {
        return Vec::new();
    };
    let mut out = Vec::new();

    for inst in &cell.instances {
        if let Some(child) = doc.cell_bbox(&inst.cell) {
            let placed = transform_rect(&inst.transform, child);
            if placed.intersects(&viewport) {
                out.push(CellBox {
                    cell: inst.cell.clone(),
                    bbox: placed,
                });
            }
        }
    }

    for array in &cell.arrays {
        let Some(child) = doc.cell_bbox(&array.cell) else {
            continue;
        };
        let base = transform_rect(&array.transform, child);
        for col in 0..array.columns {
            let dx = array.column_pitch.saturating_mul(clamp_span(col));
            for row in 0..array.rows {
                let dy = array.row_pitch.saturating_mul(clamp_span(row));
                let bbox = Rect::new(base.min.translate(dx, dy), base.max.translate(dx, dy));
                if bbox.intersects(&viewport) {
                    out.push(CellBox {
                        cell: array.cell.clone(),
                        bbox,
                    });
                }
            }
        }
    }

    out
}

/// The offset multiplier for the `index`-th element of an array, clamped to range.
fn clamp_span(index: u32) -> i32 {
    i32::try_from(index).unwrap_or(i32::MAX)
}

/// Bounding box of `rect` after applying a placement `transform` (exact for the
/// dihedral orientations and integer magnifications placements use).
fn transform_rect(transform: &reticle_geometry::Transform, rect: Rect) -> Rect {
    use reticle_geometry::Point;
    let corners = [
        rect.min,
        Point::new(rect.max.x, rect.min.y),
        rect.max,
        Point::new(rect.min.x, rect.max.y),
    ];
    Rect::from_points(corners.into_iter().map(|c| transform.apply(c))).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use reticle_geometry::{LayerId, Point};
    use reticle_model::ShapeKind;

    fn rect_shape(x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            LayerId::new(1, 0),
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    }

    #[test]
    fn lod_switches_at_low_zoom() {
        assert_eq!(lod_for_zoom(1.0), DetailLevel::Shapes);
        assert_eq!(lod_for_zoom(0.5), DetailLevel::Shapes);
        assert_eq!(lod_for_zoom(0.001), DetailLevel::CellBoxes);
        assert_eq!(lod_for_zoom(LOD_THRESHOLD_PPD), DetailLevel::CellBoxes);
    }

    #[test]
    fn chunk_lod_switches_on_projected_extent() {
        // A 1000-DBU cell at 1 px/DBU projects to 1000 px: well above threshold, so
        // its shapes are drawn.
        let cell = Rect::new(Point::new(0, 0), Point::new(1000, 1000));
        assert_eq!(chunk_lod(cell, 1.0), DetailLevel::Shapes);

        // The same cell at 0.0005 px/DBU projects to 0.5 px (sub-pixel), so it drops to
        // a bounding-box quad. (This zoom is below the global threshold too, but the
        // point is the per-cell extent decision.)
        assert_eq!(chunk_lod(cell, 0.0005), DetailLevel::CellBoxes);

        // Boundary at 1 projected pixel, isolated from the global zoom rule by keeping
        // the zoom (0.05) above LOD_THRESHOLD_PPD (0.02). A 20-DBU cell at 0.05 px/DBU
        // projects to exactly 1.0 px, which counts as sub-pixel (inclusive) => box.
        let small = Rect::new(Point::new(0, 0), Point::new(20, 20));
        assert_eq!(chunk_lod(small, 0.05), DetailLevel::CellBoxes);
        // A 40-DBU cell at the same zoom projects to 2.0 px => shapes.
        let small2 = Rect::new(Point::new(0, 0), Point::new(40, 40));
        assert_eq!(chunk_lod(small2, 0.05), DetailLevel::Shapes);
    }

    #[test]
    fn chunk_lod_respects_global_zoom_and_degenerate_inputs() {
        let big = Rect::new(Point::new(0, 0), Point::new(1_000_000, 1_000_000));
        // Even a huge cell is drawn as a box once the global zoom is in the cell-box
        // regime (at or below LOD_THRESHOLD_PPD), matching `lod_for_zoom`.
        assert_eq!(lod_for_zoom(LOD_THRESHOLD_PPD), DetailLevel::CellBoxes);
        assert_eq!(chunk_lod(big, LOD_THRESHOLD_PPD), DetailLevel::CellBoxes);
        // Above the global threshold the large cell's shapes are drawn.
        assert_eq!(chunk_lod(big, 1.0), DetailLevel::Shapes);

        // A degenerate (empty) box or non-positive zoom is treated as sub-pixel.
        let empty = Rect::new(Point::new(5, 5), Point::new(5, 5));
        assert_eq!(chunk_lod(empty, 1.0), DetailLevel::CellBoxes);
        assert_eq!(chunk_lod(big, 0.0), DetailLevel::CellBoxes);
    }

    /// A tiny anchor: a chunk that is a box under the per-cell rule but not under the
    /// global zoom rule proves the two decisions are independent (the per-cell extent
    /// can trigger LOD even when the global zoom would still draw shapes).
    #[test]
    fn chunk_lod_per_cell_extent_is_independent_of_global_zoom() {
        // 10-DBU cell at 0.05 px/DBU: global zoom 0.05 > 0.02 threshold => Shapes
        // globally, but projected extent is 0.5 px => the chunk still drops to a box.
        let tiny = Rect::new(Point::new(0, 0), Point::new(10, 10));
        assert_eq!(lod_for_zoom(0.05), DetailLevel::Shapes);
        assert_eq!(chunk_lod(tiny, 0.05), DetailLevel::CellBoxes);
    }

    #[test]
    fn query_returns_only_intersecting_shapes() {
        let shapes = vec![
            rect_shape(0, 0, 100, 100),
            rect_shape(1000, 1000, 1100, 1100),
            rect_shape(5000, 5000, 5100, 5100),
        ];
        let scene = SceneIndex::from_shapes(shapes);
        let hits = scene.query(Rect::new(Point::new(-50, -50), Point::new(1200, 1200)));
        assert_eq!(hits, vec![0, 1]);
        let none = scene.query(Rect::new(
            Point::new(20000, 20000),
            Point::new(20100, 20100),
        ));
        assert!(none.is_empty());
    }

    #[test]
    fn query_all_returns_full_scene() {
        let shapes = vec![rect_shape(0, 0, 10, 10), rect_shape(90, 90, 110, 110)];
        let scene = SceneIndex::from_shapes(shapes);
        let all = scene.query(Rect::new(Point::new(-1000, -1000), Point::new(1000, 1000)));
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn pick_returns_topmost_shape() {
        // Two overlapping shapes; the later one (index 1) is on top.
        let shapes = vec![rect_shape(0, 0, 1000, 1000), rect_shape(0, 0, 500, 500)];
        let scene = SceneIndex::from_shapes(shapes);
        assert_eq!(scene.pick(Point::new(100, 100)), Some(1));
        // Outside the small one but inside the big one.
        assert_eq!(scene.pick(Point::new(800, 800)), Some(0));
        // Outside both.
        assert_eq!(scene.pick(Point::new(5000, 5000)), None);
    }

    #[test]
    fn scene_bounds_span_all_shapes() {
        let shapes = vec![rect_shape(0, 0, 100, 100), rect_shape(900, 900, 1000, 1000)];
        let scene = SceneIndex::from_shapes(shapes);
        let b = scene.bounds().expect("non-empty");
        assert_eq!(b.min, Point::new(0, 0));
        assert_eq!(b.max, Point::new(1000, 1000));
    }

    #[test]
    fn cell_boxes_cull_to_viewport() {
        let doc = demo::demo_document();
        let full = doc.cell_bbox(demo::TOP_CELL).unwrap();
        // The whole design: should see many placements (3 instances + 48 array).
        let all = visible_cell_boxes(&doc, demo::TOP_CELL, full);
        assert!(
            all.len() >= 3 + 48,
            "expected all placements, got {}",
            all.len()
        );
        // A tiny viewport far outside the design should see nothing.
        let empty = visible_cell_boxes(
            &doc,
            demo::TOP_CELL,
            Rect::new(Point::new(-100_000, -100_000), Point::new(-99_000, -99_000)),
        );
        assert!(empty.is_empty());
    }

    #[test]
    fn build_from_document_indexes_flattened_shapes() {
        let doc = demo::demo_document();
        let scene = SceneIndex::build(&doc, demo::TOP_CELL);
        assert_eq!(scene.len(), doc.flatten(demo::TOP_CELL).len());
        assert!(!scene.is_empty());
    }
}
