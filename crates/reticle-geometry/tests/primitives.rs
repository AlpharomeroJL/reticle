//! Integration tests for the geometry primitives and shapes (the Wave 0 contract).

use reticle_geometry::{
    Endcap, Magnification, Orientation, Path, Point, Polygon, Rect, Transform, Winding,
};

#[test]
fn point_translate_and_distance() {
    let p = Point::new(3, 4);
    assert_eq!(p.translate(1, -2), Point::new(4, 2));
    assert_eq!(Point::ORIGIN.distance_squared(p), 25);
}

#[test]
fn rect_metrics() {
    let r = Rect::new(Point::new(0, 0), Point::new(4, 3));
    assert_eq!(r.width(), 4);
    assert_eq!(r.height(), 3);
    assert_eq!(r.area(), 12);
    assert!(!r.is_empty());
    assert!(r.contains(Point::new(1, 1)));
    // `max` is exclusive.
    assert!(!r.contains(Point::new(4, 1)));
}

#[test]
fn rect_normalizes_corners() {
    let r = Rect::new(Point::new(4, 3), Point::new(0, 0));
    assert_eq!(r.min, Point::new(0, 0));
    assert_eq!(r.max, Point::new(4, 3));
}

#[test]
fn rect_from_points_spans_all() {
    let r = Rect::from_points([Point::new(1, 5), Point::new(-2, 3), Point::new(0, 9)]).unwrap();
    assert_eq!(r.min, Point::new(-2, 3));
    assert_eq!(r.max, Point::new(1, 9));
    assert!(Rect::from_points([]).is_none());
}

#[test]
fn rect_intersection_and_union() {
    let a = Rect::new(Point::new(0, 0), Point::new(4, 4));
    let b = Rect::new(Point::new(2, 2), Point::new(6, 6));
    assert!(a.intersects(&b));
    assert_eq!(
        a.intersection(&b),
        Some(Rect::new(Point::new(2, 2), Point::new(4, 4)))
    );
    assert_eq!(a.union(&b), Rect::new(Point::new(0, 0), Point::new(6, 6)));

    let c = Rect::new(Point::new(10, 10), Point::new(12, 12));
    assert!(!a.intersects(&c));
    assert_eq!(a.intersection(&c), None);
}

#[test]
fn orientation_rotations() {
    let p = Point::new(1, 0);
    assert_eq!(Orientation::R0.apply(p), Point::new(1, 0));
    assert_eq!(Orientation::R90.apply(p), Point::new(0, 1));
    assert_eq!(Orientation::R180.apply(p), Point::new(-1, 0));
    assert_eq!(Orientation::R270.apply(p), Point::new(0, -1));
}

#[test]
fn orientation_mirror() {
    let p = Point::new(1, 2);
    assert_eq!(Orientation::MirrorX.apply(p), Point::new(1, -2));
    assert!(Orientation::MirrorX.is_mirrored());
    assert!(!Orientation::R90.is_mirrored());
}

#[test]
fn magnification_scale_rounds_half_away_from_zero() {
    assert_eq!(Magnification::UNITY.scale(7), 7);
    let half = Magnification::new(1, 2).unwrap();
    assert_eq!(half.scale(10), 5);
    assert_eq!(half.scale(5), 3);
    assert_eq!(half.scale(-5), -3);
    assert!(Magnification::new(1, 0).is_none());
}

#[test]
fn transform_apply_orients_then_translates() {
    let t = Transform {
        translation: Point::new(10, 20),
        orientation: Orientation::R90,
        magnification: Magnification::UNITY,
    };
    // R90 of (1,0) is (0,1); translate by (10,20) gives (10,21).
    assert_eq!(t.apply(Point::new(1, 0)), Point::new(10, 21));
    assert_eq!(
        Transform::IDENTITY.apply(Point::new(5, 6)),
        Point::new(5, 6)
    );
}

#[test]
fn polygon_area_and_winding() {
    let square = Polygon::from_rect(Rect::new(Point::new(0, 0), Point::new(2, 2)));
    // Twice the area of a 2x2 square is 8.
    assert_eq!(square.signed_double_area(), 8);
    assert!((square.area() - 4.0).abs() < f64::EPSILON);
    assert_eq!(square.winding(), Winding::CounterClockwise);
    assert_eq!(square.reversed().winding(), Winding::Clockwise);
    assert_eq!(
        square.bounding_box(),
        Rect::new(Point::new(0, 0), Point::new(2, 2))
    );
}

#[test]
fn degenerate_polygon_has_zero_area() {
    let line = Polygon::new(vec![Point::new(0, 0), Point::new(5, 0)]);
    assert_eq!(line.signed_double_area(), 0);
    assert_eq!(line.winding(), Winding::Degenerate);
}

#[test]
fn path_bounding_box_includes_width() {
    let path = Path::new(vec![Point::new(0, 0), Point::new(10, 0)], 4, Endcap::Square);
    let bb = path.bounding_box();
    // Half-width is 2, expanded on every side.
    assert_eq!(bb.min, Point::new(-2, -2));
    assert_eq!(bb.max, Point::new(12, 2));
}
