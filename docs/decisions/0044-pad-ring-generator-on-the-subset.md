# 0044, Pad-ring generator: die-aware I/O ring on the subset, power pads as via staples

## Context

The generator framework (ADR 0042) and its DRC-clean-by-construction discipline (ADR
0043) call for more generators built the same way. A pad ring is a natural one: a
die-size-aware ring of I/O pad structures around the die edge, with corner handling
and power pads, driven from a few numbers (die size, pad pitch, pad size, power-pad
count).

Two things make a real pad ring hard to represent against the committed SKY130 subset
(`tech/sky130-drc-subset.toml`). First, the subset carries no pad, bump, passivation,
or redistribution layers; it is the digital metal stack (`li1`, `met1`, `met2`,
`met3` and the `mcon`/`via`/`via2` cuts) plus poly/diff. There is no layer on which a
"pad opening" or a "power pad" is a distinct object. Second, the checker measures
per-shape bounding boxes: width is `min(w, h)` of each rectangle, spacing is the gap
between any two shapes on a layer, and a gap in `(0, min_spacing)` is a violation
while a gap of `0` (touching or overlapping) is not. So the whole ring, corners
included, has to be laid out so that no two same-layer pads ever land a sub-spacing
gap apart.

A first cut placed all four edges' pads with a shared corner keep-out and let the
bottom and top rows march across the full die width. The cleanliness proptest
immediately found the flaw: a bottom-row pad near a corner and a right-column pad
could land one DBU apart, an `m3.2` spacing violation. Rows and columns were competing
for the same corner real estate.

## Decision

Build the pad ring on `met3` (the top conductor the subset carries) with a topology
that makes corner spacing provable, and represent a power pad as a real reinforcement
structure rather than a relabelled pad.

**Corner ownership.** The left and right columns run the full die height; the bottom
and top rows fill only the interior width between the columns, inset by a pad plus the
`met3` spacing from each side. A row pad therefore always clears the perpendicular
column pad by at least the pad spacing, and the two corners never place two pads a
sub-spacing gap apart. Pads sit a fixed `met3`-spacing inset in from the die edge. The
pad pitch is validated to be at least the pad size plus the `met3` spacing, and the
die is validated to fit at least one column pad and one interior row pad and to keep
the opposite rows a full spacing apart. The one-DBU-corner case the proptest found is
structurally impossible under this layout.

**Power pads as via staples.** The subset has no distinct pad layer, so a power pad is
made geometrically real by stapling it to the level below: the same `met3` square,
plus a `met2` backing plate and a bounded `via2` cut array stitching the two, using
the same enclosure argument as the via farm (both plates cover the cut array grown by
a conservative margin, since the subset carries no `via2` enclosure). The staple is
capped at six cuts per axis regardless of pad size; a real supply pad is stitched by a
compact via array, not a pad-wide flood, and the cap keeps the emitted geometry
bounded (a 60 um pad would otherwise tile tens of thousands of cuts, which also made
the shared registry cleanliness test run for a minute). The requested power pads are
strided evenly around the ring so they are distributed, not bunched.

Cleanliness is proven the same way as the framework generators: a 400-case proptest
(`pad_ring_is_drc_clean`) sweeps valid parameters, generates, runs
`DrcEngine::new(sky130_drc_rules())`, and asserts zero violations; `validate` is
covered two-directionally (`pad_ring_validate_accepts_valid`,
`pad_ring_validate_rejects_invalid`).

## Consequences

- The pad ring is DRC-clean by construction against the committed subset, proven over
  the whole valid parameter space, with the corner interaction that a hand-argument
  missed caught and fixed by the proptest.
- It is honestly not a tape-out pad ring. A real one needs the passivation and pad
  opening, the top thick metal and redistribution, the bump or wire-bond geometry, and
  ESD devices under each pad, none of which the subset carries. "Power pad" here means
  a pad reinforced to `met2` by a via staple, not a pad tied to a real supply net. The
  crate docs and the generator description say so.
- The staple cap is a deliberate, documented bound, not an accident: it keeps geometry
  and DRC time linear in the pad count rather than the pad area.
- Because the pad layer and the reinforcement layers are baked to `met3`/`met2`/`via2`
  rather than read from the technology, loading a different technology does not
  retarget the generator; it stays a SKY130-subset pad ring, consistent with ADR 0043.
