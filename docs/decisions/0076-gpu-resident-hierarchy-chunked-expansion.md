# 0076, fully GPU-resident hierarchy: chunked expand + cull + compact past the single-dispatch cap

> Placeholder ADR number (0074). The orchestrator finalizes the number at the Wave 3
> merge; the README row below carries the same placeholder.

## Context

The retained path (`RetainedScene`, `RetainedRenderer`) expands every instance and array
element into a per-placement transform buffer *on the CPU* once, then redraws it. That
holds to tens of millions of placements, but two things scale with the flat-equivalent
shape count: the one-time CPU expansion walk, and the materialized per-placement buffer.
A 100M-element arrayed design (a via, fill, or bit-cell array, routine in real IC
layout) pays a large one-time CPU cost and stores every placement, and a plain edit
re-pays the walk.

The GPU cull stage (`CellCuller`) already flags visible cells on the GPU, and
`CellCompactor` stream-compacts a visibility buffer into an indirect draw list. But that
compaction is a single dispatch, bounded two ways on a default-limits device:

- at most `max_compute_workgroups_per_dimension` (65,535) workgroups, so ~4.19M elements
  at 64 threads or ~16.7M at 256; and
- a storage binding no larger than `max_storage_buffer_binding_size` (128 MiB).

The known follow-up this lane owns is to escape that cap and move instance/array
expansion itself onto the GPU, so the per-frame CPU cost of a huge arrayed design is
O(chunks), not O(elements).

## Decision

**One compact scene upload, one fused compute pass per frame, one indirect draw per
chunk.** `GpuHierarchy` uploads a table of `ArrayPlacement` records (one per array
reference, carrying the base transform and the columns/rows/pitches) plus a table of leaf
cells, once. Element offsets (the exclusive prefix of each placement's `columns * rows`)
are assigned at upload, making the global element space contiguous and searchable.

Each frame, `expand_cull_compact.wgsl` runs one thread per array element over a bounded
chunk of that element space:

1. **Expand.** The thread binary-searches the placement table by cumulative element
   offset to find its array and `(row, col)`, then composes the element's placement
   transform exactly as the CPU retained path does (`base.apply(x) + O·mag·d`), emitting a
   ready-to-draw `RectInstanceT`.
2. **Cull.** It transforms the leaf rect's bounding box and tests it against the viewport
   with the same half-open rule as `cull.wgsl` and `Rect::intersects`.
3. **Compact.** Survivors are stream-compacted with the same per-workgroup exclusive
   prefix scan and single atomic range reservation as `compact.wgsl`, but the scattered
   payload is the full expanded instance (not an index), and each survivor also bumps the
   chunk's indirect `instance_count`.

**The single-dispatch cap is escaped by chunk COUNT, not a bigger dispatch.** The global
element space is split into fixed-size chunks, each no larger than the storage-binding and
workgroup-count limits (derived from the live device limits; ~2.79M survivors per chunk on
the 128 MiB-binding host). One compute dispatch and one `draw_indirect` are issued per
chunk. The compacted output is byte-identical to the retained vertex layout, so the draw
reuses the existing retained rect pipeline, with no new draw pipeline and no repack.

**The compacted survivors are drawn with a non-indexed `draw_indirect` per chunk,** on the
retained rect pipeline (a four-vertex triangle-strip unit quad, instance-step
`RectInstanceT`). The compute pass fills the instance count; the CPU passes no count.

## Consequences

- **Zero per-frame CPU draw-list touch.** The per-frame path (`expand` + `draw`) iterates
  only the chunk list (a handful of entries); the scene tables are uploaded once. A
  process-wide `cpu_expand_ops` counter, bumped only by the CPU reference expansion used
  in tests, stays flat across frames, asserted by `frame_path_does_no_cpu_per_element_work`.
- **Scales past the cap by construction.** 30M elements span 11 chunks, 100M span 36; the
  per-chunk cost is fixed, so arbitrarily large arrayed designs stay bounded.
- **Correctness is pinned against a CPU reference.** The GPU survivors equal a trivial CPU
  expansion+cull as a multiset (expanded instances are bit-identical: integer translation,
  floats copied straight from the cell/placement), including across many small chunks.
- **v1 leaf scope: one rect per cell.** The benchmark leaf is a single rect, so the
  flat-equivalent count equals the element count. A multi-rect leaf is a direct extension
  (each visible element scatters its cell's rects, the scan counting rects rather than
  elements) and is a follow-up, not a rework.
- **Honest 100M shortfall.** Expansion + culling runs at 3.4-3.7 G elements/s, so a 100M
  design pans interactively at 111 fps when culling keeps the on-screen subset. Drawing
  *all* 100M sub-pixel quads at once is fill/vertex-bound at 10 fps, an LOD follow-up
  (a coarser representation when the whole design is in view), for which this
  GPU-resident expansion is the prerequisite. Numbers in `docs/PERF.md`.
- The existing `CellCuller` / `CellCompactor` public API is unchanged; this is an
  additive module (`gpu_hierarchy.rs`) plus one shader, reusing the retained pipeline and
  the compaction scan pattern.
