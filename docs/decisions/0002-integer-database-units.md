# 0002, Integer database units; `Dbu = i32`, widened math

## Context

Chip layout is defined on an integer grid (database units, DBU), not floating
point: coordinates are exact and booleans must be robust. GDSII stores coordinates
as 32-bit integers; OASIS supports larger deltas. We must pick a coordinate width
that interoperates with GDSII yet avoids overflow in area and bounding-box math
(where products of two coordinates can exceed 32 bits).

## Decision

Use `type Dbu = i32` for stored coordinates (GDSII-compatible, cache-friendly,
half the memory of `i64` for the millions of points we hold). Perform intermediate
area, product, and accumulation math in `i64` (or `i128` for area sums) and
saturate/validate on import. Expose `Dbu` as a single alias in `reticle-geometry`
so the width can change in one place if a design ever needs wider coordinates.

## Consequences

Memory-efficient point storage and exact arithmetic. Every multiply of two `Dbu`
values must widen to `i64` first; a lint and helper functions (`area_i64`,
`checked_bbox`) enforce this. Designs requiring coordinates beyond ±2^31 DBU are
out of scope for v3 and would require changing the alias and re-testing overflow
paths. The `i_overlay` integer API (see [0003](0003-i-overlay-for-booleans.md))
operates on `i32`, matching this choice.
