# xschem interoperability

Reticle exchanges netlists and probe selections with
[xschem](https://xschem.sourceforge.io/), the open-source schematic capture and
simulation front end: `file.export_spice` writes the open design's extracted
devices as a SPICE subcircuit, and `xschem.import_probe` reads back a list of
nodes a user chose to plot, so a layout can round-trip through xschem/ngspice for
schematic-level verification or simulation setup.

Implemented in `crates/reticle-app/src/xschem.rs` (ADR
[0112](../decisions/0112-xschem-interop.md)), fixture-first against the committed
SPICE exchange contract
(`crates/reticle-extract/tests/fixtures/contracts/spice_exchange_inverter.{spice,json}`),
so this piece did not block on the parallel `netlist` and `waveform-ui` lanes.

## SPICE export

`file.export_spice` (File > Export > Export SPICE netlist...) runs
`reticle_extract::extract_devices` over the open document's top cell, then writes
the recognised devices as one SPICE subcircuit: a `.subckt NAME <ports...>`
header, one `X` card per device (`Xn <drain> <gate> <source> <bulk> <model>
w=... l=...`), `.ends`, `.end`. The interchange subset matches
[SPICE netlist export](spice-export.md).

The device-name table and the decimal-micron formatting are a small, temporary
bridge (`spice_netlist_from_devices` in `xschem.rs`): exact integer arithmetic for
the width/length conversion, but only the two model names the committed contract
fixture names. Once the `netlist` lane's real writer (`reticle_extract::spice`)
merges, this bridge is replaced by a direct call to it; see ADR 0112 for the full
reasoning. An open design with no recognised devices reports that honestly rather
than exporting an empty or invented subcircuit.

## Probe import

`xschem.import_probe` reads Reticle's own minimal probe-list interchange subset,
not xschem's native schematic file format (out of scope for this lane): one probe
per line, whitespace-separated `<id> <node> <quantity>`, `#` starts a full-line
comment, blank lines are skipped.

```text
# probe list: id node quantity
in A voltage
out Y voltage
```

`quantity` is `voltage`, `current`, or `charge`, the same three variants and wire
strings as [`reticle_sim::Quantity`](https://docs.rs/reticle-sim) (the
[F4 waveform-record contract](waveforms.md)), so an imported probe already matches
the shape a later lane needs to promote it into a real waveform-panel probe once a
`WaveformSet` exists. The parser is capped (input size and probe count checked
before any per-line work) and never panics on malformed input; a bad line is a
clear, structured error naming the line and the problem. Native only for now: the
browser file picker for probe lists is not wired in this lane.

## What this lane does not do

xschem's native schematic file format (the `.sch`/`.sym` grammar, symbols,
graphical placement) is out of scope entirely; only the two interchange pieces
above are implemented. The live SPICE-writer bridge above is deliberately narrow
(two device kinds, one technology); it is not a substitute for
`reticle_extract::spice` once that lands.
