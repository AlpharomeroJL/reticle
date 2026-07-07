<!--
Captured output of TinyTapeout's own precheck run over this tile via
`just tt-precheck examples/tapeout/tt_um_reticle_tile.gds` (pinned image
hpretl/iic-osic-tools:2025.01, tt-support-tools main, tech sky130A), on 2026-07-06.
This is TinyTapeout's report verbatim; see docs/src/tapeout.md and ADR 0059 for the
reading of it. The tile passes every geometry, DRC, and structural check against
TinyTapeout's own Magic+KLayout decks. The four remaining failures are submission
artifacts a GDS-geometry generator does not produce: a LEF pin abstract, a Verilog
interface view (twice), and analog pins wired to a design (the six ua[*] here are
met4 landing pads on a template plus an isolated probe-able structure, not a wired
design). Nothing here is edited; regenerate it by re-running the command above.
-->

# Tiny Tapeout Precheck Results

| Check | Result |
|-----------|--------|
| Magic DRC | ✅ |
| KLayout FEOL | ✅ |
| KLayout BEOL | ✅ |
| KLayout offgrid | ✅ |
| KLayout pin label overlapping drawing | ✅ |
| KLayout zero area | ✅ |
| KLayout Checks | ✅ |
| Pin check | ❌ Fail: [Errno 2] No such file or directory: '/work/gds/tt_um_reticle_tile.lef' |
| Boundary check | ✅ |
| Power pin check | ❌ Fail: [Errno 2] No such file or directory: '/work/gds/tt_um_reticle_tile.v' |
| Layer check | ✅ |
| Cell name check | ✅ |
| urpm/nwell check | ✅ |
| Analog pin check | ❌ Fail: Analog pin `ua[0]` is not connected to any adjacent metal but `analog_pins` is set to 6 in `info.yaml`. Either wire up `ua[0]` to your design or decrease `analog_pins` to 0. |
| Verilog syntax check | ❌ Fail: [Errno 2] No such file or directory: 'yowasp-yosys' |
