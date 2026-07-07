# 0078 (placeholder), Layout diff: a pure geometric diff crate and a canvas overlay

> Placeholder ADR number, assigned at the Wave 4 merge gate.

## Context

Reticle can open, edit, and render a layout, but nothing answered "what changed
between these two versions?" That is the everyday question when reviewing an edit,
comparing a regenerated block against its predecessor, or checking what a scripted
pass touched. A visual diff, shapes added/removed/changed painted over the canvas,
is the "compare two versions of a layout" demo moment, and the comparison itself
is pure geometry that belongs in a small, testable crate rather than buried in the
app.

## Decision

**A new CPU-only crate, `reticle-diff`, that reads the frozen public APIs of
`reticle-model` and `reticle-geometry` and never mutates a document.** Its one
entry point is `diff(before, after) -> LayoutDiff`, where
`LayoutDiff { added, removed, changed }` each hold `DiffShape { layer, rect,
label }`.

- **Flattened, multiset, exact-geometry keyed.** The diff runs over each
  document's flattened top cell and compares the two as multisets keyed by
  `(layer, exact geometry)`. A shape in `after` with no match in `before` is
  added; a shape in `before` with no match in `after` is removed; matched shapes
  (including matched duplicates, counted) are neither. Because it keys on exact
  geometry rather than bounding box, a shape that only moved reads as one remove
  plus one add.
- **`changed` is deferred in v1.** Distinguishing a moved or resized shape from an
  independent add-plus-remove is a fuzzy match, and emitting it wrongly is worse
  than not emitting it. v1 reports every geometric difference as an add or a
  remove and always leaves `changed` empty. The field exists so the overlay and a
  future revision keep a stable shape.
- **Correctness pinned by property tests**, the same discipline `reticle-drc` and
  `reticle-metrology` use: `diff(d, d)` is empty; `diff(empty, d)` is all-added
  with `|added|` equal to the flattened shape count; `diff(d, empty)` is
  all-removed; and the single-insertion oracle, appending one rectangle to any
  base document, yields exactly one added and zero removed.

**The app consumes it through a new `diff_overlay` module** (egui-free logic,
unit-tested, mirroring the `drc_panel`/`app` split) plus a minimal additive mount:
a "Layout diff" side panel and a canvas overlay painting added green, removed red,
changed amber, with a show/hide toggle.

**The comparison document comes from a two-snapshot flow, not a file loader.** No
clean second-document open path exists in this build, so the overlay captures the
current document as a baseline ("Snapshot"), the user edits, and "Diff vs
snapshot" compares the baseline against the now-current document. This ships the
demo without blocking on a file dialog.

## Consequences

- A normal workspace member in the default `just ci` set. It adds no new external
  dependency beyond (dev-only) `proptest`; it reuses `reticle-geometry` and
  `reticle-model` only through their public surface.
- The diff is exact for whatever geometry the model carries (rectangles, polygons,
  paths), because the match key is the geometry itself, not an approximation.
- Flattening collapses hierarchy, so `DiffShape::label` names the flattened top
  cell rather than a full instance path. A per-instance diff is open work.
- `changed` being always empty in v1 is a deliberate, documented gap; a same-place
  resize shows as a red box plus a green box, which is honest if coarser than an
  amber "changed" box would be.
- The two-snapshot flow means the comparison is against an in-memory baseline, not
  a loaded file. A comparison-document file loader is open work that can reuse the
  same `LayoutDiff` unchanged.
