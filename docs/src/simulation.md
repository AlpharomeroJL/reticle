# Circuit simulation

Scaffolded in the v8.2 campaign Phase 3. The bounded circuit simulator (crate
`reticle-sim`) turns an extracted netlist into F4 waveform records
([`WaveformSet`](https://docs.rs/reticle-sim)), wall-clock and memory bounded,
with deterministic ordering.

This chapter is filled by the `sim-engine` lane once the simulation route is
chosen. The route decision (a vendored ngspice-WASM build, a pinned emscripten
toolchain, or a pure-Rust modified-nodal-analysis solver) is made by the
`oracle-feasibility` lane and recorded in its ADR; whichever route ships is
labelled honestly here, never presented as something it is not.
