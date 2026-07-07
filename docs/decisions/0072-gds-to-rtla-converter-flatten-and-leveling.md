# 0072, the `reticle convert` GDS-to-`.rtla` flatten scope and pyramid leveling

## Context

Lane 2A froze two streaming pieces: the forward-only `GdsRecordReader` (one record at a
time over any `Read`, ADR 0062) and the external `build_rtla` builder (bounded memory,
disk spill, ADR 0068). `reticle convert <in.gds> <out.rtla>` joins them into a CLI
command that turns a GDSII file into a streamable archive. Two choices are not implied
by either frozen surface and have to be pinned in writing.

First, **what geometry becomes a record.** A `.rtla` archive stores flat, world-space
`TileRecord`s; a GDSII file is a hierarchy of structures placed by `SREF`/`AREF`
references. Composing a placement into world space needs the referenced structure's
geometry, which may be defined anywhere in the file, so true hierarchical flattening
needs random access to every cell at once (a whole-file DOM). That is exactly what the
streaming reader (and the multi-gigabyte-die use case behind it) exists to avoid.

Second, **how deep the pyramid is.** `build_rtla` takes the per-level grid dimensions as
given in the header; it does not choose them. The converter must pick a level count and
per-level grid from nothing but the world bounding box it discovers while scanning.

## Decision

**Flatten scope (v1): drawn geometry only, in authored coordinates.** Each `BOUNDARY`
and `PATH` becomes one `TileRecord`: its bounding box, in database units, exactly as
written (a path's box is inflated by half its width so the drawn wire is covered).
`SREF`/`AREF` references are **not** expanded; a referenced cell's own shapes are still
captured where they are drawn, in that cell's local frame. So a hierarchical GDS
converts to the union of every cell's drawn shapes in their own coordinates. This keeps
the converter fully streaming (no DOM), at the cost of not reproducing instanced
placements. True hierarchical flattening is a documented follow-up; a flat or
already-flattened GDS (what `xtask gen-layout` and the export path emit) converts
faithfully today.

**Two streaming passes.** Pass 1 streams the file once to accumulate the world box,
recover `dbu_per_micron`, and count records, holding only running totals. Pass 2 reopens
the file as a lazy `Iterator<Item = TileRecord>` and hands it to `build_rtla`. Peak
memory is the builder's spill budget, never the file size. An empty input yields a valid
unit-box archive; a degenerate (zero-width or -height) world is nudged out one DBU so the
builder's positive-area check passes.

**Leveling.** The world span (the longer of width/height) picks the depth: the finest
level is sized so a finest tile is roughly `TARGET_FINEST_TILE_DBU` (1024 DBU) across,
rounded to a power of two and clamped to `[1, MAX_LEVELS]` (12). Level `i` is a square
`2^i × 2^i` grid over the world box, coarsest (index 0) to finest (last), matching
`build_rtla`'s "finest is last" convention. A square grid over a very non-square world
gives non-square tiles, which the builder's tile-span math handles; making the grid
aspect-aware is a v1 simplification left out.

**Determinism.** Records are emitted in document order, the world box is an
order-independent union, the pyramid is a pure function of the world span, and
`build_rtla` writes no timestamps. So the same input produces byte-identical output.

## Consequences

- The converter never holds a DOM, so it scales to the same dies `build_rtla` targets.
- Instanced/arrayed placements are dropped in v1; a design that draws all geometry in one
  cell (the common tape-out flat GDS) round-trips exactly, a deeply hierarchical one does
  not reproduce its placements until the follow-up lands. The limitation is stated in the
  book and surfaced here rather than failing silently.
- A byte-determinism test (two conversions of one fixture are byte-equal) and a
  round-trip test (the archive is read back by lane 2B's `MmapTileSource`, header and a
  finest-level tile validated) guard the contract.
- The `1024`-DBU target and `12`-level cap are heuristics, not frozen surface; they can be
  tuned without changing the format or the reader.
