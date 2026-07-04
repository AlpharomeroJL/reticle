<p align="center">
  <img src="assets/hero.png" alt="The Reticle editor on an imported SKY130 standard cell: layers panel, a populated design-rule-check panel, the properties inspector, a minimap, and a highlighted net on the canvas" width="100%" />
</p>

# Reticle

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![Runs in the browser via WebGPU and WASM](https://img.shields.io/badge/web-WebGPU%20%2B%20WASM-orange.svg)](https://alpharomerojl.github.io/reticle/)

A local 20B model solves 52 of 75 design-rule-verified layout tasks in
this editor's command API. The editor itself runs in your browser.

**[Open the live demo.](https://alpharomerojl.github.io/reticle/)** It runs entirely in
your browser on WebGPU and WebAssembly. Use current Chrome or Edge for the WebGPU path;
the app falls back to WebGL2 elsewhere. The page opens into a recorded
propose-verify-correct run playing in the replay theater, with the full editor one click
away.

Reticle is an editor for very large hierarchical 2D layout scenes, the kind a chip's
physical design is made of. It renders and edits integer-coordinate geometry (rectangles,
polygons, and paths on named layers) organized into cells, instances, and arrays. A cell
placed thousands of times expands to billions of leaf shapes that still browse at 60 fps,
because the hierarchy is never flattened for viewing. It is written in Rust and compiled
to native and to WebAssembly from one codebase.

## What you can do with it

- Import a SKY130 standard cell from GDSII, then pan and zoom its geometry at interactive
  speed.
- Run the design-rule checker, click a violation, and jump the canvas to its marker.
- Draw a polygon, drag a vertex, boolean-union it against a neighbor, and array the result.
- Filter shapes with a query (`layer:met1 width<400`) and locate a cell from the outline tree.
- Switch to a 3D layer-stack view and orbit the extruded metals.
- Watch an AI agent propose edits, hit a red violation, and correct until the check passes.
- Edit alongside a second person over a CRDT, with live cursors and offline reconcile.

## A tour of the actual editor

Each clip is a full-window capture of the running application, produced by
`just capture-ui` from a committed script under
[`crates/reticle-app/demo-scripts/`](crates/reticle-app/demo-scripts/).

**Find and fix a design-rule violation.** Run DRC, the violation list fills, click one,
the canvas frames its marker.

<p align="center"><img src="assets/tour-drc.gif" alt="Running DRC, the violation list populating, and the canvas zooming to a clicked violation" width="100%" /></p>

**Draw and edit real geometry.** Draw a polygon, drag a vertex, boolean-union two shapes,
then array-duplicate the result. Every step is one undoable edit.

<p align="center"><img src="assets/tour-edit.gif" alt="Drawing a polygon, moving a vertex, boolean-union, and array-duplicate" width="100%" /></p>

**Watch the agent close the loop.** The replay theater plays a recorded run: the narration
feed advances and the violation counter reaches zero.

<p align="center"><img src="assets/tour-agent.gif" alt="The replay theater playing an agent run, narration advancing and the violation count reaching zero" width="100%" /></p>

**Query the layout.** Type a filter to select shapes by layer and size; click the outline
tree to locate a cell.

<p align="center"><img src="assets/tour-query.gif" alt="Filtering shapes with a query and locating a cell from the outline tree" width="100%" /></p>

**See it in 3D.** Switch to the layer-stack view and orbit the extruded metals.

<p align="center"><img src="assets/tour-3d.gif" alt="Orbiting the 3D extruded layer stack of a SKY130 cell" width="100%" /></p>

## The agent benchmark

The same engine is drivable by an AI agent. A serializable command API exposes every edit,
and a propose-verify-correct harness makes a model build layouts under the real
design-rule and connectivity checks, so a task passes only when an objective checker
accepts it. Each checker is two-way tested: it must accept the intended solution and
reject a perturbed one. Every result is labeled with the backend, model, and quantization
that produced it.

The suite is [`benchmarks/layout-tasks/manifest.toml`](benchmarks/layout-tasks/manifest.toml),
version 0.4.0, 75 tasks across five tiers. The numbers below are two local models run on
the host through Ollama. The raw per-task `ResultRecord` files are committed under
[`benchmarks/results/`](benchmarks/results/); each row here is computed from them.

<!-- BENCH_TABLE_START -->
| Model | Quantization | Tier 1 | Tier 2 | Tier 3 | Tier 4 | Tier 5 | Overall |
|---|---|---:|---:|---:|---:|---:|---:|
| `gpt-oss:16k` (20B) | MXFP4 | 9/9 | 11/11 | 19/34 | 5/11 | 8/10 | **52/75 (69%)** |
| `qwen2.5-coder:16k` (14B) | Q4_K_M | 6/9 | 8/11 | 6/34 | 3/11 | 6/10 | **29/75 (39%)** |
<!-- BENCH_TABLE_END -->

The gap has a concrete cause. `gpt-oss:16k` returns native tool calls; `qwen2.5-coder:16k`
often ignores the forced tool choice and embeds the call in message text, which a text
fallback recovers less reliably. Both paths are handled and regression-tested. These are
small quantized local models, so the numbers are a floor, not an upper bound. See
[Benchmark methodology](docs/src/benchmark.md) for how a run is scored and replayed.

## Performance

Measured on an RTX 4060 Ti; [PERF.md](PERF.md) has the methodology and the full table.

| Operation | Measured |
|---|---:|
| Retained render, 10,000,000 leaf shapes | 113 fps (was 6.1) |
| Retained render, 1,000,000 leaf shapes | 295 fps |
| WASM cold load to first interactive frame (WebGPU, loopback) | ~640 ms |
| Collaboration echo through the localhost relay (median) | 788 us |
| Bulk-load an R-tree over 1,000,000 shapes | 232 ms |
| Nearest-shape query over 1,000,000 shapes | 888 ns |
| Polygon union of 1,024 overlapping squares | 1.49 ms |
| Import a 4,194,304-leaf hierarchical layout (headless CLI) | 37 ms, 7.5 MB peak |
| Render 4,194,304 leaf shapes offscreen to 2560x1440 (headless CLI) | 809 ms, 594 MB peak |

The retained renderer caches per-cell tessellation once and uploads geometry to fixed-size
GPU pages, so each frame is a draw, not a rebuild; that is what lifts the 10,000,000-shape
scene from 6.1 to about 113 fps. Hierarchy is never flattened for browsing, so cell culling
keeps the on-screen cost proportional to what is visible, not to the size of the design.

## Quickstart

Prerequisites: a recent Rust toolchain (see `rust-toolchain.toml`) and
[`just`](https://github.com/casey/just). A WebGPU-capable browser (current Chrome or Edge)
is needed for the web demo. The local-model benchmark additionally needs a running
[Ollama](https://ollama.com) endpoint.

```sh
# Build everything and run the full local gate (style, format, clippy, tests, docs, wasm,
# licenses, spelling). There is no CI service; this recipe is the gate.
just ci

# Native application.
cargo run -p reticle-app --release

# Web demo (WebGPU with a WebGL2 fallback), served locally.
just web-serve

# Regenerate the README media (hero still plus the tour GIFs) from committed demo scripts.
just capture-ui

# Headless pipeline: import, DRC, route, extract, export, render-to-image.
cargo run -p reticle-cli --release -- --help

# Generate a deterministic chip-like layout to browse or benchmark.
just gen-layout 1000000 8 3 scratch/gen.gds

# Score the agent across the 75-task suite (deterministic mock by default).
just bench-agent

# Score it against a LOCAL model via Ollama (set the model first).
# $env:RETICLE_MODEL_NAME = 'gpt-oss:16k'
just bench-agent-ollama
```

## How it works

**Hierarchy culling.** Geometry is indexed in a bulk-loaded R-tree and a tile/level-of-detail
pyramid. Hierarchy is never flattened to browse; each cell's bounding box is computed once,
and rendering culls whole instances and arrays that fall outside the view. A compute shader
flags which cell boxes overlap the viewport, a workgroup scan compacts the survivors into an
indirect-draw buffer, and one indirect draw paints them, so the draw count comes from the GPU.

**Design-rule checking.** A declarative engine evaluates width, spacing, enclosure,
extension, notch, area, density, and angle rules against the indexed geometry. On an edit it
re-checks only the changed neighbourhood: 5 us at 100k shapes and 37 us at 1M, against the
100 ms interactive target. A property test pins the engine to a naive reference oracle over
400 random layouts. A cited SKY130 rule subset grounds the periphery rules.

**The agent verify loop.** A serializable command API exposes every edit as a replayable
transcript with a document hash. The harness drives a model against the SKY130 DRC subset and
a connectivity intent, feeds the violations back, and stops only when an objective checker
passes. For a local repair it can hand the model a region-scoped context pack instead of the
whole layout, and a user can fold a new constraint into the running loop between iterations.

**CRDT sync.** The document mirrors onto a `yrs` CRDT with unique `actor:counter` keys, so
concurrent edits converge regardless of delivery order, proven by order-independent
convergence tests. A thin relay broadcasts updates and presence; edits made offline
reconcile on reconnect. A remote edit echoes to a peer in about 788 us on the localhost relay.

## What it does not do, and where it is thin

Reticle is a portfolio-grade engineering project and a research vehicle for machine-driven
layout, not a production EDA tool. It is honest about its edges, audited in
[docs/STATUS.md](docs/STATUS.md):

- No logic or physical synthesis, no timing (no STA, no parasitic extraction), no
  device-level LVS, and no tape-out signoff. Extraction is geometric net connectivity, not
  device recognition. The SKY130 DRC subset is a fast first filter, not tape-out clean.
- The benchmark is small quantized local models, a realistic floor rather than a ceiling.
- The in-editor "ask the agent to fix this violation" button scopes the run by narration,
  not yet by a hard region constraint on the model.
- The fuzz targets are committed but libFuzzer does not link under Windows/MSVC; parser and
  boolean robustness are covered instead by proptests in the gate. Run the fuzzers on Linux.
- OASIS round-trips rectangles, polygons, paths, instances, and arrays; GDSII carries the
  full hierarchy.

For where Reticle sits among layout tools and the full list of non-goals, see
[Positioning](docs/src/positioning.md).

## Tech stack

Rust, `wgpu` (WebGPU / Vulkan / Metal / DX12 with a WebGL2 fallback), `egui` and `eframe`
(with an `egui-wgpu` paint callback for the canvas), `i_overlay`, `rstar`, `gds21`, `lyon`,
`yrs`, `axum`, `prost`, `rhai`, `pathfinding`, `criterion`, `proptest`, and `cargo-fuzz`.

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
