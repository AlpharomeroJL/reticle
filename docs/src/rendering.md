# Rendering and scale

`reticle-render` is the `wgpu` renderer. It targets WebGPU in the browser and
Vulkan, Metal, or DX12 natively, with a WebGL2 fallback for reach (ADR 0009).

## GPU-driven culling

The central trick for scale is to keep the hierarchy on the GPU. Rather than the CPU
walking billions of leaf shapes, a compute shader tests each cell's bounding box
against the current view and flags the visible ones, the first stage of a GPU-driven
draw list (compacting the survivors into an indirect draw is a follow-up). The work is
proportional to the number of cells considered, not the flattened shape count. The
interactive egui canvas currently culls on the CPU with the same R-tree, and the GPU
compute cull is validated against that CPU result in a golden test.

## Instanced draws and tessellation

Axis-aligned rectangles are drawn as instanced quads; polygons and paths are
tessellated once into vertex and index buffers (`lyon`) and drawn with per-layer style.
Colors come from the technology layer table with a fallback palette. A tile and
level-of-detail pyramid in the index provides coarser representations for zoomed-out
browsing.

## Offscreen rendering

The renderer renders to an offscreen texture today, the path that drives both the
golden-image tests and the media capture (the hero image and browse GIF). Window and
surface presentation, and overlays (a minimap, design-rule violation markers,
highlighted nets, and a 3D layer-stack cross-section), are tracked follow-ups noted in
`STATUS.md`; the render crate's module docs frame them the same way.

## Targets

One million flat shapes at a sustained 60 fps at typical zoom, ten million
interactive at 30 fps or better, and hierarchical designs with effectively billions
of leaf shapes at 60 fps through cell culling and LOD. Measured numbers are in the
[performance chapter](performance.md).
