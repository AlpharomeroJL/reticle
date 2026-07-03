# Guided tour

The first time the editor opens on a fresh install it runs a short guided tour: a
dismissable overlay that walks through the real panels one at a time. Each step
names the actual control it points at, draws a highlight box around that region,
and waits for a Next, Skip, or Close. The tour shows once, remembers that it has
been shown, and can be relaunched at any time from the Help menu.

## What the tour covers

The tour is split into two chapters. The first is the core walkthrough and always
runs; the second covers the Wave 2 tools and is optional.

**Chapter 1, getting started:**

1. **The canvas.** Pan by dragging and zoom toward the cursor by scrolling; Fit
   frames the whole design.
2. **Layers.** The left-hand layer manager toggles visibility and filters layers by
   name.
3. **Measure.** Pick the Measure tool from the toolbar and click two points to read
   a distance in database units and microns.
4. **Design-rule checking.** Run DRC and click a violation to zoom straight to it.
5. **Net highlight.** Light up every shape connected to a net across the design.
6. **Minimap.** The overview panel frames the current view; click inside it to jump
   the camera.
7. **Agent and replay.** The agent panel runs a scripted edit session and the replay
   theater plays a recorded run back step by step.

**Chapter 2, Wave 2 tools:**

1. **Drawing tools:** rectangles, polygons, paths, and vertex editing.
2. **Boolean and transform:** union, intersect, subtract, and transforms, all
   undoable.
3. **Productivity:** copy, duplicate, arrays, move-by-delta, and via stacks.
4. **Snapping and guides:** snap to vertices, edges, midpoints, and centers, with
   draggable ruler guides.
5. **Layer and technology editing:** reorder, recolor, restyle, and edit the
   technology definition.
6. **Search and selection:** filter shapes with a query, save selection sets, and
   navigate the cell outline.
7. **View and export:** theme, camera bookmarks, and SVG/PNG export.

At the end of the core chapter, Next advances into the Wave 2 chapter and Skip ends
the tour, so a user who only wants the basics is never forced through the second
half.

## Showing once, and relaunching

Whether the tour has run is a single bit stored with the rest of the view state in
the session file, next to the camera, tool, grid, theme, and hidden layers. On the
first launch with no saved session that bit is unset, so the tour starts
automatically; once it finishes or is dismissed the bit is written, so it never
opens unprompted again. A session file written by an older build has no such bit and
reads as unset, so an upgrade shows the tour once rather than suppressing it.

The **Help** menu on the toolbar relaunches the tour on demand. "Take the tour"
replays the core chapter followed by the Wave 2 chapter; "Core tour only" replays
just the core chapter. Either choice restarts from the first step, and a relaunch is
not treated as a first run, so it does not disturb the persisted "seen" bit.

On the web there is no filesystem to persist the bit between page loads, so the tour
is treated as already seen and does not reopen on every visit; the Help menu still
relaunches it within a session.

## How it is built

The tour logic is a pure state machine in `reticle-app`'s `tour` module: the ordered
list of steps, the current position, and the transitions between them (next, skip,
finish), plus the first-run-versus-relaunched distinction and whether the second
chapter is included. It holds no `egui`, GPU, or filesystem types, so it compiles
unchanged for the browser and is unit-tested in full without a window. Highlighting
a control is deliberately abstract: each step names a target region rather than a
pixel rectangle, and the egui layer maps that name to the panel or canvas rectangle
it already lays out that frame. Nothing depends on exact coordinates, so the
highlight tracks the layout even after the panels are resized.
