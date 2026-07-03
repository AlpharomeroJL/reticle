# Snapping and guides

Placing geometry by eye is imprecise. Reticle snaps the cursor onto meaningful
positions so a new point lands exactly where it should: on the grid, on existing
geometry, or on a guide line the user has pulled out. The snap logic is pure DBU
arithmetic and lives in `reticle-app`'s `snap` module, unit-tested without a
canvas, a camera, or any drawing.

There are three things the cursor can snap to, checked together and resolved to
whichever is nearest:

- **The grid.** The background grid rounds a point onto the nearest grid
  intersection. This is the baseline snap and is described under
  [Rendering and scale](rendering.md); it is owned by `GridSettings`.
- **Nearby geometry.** The vertices, edges, edge midpoints, and bounding-box
  centers of the shapes around the cursor.
- **User guides.** Draggable horizontal and vertical lines, pulled off the
  rulers, that behave like an extra piece of geometry to snap to.

## Snapping to geometry

For each shape near the cursor the engine emits a set of snap candidates:

- a **vertex** at every corner of a rectangle and every vertex of a polygon or
  path;
- an **edge** for every segment between consecutive vertices (a polygon also
  closes the loop back to its first vertex; an open path does not);
- a **midpoint** at the center of every edge;
- a **center** at the middle of the shape's bounding box.

A discrete candidate (vertex, midpoint, center) snaps to its own position. An edge
snaps to the perpendicular foot of the cursor on the segment, clamped to the
endpoints, so the cursor slides along the edge rather than only catching its ends.

The nearest resolved candidate within the snap radius wins. When two candidates
sit at exactly the same distance the more specific one is chosen, so an exact
corner is preferred over the two edges that meet there, and a midpoint is preferred
over the edge it lies on. Only shapes on visible layers are considered, so a hidden
layer never steals a snap.

The snap radius is set in screen pixels and converted to world units at the current
zoom, so the feel is the same whether zoomed in or out. Candidates are gathered
only from the shapes whose bounding box lies within that radius of the cursor, so
the cost is bounded by what is under the cursor rather than by the size of the
design.

## User guides

Guides are horizontal or vertical reference lines at a fixed world coordinate. A
horizontal guide pins a `y` value; a vertical guide pins an `x` value. Drag inward
from the top ruler to pull out a horizontal guide, or from the left ruler to pull
out a vertical one; the guide drops at the cursor, rounded onto the grid. Guides
are listed in the snap panel, where each can be removed individually or the whole
set cleared, and guides can also be added at the view center from the panel.

Guide snapping runs per axis: the nearest vertical guide within range pins the
cursor's `x`, and the nearest horizontal guide pins its `y`. When a horizontal and
a vertical guide are both in range the cursor lands on their intersection, so
crossing guides give an exact point to snap to.

Guides are view and session state. They are never written into the document, so
adding, moving, or clearing a guide is not an undoable edit and does not touch the
layout.

## The snap indicator

Whenever the cursor catches something, a small diamond is drawn at the snapped
point with a short caption naming what it hit (vertex, edge, midpoint, center, or
guide), colored by kind. The indicator rides on top of the geometry so it is never
hidden, and disappears the moment the cursor moves off every candidate.

## Settings

The snap panel in the right sidebar surfaces every knob:

- **Show grid**, **Snap to grid**: the grid's visibility and grid snapping.
- **Snap to geometry**: whether nearby vertices, edges, midpoints, and centers
  catch the cursor.
- **Snap to guides**: whether guide lines catch the cursor.
- **Grid spacing**: the base grid step, in DBU.
- **Snap radius**: how close, in screen pixels, a candidate must be to catch the
  cursor.

Grid visibility and grid snapping are also toggleable from the toolbar. Turning off
both geometry and guide snapping leaves only the grid, and turning off grid
snapping as well places points exactly where the cursor is.

## How it fits together

The canvas routes its cursor through a single snap seam that tries geometry and
guide snapping first and falls back to the grid when nothing is in range. That seam
returns both the snapped point, which is what a tool places, and the hint that
drives the on-canvas indicator. Because the drawing tools place at the point this
seam returns, geometry drawn with a tool snaps to existing geometry and guides, not
only to the grid.
