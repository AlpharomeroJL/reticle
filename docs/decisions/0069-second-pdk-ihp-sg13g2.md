# 0069, A second PDK (IHP SG13G2) as data, proven by the both-PDK cleanliness proptests

## Context

ADR 0068 made the generator process numbers data (`GenTech`). The point of that work is
a second PDK: a real process, added as data, that the existing generators produce
DRC-clean geometry for without touching the topology code. IHP's SG13G2 - an open
130 nm SiGe BiCMOS process published as the IHP-Open-PDK under Apache-2.0 - is the
choice. SKY130's tech files are frozen (ADR 0061); the second PDK must be purely
additive.

## Decision

Commit `tech/ihp-sg13g2.tech` (the layer table, the physical stack from the 2.5-D BEOL
file, and the DRC subset inline as `rule` lines) and `tech/sg13g2-drc-subset.toml` (the
cited source of record, format-matching `sky130-drc-subset.toml`). Every number is
transcribed from the IHP-Open-PDK KLayout DRC runset with the rule ids preserved, and
the provenance and Apache-2.0 license are recorded in both files. A test asserts the two
committed representations agree.

The generators' roles bind to the bottom of the SG13G2 aluminium BEOL: conductors
Metal1–Metal4, cuts Via1/Via2/Via3, and `Cont` as the substrate tap (the role table is
in the second-PDK book chapter). The subset carries Metal1–Metal4 width/spacing, the
Cont/Via1–3 sizes, and the via metal enclosures; it deliberately omits what the
generators do not draw against and what the DRC engine cannot express: the wide-metal
and pattern-density spacing variants (width-conditional), the FEOL Activ/GatPoly contact
enclosures, and the thick TopMetal stack. Passing it is not tape-out clean, exactly as
for the SKY130 subset.

The proof is the generalized cleanliness oracle (`tests/second_pdk.rs`): every generator
is generated over both `[sky130, sg13g2]`, with each process's DRC engine built from its
own committed rules, and asserted zero-violation. SKY130's rules come from
`reticle_drc::sky130_drc_rules()`; SG13G2's come from parsing the committed `.tech` with
`reticle-io` (a **dev-dependency** only, so the library stays `wasm32`-clean). Two
provenance tests anchor the data: `derive_gentech(parsed .tech) == GenTech::sg13g2()`,
and the `.tech` rules equal the `.toml` subset.

## Consequences

The second PDK is data plus one generator change (the contact chain's both-level
enclosure, ADR 0068). No topology, schema, or validation code is PDK-specific. The
subset's honesty limits carry forward - a wider deck would need DRC-engine support for
width-conditional spacing, out of scope here. The role mapping assumes a four-conductor
routing stack with a substrate contact, which both shipped processes have; a process
shaped differently (more or fewer routable levels) would need the role model to grow.
