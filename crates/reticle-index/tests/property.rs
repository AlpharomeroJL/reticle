//! Property tests: [`RTreeIndex`] and [`UniformGrid`] must return the same
//! answers as the brute-force [`LinearIndex`] oracle for every query.
//!
//! Because equal-distance ties are broken differently by each index, nearest and
//! k-nearest are compared by the *distances* they return, and rectangle queries
//! by the *set* of matched items, never by object identity.

use std::collections::BTreeSet;

use proptest::prelude::*;
use reticle_geometry::{Point, Rect, SpatialIndex};
use reticle_index::{LinearIndex, RTreeIndex, UniformGrid};

/// Coordinate bound for generated shapes. Moderate so that random small queries
/// meaningfully overlap the generated shapes (making the comparisons exercise
/// non-empty results). Wide-coordinate correctness, where an `i32`-scalar tree
/// would overflow, is covered separately by [`wide_coordinates_do_not_overflow`].
const BOUND: i32 = 2_000;

/// Squared distance from a point to the closed rectangle, in DBU² (the oracle's
/// metric), for comparing answers by distance rather than identity.
fn dist2(p: Point, r: &Rect) -> i64 {
    let clamp = |v: i32, lo: i32, hi: i32| v.max(lo).min(hi.max(lo));
    let nx = clamp(p.x, r.min.x, r.max.x);
    let ny = clamp(p.y, r.min.y, r.max.y);
    let dx = i64::from(p.x) - i64::from(nx);
    let dy = i64::from(p.y) - i64::from(ny);
    dx * dx + dy * dy
}

/// A rectangle with positive area within `[-BOUND, BOUND]`.
fn rect_strategy() -> impl Strategy<Value = Rect> {
    (-BOUND..BOUND, -BOUND..BOUND, 1..=500i32, 1..=500i32)
        .prop_map(|(x, y, w, h)| Rect::new(Point::new(x, y), Point::new(x + w, y + h)))
}

fn point_strategy() -> impl Strategy<Value = Point> {
    (-BOUND..BOUND, -BOUND..BOUND).prop_map(|(x, y)| Point::new(x, y))
}

/// Sorted set of matched item ids from a query result.
fn id_set(items: Vec<&u32>) -> BTreeSet<u32> {
    items.into_iter().copied().collect()
}

/// The sorted distances of a list of returned item ids to `p`, using `rects` for
/// lookup. Used to compare nearest/k-nearest answers modulo tie-breaking.
fn sorted_dists(items: &[&u32], rects: &[Rect], p: Point) -> Vec<i64> {
    let mut ds: Vec<i64> = items
        .iter()
        .map(|&&i| dist2(p, &rects[i as usize]))
        .collect();
    ds.sort_unstable();
    ds
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// `query_rect` returns the same matched-item set across all three indices.
    #[test]
    fn query_rect_matches_oracle(
        rects in prop::collection::vec(rect_strategy(), 0..80),
        query in rect_strategy(),
    ) {
        let oracle = LinearIndex::bulk_load(rects.iter().enumerate().map(|(i, r)| (*r, i as u32)));
        let rtree = RTreeIndex::bulk_load(rects.iter().enumerate().map(|(i, r)| (*r, i as u32)));
        let grid = UniformGrid::bulk_load(128, rects.iter().enumerate().map(|(i, r)| (*r, i as u32)));

        let expected = id_set(oracle.query_rect(query));
        prop_assert_eq!(id_set(rtree.query_rect(query)), expected.clone(), "rtree query_rect");
        prop_assert_eq!(id_set(grid.query_rect(query)), expected, "grid query_rect");
    }

    /// `nearest` returns an item at the same distance as the oracle's nearest.
    #[test]
    fn nearest_matches_oracle(
        rects in prop::collection::vec(rect_strategy(), 0..80),
        p in point_strategy(),
    ) {
        let oracle = LinearIndex::bulk_load(rects.iter().enumerate().map(|(i, r)| (*r, i as u32)));
        let rtree = RTreeIndex::bulk_load(rects.iter().enumerate().map(|(i, r)| (*r, i as u32)));
        let grid = UniformGrid::bulk_load(128, rects.iter().enumerate().map(|(i, r)| (*r, i as u32)));

        let expected = oracle.nearest(p).map(|&i| dist2(p, &rects[i as usize]));
        let got_rtree = rtree.nearest(p).map(|&i| dist2(p, &rects[i as usize]));
        let got_grid = grid.nearest(p).map(|&i| dist2(p, &rects[i as usize]));
        prop_assert_eq!(got_rtree, expected, "rtree nearest distance");
        prop_assert_eq!(got_grid, expected, "grid nearest distance");
    }

    /// `k_nearest` returns the same multiset of distances as the oracle, in order.
    #[test]
    fn k_nearest_matches_oracle(
        rects in prop::collection::vec(rect_strategy(), 0..80),
        p in point_strategy(),
        k in 0usize..12,
    ) {
        let oracle = LinearIndex::bulk_load(rects.iter().enumerate().map(|(i, r)| (*r, i as u32)));
        let rtree = RTreeIndex::bulk_load(rects.iter().enumerate().map(|(i, r)| (*r, i as u32)));
        let grid = UniformGrid::bulk_load(128, rects.iter().enumerate().map(|(i, r)| (*r, i as u32)));

        let expected = sorted_dists(&oracle.k_nearest(p, k), &rects, p);
        prop_assert_eq!(expected.len(), k.min(rects.len()));
        prop_assert_eq!(sorted_dists(&rtree.k_nearest(p, k), &rects, p), expected.clone(), "rtree k_nearest");
        prop_assert_eq!(sorted_dists(&grid.k_nearest(p, k), &rects, p), expected, "grid k_nearest");
    }

    /// `len`/`is_empty` agree with the oracle.
    #[test]
    fn len_matches_oracle(rects in prop::collection::vec(rect_strategy(), 0..80)) {
        let oracle = LinearIndex::bulk_load(rects.iter().enumerate().map(|(i, r)| (*r, i as u32)));
        let rtree = RTreeIndex::bulk_load(rects.iter().enumerate().map(|(i, r)| (*r, i as u32)));
        let grid = UniformGrid::bulk_load(128, rects.iter().enumerate().map(|(i, r)| (*r, i as u32)));
        prop_assert_eq!(rtree.len(), oracle.len());
        prop_assert_eq!(grid.len(), oracle.len());
        prop_assert_eq!(rtree.is_empty(), oracle.is_empty());
        prop_assert_eq!(grid.is_empty(), oracle.is_empty());
    }
}

