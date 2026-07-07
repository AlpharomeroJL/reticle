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

## GPU-resident hierarchy

The cull stage above flags visible cells; `GpuHierarchy` closes the loop by keeping the
whole arrayed hierarchy resident on the GPU and never touching a per-element draw list on
the CPU. A compact table of *array placements* (one record per array reference, not per
element) plus a table of leaf cells is uploaded once. Every frame a single compute pass
(`expand_cull_compact.wgsl`) does three things at once: it **expands** each array
element-by-element (one thread per element, binary-searching the placement table by a
precomputed cumulative element offset), **culls** each element's transformed bounding box
against the viewport with the same half-open rule as the CPU, and **compacts** the
survivors into ready-to-draw `RectInstanceT` buffers with a per-workgroup prefix scan and
a single atomic range reservation, filling an indirect instance count. One
`draw_indirect` per chunk then draws exactly the survivors; the GPU, not the CPU, decides
how many.

A single compute dispatch is bounded two ways: at most `max_compute_workgroups_per_dimension`
(65,535) workgroups of 256 threads, and a storage binding no larger than
`max_storage_buffer_binding_size` (128 MiB, so about 2.79M 48-byte survivors on a
default-limits device). `GpuHierarchy` escapes both by splitting the global element space
into fixed-size **chunks** and issuing one dispatch and one draw per chunk; the cap is
beaten by chunk *count*, never by a bigger dispatch, so the design scales to arbitrarily
many elements at a fixed per-chunk cost. A 100M-element array (a via, fill, or bit-cell
field, routine in real layout) spans 36 chunks; a 30M array spans 11.

Because the per-frame path iterates only the chunk list (a handful of entries) and the
scene tables are uploaded once, the CPU does no per-element work per frame; the
`cpu_expand_ops` counter, bumped only by the CPU reference expansion, stays flat across
frames, which a test asserts. Measured throughput and the honest 100M shortfall are in
the [performance chapter](performance.md) and `docs/PERF.md`: expansion and culling run
at 3.4-3.7 billion elements per second, a 100M design pans interactively at 111 fps when
culling keeps the on-screen subset, and drawing all 100M sub-pixel quads at once stays
fill-bound at 10 fps (an LOD follow-up, for which this GPU-resident expansion is the
prerequisite).

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
