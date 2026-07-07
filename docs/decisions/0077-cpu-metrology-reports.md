# 0077, CPU metrology reports: exact area/perimeter, connectivity stats, and an antenna screen

## Context

Reticle already computes design-rule results (`reticle-drc`) and geometric net connectivity
(`reticle-extract`), but nothing summarized a laid-out document quantitatively: how much of
each layer is covered, how the nets are shaped, or whether any net looks antenna-risky. A
metrology pass is the natural next report. The wider metrology idea also included a GPU
density overlay, which is a different concern (rendering, not measurement) with its own
surface area.

## Decision

**A new CPU-only crate, `reticle-metrology`, that reads the frozen public APIs of
`reticle-model`, `reticle-geometry`, and `reticle-extract` and never mutates a document.**
It provides four things over the flattened top cell:

1. **Per-layer area and perimeter.** Each layer's shapes are unioned on the exact
   `i_overlay` integer engine through `reticle_geometry::polygon_boolean`; area is the sum of
   signed contour areas (holes subtracted) and perimeter is the sum of contour edge lengths.
   Both are exact for integer geometry. Correctness is pinned by a property test against an
   independent coordinate-compression oracle, the same discipline `reticle-drc` uses.

2. **Connectivity statistics** from `reticle-extract` with the SKY130 via stack: net count,
   shapes per net, total shapes, and maximum fanout.

3. **A simplified antenna screen** over the SKY130 poly/li1/met1-4 subset: per net,
   `connected_metal_area / gate_area`, flagging nets above a threshold. Its limits are stated
   honestly in the module docs (single total ratio, poly approximated as gate, no per-layer
   accumulation, no diode protection). It is a screen, not sign-off.

4. **Byte-stable CSV and Markdown export** of the combined report, pinned by golden tests.

**The GPU density overlay is explicitly deferred.** It is not built in this crate or this
lane. Metrology here is measurement on the CPU; the overlay is a rendering feature that can
be taken up separately without disturbing this surface.

## Consequences

- A normal workspace member that compiles in the default `just ci` set. It adds no new
  external dependency beyond `serde` and (dev-only) `proptest`; the exact boolean engine is
  reused through `reticle-geometry`, not added directly.
- Area and perimeter are exact for manhattan geometry; perimeter of a diagonal edge is an
  irrational length carried in `f64`. Paths are stroked per segment (manhattan segments
  exact, a diagonal segment approximated by its grown bounding box), documented at the
  conversion site.
- The antenna number is a screening heuristic. Anyone treating it as a rule result is
  misreading the module documentation, which says so plainly.
- The deferred density overlay remains open work; nothing here blocks it.
