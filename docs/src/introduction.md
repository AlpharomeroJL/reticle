# Introduction

A local 20B model solves 52 of 75 design-rule-verified layout tasks in this editor's
command API. The editor itself runs in your browser.

Reticle is an editor for very large hierarchical 2D layout scenes, the kind a chip's
physical design is made of. It renders and edits integer-coordinate geometry (rectangles,
polygons, and paths on named layers) organized into cells, instances, and arrays. A cell
placed thousands of times expands to billions of leaf shapes that still browse at 60 fps,
because the hierarchy is never flattened for viewing. It is written in Rust and compiled to
native and to WebAssembly from one codebase.

## Three things you can do with it

**Open and share real chips in a browser.** Import a GDSII or OASIS layout and browse it at
interactive speed with nothing installed: open a local file, drop one onto the window, or
pass `?gds=<url>` to load a stream from a link. A session can be made read-only and shared,
so a reviewer opens the exact view you are looking at and pans it themselves without being
able to change it.

**Generate verified structures from language.** Six parameterized generators (a guard ring,
a via farm, a pad ring, a seal ring, a density fill, and a probe-able test structure) turn a
few numbers into structure that would otherwise be drawn by hand. Each is DRC-clean by
construction against the SKY130 subset, checked by a property test that runs every generator
over 400 random valid parameter sets and asserts zero design-rule violations. The same six
surface three ways from one schema: the Generate panel in the app, one MCP tool per
generator, and the generator tasks in the benchmark. See [Layout generators](generators.md).

**Benchmark agents on physically verified tasks.** A serializable command API exposes every
edit, and a propose-verify-correct harness makes a model build layouts under the real
design-rule and connectivity checks, so a task passes only when an objective checker accepts
it. The suite is 83 tasks across five tiers; a bare local 20B model driven by Reticle's own
loop passes 52 of the 75 v0.4.0 tasks. An agent system such as Claude Code brings its own
loop, so its result is not head-to-head comparable with a bare model. See the
[Agent benchmark suite](benchmarks.md) and [Benchmark methodology](benchmark.md).

On top of the geometry that carries all of this sit pan, zoom, select, measure, and annotate
tools; a full drawing and vertex-editing suite; boolean and transform operations; a
design-rule checker; a router; connectivity extraction; GDSII and OASIS import and export;
embedded scripting; and real-time multi-user collaboration.

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
**Automation and agents** chapters cover the command API, the verify loop, the layout
generators, the MCP server, and the benchmark. The **Reference** chapters cover how
performance is measured, how to use the application, and how to contribute. Every subsystem
is a separate crate; see [Architecture](architecture.md) for the crate graph.
