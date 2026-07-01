//! Convex-decomposition tests: exact unit cases plus a property test that checks
//! the ear-clipping decomposition against independent invariants, area
//! conservation (the brute-force oracle), per-piece convexity, and, for fixtures of
//! known size, the `n - 2` triangle count.

use proptest::prelude::*;
use reticle_geometry::{Point, Polygon, Rect, convex_decompose};

fn rect_poly(x0: i32, y0: i32, x1: i32, y1: i32) -> Polygon {
    Polygon::from_rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1)))
}

/// Sum of the absolute doubled areas of every piece (exact integer). For a correct
/// decomposition this equals the input's absolute doubled area.
fn total_double_area(pieces: &[Polygon]) -> i128 {
    pieces.iter().map(|p| p.signed_double_area().abs()).sum()
}

/// Independent convexity test: a polygon is convex when every consecutive edge turns
/// the same way, i.e. all non-zero cross products of adjacent edges share one sign.
/// Collinear turns (zero) are permitted. Returns `true` for fewer than three
/// vertices (vacuously convex).
fn is_convex(poly: &Polygon) -> bool {
    let verts = poly.vertices();
    let n = verts.len();
    if n < 3 {
        return true;
    }
    let mut sign = 0i64;
    for i in 0..n {
        let prev = verts[i];
        let curr = verts[(i + 1) % n];
        let next = verts[(i + 2) % n];
        let cross = (i64::from(curr.x) - i64::from(prev.x))
            * (i64::from(next.y) - i64::from(prev.y))
            - (i64::from(curr.y) - i64::from(prev.y)) * (i64::from(next.x) - i64::from(prev.x));
        if cross != 0 {
            let turn = cross.signum();
            if sign == 0 {
                sign = turn;
            } else if turn != sign {
                return false;
            }
        }
    }
    true
}

/// Asserts the universal invariants that must hold for any input: area is conserved
/// exactly and every returned piece is convex.
fn assert_valid_decomposition(poly: &Polygon) {
    let pieces = convex_decompose(poly);
    assert_eq!(
        total_double_area(&pieces),
        poly.signed_double_area().abs(),
        "area not conserved for {poly:?}"
    );
    for piece in &pieces {
        assert!(is_convex(piece), "non-convex piece {piece:?}");
    }
}

// ---- Unit tests ----

#[test]
fn square_yields_two_triangles_with_area_preserved() {
    let sq = rect_poly(0, 0, 4, 4);
    let pieces = convex_decompose(&sq);
    assert_eq!(pieces.len(), 2);
    assert_eq!(total_double_area(&pieces), sq.signed_double_area().abs());
    assert_eq!(total_double_area(&pieces), 32); // 2 * 4 * 4
    for piece in &pieces {
        assert!(is_convex(piece));
    }
}

#[test]
fn triangle_decomposes_to_itself() {
    let tri = Polygon::new(vec![Point::new(0, 0), Point::new(6, 0), Point::new(0, 6)]);
    let pieces = convex_decompose(&tri);
    assert_eq!(pieces.len(), 1);
    assert_eq!(
        pieces[0].signed_double_area().abs(),
        tri.signed_double_area().abs()
    );
    assert!(is_convex(&pieces[0]));
}

#[test]
fn l_shape_is_concave_but_decomposes_into_convex_pieces() {
    // An L-shape (6 vertices): area preserved, every piece convex, n - 2 = 4 pieces.
    let l = Polygon::new(vec![
        Point::new(0, 0),
        Point::new(4, 0),
        Point::new(4, 2),
        Point::new(2, 2),
        Point::new(2, 4),
        Point::new(0, 4),
    ]);
    let pieces = convex_decompose(&l);
    assert_eq!(pieces.len(), 4);
    assert_eq!(total_double_area(&pieces), l.signed_double_area().abs());
    assert_eq!(total_double_area(&pieces), 24); // filled area 12, doubled
    for piece in &pieces {
        assert!(is_convex(piece), "non-convex piece {piece:?}");
    }
}

