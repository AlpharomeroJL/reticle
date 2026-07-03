# Productivity editing

The productivity panel gathers the everyday layout-editing shortcuts into one place:
an in-app clipboard, an array tool with a live preview, numeric move-by-delta, and a
via-stack builder. Every change it makes to the document goes through the undo
history, so anything the panel does can be undone and redone from the history panel.

The panel lives at the bottom of the right-hand side panel. Its window-free logic
sits in the `productivity` module and is unit-tested without a GPU or a window; the
panel itself is thin glue that binds widgets to that logic and routes the resulting
edits through the editing history.

## Clipboard: copy, cut, paste, and duplicate

Copy snapshots the current selection into an in-app clipboard as resolved shapes in
top-cell coordinates. Because the clipboard holds geometry rather than selection
indices, it survives later edits, undo, and selection changes.

Paste stamps the clipboard back into the top cell, shifted by the panel's offset
(dx, dy). Duplicate is copy-plus-paste in one step over the current selection, so it
works on instanced geometry too: the duplicate is flat geometry drawn directly in the
top cell.

Cut copies the selection and then removes it. Only shapes drawn directly in the top
cell can be removed, because the model's remove operation addresses a cell's own
shape list; any selected geometry that belongs to an instance or an array is copied
but left in place, and the status line reports how many were skipped.

## Move by delta

Move applies a numeric (dx, dy) shift to the selected direct shapes. A move is a
remove of each original followed by an add of its translated copy, both through the
history, so it undoes cleanly. As with cut, only directly-owned shapes move;
instanced geometry is left in place.

## Array tool

The array tool repeats the current selection into a grid of rows by columns at a
given row pitch and column pitch. Element (0, 0) reproduces the selection in place,
so the originals stay put and the grid grows from them.

With the live-preview checkbox on, the pending elements are outlined on the canvas
before you commit, so you can dial in the counts and pitches and see the result
first. Element (0, 0) is not drawn in the preview because it coincides with the
existing selection.

The tool is for tractable, previewable repeats. The element count (rows times
columns) is capped; past the cap the commit is refused and the panel says so. Very
large regular repeats belong in a hierarchical array placement instead, which the
renderer expands lazily rather than materializing every leaf shape.

On commit, each array element is added as its own edit, so each is individually
undoable and the scene rebuilds once at the end.

## Via-stack builder

The via-stack builder places a connecting cut between two picked layers together with
the enclosure rectangles those layers need around it. You pick a lower layer, an
upper layer, and the cut layer, set the cut's square size, and place it at a chosen
center.

The enclosure margin for each picked layer is read from the technology's enclosure
rules: the rule whose enclosing layer is that picked layer and whose enclosed layer
is the cut layer. When several such rules apply, the largest margin wins so the drawn
geometry satisfies all of them at once. A layer with no matching rule falls back to a
default margin you can set in the panel, so the builder still produces a sane stack
against a technology that omits the rule.

Each enclosure rectangle is the cut expanded outward by its margin, so it overlaps
the cut on every side by at least the required amount. The cut and its two enclosures
are placed as three separate edits, so the whole stack is undoable.

## Undo integration

Everything here is built on the same contract: geometry helpers return owned shapes,
and the panel wraps each in an add or remove edit applied through the editing history,
then rebuilds the scene once. Nothing mutates the document directly. That is what
keeps copy, paste, duplicate, move, array, and via placement all reversible from the
history panel.
