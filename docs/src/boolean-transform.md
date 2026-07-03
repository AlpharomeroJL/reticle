# Boolean and transform operations

The Operations panel turns the current shape selection into edits: planar
booleans, an offset, rotate and mirror, and align and distribute. It lives in
`reticle-app`'s `ops` module. The heavy geometry is delegated to
`reticle-geometry` (the same robust `i_overlay` engine the rest of the app uses);
`ops` is the glue that maps selected scene shapes back to editable cell shapes,
runs the operation, and records the result as one undo step.

## What the selection points at

Selection is a set of indices into the *flattened* top-cell scene, the same list
the canvas hit-tests. Only the top cell's own shapes can be edited by index, and
`Document::flatten` emits those first, so a scene index below the editable-shape
count maps one-to-one onto a top-cell shape index. Selected indices at or above
that count come from placed instances and arrays; there is no single shape in the
top cell to rewrite, so the operations skip them.

Booleans and offset act on *filled* geometry, so they consider rectangles and
polygons. A path is a stroked wire rather than a fill region, and turning it into a
fill needs the render tessellator, so paths are skipped from boolean and offset
input. Transforms (rotate, mirror, align, distribute) are coordinate maps and apply
to every shape kind.

## Booleans, per layer

Union, intersection, difference, and exclusive-or run through
`reticle_geometry::polygon_boolean`. The selection is grouped by layer first: only
shapes on the *same* layer combine, and each group's result stays on that layer.
This keeps a boolean from silently merging, say, metal-1 and metal-2 geometry. A
layer group needs at least two fillable shapes to do anything.

Union, intersection, and exclusive-or fold pairwise across the group (they are
associative). Difference subtracts every later shape from the first, matching the
usual "A minus the rest" editor behavior. When the engine produces no geometry (an
empty intersection, for instance) the inputs are left untouched rather than being
deleted.

## Offset, grow, and shrink

The offset control feeds `reticle_geometry::offset`, which grows for a positive DBU
amount and shrinks for a negative one, with mitered corners. Each selected fillable
shape is offset independently and its result replaces it on the same layer. A shrink
that collapses a shape to nothing simply drops it. The offset runs on the float
outline engine and rounds back to the grid, so results are on-grid but not
guaranteed bit-exact for pathological input.

## Rotate and mirror

Rotate takes a numeric angle in degrees and turns the selection about its combined
bounding-box center, counter-clockwise. Rotation runs on a floating basis and rounds
each vertex to the nearest DBU, so a multiple of 90 degrees is exact while an
arbitrary angle is on-grid but not exactly reversible. A rotated rectangle is
promoted to a polygon, since a non-orthogonal angle tilts it off the axes.

Mirror reflects the selection across its center, either about the vertical center
line (left to right) or the horizontal one (top to bottom). An axis mirror is exact
integer arithmetic (`2 * center - coordinate`), so mirroring twice about the same
axis returns the original geometry.

## Align and distribute

Align moves every selected shape to a shared edge or center of the selection's
combined bounding box: left, right, or horizontal center; top, bottom, or vertical
center. A shape already in place does not move. Distribute needs at least three
shapes: it fixes the two extreme shapes and respaces the ones between them so the
edge-to-edge gaps along the chosen axis are equal.

The alignment and distribution math is pure and unit-tested against hand-computed
offsets, independent of the UI.

## One edit, one undo step

A single operation is usually several edits: a boolean removes each of its inputs
and adds one result. Those must undo together. The frozen `Edit` vocabulary has no
group variant, so the app layer's `History` records how many underlying edits make
up each logical step (`History::apply_group`) and steps `undo` and `redo` over a
whole group at once. Removals within a group are ordered highest-index-first so that
removing one input does not shift the index of another before it is removed. The
result: a boolean over three rectangles is a single entry on the undo stack, and one
undo restores all three.