#[test]
fn clockwise_input_is_normalized() {
    // Same L-shape wound clockwise: results must match the CCW case.
    let l_ccw = Polygon::new(vec![
        Point::new(0, 0),
        Point::new(4, 0),
        Point::new(4, 2),
        Point::new(2, 2),
        Point::new(2, 4),
        Point::new(0, 4),
    ]);
    let l_cw = l_ccw.reversed();
    let pieces = convex_decompose(&l_cw);
    assert_eq!(pieces.len(), 4);
    assert_eq!(total_double_area(&pieces), l_cw.signed_double_area().abs());
    for piece in &pieces {
        assert!(is_convex(piece));
    }
}

#[test]
fn u_shape_decomposes_into_convex_pieces() {
    // A U-shape has 8 vertices, so n - 2 = 6 triangles.
    let u = Polygon::new(vec![
        Point::new(0, 0),
        Point::new(6, 0),
        Point::new(6, 6),
        Point::new(4, 6),
        Point::new(4, 2),
        Point::new(2, 2),
        Point::new(2, 6),
        Point::new(0, 6),
    ]);
    let pieces = convex_decompose(&u);
    assert_eq!(pieces.len(), 6);
    assert_eq!(total_double_area(&pieces), u.signed_double_area().abs());
    for piece in &pieces {
        assert!(is_convex(piece), "non-convex piece {piece:?}");
    }
}

#[test]
fn arrow_shape_decomposes_into_convex_pieces() {
    // A concave arrow / chevron: the bottom notch at (4,3) is the reflex vertex.
    // 6 vertices -> n - 2 = 4 triangles.
    let arrow = Polygon::new(vec![
        Point::new(0, 0),
        Point::new(4, 3),
        Point::new(8, 0),
        Point::new(8, 2),
        Point::new(4, 6),
        Point::new(0, 2),
    ]);
    let pieces = convex_decompose(&arrow);
    assert_eq!(pieces.len(), 4);
    assert_eq!(total_double_area(&pieces), arrow.signed_double_area().abs());
    for piece in &pieces {
        assert!(is_convex(piece), "non-convex piece {piece:?}");
    }
}

#[test]
fn star_shape_decomposes_into_convex_pieces() {
    // A 4-pointed star (8 vertices alternating outer/inner) -> n - 2 = 6 triangles.
    let star = Polygon::new(vec![
        Point::new(0, 10),
        Point::new(3, 3),
        Point::new(10, 0),
        Point::new(3, -3),
        Point::new(0, -10),
        Point::new(-3, -3),
        Point::new(-10, 0),
        Point::new(-3, 3),
    ]);
    let pieces = convex_decompose(&star);
    assert_eq!(pieces.len(), 6);
    assert_eq!(total_double_area(&pieces), star.signed_double_area().abs());
    for piece in &pieces {
        assert!(is_convex(piece), "non-convex piece {piece:?}");
    }
}

#[test]
fn degenerate_inputs_return_empty() {
    // Fewer than three vertices.
    assert!(convex_decompose(&Polygon::new(vec![])).is_empty());
    assert!(convex_decompose(&Polygon::new(vec![Point::new(1, 1)])).is_empty());
    assert!(convex_decompose(&Polygon::new(vec![Point::new(0, 0), Point::new(5, 5)])).is_empty());
    // Three collinear points: zero area, no interior.
    let collinear = Polygon::new(vec![Point::new(0, 0), Point::new(2, 2), Point::new(4, 4)]);
    assert!(convex_decompose(&collinear).is_empty());
}

#[test]
fn collinear_edge_vertices_are_handled() {
    // A square with an extra collinear vertex on one edge (5 vertices, area 16).
    let poly = Polygon::new(vec![
        Point::new(0, 0),
        Point::new(2, 0), // collinear midpoint on the bottom edge
        Point::new(4, 0),
        Point::new(4, 4),
        Point::new(0, 4),
    ]);
    let pieces = convex_decompose(&poly);
    assert_eq!(total_double_area(&pieces), poly.signed_double_area().abs());
    assert_eq!(total_double_area(&pieces), 32);
    for piece in &pieces {
        assert!(is_convex(piece), "non-convex piece {piece:?}");
        // No emitted piece should be a zero-area sliver.
        assert_ne!(piece.signed_double_area(), 0, "degenerate piece {piece:?}");
    }
}

// ---- Property test: decomposition invariants over random simple polygons ----

