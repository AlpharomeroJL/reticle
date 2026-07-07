# 0075, GPU DRC heatmap: a bin-and-check compute overlay whose flags are pinned to the CPU oracle

> Placeholder number. This ADR was written on lane `v8-3b-gpu-heatmap`; renumber it to
> the next free record at the Wave 3 merge gate (0073 was the last committed record).

## Context

The renderer needs an interactive design-rule overlay: while browsing a layout, the user
wants to see *where* the min-width and min-spacing hot spots are and drag a rule-value
slider to explore them, per frame, over the shapes currently visible. The exact CPU
`DrcEngine` (ADR 0019) is the sign-off oracle, but running a full R-tree DRC pass on every
slider drag is the wrong tool: it is a whole-database batch check, not a per-frame overlay
of the visible working set.

Two forces shape the design. First, the check must be trustworthy: an overlay that
silently disagrees with the CPU engine is worse than none, so the GPU result has to be
provably the same set of violations the engine would report, not a lookalike. Second, the
compute path works in `f32` while the CPU engine is exact integer math, and a naive GPU
spacing check that scanned all pairs would be `O(n^2)`.

The binning half of the problem is already solved elsewhere in the crate: the GPU cull
stage (ADR 0009) uses a 256-thread workgroup prefix scan to compact a draw list. That same
scan is exactly what a counting-sort spatial bin needs.

## Decision

Ship the heatmap as two compute passes plus an overlay, WebGPU-only like the cull gates
(ADR 0027); the WebGL2 fallback keeps the CPU DRC path.

**Bin, reusing the cull scan.** A counting sort buckets the visible instances into a
uniform grid: an atomic count per bin, the *same* Hillis-Steele workgroup scan the stream
compactor uses to turn counts into offsets, then a scatter into dense per-bin slices. The
grid is capped at 256 bins so the whole scan is one workgroup.

**Check a 3x3 neighbourhood, sized for exhaustiveness.** One thread per instance flags
min-width from its own box and min-spacing by scanning the 3x3 bin neighbourhood. The bin
size is chosen on the CPU to be at least `max_instance_extent + min_spacing`, which
guarantees any two instances within the spacing rule land at most one bin apart, so the
3x3 search cannot miss a violation. Flags go to a per-instance buffer; each flagged
instance also `atomicAdd`s into a coarse per-bin heatmap the overlay draws, kept
GPU-resident so a slider drag re-runs the passes with no readback.

**Pin the flags to the oracle, and bound the coordinate range.** The shader geometry
mirrors `reticle-drc`'s exact integer helpers (signed interval gap, floor integer square
root for the diagonal). It is bit-exact against the CPU engine while every coordinate,
width, and height is an integer below `2^24`, the range over which `f32` represents
consecutive integers exactly. An adapter-gated property test runs the CPU `DrcEngine`
(width + spacing) as the oracle over randomized layouts in that range and asserts the
GPU-flagged instance set equals the CPU violating-instance set, skipping honestly where no
adapter exists (mirroring ADR 0027).

## Consequences

The overlay is cheap and trustworthy on its intended input, the post-cull visible working
set: recompute is 0.66 ms at 10,000 instances and 3.57 ms at 50,000 on an RTX 4060 Ti, so
the slider stays live under a frame, and every flag is provably one the CPU engine agrees
with. The 256-bin cap is what makes the scan single-workgroup and the invariant simple; it
also means per-instance cost is the fixed 3x3 scan of about `n / 256` instances per bin, so
this targets thousands-to-tens-of-thousands of visible instances, not a whole 1M-shape
database, which remains the CPU engine's job.

The costs are honest and bounded. The `f32` bit-exactness holds only below `2^24`; DBU
coordinates under interactive DRC sit far below that, and the exact engine covers the full
database. Only width and spacing are on the GPU (the two rules an eye scans a layout for);
the other rule kinds stay CPU-only. And, like every WebGPU compute stage here, the heatmap
does not run on the WebGL2 fallback. A follow-up is wiring the slider into the interactive
app (the app crate is out of this module's scope) and measuring the browser WebGPU figure,
which is not yet instrumented.
