# xschem interoperability

Scaffolded in the v8.2 campaign Phase 3. Reticle exchanges netlists and probe
points with [xschem](https://xschem.sourceforge.io/), the open-source schematic
capture tool: `file.export_spice` writes the extracted netlist, and
`xschem.import_probe` reads back the nodes a user chose to plot.

The lane builds fixture-first against the committed SPICE exchange contract
(`crates/reticle-extract/tests/fixtures/contracts/spice_exchange_inverter.*`) so
it does not block on the `netlist` lane's writer. This chapter is filled by the
`xschem` lane.