// --- Explicit edge cases -----------------------------------------------------

#[test]
fn empty_index_answers() {
    let rtree: RTreeIndex<u32> = RTreeIndex::bulk_load(std::iter::empty());
    let grid: UniformGrid<u32> = UniformGrid::bulk_load(64, std::iter::empty());
    let p = Point::new(0, 0);
    let area = Rect::new(Point::new(-10, -10), Point::new(10, 10));

    assert!(rtree.is_empty());
    assert!(grid.is_empty());
    assert_eq!(rtree.len(), 0);
    assert_eq!(grid.len(), 0);
    assert!(rtree.nearest(p).is_none());
    assert!(grid.nearest(p).is_none());
    assert!(rtree.k_nearest(p, 5).is_empty());
    assert!(grid.k_nearest(p, 5).is_empty());
    assert!(rtree.query_rect(area).is_empty());
    assert!(grid.query_rect(area).is_empty());
}

#[test]
fn single_element_index() {
    let bbox = Rect::new(Point::new(0, 0), Point::new(10, 10));
    let rtree = RTreeIndex::bulk_load([(bbox, 7u32)]);
    let grid = UniformGrid::bulk_load(4, [(bbox, 7u32)]);
    let p = Point::new(100, 100);

    assert_eq!(rtree.len(), 1);
    assert_eq!(grid.len(), 1);
    assert_eq!(rtree.nearest(p), Some(&7));
    assert_eq!(grid.nearest(p), Some(&7));
    assert_eq!(rtree.k_nearest(p, 5), vec![&7]);
    assert_eq!(grid.k_nearest(p, 5), vec![&7]);

    // Overlapping query hits; disjoint (and merely-touching) queries miss.
    let hit = Rect::new(Point::new(5, 5), Point::new(15, 15));
    let miss = Rect::new(Point::new(50, 50), Point::new(60, 60));
    let touch = Rect::new(Point::new(10, 10), Point::new(20, 20)); // shares only a corner
    assert_eq!(rtree.query_rect(hit), vec![&7]);
    assert_eq!(grid.query_rect(hit), vec![&7]);
    assert!(rtree.query_rect(miss).is_empty());
    assert!(grid.query_rect(miss).is_empty());
    assert!(
        rtree.query_rect(touch).is_empty(),
        "touching-only must not match (positive-area rule)"
    );
    assert!(
        grid.query_rect(touch).is_empty(),
        "touching-only must not match (positive-area rule)"
    );
}

#[test]
fn insert_matches_bulk_load() {
    let rects = [
        Rect::new(Point::new(0, 0), Point::new(5, 5)),
        Rect::new(Point::new(20, 20), Point::new(25, 25)),
        Rect::new(Point::new(-10, -10), Point::new(-5, -5)),
    ];
    let mut rtree = RTreeIndex::new();
    let mut grid = UniformGrid::new(8);
    for (i, r) in rects.iter().enumerate() {
        rtree.insert(i as u32, *r);
        grid.insert(i as u32, *r);
    }
    let area = Rect::new(Point::new(-100, -100), Point::new(100, 100));
    assert_eq!(id_set_owned(rtree.query_rect(area)), (0..3).collect());
    assert_eq!(id_set_owned(grid.query_rect(area)), (0..3).collect());
}

fn id_set_owned(items: Vec<&u32>) -> BTreeSet<u32> {
    items.into_iter().copied().collect()
}

/// Shapes spread across nearly the whole `i32` coordinate range: squared
/// distances here exceed `i32::MAX`, so an `i32`-scalar R-tree would overflow and
/// rank neighbours incorrectly. The `i64`-based tree must still match the oracle.
#[test]
fn wide_coordinates_do_not_overflow() {
    let big = 1_000_000_000; // ~half of i32::MAX
    let rects = [
        Rect::new(Point::new(-big, -big), Point::new(-big + 100, -big + 100)),
        Rect::new(Point::new(big - 100, big - 100), Point::new(big, big)),
        Rect::new(Point::new(0, 0), Point::new(50, 50)),
    ];
    let oracle = LinearIndex::bulk_load(rects.iter().enumerate().map(|(i, r)| (*r, i as u32)));
    let rtree = RTreeIndex::bulk_load(rects.iter().enumerate().map(|(i, r)| (*r, i as u32)));

    for p in [
        Point::new(big, big),
        Point::new(-big, -big),
        Point::new(10, 10),
        Point::new(0, 0),
    ] {
        let expected = oracle.nearest(p).map(|&i| dist2(p, &rects[i as usize]));
        let got = rtree.nearest(p).map(|&i| dist2(p, &rects[i as usize]));
        assert_eq!(got, expected, "nearest at {p:?}");

        let ek = sorted_dists(&oracle.k_nearest(p, 3), &rects, p);
        let gk = sorted_dists(&rtree.k_nearest(p, 3), &rects, p);
        assert_eq!(gk, ek, "k_nearest at {p:?}");
    }
}
