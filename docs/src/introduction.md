# Introduction

Reticle is a browser-native, GPU-accelerated editor for very large hierarchical
2D layout scenes, written in Rust and compiled to both native and WebAssembly. It
renders and edits integrated-circuit geometry: rectangles, polygons, and paths on
named layers, organized into cells, instances, and arrays, so a small cell placed
thousands of times yields effectively billions of leaf shapes that still browse at
interactive frame rates.

On top of the geometry sit CAD-style pan, zoom, select, measure, and annotate
tools, a design-rule checker, a router, connectivity extraction, GDSII and OASIS
import and export, embedded scripting, and real-time multi-user collaboration.

## Why it exists

Reticle works the exact problem a semiconductor tooling team solves: visualizing
and editing massive layout geometry at interactive speed. That single problem
pulls together performance engineering, computational geometry, GPU rendering,
spatial indexing, schema evolution, and distributed collaboration.

## The north-star

Open a dense chip-like layout of over one million polygons in a browser, pan and
zoom at 60 fps, measure a spacing, run an incremental design-rule check and jump to
a violation, drop an annotation, and watch a second user's cursor and edits appear
live.

## How this book is organized

The **Design** chapters walk through each subsystem in dependency order, from the
exact-integer geometry core up through rendering, checking, routing, and
collaboration. The **Reference** chapters cover how performance is measured, how to
use the application, and how to contribute. Every subsystem is a separate crate;
see [Architecture](architecture.md) for the crate graph.
