# View and export

The editor's right-hand column ends with a **View and export** panel that groups
four pieces of view polish: switching the colour theme, saving and restoring
camera positions, exporting the current view or selection to a file, and a
print-style monochrome render mode. The logic that can be pure is pure and lives
in `reticle-app`'s `viewexport` module, unit-tested without a window; the egui and
GPU wiring is a thin layer in the `app` module.

## Theme switching

Reticle opens in a dark theme. The **Toggle theme** button flips a stored
`Theme` between dark and light, and the app applies it to the egui `Visuals`
every frame at the top of its `ui` method, calling `ctx.set_visuals` with
`egui::Visuals::dark()` or `egui::Visuals::light()`. Applying it each frame keeps
the whole UI, panels and canvas chrome alike, consistent after a toggle.

The theme is part of the persisted view state: it is captured into the session
snapshot alongside the camera, tool, and grid (see the `session` module) and
serialized as a `theme=dark` or `theme=light` line, so the choice survives a
restart. An unknown or missing value falls back to the dark default.

## View bookmarks

A bookmark records where the camera was looking so you can return to it later. It
stores only the world center and the zoom (pixels per DBU), not a snapshot of the
document, so it stays a tiny value that reconstructs a live camera exactly.

- Type a name and press **Save view** to capture the current camera. A blank name
  is filled in with an auto-generated `View N` label so the entry is always
  clickable.
- Press a bookmark's name button to jump back: the app rebuilds the camera with
  `ViewCamera::new(center, pixels_per_dbu)`, which round-trips the saved position.
- Press the small `x` next to a bookmark to remove it.

Because a bookmark is just a `(center, zoom)` pair, saving and restoring is a pure
round-trip through the camera constructor, which the tests pin down directly.

## Exporting

Export has two independent choices: a **scope** and a **format**.

- **Scope** is either the whole **View** (every shape in the flattened scene) or
  just the current **Selection**. Exporting an empty selection is refused with a
  status message rather than writing a blank file.
- **Format** is **SVG** or **PNG**.

Files are written into the working directory as `reticle-export.svg` or
`reticle-export.png`, and the status bar reports the path.

### SVG

SVG is the primary export and is generated purely from the shape list by
`shapes_to_svg`, a function of its inputs with no egui or GPU involvement. Each
shape kind maps to one SVG element:

- a rectangle becomes a `<rect>`;
- a polygon becomes a `<polygon>` with its vertices;
- a path (wire) becomes a stroked `<polyline>` whose stroke width is the path
  width scaled into output pixels.

Shapes are placed by an affine fit of the export bounds onto the pixel canvas
(the `Projection` type), preserving aspect ratio and flipping world `+y` up to
image `+y` down so the picture matches the on-screen canvas. Fill colours come
from the live layer table, the same colours the canvas draws with.

### PNG

PNG export takes one of two paths:

- A full-colour export of the **whole view** reuses the native offscreen GPU
  renderer (`render_document_offscreen`) for a pixel-accurate frame that matches
  the canvas exactly. This is native-only and is skipped with a status note when
  no GPU is available.
- A **selection** export, or **any monochrome** export, uses the pure rasterizer
  in the `viewexport` module: a small scanline filler that paints each shape over
  a white page and feeds the crate's dependency-free PNG encoder. It needs no GPU,
  so it is unit-tested, and it is what makes selection-only and print-mode PNGs
  possible.

On the web there is no filesystem and no blocking GPU context, so PNG export
reports that it is native-only and SVG generation reports the byte count instead
of writing a file.

## Monochrome (print) mode

The **Monochrome (print) mode** checkbox switches both export paths to a
print-style render: shapes are drawn as pure black on a white page regardless of
their layer colour. In SVG, filled shapes become unfilled black outlines and wires
stay black strokes; in the raster path every shape is filled black. This produces
a clean, ink-friendly rendering suitable for printing or embedding in a document,
independent of the on-screen theme.
