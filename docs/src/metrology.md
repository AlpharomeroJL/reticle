# Metrology

`reticle-metrology` turns a laid-out [`Document`](model.md) into a small set of
quantitative reports. Every measurement runs on the CPU with exact integer
geometry (no GPU, no sampling), over the flattened top cell, and never mutates the
document. It reads only the public APIs of `reticle-model`, `reticle-geometry`,
and `reticle-extract`.

## Reports

**Area and perimeter per layer.** For each layer that carries geometry,
`area::report` unions that layer's flattened shapes on the exact `i_overlay`
integer engine (through `reticle_geometry::polygon_boolean`) and returns a
`LayerMetrics { layer, area, perimeter, shape_count }`. Area is the covered area
in DBU squared with overlaps counted once; perimeter is the total union boundary
length in DBU, including the boundaries of holes. Both are exact for integer
(manhattan) geometry. A property test cross-checks area and perimeter against an
independent coordinate-compression oracle over 200 random rectangle layouts.

**Connectivity statistics.** `connectivity::stats` extracts nets with
`reticle-extract` using the SKY130 via/contact stack, so conductors on different
layers joined by a via count as one net. It reports `net_count`, `net_sizes`
(shapes per net, largest first), `total_shapes`, and `max_fanout` (the largest
net's shape count).

**Antenna ratio.** `antenna::check` is a deliberately small screen of the antenna
effect: for each net it computes `connected_metal_area / gate_area` over a SKY130
layer subset and flags nets above a threshold. The semantics and their limits are
stated in full in the module documentation and repeated here so no one mistakes
the screen for sign-off:

- `gate_area` is the union area of polysilicon (`poly`, 66/20) on the net,
  approximated as all poly, not poly intersected with diffusion.
- `connected_metal_area` is the union area of `li1` (67/20) and `met1`..`met4`
  (68/20, 69/20, 70/20, 71/20) on the net. Contacts and vias join nets but are
  not counted as metal.
- A net with no poly has no gate and is never flagged.

This reduces the whole net to a single total-metal-to-gate ratio. It does not
model per-metal-layer cumulative ratios across fabrication steps, sidewall or
perimeter terms, diffusion-diode protection, or partial-route (as-built) area.
Treat a flag as "worth a closer look", not as a rule violation.

## Export

`MetrologyReport::generate` bundles all three reports; `to_csv` and `to_markdown`
render the bundle deterministically (newline-only line endings, fixed number
formatting), so the output is byte-stable and safe to diff. Golden tests pin both
renderings.

## Scope

This is the CPU half of the metrology work. A GPU density overlay was scoped and
explicitly deferred (see the metrology decision record); it is not built here.
