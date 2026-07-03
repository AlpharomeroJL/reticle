# Layer and technology editing

The layer-and-technology panel groups two related jobs into the end of the right-hand
side panel: an upgraded layer manager for arranging and styling the layer table, and a
technology editor for viewing and changing the process description that drives the
whole document. Its window-free logic lives in the `tech_editor` module and the layer
state lives in `layers`, so the interesting behavior is unit-tested without a GPU or a
window; the panel itself is thin glue that binds widgets to that logic.

## Layer manager

The layer manager is a view over the app's layer state, the same table the canvas
consults when it culls hidden layers. Every action here is a cheap, view-only change:
it never mutates the document and never lands on the undo stack.

- **Reorder.** The up and down arrows move a layer one position earlier or later in
  the table. The first row cannot move up and the last cannot move down. Reordering
  keeps the layer table's internal id-to-index lookup in sync, so visibility and
  name queries keep resolving to the right rows after a move.
- **Recolor.** The color button opens a picker seeded with the layer's current color.
  Choosing a new color repacks it into the layer's `0xRRGGBBAA` value; the canvas
  palette re-reads layer colors, so the change shows up on the next scene rebuild.
- **Fill style.** Each layer carries a fill style (solid, hatch, or outline) chosen
  from a small drop-down. Fill style is display metadata for the manager preview and
  the legend; it stays inside the app and does not change the geometry.
- **Solo.** Solo shows only the chosen layer and hides every other, the fast way to
  isolate one layer. Soloing an unknown layer is a no-op rather than a blank canvas,
  so a stale id never hides everything.
- **Show all / Hide all** flip every layer visible or hidden at once, and the
  per-row checkbox toggles a single layer.

## Technology editor

The technology editor edits a working *draft* of the document's technology: the
database resolution (`dbu_per_micron`), the layer table, and the DRC rule
thresholds. The draft is seeded from the live document the first time the panel is
shown, and from then on it is yours to edit; nothing touches the document until you
press **Apply**.

You can edit the technology name and resolution, recolor and renumber layers or
rename them, and adjust each rule's threshold value. The rule rows show each rule's
kind and its layer (or layer pair) for context, with the threshold as the editable
field, because retuning a value is the common change.

### Validation

**Apply** validates the whole draft before it commits, and if anything is wrong it
applies nothing and lists every problem inline. The checks are the invariants the DRC
engine and the file format assume but the in-memory type does not enforce on its own:

- the resolution must be positive,
- every rule threshold must be non-negative, and a length-style rule
  (width, spacing, enclosure, extension, notch) or an area rule must be strictly
  positive, so a negative or zero width is rejected,
- a two-layer rule kind (spacing, enclosure, extension) must carry a second layer and
  a single-layer kind must not, and
- every physical-stack entry's thickness must be positive.

Only a clean draft reaches the document. **Revert** throws the draft away and reloads
it from the current document, discarding any in-progress edits.

### How the change reaches the document

Setting a technology is not one of the document's undoable edits: the edit vocabulary
is geometry-only (add and remove shapes, cells, instances, and labels), and the
undo/redo log records only those. So a validated technology is applied directly to the
document rather than pushed onto the undo stack. The document revision still advances,
which is what the retained renderer keys its cache invalidation on, so the canvas
re-reads the new technology. The practical consequence is that a technology change is
not itself undoable, and it leaves the existing shape-edit history intact: undoing a
shape edit made before the technology change still works.

## Technology file round-trip

The editor round-trips the draft through the line-oriented technology-file text
format described in [File formats](io.md). The collapsible **Technology file (text)**
panel shows the draft serialized to that format; **Refresh from draft** re-serializes
the current draft, and **Load from text** parses the text box back into the draft,
reporting a parse error instead of loading a malformed file.

Serialization is canonical and byte-stable rather than a verbatim copy. Parsing
discards comments, blank lines, token spacing, keyword case, and any `0x` or `#` color
prefix, so those cannot survive a load; the serializer emits one directive per line in
a fixed order, with colors as eight uppercase hex digits. The guarantee the editor
relies on is a fixpoint: parsing the serialized draft yields an equal technology, and
re-serializing it reproduces the same bytes. Re-saving a file the editor wrote gives
back exactly that file; a hand-authored file with comments comes back in canonical form
without them.
