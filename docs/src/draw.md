# Drawing and vertex editing

The editor's drawing tools add geometry to the top cell, and the vertex-edit tool
reshapes a shape already there. As with the rest of the app, the interesting logic is
window-free and lives in `reticle-app`'s `draw` module, unit-tested without a GPU; the
egui layer only turns pixels into world points, calls in, and paints the live
preview.

Every one of these actions goes through the undo history, so drawing a shape or moving
a vertex can be undone and redone like any other edit.

## Tools

Four tools sit on the toolbar next to Select, Pan, Measure, and Cut. They are also in
the command palette under `Tool:`.

- **Rect** draws an axis-aligned rectangle by dragging from one corner to the opposite
  one.
- **Polygon** draws a closed polygon by clicking to place each vertex.
- **Path** draws a wire: a polyline with a width and an end-cap style.
- **Vertices** edits the vertices of the selected shape.

Switching to any non-drawing tool discards a half-drawn shape so nothing leaks between
tools. The path width and end cap you pick survive the switch.

## Rectangle constraints

A plain drag spans the two corners. Two modifiers refine it, and they combine:

- **Shift** constrains the rectangle to a square, growing the shorter side to match the
  longer one while keeping the far corner in the direction you dragged.
- **Alt** or **Ctrl** treats the drag's start point as the rectangle's center rather
  than a corner, so the box grows symmetrically. Held together with shift, a centered
  square uses the larger half-extent on both axes.

The start and end points are snapped to the grid first (see [Snapping](#snapping)), so
the committed rectangle lands on grid coordinates.

## Placing polygons and paths

Both tools accumulate vertices click by click, drawing a live preview of the edges
placed so far plus a faint segment out to the cursor. An immediate repeat of the last
point is ignored, so a closing double-click never leaves a zero-length edge.

- A **double-click**, or **Enter**, finishes the shape. A polygon needs at least three
  distinct vertices; a path needs at least two points. A finish gesture with too few
  points is declined rather than committing a degenerate shape.
- **Escape** cancels the shape in progress.

A path carries a width in database units and one of three end caps, both set on the
toolbar while the path tool is active:

- **Flat** ends the wire exactly at its endpoints.
- **Square** extends each end by half the width.
- **Round** rounds each end by a half-width radius.

## Editing vertices

Select a single shape you drew, then switch to the Vertices tool. It ticks every vertex
of the shape so you can see what is grabbable. Only shapes owned directly by the top
cell are editable; geometry that comes from a placed instance is not.

- **Drag** a vertex to move it. The new position is snapped to the grid.
- **Click on an edge** to insert a vertex there. The new vertex lands on the point of
  the segment nearest the click.
- **Alt-click** (or ctrl-click) a vertex to delete it. A polygon keeps at least three
  vertices and a path at least two, so a deletion that would collapse the shape is
  refused.

A rectangle promotes to a polygon the moment one of its corners is edited off the axis,
since a rectangle can no longer describe the result. A path keeps its width and end cap
through an edit. Each move, insert, or delete is one undoable step, applied as a
remove-then-add of the reshaped shape.

## Snapping

Every placed point and moved vertex is snapped through the shared grid before it is
committed, so drawn geometry lands on grid coordinates whenever snapping is on. Turning
snapping off (the Snap toggle) places points at the exact cursor position instead.
Snapping to nearby existing geometry, as opposed to the grid, is handled elsewhere in
the editor.

## Testing

The module's geometry is unit-tested in isolation: the rectangle-from-drag math for
each modifier combination, the polygon and path builders' finish thresholds and
deduplication, vertex hit-testing by exact squared distance, edge projection for
insertion, and the insert, delete, and move operations against their vertex-count
floors. Because this logic is pure, the tests need no window and run with the rest of
the `reticle-app` suite.
