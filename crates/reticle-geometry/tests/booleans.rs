//! Boolean-operation tests: exact unit cases plus a property test comparing the
//! `i_overlay`-backed engine against an independent winding-number oracle.

use proptest::prelude::*;
use reticle_geometry::{BooleanOp, Point, Polygon, Rect, offset, polygon_boolean};

fn rect_poly(x0: i32, y0: i32, x1: i32, y1: i32) -> Polygon {
    Polygon::from_rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1)))
}

/// Net signed area, doubled (exact integer). Outer contours are positive, holes
/// negative, so this is the true filled area times two.
fn net_double_area(polys: &[Polygon]) -> i128 {
    polys.iter().map(Polygon::signed_double_area).sum()
}

#[test]
fn union_of_overlapping_squares() {
    let a = rect_poly(0, 0, 2, 2);
    let b = rect_poly(1, 1, 3, 3);
    let result = polygon_boolean(BooleanOp::Union, &[a], &[b]).unwrap();
    // 4 + 4 - 1 = 7, doubled = 14.
    assert_eq!(net_double_area(&result), 14);
}

#[test]
fn intersection_of_overlapping_squares() {
    let a = rect_poly(0, 0, 2, 2);
    let b = rect_poly(1, 1, 3, 3);
    let result = polygon_boolean(BooleanOp::Intersection, &[a], &[b]).unwrap();
    assert_eq!(net_double_area(&result), 2); // 1x1
}

#[test]
fn difference_of_overlapping_squares() {
    let a = rect_poly(0, 0, 2, 2);
    let b = rect_poly(1, 1, 3, 3);
    let result = polygon_boolean(BooleanOp::Difference, &[a], &[b]).unwrap();
    assert_eq!(net_double_area(&result), 6); // 4 - 1 = 3
}

#[test]
fn xor_of_overlapping_squares() {
    let a = rect_poly(0, 0, 2, 2);
    let b = rect_poly(1, 1, 3, 3);
    let result = polygon_boolean(BooleanOp::Xor, &[a], &[b]).unwrap();
    assert_eq!(net_double_area(&result), 12); // (4-1) + (4-1) = 6
}

#[test]
fn union_of_disjoint_squares_yields_two_polygons() {
    let a = rect_poly(0, 0, 2, 2);
    let b = rect_poly(10, 10, 12, 12);
    let result = polygon_boolean(BooleanOp::Union, &[a], &[b]).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(net_double_area(&result), 16); // 4 + 4
}

#[test]
fn difference_creating_a_hole() {
    // A large square minus a fully-interior square leaves an outer ring and a hole.
    let outer = rect_poly(0, 0, 10, 10);
    let inner = rect_poly(3, 3, 7, 7);
    let result = polygon_boolean(BooleanOp::Difference, &[outer], &[inner]).unwrap();
    // 100 - 16 = 84, doubled = 168 (hole contributes negative area).
    assert_eq!(net_double_area(&result), 168);
    // One outer contour plus one hole contour.
    assert_eq!(result.len(), 2);
}

#[test]
fn grow_offset_increases_area() {
    let sq = rect_poly(0, 0, 10, 10);
    let grown = offset(&[sq], 2).unwrap();
    let area = net_double_area(&grown);
    // Grew from 100; mitered square is close to 14x14 = 196.
    assert!(area > 200, "expected growth, got doubled area {area}");
    assert!(area < 400);
}

#[test]
fn shrink_offset_decreases_area() {
    let sq = rect_poly(0, 0, 10, 10);
    let shrunk = offset(&[sq], -2).unwrap();
    let area = net_double_area(&shrunk);
    // Shrank from 100 toward 6x6 = 36.
    assert!(area > 0, "expected positive area, got {area}");
    assert!(area < 200, "expected shrink, got doubled area {area}");
}

// ---- Property test: engine vs winding-number oracle ----

/// Cross product of edge `from->to` with `from->p`.
fn cross(from: (i64, i64), to: (i64, i64), p: (i64, i64)) -> i64 {
    (to.0 - from.0) * (p.1 - from.1) - (to.1 - from.1) * (p.0 - from.0)
}

/// Winding number of `p` with respect to all contours in `polys` (Sunday's
/// algorithm). Non-zero means inside under the non-zero fill rule. `p` must not lie
/// on any edge (the generators guarantee this).
fn winding(polys: &[Polygon], p: (i64, i64)) -> i64 {
    let mut wind = 0;
    for poly in polys {
        let verts = poly.vertices();
        let len = verts.len();
        for i in 0..len {
            let from = (i64::from(verts[i].x), i64::from(verts[i].y));
            let to = (
                i64::from(verts[(i + 1) % len].x),
                i64::from(verts[(i + 1) % len].y),
            );
            if from.1 <= p.1 {
                if to.1 > p.1 && cross(from, to, p) > 0 {
                    wind += 1;
                }
            } else if to.1 <= p.1 && cross(from, to, p) < 0 {
                wind -= 1;
            }
        }
    }
    wind
}

/// Generates 1..=3 axis-aligned rectangles on a grid of 10 DBU, so every edge lies
/// on a multiple of 10 and never touches the +5 query grid.
fn rects() -> impl Strategy<Value = Vec<Polygon>> {
    prop::collection::vec((0i32..9, 0i32..9, 1i32..=5, 1i32..=5), 1..=3).prop_map(|specs| {
        specs
            .into_iter()
            .map(|(x, y, w, h)| rect_poly(x * 10, y * 10, (x + w) * 10, (y + h) * 10))
            .collect()
    })
}

proptest! {
    #[test]
    fn boolean_matches_winding_oracle(a in rects(), b in rects()) {
        for op in [BooleanOp::Union, BooleanOp::Intersection, BooleanOp::Difference, BooleanOp::Xor] {
            let result = polygon_boolean(op, &a, &b).unwrap();
            for i in 0..10i64 {
                for j in 0..10i64 {
                    let p = (10 * i + 5, 10 * j + 5);
                    let in_a = winding(&a, p) != 0;
                    let in_b = winding(&b, p) != 0;
                    let expected = match op {
                        BooleanOp::Union => in_a || in_b,
                        BooleanOp::Intersection => in_a && in_b,
                        BooleanOp::Difference => in_a && !in_b,
                        BooleanOp::Xor => in_a != in_b,
                    };
                    let got = winding(&result, p) != 0;
                    prop_assert_eq!(got, expected, "op={:?} point={:?}", op, p);
                }
            }
        }
    }
}
