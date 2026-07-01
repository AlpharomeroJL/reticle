//! Integration tests for the linear (oracle) spatial index.

use reticle_geometry::{Point, Rect, SpatialIndex};
use reticle_index::LinearIndex;

fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
    Rect::new(Point::new(x0, y0), Point::new(x1, y1))
}

#[test]
fn query_rect_returns_intersecting_items() {
    let index = LinearIndex::bulk_load([
        (rect(0, 0, 2, 2), "a"),
        (rect(10, 10, 12, 12), "b"),
        (rect(1, 1, 3, 3), "c"),
    ]);
    assert_eq!(index.len(), 3);
    let mut hits = index.query_rect(rect(0, 0, 2, 2));
    hits.sort_unstable();
    assert_eq!(hits, vec![&"a", &"c"]);
}

#[test]
fn nearest_and_k_nearest() {
    let index = LinearIndex::bulk_load([
        (rect(0, 0, 1, 1), "a"),
        (rect(100, 100, 101, 101), "b"),
        (rect(5, 5, 6, 6), "c"),
    ]);
    assert_eq!(index.nearest(Point::new(0, 0)), Some(&"a"));
    assert_eq!(index.k_nearest(Point::new(0, 0), 2), vec![&"a", &"c"]);
}

#[test]
fn empty_index() {
    let index: LinearIndex<u32> = LinearIndex::new();
    assert!(index.is_empty());
    assert_eq!(index.nearest(Point::ORIGIN), None);
    assert!(index.k_nearest(Point::ORIGIN, 4).is_empty());
}

#[test]
fn insert_grows_the_index() {
    let mut index = LinearIndex::new();
    index.insert("x", rect(0, 0, 1, 1));
    index.insert("y", rect(2, 2, 3, 3));
    assert_eq!(index.len(), 2);
    assert_eq!(index.query_rect(rect(2, 2, 3, 3)), vec![&"y"]);
}
