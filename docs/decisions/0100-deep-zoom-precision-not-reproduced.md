# 0100, Deep-zoom rendering: "starry" not reproduced on shipped examples; floating-origin deferred

## Context

Packet v8.1.0-R reported a "starry"/unreadable canvas after zooming into an
example, with suspects at extreme pixels-per-DBU: an LOD/culling threshold,
depth precision, or degenerate tessellation. The projection is an f32
`clip_from_world` matrix (`reticle-render` `view.rs`, `shapes.wgsl`), so at
extreme zoom the term `scale*world - scale*center` can cancel catastrophically in
f32 for geometry far from the world origin, which would scatter vertices.

This was investigated by reproduction, not theory: a headed zoom ladder from fit
to the `1e6` px/DBU cap, on both the WebGL2 and WebGPU backends, against the
now-correctly-colored start-screen examples (see the SKY130 layermap fix that
preceded this).

## Decision

**Do not implement the floating-origin GPU fix now; add a zoom-cycle coherence
guard and document the precision limit.**

The "starry" scatter did not reproduce on any shipped example at any zoom on
either backend. What deep zoom actually shows is one of: a solid colored fill
(the camera is inside one shape), the multi-layer structure (when the view still
frames features), or an empty field (over-zoomed past the integer-DBU geometry
into a sub-DBU gap). All three are expected for an integer-grid layout. The
shipped examples are small and near the origin (the SKY130 inverter spans
~1800 DBU; the Tiny Tapeout sample is a 4-cell tile), so the f32
catastrophic-cancellation regime, which needs coordinates millions of DBU from
the origin, is never exercised.

Implementing the correct remedy, a floating-origin (camera-relative) projection,
is a broad, risky change: it touches `ViewUniform`, all three `shapes.wgsl`
vertex entry points, and every other consumer of `clip_from_world`
(`drc_heatmap`, `gpu_hierarchy`, `indirect`). Making that change to fix a defect
that does not reproduce on shipped content would trade real regression risk for
no observed benefit, against the project's "root-cause before patching" and
"measure, never fabricate" rules. `MAX_ZOOM` is left unchanged: capping it would
not remove the inherent over-zoom-into-empty-space (any ppd past ~viewport
pixels per DBU lands between integer grid points) and would only take away user
zoom range.

## Consequences

- The demo's zoom is coherent and now guarded: `e2e/tests/demo-deep-zoom.spec.ts`
  drives a headed zoom-in-to-max-and-back cycle on both backends and asserts the
  app stays live (`render_nonblank`) at deep zoom and the design returns intact
  (`applied_scene_shapes` unchanged) after a Fit, with no fatal console errors.
- The f32 precision limit remains for geometry far from the world origin (a full
  die placed at millions of DBU, zoomed to the extreme). If such a design ever
  ships in the demo, the scoped follow-up is a floating-origin projection: add an
  `i32` world origin (the camera center, snapped) to the view uniform, subtract it
  from the per-instance integer translate in the shader before widening to f32,
  and build the ortho around that origin. This is recorded here so that case is a
  known, bounded task rather than a surprise.
- No engine, GPU, or shader code changed in this packet for zoom; the deliverable
  is the guard plus this record.
