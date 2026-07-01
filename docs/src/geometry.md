# Geometry

`reticle-geometry` is the exact-integer foundation. It has no GPU, async, or UI
code, so it is fast to test and simple to reason about.

## Coordinates are integers

Chip layout lives on an integer grid measured in *database units* (DBU). Reticle
uses `type Dbu = i32`, which matches GDSII and keeps points dense in memory. All
area and product arithmetic widens to `i64` (or `i128` for area sums) to avoid
overflow, so for example `Rect::area` returns an `i64` and `Polygon::signed_double_area`
returns an `i128`. See ADR 0002 for the reasoning and the trade-offs.

## Primitives

- `Point { x, y }` on the DBU grid, with saturating translation and squared
  distance in `i64`.
- `Rect` stored as `[min, max)` corners, with width, height, area, containment,
  intersection, union, and margin expansion.
- `Polygon`, an implicitly-closed ring of vertices, with the shoelace signed
  double-area (exact in `i128`), winding classification, and a bounding box.
- `Path`, a polyline with a width and an end-cap style (flat, square, round, or a
  custom extension), with a conservative bounding box.
- `Transform`, an orientation from the dihedral group of eight (rotate by a
  multiple of 90 degrees, optionally reflected), a rational `Magnification`, and a
  translation, applied in that order. This matches the GDSII and OASIS placement
  model.

## Robust booleans

Union, intersection, difference, and exclusive-or run on the `i_overlay` integer
engine over DBU coordinates (ADR 0003), wrapped behind `polygon_boolean` so the
dependency stays swappable. Input contours are interpreted by winding under the
non-zero fill rule, so a clockwise ring is treated as a hole. Results are flattened
to polygons with outer boundaries wound counter-clockwise and holes clockwise, so a
caller can tell them apart by `Polygon::winding`.

Offsetting (growing or shrinking by a delta) runs on `i_overlay`'s float outline
engine with mitered corners and is rounded back to the grid.

## Testing

The boolean engine is validated two ways. Exact unit cases check known areas,
including a difference that produces a hole. A property test compares the engine
against an independent winding-number oracle: for randomized sets of rectangles and
every operation, it asserts that a grid of off-edge query points is classified
identically by the engine and by the oracle. This catches sign, winding, and
hole-handling mistakes that example-based tests miss.
