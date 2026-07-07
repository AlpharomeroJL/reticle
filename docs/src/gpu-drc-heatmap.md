# GPU DRC heatmap

The GPU DRC heatmap is a compute-shader design-rule overlay: it evaluates two rules,
minimum feature width and minimum edge-to-edge spacing, over the *visible* rectangle
instances entirely on the GPU, and paints the result as a coarse violation heatmap on top
of the rendered layout, with a live rule-value slider. It is the interactive, at-a-glance
companion to the exact CPU [`DrcEngine`](drc.md): the CPU engine is the source of truth
for a full sign-off pass; the heatmap is the cheap, per-frame "where are the hot spots"
view of what the eye is currently looking at.

Like the GPU-driven cull stage, it is WebGPU-only (ADR 0009 / ADR 0027); the WebGL2
fallback keeps the CPU DRC path.

## Two passes, reusing the cull scan

The heatmap runs as two compute stages over the same instance buffer.

**Binning** sorts the visible instances into a uniform grid with a counting sort. A first
pass computes each instance's grid cell from the minimum corner of its world bounding box
and does an `atomicAdd` into a per-bin counter. A single 256-thread workgroup then runs an
exclusive prefix scan over those counts, the *same* Hillis-Steele scan the stream
compactor uses for the draw list, to turn per-bin counts into per-bin start offsets. A
final scatter pass reserves a slot per instance with one more `atomicAdd` and writes the
instance index into its bin's dense slice. The grid is capped at 256 bins so the whole
scan fits one workgroup.

**Checking** runs one thread per instance. It flags a min-width violation when the smaller
side of the instance's world box is below the width rule, and a min-spacing violation when
any instance in its 3x3 bin neighbourhood sits at a strictly positive edge gap below the
spacing rule. Each flagged instance is recorded in a per-instance flag buffer and adds one
to its bin's entry in a coarse heatmap buffer, so the heatmap holds the count of violating
instances per bin, the field the overlay draws.

### Why 3x3 is enough

The neighbourhood search is only correct if no violating partner can hide outside the 3x3
window. The binning grid guarantees that: the bin size on each axis is chosen (on the CPU)
to be at least `max_instance_extent + min_spacing`, so any two instances whose edge gap is
below the spacing rule must land in bins at most one apart on each axis. The 3x3 search is
therefore exhaustive, the GPU never misses a violation the CPU engine would find. Capping
the grid at 256 bins only makes bins *larger*, never smaller, so it preserves the
invariant.

## Agreement with the CPU oracle

The compute path works in `f32`, but its geometry mirrors `reticle-drc`'s exact integer
helpers, the same signed interval gap, the same floor-integer-square-root for the diagonal
corner distance, so it agrees with the CPU engine bit-for-bit as long as every coordinate,
width, and height is an integer strictly below `2^24` (the range over which `f32`
represents consecutive integers exactly). Inside that range, a property test generates
randomized layouts, runs the CPU `DrcEngine` restricted to width and spacing as the
oracle, reads back the GPU flags, and asserts the GPU-flagged instance set equals the CPU
violating-instance set. It skips honestly when no GPU adapter is present, mirroring the
other compute gates.

The `2^24` bound is not a limitation in practice: database-unit coordinates for a cell
under interactive DRC sit far below it, and the CPU engine remains available for the exact
full-database pass.

## The overlay and the slider

The per-bin heatmap stays GPU-resident. A small render pipeline draws one alpha-blended
quad per grid bin, coloured by a heat ramp scaled by the bin's violation count; empty bins
are discarded so the layout shows through. Because the heatmap never leaves the GPU,
dragging the rule-value slider is cheap: it re-runs the two compute passes with the new
rule value and redraws, with no CPU readback in the loop. Measured recompute times are in
the [performance chapter](performance.md); on the visible working set a recompute stays
well under a frame.
