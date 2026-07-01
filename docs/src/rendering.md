# Rendering and scale

`reticle-render` is the `wgpu` renderer. It targets WebGPU in the browser and
Vulkan, Metal, or DX12 natively, with a WebGL2 fallback for reach (ADR 0009).

## GPU-driven culling

The central trick for scale is to keep the hierarchy on the GPU. Rather than the
CPU walking billions of leaf shapes, a compute shader builds the visible draw list
from cell bounding boxes and the current view, and an indirect draw renders it.
The CPU work per frame is proportional to the number of visible cells, not the size
of the design. On the WebGL2 path, which has no compute, the draw list is built on
the CPU instead.

## Instanced draws, tiles, and LOD

Geometry is tessellated once into vertex and index buffers (`lyon`) and drawn with
per-layer style through instanced draws. A tile and level-of-detail pyramid swaps
dense geometry for coarser representations as the camera zooms out, so a full-chip
view costs no more than a zoomed-in one. Edges are anti-aliased, and labels are
drawn from a glyph atlas (`glyphon`).

## Overlays and views

On top of the base layers the renderer draws a minimap, design-rule violation
markers, highlighted nets, and an optional 3D cross-section of the layer stack. It
renders either to a window surface or to an offscreen texture; the offscreen path
drives both the golden-image tests and the media capture.

## Targets

One million flat shapes at a sustained 60 fps at typical zoom, ten million
interactive at 30 fps or better, and hierarchical designs with effectively billions
of leaf shapes at 60 fps through cell culling and LOD. Measured numbers are in the
[performance chapter](performance.md).
