# TinyTapeout analog-tile validation fixtures

These small text files hold coordinates *extracted* from TinyTapeout's own
published files, used to validate the "New TinyTapeout tile" template
(`crates/reticle-app/src/tinytapeout.rs`) against the real thing. No whole
third-party file is committed; only the few numbers the validation needs are kept,
with the source URL for each. All were fetched 2026-07-06 (shuttle TTSKY26c).

## Files

| file | provenance |
|------|------------|
| `analog_1x2_template.txt` | Extracted from TinyTapeout's canonical analog tile DEF template `tt_analog_1x2.def`: the `DIEAREA`, and the `ua[0]`..`ua[5]` met4 pin PORT rectangle and PLACED coordinates. This is the frame every current SKY130 analog submission is built on. Source: <https://raw.githubusercontent.com/TinyTapeout/tt-support-tools/main/tech/sky130A/def/analog/tt_analog_1x2.def> (Apache-2.0, TinyTapeout LTD). |
| `analog_power_straps.txt` | Extracted from TinyTapeout's official analog init script `magic_init_project.tcl`: the met4 power-strap vertical extent, width, and the per-net x positions (VDPWR, VGND, VAPWR). Source: <https://raw.githubusercontent.com/TinyTapeout/tt-support-tools/main/tech/sky130A/def/analog/magic_init_project.tcl> (Apache-2.0, TinyTapeout LTD). |
| `published_example_tt_um_analog_mux.txt` | Cross-check facts extracted from a real published GDS-mode submission, `tt_um_analog_mux`: its `DIE_AREA`, top routing layer, and the met5 prohibition, from its `config.json`, plus the `DIEAREA` of its committed floorplan DEF `def/tt_block_1x2.def`. Sources: <https://raw.githubusercontent.com/TinyTapeout/tt-analog-mux/main/config.json> and <https://raw.githubusercontent.com/TinyTapeout/tt-analog-mux/main/def/tt_block_1x2.def> (Apache-2.0, TinyTapeout). |

## Why extracted numbers and not the GDS/DEF files

The rule for this repo is to not commit large third-party design files. The full
DEF templates are hundreds of lines and the real submission carries megabytes of
GDS. The validation only needs the die area, the six analog-pin rectangles, and the
strap geometry, so those exact numbers are transcribed here verbatim (units are
DBU, 1 dbu = 1 nm, matching `UNITS DISTANCE MICRONS 1000` in the sources). Anyone
can re-fetch the source URLs above to confirm every number.