/// Generates a random simple polygon by sampling distinct points and sorting them by
/// angle around their centroid, which always yields a convex (hence simple) ring.
/// Coordinates are spread out enough that the sort is unambiguous.
fn convex_polygon() -> impl Strategy<Value = Polygon> {
    prop::collection::hash_set((-500i32..=500, -500i32..=500), 3..=12).prop_map(|pts| {
        let mut pts: Vec<(i32, i32)> = pts.into_iter().collect();
        // Centroid (doubled to stay integral); ties broken deterministically.
        let n = i64::try_from(pts.len()).unwrap();
        let cx = pts.iter().map(|p| i64::from(p.0)).sum::<i64>() / n;
        let cy = pts.iter().map(|p| i64::from(p.1)).sum::<i64>() / n;
        pts.sort_by(|a, b| {
            let angle_a = f64::from(a.1 - cy as i32).atan2(f64::from(a.0 - cx as i32));
            let angle_b = f64::from(b.1 - cy as i32).atan2(f64::from(b.0 - cx as i32));
            angle_a
                .partial_cmp(&angle_b)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Polygon::new(pts.into_iter().map(|(x, y)| Point::new(x, y)).collect())
    })
}

proptest! {
    /// The core brute-force oracle: for any random convex polygon, the pieces'
    /// absolute areas must sum exactly to the input's, and each piece must be convex.
    #[test]
    fn random_convex_polygon_conserves_area_and_stays_convex(poly in convex_polygon()) {
        let pieces = convex_decompose(&poly);
        prop_assert_eq!(
            total_double_area(&pieces),
            poly.signed_double_area().abs(),
            "area not conserved"
        );
        for piece in &pieces {
            prop_assert!(is_convex(piece), "non-convex piece {:?}", piece);
        }
        // A non-degenerate polygon must produce at least one piece.
        if poly.signed_double_area() != 0 {
            prop_assert!(!pieces.is_empty());
        }
    }

    /// Fixed concave fixtures (L, U, arrow, star) with random uniform translation:
    /// translation preserves area and shape, so the invariants must still hold and
    /// the known `n - 2` triangle count must be produced.
    #[test]
    fn translated_concave_fixtures_hold_invariants(dx in -1000i32..=1000, dy in -1000i32..=1000) {
        let fixtures: [(Vec<Point>, usize); 4] = [
            // L-shape: 6 vertices -> 4 triangles.
            (vec![
                Point::new(0, 0), Point::new(4, 0), Point::new(4, 2),
                Point::new(2, 2), Point::new(2, 4), Point::new(0, 4),
            ], 4),
            // U-shape: 8 vertices -> 6 triangles.
            (vec![
                Point::new(0, 0), Point::new(6, 0), Point::new(6, 6), Point::new(4, 6),
                Point::new(4, 2), Point::new(2, 2), Point::new(2, 6), Point::new(0, 6),
            ], 6),
            // Arrow / chevron: 6 vertices -> 4 triangles.
            (vec![
                Point::new(0, 0), Point::new(4, 3), Point::new(8, 0),
                Point::new(8, 2), Point::new(4, 6), Point::new(0, 2),
            ], 4),
            // 4-pointed star: 8 vertices -> 6 triangles.
            (vec![
                Point::new(0, 10), Point::new(3, 3), Point::new(10, 0), Point::new(3, -3),
                Point::new(0, -10), Point::new(-3, -3), Point::new(-10, 0), Point::new(-3, 3),
            ], 6),
        ];
        for (verts, expected_count) in fixtures {
            let poly = Polygon::new(
                verts.iter().map(|p| Point::new(p.x + dx, p.y + dy)).collect(),
            );
            let pieces = convex_decompose(&poly);
            prop_assert_eq!(pieces.len(), expected_count, "piece count for {:?}", poly);
            prop_assert_eq!(
                total_double_area(&pieces),
                poly.signed_double_area().abs(),
                "area not conserved for {:?}", poly
            );
            for piece in &pieces {
                prop_assert!(is_convex(piece), "non-convex piece {:?}", piece);
            }
        }
    }
}

#[test]
fn assert_valid_decomposition_smoke() {
    // Exercise the shared helper on a couple of shapes directly.
    assert_valid_decomposition(&rect_poly(0, 0, 10, 10));
    assert_valid_decomposition(&Polygon::new(vec![
        Point::new(0, 0),
        Point::new(4, 0),
        Point::new(4, 2),
        Point::new(2, 2),
        Point::new(2, 4),
        Point::new(0, 4),
    ]));
}
