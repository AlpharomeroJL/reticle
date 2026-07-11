# SPICE netlist export

Scaffolded in the v8.2 campaign Phase 3. Reticle extracts a `Netlist` from a
layout (crate `reticle-extract`) and writes it as a SPICE netlist for exchange
with external simulators and schematic tools.

The interchange subset is fixed by the committed contract fixture
`crates/reticle-extract/tests/fixtures/contracts/spice_exchange_inverter.spice`
(with its structural companion `.json`): `*` comments, one `.subckt` / `.ends`
wrapper, subcircuit device instance `X` cards with decimal-micron `w=`/`l=` and
area/perimeter params, and `.end`. This chapter is filled by the `netlist` lane.
