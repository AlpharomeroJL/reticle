# 0047, Probe-able test structures, scoped to what the DRC subset can vouch for

## Context

A test tile carries the structures a probe station measures to characterize a process:
a van der Pauw cross for sheet resistance, a contact chain for per-contact resistance, a
comb for inter-line leakage and shorts, and a serpentine for line continuity and
resistance. These are standard, well-defined geometries, which makes them a natural fit
for the generator framework (ADR 0042): a kind, a layer, a line width and length, and a
repeat count in; DRC-clean geometry out.

The committed SKY130 subset (`tech/sky130-drc-subset.toml`) bounds what "DRC-clean" can
honestly mean. It carries min width, min spacing, min area for `li1` and `met1`, and three
enclosures, of which exactly one is a cut-to-interconnect enclosure the generators can
build a contact around: `mcon` enclosed by `met1` by 30 (`m1.4`). There is no
contact-to-poly or contact-to-diffusion enclosure, no notch rule, and no cut-to-cut
spacing. The DRC engine checks bounding boxes and never flags shapes that touch or
overlap. So a structure is clean on this deck when every drawn rectangle clears width and
area on its own, same-layer features that must not touch stay at least the minimum spacing
apart, features that connect touch or overlap, and every `mcon` sits inside a `met1`
feature by at least 30.

The temptation is to emit the "real" structures with all their layers (a poly serpentine,
a diffusion van der Pauw, a poly-contact chain). The subset cannot vouch for most of that,
so doing it would ship geometry the cleanliness test cannot actually check, which is the
opposite of the framework's promise.

## Decision

Ship a `test_structure` generator (`TestStructure`, id `"test_structure"`) with a
`StructureKind` parameter selecting the van der Pauw cross, contact chain, comb, or
serpentine, built only from geometry the subset can check.

Each structure is axis-aligned rectangles (plus `mcon` cuts for the chain), which the
bounding-box engine checks exactly:

- **Van der Pauw** is a horizontal and a vertical bar of the line width that overlap in
  the centre (overlap, not a spacing violation); each bar clears width and area on its
  bounding box.
- **Contact chain** is a row of `mcon` contacts with `met1` and `li1` bridges alternating
  along it, so current threads metal-contact-metal down the chain. Every contact sits at
  the end of a `met1` bridge (or, for the last contact when the last bridge is `li1`, a
  `met1` end pad grown to the `met1` minimum area), enclosed by at least the `m1.4` margin.
  Same-layer bridges are pitched so they clear the layer minimum spacing.
- **Comb** is two interdigitated combs on one layer whose fingers interleave at exactly
  the layer minimum spacing; a finger touches its own spine and stays a min-spacing gap
  from the other comb.
- **Serpentine** is parallel bars joined end to end by alternating links; a bar and its
  link touch, and adjacent bars stay a min-spacing gap apart.

Validation ties the free dimensions to the rules. The line width is validated against the
layer minimum width; the line length against the layer area-derived floor
(`ceil(min_area / width)`) and, for the serpentine, against a geometric floor of
`2 * width + min_spacing` so the two end-links of a bar clear each other. The single-layer
structures use the chosen interconnect layer; the contact chain is fixed to `li1`/`met1`
through `mcon`, the layers the subset gives the contact enclosure for, regardless of the
layer parameter.

## Consequences

- The 400-case cleanliness proptest (`test_structure_is_drc_clean`) sweeps all four kinds,
  every single-layer choice, and random in-range widths, lengths, and counts, runs the
  real engine, and asserts zero violations. A dedicated unit test also asserts every
  `mcon` in a chain is enclosed by `met1` by at least the `m1.4` margin, including the odd
  count case that exercises the `met1` end pad. `validate` is covered two-directionally.
- The serpentine's geometric length floor is load-bearing, not cosmetic. A bar shorter
  than `2 * width + min_spacing` puts its left-end and right-end links a hair apart (the
  proptest found a 1 DBU gap on an early version), so the floor is enforced in `validate`
  and has its own regression test. This is the same class of "the min rule forces a
  dimension" lesson as the via-farm plate area in ADR 0043.
- The scope is honestly narrow. These are clean on the subset, which is not tape-out
  clean, and the contact chain is an `mcon` chain because that is the one cut the subset
  gives an interconnect enclosure for; there is no poly or diffusion test structure,
  because the subset carries no contact-to-active enclosure. The crate docs state this
  coverage limit plainly rather than emitting layers the checker cannot vouch for.
- The four structures share one parameter struct (kind, layer, width, length, count),
  which keeps the schema small and the form simple, at the cost of a couple of parameters
  being ignored per kind (the cross ignores `count`; the chain ignores `layer`). The docs
  name each ignored case so the behaviour is not a surprise.
