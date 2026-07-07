//! Per-layer area and perimeter over the flattened top cell.
//!
//! For each layer that carries geometry, [`report`] computes the exact union of
//! that layer's shapes and reports:
//!
//! - `area`: the covered area in DBU squared, overlaps counted once.
//! - `perimeter`: the total length of the union boundary in DBU, including the
//!   boundaries of any holes.
//! - `shape_count`: how many source shapes the flattened top cell placed on the
//!   layer (before the union).
//!
//! The union runs on the exact `i_overlay` integer engine through
//! [`reticle_geometry::polygon_boolean`], so area and perimeter are exact for
//! integer (manhattan) geometry. Area is an integer for such geometry; perimeter
//! is a length, so a diagonal edge contributes an irrational value carried in the
//! returned `f64`.

use std::collections::BTreeMap;

use reticle_geometry::{BooleanOp, LayerId, Polygon, polygon_boolean};
use reticle_model::Document;

use crate::polyize::shape_polygons;

/// Area and perimeter of one layer's flattened, unioned geometry.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct LayerMetrics {
    /// The GDSII layer/datatype this row measures.
    pub layer: LayerId,
    /// Union area in DBU squared, overlaps counted once.
    pub area: f64,
    /// Union boundary length in DBU, including hole boundaries.
    pub perimeter: f64,
    /// Number of source shapes on this layer in the flattened top cell.
    pub shape_count: usize,
}

/// Computes per-layer area and perimeter for a document's flattened top cell.
///
/// Returns one [`LayerMetrics`] per layer that carries at least one shape,
/// ordered by [`LayerId`]. A document with no declared top cell yields an empty
/// report.
#[must_use]
pub fn report(doc: &Document) -> Vec<LayerMetrics> {
    let Some(top) = crate::top_cell(doc) else {
        return Vec::new();
    };
    let flat = doc.flatten(top);

    // Group polygons and a raw shape count per layer.
    let mut by_layer: BTreeMap<LayerId, (Vec<Polygon>, usize)> = BTreeMap::new();
    for shape in &flat {
        let entry = by_layer.entry(shape.layer).or_default();
        entry.0.extend(shape_polygons(shape));
        entry.1 += 1;
    }

    by_layer
        .into_iter()
        .map(|(layer, (polys, shape_count))| {
            let union = polygon_boolean(BooleanOp::Union, &polys, &[]).unwrap_or_default();
            LayerMetrics {
                layer,
                area: union_area(&union),
                perimeter: union_perimeter(&union),
                shape_count,
            }
        })
        .collect()
}

/// Exact union area of a set of (possibly overlapping) polygons, in DBU squared.
///
/// Overlaps are counted once. Shared with the antenna check so per-net area sums
/// use the same exact boolean path as the per-layer report.
pub(crate) fn union_area_of(polys: &[Polygon]) -> f64 {
    let union = polygon_boolean(BooleanOp::Union, polys, &[]).unwrap_or_default();
    union_area(&union)
}

/// Exact area of a set of union contours (CCW outers positive, CW holes
/// negative), in DBU squared.
fn union_area(contours: &[Polygon]) -> f64 {
    let double: i128 = contours.iter().map(Polygon::signed_double_area).sum();
    // A well-formed union nets non-negative; clamp guards against numeric noise.
    double.max(0) as f64 / 2.0
}

/// Total boundary length of a set of union contours, in DBU.
fn union_perimeter(contours: &[Polygon]) -> f64 {
    contours.iter().map(ring_perimeter).sum()
}

/// Perimeter of a single closed ring.
fn ring_perimeter(ring: &Polygon) -> f64 {
    let v = ring.vertices();
    if v.len() < 2 {
        return 0.0;
    }
    let mut total = 0.0;
    for i in 0..v.len() {
        let a = v[i];
        let b = v[(i + 1) % v.len()];
        let dx = f64::from(b.x) - f64::from(a.x);
        let dy = f64::from(b.y) - f64::from(a.y);
        total += dx.hypot(dy);
    }
    total
}

#[cfg(test)]
mod tests {
    // These tests use integer (manhattan) geometry, whose area and perimeter are
    // exactly representable in f64, so exact equality is the correct assertion.
    #![allow(clippy::float_cmp)]

    use super::{LayerMetrics, report};
    use proptest::prelude::*;
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{Cell, Document, DrawShape, ShapeKind};

    const L: LayerId = LayerId::new(68, 20);

