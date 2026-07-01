//! Unit tests for the [`LodPyramid`] tile / level-of-detail structure.

use reticle_geometry::{Point, Rect};
use reticle_index::{LodPyramid, TileId};

fn world() -> Rect {
    Rect::new(Point::new(0, 0), Point::new(1000, 1000))
}

#[test]
fn level_zero_is_a_single_tile_covering_the_world() {
    let pyramid = LodPyramid::build(world(), 3, &[]);
    assert_eq!(pyramid.num_levels(), 3);
    assert_eq!(pyramid.tiles_per_side(0), Some(1));
    assert_eq!(pyramid.tiles_per_side(1), Some(2));
    assert_eq!(pyramid.tiles_per_side(2), Some(4));
    assert_eq!(pyramid.tiles_per_side(3), None);
    assert_eq!(pyramid.tile_bounds(TileId::new(0, 0, 0)), Some(world()));
}

#[test]
fn tiles_tile_the_world_exactly() {
    let pyramid = LodPyramid::build(world(), 3, &[]);
    // Level 2 => 4x4 grid, each 250 wide.
    assert_eq!(
        pyramid.tile_bounds(TileId::new(2, 0, 0)),
        Some(Rect::new(Point::new(0, 0), Point::new(250, 250)))
    );
    assert_eq!(
        pyramid.tile_bounds(TileId::new(2, 3, 3)),
        Some(Rect::new(Point::new(750, 750), Point::new(1000, 1000)))
    );
    // Out-of-range tile indices are rejected.
    assert_eq!(pyramid.tile_bounds(TileId::new(2, 4, 0)), None);
}

#[test]
fn shape_lands_in_the_expected_tiles() {
    // One shape in the lower-left quadrant.
    let shapes = [Rect::new(Point::new(10, 10), Point::new(90, 90))];
    let pyramid = LodPyramid::build(world(), 2, &shapes);

    // Level 0: the single tile holds it.
    assert_eq!(
        pyramid.shapes_in_tile(TileId::new(0, 0, 0)),
        Some(&[0u32][..])
    );
    // Level 1 (2x2, each 500 wide): only tile (0,0) holds it.
    assert_eq!(
        pyramid.shapes_in_tile(TileId::new(1, 0, 0)),
        Some(&[0u32][..])
    );
    assert_eq!(pyramid.shapes_in_tile(TileId::new(1, 1, 0)), Some(&[][..]));
    assert_eq!(pyramid.shapes_in_tile(TileId::new(1, 0, 1)), Some(&[][..]));
    assert_eq!(pyramid.shapes_in_tile(TileId::new(1, 1, 1)), Some(&[][..]));
}

#[test]
fn shape_spanning_multiple_tiles_is_recorded_in_each() {
    // A shape straddling the vertical midline at level 1.
    let shapes = [Rect::new(Point::new(400, 100), Point::new(600, 200))];
    let pyramid = LodPyramid::build(world(), 2, &shapes);
    assert_eq!(
        pyramid.shapes_in_tile(TileId::new(1, 0, 0)),
        Some(&[0u32][..])
    );
    assert_eq!(
        pyramid.shapes_in_tile(TileId::new(1, 1, 0)),
        Some(&[0u32][..])
    );
}

#[test]
fn out_of_world_shape_is_clipped_into_border_tiles() {
    // Extends past the right/top edges; should still land in the far tiles, not
    // be dropped.
    let shapes = [Rect::new(Point::new(900, 900), Point::new(5000, 5000))];
    let pyramid = LodPyramid::build(world(), 2, &shapes);
    assert_eq!(
        pyramid.shapes_in_tile(TileId::new(1, 1, 1)),
        Some(&[0u32][..])
    );
}

#[test]
fn visible_tiles_selects_the_overlapping_tiles() {
    let pyramid = LodPyramid::build(world(), 2, &[]);
    // A viewport covering the lower-left quarter at level 1 (2x2 grid).
    let viewport = Rect::new(Point::new(10, 10), Point::new(200, 200));
    let visible = pyramid.visible_tiles(viewport, 1);
    assert_eq!(visible, vec![TileId::new(1, 0, 0)]);

    // A viewport spanning the whole world sees all four level-1 tiles.
    let all = pyramid.visible_tiles(world(), 1);
    assert_eq!(all.len(), 4);
}

#[test]
fn shapes_in_viewport_dedups_across_tiles() {
    // Shape spans two level-1 tiles; a viewport covering both must list it once.
    let shapes = [Rect::new(Point::new(400, 400), Point::new(600, 600))];
    let pyramid = LodPyramid::build(world(), 2, &shapes);
    let got = pyramid.shapes_in_viewport(world(), 1);
    assert_eq!(got, vec![0]);
}

#[test]
fn level_for_viewport_picks_coarser_levels_for_bigger_targets() {
    // World extent 1000; num_levels 4 => tile extents 1000, 500, 250, 125.
    let pyramid = LodPyramid::build(world(), 4, &[]);
    // A huge target tile => coarsest level.
    assert_eq!(pyramid.level_for_viewport(10_000), 0);
    // Target 250 => first level whose tiles are <= 250, i.e. level 2.
    assert_eq!(pyramid.level_for_viewport(250), 2);
    // Very small target => finest level (clamped).
    assert_eq!(pyramid.level_for_viewport(1), 3);
}

#[test]
#[should_panic(expected = "at least one level")]
fn zero_levels_panics() {
    let _ = LodPyramid::build(world(), 0, &[]);
}

#[test]
#[should_panic(expected = "positive area")]
fn degenerate_world_panics() {
    let degenerate = Rect::new(Point::new(0, 0), Point::new(0, 100));
    let _ = LodPyramid::build(degenerate, 2, &[]);
}
