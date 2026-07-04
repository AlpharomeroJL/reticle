# Introduction

A local 20B model solves 52 of 75 design-rule-verified layout tasks in
this editor's command API. The editor itself runs in your browser.

Reticle is an editor for very large hierarchical 2D layout scenes, the kind a chip's
physical design is made of. It renders and edits integer-coordinate geometry (rectangles,
polygons, and paths on named layers) organized into cells, instances, and arrays. A cell
placed thousands of times expands to billions of leaf shapes that still browse at 60 fps,
because the hierarchy is never flattened for viewing. It is written in Rust and compiled to
native and to WebAssembly from one codebase.

On top of the geometry sit pan, zoom, select, measure, and annotate tools; a full drawing
and vertex-editing suite; boolean and transform operations; a design-rule checker; a
router; connectivity extraction; GDSII and OASIS import and export; embedded scripting; and
real-time multi-user collaboration. On top of that sits an agent layer: a serializable
command API, a propose-verify-correct harness graded by objective checkers, an MCP server,
and the benchmark whose number opens this page.

## Why it exists

Reticle works the exact problem a semiconductor tooling team solves: visualizing and editing
massive layout geometry at interactive speed. That one problem pulls together performance
engineering, computational geometry, GPU rendering, spatial indexing, schema evolution, and
distributed collaboration, with a checker-graded agent layer on top.

## The north-star

Open a dense chip-like layout of over one million polygons in a browser. Pan and zoom at
60 fps. Run an incremental design-rule check and jump to a violation. Draw a polygon and
boolean-union it against a neighbor. Watch a second user's cursor and edits appear live, and
watch an agent build under the same checks a person would.

## How this book is organized

The **Design** chapters walk through each subsystem in dependency order, from the
exact-integer geometry core up through rendering, checking, routing, and collaboration. The
**Automation and agents** chapters cover the command API, the verify loop, the MCP server,
and the benchmark. The **Reference** chapters cover how performance is measured, how to use
the application, and how to contribute. Every subsystem is a separate crate; see
[Architecture](architecture.md) for the crate graph.
