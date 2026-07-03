# Bundled asset attribution

## sky130_fd_sc_hd__inv_1.gds

An unmodified copy of the inverter standard-cell layout from the SkyWater
SKY130 high-density standard cell library (`sky130_fd_sc_hd`), bundled so the
"Inspect a SKY130 cell" worked use case (see `crate::usecases`) loads a real
production cell on both native and wasm with no filesystem access.

- Source: <https://github.com/google/skywater-pdk-libs-sky130_fd_sc_hd>
- Upstream path: `cells/inv/sky130_fd_sc_hd__inv_1.gds`
- Fetched from branch `main` at commit
  `ac7fb61f06e6470b94e8afdf7c25268f62fbd7b1` on 2026-07-02
- Copyright 2020 The SkyWater PDK Authors
- License: Apache License, Version 2.0 (the same license text is included in
  this repository as `LICENSE-APACHE`)

This is the same byte-for-byte file committed as an importer test fixture at
`crates/reticle-io/tests/corpus/sky130/sky130_fd_sc_hd__inv_1.gds`; see that
directory's `NOTICE.md` for the full corpus attribution. It is duplicated here
(rather than referenced across crates) so it ships as a first-class bundled
runtime asset, mirroring the bundled replay transcript next to it.

## theater-demo.transcript.jsonl

The bundled replay transcript for the wasm replay theater. Generated from the
model-free scripted agent run; see `crate::store` for how it is (re)produced.
