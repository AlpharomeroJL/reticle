# User guide

## Running

- **Native application:** `cargo run -p reticle-app --release`.
- **Browser demo:** `just web-serve`, then open the printed local URL in a
  WebGPU-capable browser (current Chrome or Edge). Where WebGPU is unavailable the
  demo falls back to WebGL2 with a notice.
- **Collaboration relay:** `cargo run -p reticle-server --release`, then point two
  application instances at it to edit together.
- **Headless pipeline:** `cargo run -p reticle-cli --release -- --help` lists the
  import, DRC, route, extract, export, and render-to-image commands.
- **Generate a layout:** `just gen-layout 1000000 8 3 scratch/gen.rgds` writes a
  deterministic chip-like layout with the given shape count, layer count, and
  hierarchy depth to browse or benchmark.

## Editing

The application is a CAD-style editor: pan and zoom the canvas, select and measure,
and draw shapes on the active layer. A command palette exposes every action and its
rebindable shortcut. The layer manager toggles visibility and style, selection
filters and a query bar narrow what you are working on, and rulers, a grid, snap,
and guides keep edits on-grid. Multiple viewports show different parts of the design
at once.

Sessions are saved and restored, autosave and crash recovery protect in-progress
work, and an undo-history panel lets you step through and jump within the edit
history.

On a touch device (a phone that opened a shared link, or a tablet), the canvas
navigates by touch: a two-finger pinch zooms, anchored at the point between your
fingers so what you are pinching stays put, and a two-finger drag pans. A single-finger
drag pans when the Pan tool is active. The gesture math is a pure camera helper with the
zoom-anchoring invariant unit-tested, so the world point under the pinch centroid stays
fixed as the zoom changes.

## Checking and routing

Run the design-rule checker to populate the violation overlay, and use the error
browser to zoom to each violation in turn. Route selected nets, and inspect the
congestion and length report. Highlight a net to trace its connectivity across the
layout.