    fn doc_from_rects(rects: &[Rect]) -> Document {
        let mut cell = Cell::new("top");
        for r in rects {
            cell.shapes.push(DrawShape::new(L, ShapeKind::Rect(*r)));
        }
        let mut doc = Document::new();
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["top".into()]);
        doc
    }

    /// Brute-force union area and perimeter of axis-aligned rectangles by
    /// coordinate compression: an independent oracle for the boolean engine path.
    fn oracle(rects: &[Rect]) -> (i64, i64) {
        let rects: Vec<Rect> = rects.iter().copied().filter(|r| !r.is_empty()).collect();
        if rects.is_empty() {
            return (0, 0);
        }
        let mut xs: Vec<i64> = rects
            .iter()
            .flat_map(|r| [i64::from(r.min.x), i64::from(r.max.x)])
            .collect();
        let mut ys: Vec<i64> = rects
            .iter()
            .flat_map(|r| [i64::from(r.min.y), i64::from(r.max.y)])
            .collect();
        xs.sort_unstable();
        xs.dedup();
        ys.sort_unstable();
        ys.dedup();

        let nx = xs.len() - 1;
        let ny = ys.len() - 1;
        // covered[i][j] for cell [xs[i], xs[i+1]) x [ys[j], ys[j+1]).
        let mut covered = vec![vec![false; ny]; nx];
        for (i, cov_row) in covered.iter_mut().enumerate() {
            for (j, cell) in cov_row.iter_mut().enumerate() {
                let (cx, cy) = (xs[i], ys[j]);
                *cell = rects.iter().any(|r| {
                    i64::from(r.min.x) <= cx
                        && cx < i64::from(r.max.x)
                        && i64::from(r.min.y) <= cy
                        && cy < i64::from(r.max.y)
                });
            }
        }

        let mut area = 0i64;
        let mut perim = 0i64;
        for i in 0..nx {
            let dx = xs[i + 1] - xs[i];
            for j in 0..ny {
                if !covered[i][j] {
                    continue;
                }
                let dy = ys[j + 1] - ys[j];
                area += dx * dy;
                // A side is boundary when its neighbour cell is uncovered or off-grid.
                if i == 0 || !covered[i - 1][j] {
                    perim += dy;
                }
                if i + 1 == nx || !covered[i + 1][j] {
                    perim += dy;
                }
                if j == 0 || !covered[i][j - 1] {
                    perim += dx;
                }
                if j + 1 == ny || !covered[i][j + 1] {
                    perim += dx;
                }
            }
        }
        (area, perim)
    }

    #[test]
    fn empty_document_reports_nothing() {
        assert!(report(&Document::new()).is_empty());
    }

    #[test]
    fn single_rect_area_and_perimeter() {
        let doc = doc_from_rects(&[Rect::new(Point::new(0, 0), Point::new(4, 6))]);
        let rep = report(&doc);
        assert_eq!(
            rep,
            vec![LayerMetrics {
                layer: L,
                area: 24.0,
                perimeter: 20.0,
                shape_count: 1,
            }]
        );
    }

    #[test]
    fn overlap_is_counted_once() {
        let doc = doc_from_rects(&[
            Rect::new(Point::new(0, 0), Point::new(10, 10)),
            Rect::new(Point::new(5, 5), Point::new(15, 15)),
        ]);
        let rep = report(&doc);
        assert_eq!(rep.len(), 1);
        // Two 100-unit squares overlapping in a 25-unit square: 175, not 200.
        assert_eq!(rep[0].area, 175.0);
        assert_eq!(rep[0].shape_count, 2);
    }

    #[test]
    fn distinct_layers_report_separately() {
        let a = LayerId::new(66, 20);
        let b = LayerId::new(68, 20);
        let mut cell = Cell::new("top");
        cell.shapes.push(DrawShape::new(
            a,
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(2, 2))),
        ));
        cell.shapes.push(DrawShape::new(
            b,
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(3, 3))),
        ));
        let mut doc = Document::new();
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["top".into()]);
        let rep = report(&doc);
        assert_eq!(rep.len(), 2);
        assert_eq!(rep[0].layer, a); // ordered by LayerId
        assert_eq!(rep[0].area, 4.0);
        assert_eq!(rep[1].layer, b);
        assert_eq!(rep[1].area, 9.0);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn matches_bruteforce_oracle(
            rects in prop::collection::vec(
                (0i32..40, 0i32..40, 1i32..20, 1i32..20)
                    .prop_map(|(x, y, w, h)| Rect::new(
                        Point::new(x, y),
                        Point::new(x + w, y + h),
                    )),
                0..8,
            )
        ) {
            let doc = doc_from_rects(&rects);
            let rep = report(&doc);
            let (want_area, want_perim) = oracle(&rects);
            if rects.is_empty() {
                prop_assert!(rep.is_empty());
            } else {
                prop_assert_eq!(rep.len(), 1);
                // Integer manhattan geometry: values are exact in f64.
                prop_assert_eq!(rep[0].area, want_area as f64);
                prop_assert_eq!(rep[0].perimeter, want_perim as f64);
            }
        }
    }
}
