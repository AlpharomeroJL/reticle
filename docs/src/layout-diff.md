# Layout diff

`reticle-diff` answers "what changed between these two versions of a layout?" It
computes a pure geometric diff between two [`Document`](model.md) snapshots, and
the app paints the result over the canvas: shapes added in green, removed in red,
changed in amber. Like the metrology and DRC crates, it runs on the CPU over the
flattened top cell, reads only the public APIs of `reticle-model` and
`reticle-geometry`, and never mutates a document.

## The pure diff

The one entry point is `diff(before, after) -> LayoutDiff`, where

```text
LayoutDiff { added: Vec<DiffShape>, removed: Vec<DiffShape>, changed: Vec<DiffShape> }
DiffShape { layer, rect, label }
```

The comparison runs over each document's flattened top cell and treats the two as
multisets keyed by `(layer, exact geometry)`:

- A shape present in `after` with no match in `before` is **added**.
- A shape present in `before` with no match in `after` is **removed**.
- Matched shapes, including matched duplicates (the counts are compared, not just
  presence), are reported as neither.

Because the key is the exact geometry, not a bounding box, two shapes match only
when they are geometrically identical. A shape that merely moved therefore reads as
one removed plus one added. Flattening happens first, so the diff compares leaf
geometry regardless of how the two documents structured their cells; `DiffShape`'s
`rect` is the shape's bounding box (what the overlay paints) and `label` names the
flattened top cell.

### `changed` is deferred in v1

Telling a moved or resized shape apart from an independent add-plus-remove is a
fuzzy match, and emitting it wrongly is worse than not emitting it at all. v1
therefore reports every geometric difference as an add or a remove and always
leaves `changed` empty. The field exists so the overlay and any future revision
keep a stable shape; a same-place resize shows today as a red box plus a green box.

### Correctness

The diff is pinned by property tests, the same discipline the DRC and metrology
crates use:

- `diff(d, d)` is empty for any document.
- `diff(empty, d)` is all-added, with the added count equal to the flattened shape
  count of `d`.
- `diff(d, empty)` is all-removed.
- The single-insertion oracle: appending one rectangle to any base document yields
  exactly one added shape and zero removed, whatever the base and whatever the
  rectangle (a duplicate still raises the multiset count by exactly one).

## The app overlay

The app consumes the diff through a `diff_overlay` module (egui-free logic, unit
tested, mirroring the [DRC panel](drc.md) split) and a "Layout diff" side panel.
The comparison document comes from a two-snapshot flow rather than a second file
open, since no clean comparison-document loader exists in this build:

1. **Snapshot** captures the current document as the baseline (the *before*).
2. Edit the layout.
3. **Diff vs snapshot** compares the baseline against the now-current document and
   paints the difference on the canvas.

A **Show diff overlay** checkbox hides or shows the painted rectangles without
discarding the computed diff, and **Clear** drops the baseline and the diff.
Loading a new document clears the baseline, since it snapshotted the previous one.

A comparison-document file loader and a true per-instance (unflattened) diff are
open work; both can reuse the `LayoutDiff` surface unchanged. See
[ADR 0079](../decisions/0079-layout-diff-overlay.md).
