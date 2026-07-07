# Streamed documents

A layout that fits in RAM is opened, edited, and undone through the in-memory document
model. A multi-gigabyte die does not fit, and cannot be built in a browser tab at all, so
it is *streamed*: written once into a tiled `.rtla` archive (the Wave 2 container, ADR
0062) and then fetched a tile at a time over HTTP Range requests as the camera moves. This
chapter covers what a streamed document can and cannot do, and how the view stays
responsive while tiles are still in flight.

## Streamed documents are read-mostly

A streamed document is browse, measure, query, and share only. It is deliberately **not**
editable, and that is enforced by the type system rather than by a runtime check.

An open document in the app is a `DocHost`, which is one of two things:

- `Edited(History)`, an in-memory document with its undo/redo history, or
- `Streamed(StreamedScene)`, a document paged in from an `.rtla` archive.

Every mutating path in the app (drawing a shape, running a boolean, undo, redo) takes a
`&mut History`. A `DocHost` hands out a `History` only when you match its `Edited` arm.
There is no total accessor that returns a `&mut History` regardless of which arm is
active; the only mutable accessor is fallible and returns `Option`, and a `StreamedScene`
carries no mutation method of any kind. So an editing tool cannot even name a `History`
to mutate on a streamed document: code that tries either fails to compile because it never
destructured `Edited`, or fails to compile because it called a mutator on an `Option` it
did not unwrap. Editing a streamed document is a compile error, not a runtime refusal that
a later feature could forget to add.

This is the scope line drawn in ADR 0062 and made concrete in ADR 0068. The payoff is
that the read-mostly guarantee holds for free as the app grows: any new edit path, written
the ordinary way against `&mut History`, is automatically inapplicable to a streamed
document, with no reviewer needing to remember to guard it.

## Coarse-then-fine: painting while tiles stream in

When the camera moves over a streamed document, the tiles that cover the new viewport at
the zoom's level of detail may not be resident yet. Blocking the frame on the network
would stutter the view; painting nothing would flash it blank. Instead the scene refines
progressively.

The `StreamedScene` keeps a working set of resident tiles, bounded by an LRU so RAM does
not grow without limit, and a viewport-to-tile mapper built from the archive's level grid.
On a camera move:

1. It computes the tiles that cover the viewport at the target (finest appropriate) level
   and fetches the ones not already resident, over the archive's `TileSource`. Each
   fetched tile is validated and decoded exactly as the memory-mapped path is, so a
   truncated or corrupt tile is an error, never undefined behaviour.
2. Until those fine tiles arrive, it paints the **coarsest resident level that still fully
   covers the viewport**. Coarse levels have fewer, larger tiles, so they are far more
   likely to be complete; the view shows less detail for a moment rather than going blank.
3. As tiles arrive (on the browser microtask queue in wasm, on a task in native), they are
   posted to an inbox the UI loop drains, become resident, and the painted level rises to
   the fine level.

The fetched tiles' vertices are uploaded into the renderer's existing paged GPU buffers;
streaming changes what is paged into memory, never the silicon, so it is not an edit.

This behaviour is proven headlessly. A residency test stands up an in-memory archive
behind a source that injects a per-tile fetch latency and asserts the full sequence:
immediately after a zoom-in the scene paints from the coarse resident level with no fine
tile resident, and after the injected delay elapses the resident set has transitioned
coarse to fine, the painted level is the fine level, and the painted record set matches
the fine-level query exactly.
