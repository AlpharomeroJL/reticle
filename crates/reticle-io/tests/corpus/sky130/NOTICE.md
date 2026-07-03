# SKY130 standard-cell corpus attribution

The `.gds` files in this directory are unmodified copies of standard-cell
layouts from the SkyWater SKY130 high-density standard cell library
(`sky130_fd_sc_hd`):

- Source: <https://github.com/google/skywater-pdk-libs-sky130_fd_sc_hd>
- Upstream path: `cells/<name>/sky130_fd_sc_hd__<name>_1.gds`
- Fetched from branch `main` at commit
  `ac7fb61f06e6470b94e8afdf7c25268f62fbd7b1` on 2026-07-02
- Copyright 2020 The SkyWater PDK Authors
- License: Apache License, Version 2.0 (the library's `LICENSE`; the same
  license text is included in this repository as `LICENSE-APACHE`)

They are committed as test fixtures for the GDSII importer
(`tests/sky130_cells.rs`) and the SKY130 DRC subset (`tests/sky130_drc.rs`).
The three files here are the smallest representative cells (a filler, a well
tap, and an inverter); `scripts/fetch-sky130-cells.ps1` re-fetches the full
set, including the larger `nand2_1` and `dfxtp_1` used by the ignored
external round-trip test.
