# 0045, Seal-ring generator: a stacked-metal-plus-cut barrier on the subset

## Context

Alongside the pad ring (ADR 0044), a seal ring is the other classic die-edge
structure worth generating: a continuous barrier that encircles the whole die to stop
cracks and moisture ingress at dicing. Physically it is a stack of metal running from
a low level to the top, stitched by a wall of cuts, closed around the die perimeter.

The committed SKY130 subset (`tech/sky130-drc-subset.toml`) constrains how much of
that is representable. It carries the digital metal stack (`li1`, `met1`, `met2`,
`met3`) with width, spacing, and (for `li1`/`met1`) area rules, and the
`mcon`/`via`/`via2` cuts with their sizes and the enclosures the deck lists
(`m1.4` mcon-by-met1 30, `m2.4` via-by-met2 55; `via2` has no subset enclosure). It
does not carry the seal-ring or pad-protection marker layers, the top thick metal, or
the passivation and redistribution openings a real seal ring also needs. The checker
is per-shape and bounding-box based, exactly as for the pad ring: every rectangle's
short side must clear the layer width, opposite faces across the die interior must
clear the layer spacing, and every cut must be fully enclosed by a covering plate.

## Decision

Build the seal ring as a closed frame on each conductor level of a chosen stack,
stitched by rings of cuts, reusing the guard-ring frame and the via-farm enclosure
arguments (ADR 0043).

**Frames.** For each conductor level in the stack, emit four overlapping strips
forming a closed frame flush with the die outline, `ring_width` thick, sharing the
corner squares so the loop is connected (the same construction as the guard ring).
Because each level is on its own GDS layer, the frames stack directly on top of one
another with no cross-layer spacing rule to worry about. `ring_width` is validated to
be at least the widest conductor minimum width in the stack, so every strip clears
width; the die interior (`die_width`/`die_height` minus two ring widths) is validated
to be at least the widest conductor minimum spacing, so opposite strips clear.

**Cut rings.** Between each adjacent pair of levels, run a ring of square cuts along
all four strips: a row centered in the bottom and top strips, a column centered in the
left and right strips, each cut at its exact drawn size and centered across the strip
thickness. `ring_width` is validated to cover a cut plus the largest step enclosure on
both sides, so every cut is enclosed on all four sides by the frames above and below.
Cuts step at the cut size plus the shared safe margin, and the corners are left
cut-free so no two cuts on a layer crowd; the subset carries no cut-to-cut spacing
rule, so this pitch is a conservative choice, stated at the call site, not a checked
constraint. The `mcon` step uses the `m1.4` margin (30) for both frames since `li1`
has no subset mcon enclosure; the `via` step uses `m2.4` (55); the `via2` step uses a
conservative positive margin because the subset gives it no rule, mirroring the via
farm.

**Stack choice.** A `SealStack` enum picks how tall the barrier is: `li1`+`met1`;
up to `met2`; or up to `met3` (the tallest the subset supports, the default). A taller
stack is a more complete wall.

Cleanliness is proven as elsewhere: a 400-case proptest (`seal_ring_is_drc_clean`)
sweeps valid parameters across all three stacks, generates, runs
`DrcEngine::new(sky130_drc_rules())`, and asserts zero violations; `validate` is
covered two-directionally (`seal_ring_validate_accepts_valid`,
`seal_ring_validate_rejects_invalid`).

## Consequences

- The seal ring is DRC-clean by construction against the committed subset, over the
  whole valid parameter space and all three stack depths, against the same engine the
  app runs.
- It is honestly not a tape-out seal ring. A real one needs a dedicated seal-ring
  marker, the top thick metal, and the passivation and redistribution openings, none
  of which the subset carries. This generator builds the barrier only on the digital
  metal stack the subset checks. The crate docs and the generator description say so.
- The design is deliberately the guard ring's frame plus the via farm's enclosed-cut
  argument, generalized to a stack. Reusing those two proven constructions is what
  keeps the cleanliness argument short and the code small, and it keeps the three
  generators consistent rather than each inventing its own geometry discipline.
- As with the other generators (ADR 0043), the layer numbers are baked to the SKY130
  subset rather than read from the technology, so loading a different technology does
  not retarget it; it stays a SKY130-subset seal ring.
