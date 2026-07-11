# Circuit simulation

Scaffolded in the v8.2 campaign Phase 3. The bounded circuit simulator (crate
`reticle-sim`) turns an extracted netlist into F4 waveform records
([`WaveformSet`](https://docs.rs/reticle-sim)), wall-clock and memory bounded,
with deterministic ordering.

## Route: a pure-Rust MNA solver

The route is decided in [ADR 0109](https://github.com/AlpharomeroJL/reticle/blob/main/docs/decisions/0109-simulation-route.md):
the simulator is a **pure-Rust modified-nodal-analysis (MNA) transient/DC
solver**, not ngspice and not a SPICE engine compiled to WebAssembly. It is
licensed MIT OR Apache-2.0 (the workspace license) and is described as a
pure-Rust MNA solver everywhere, never as ngspice.

Two routes were rejected on measured evidence. Vendoring ngspice to WASM ships a
multi-megabyte module (`eecircuit-engine` alone is ~20 MB per build variant) that
overruns the sim bundle headroom by roughly two orders of magnitude, and a stock
ngspice build statically links LGPL code (`numparam`) into the wasm, which the
project's no-GPL-linking rule forbids. Building our own SPICE engine through a
pinned emscripten toolchain inherits the same bundle overrun and adds a second,
non-Rust wasm toolchain. The pure-Rust route instead compiles into the existing
wasm module for a small gz cost, keeps GPL/LGPL code out of the link entirely,
and is deterministic across native and browser because WebAssembly arithmetic is
strict IEEE-754 with no implicit fused-multiply-add.

The solver itself is built by the `sim-engine` lane, which emits the F4
`WaveformSet` shape directly. Its scope is bounded small circuits: linear
resistors, capacitors, inductors, and sources for DC operating point and
transient analysis, with any nonlinear device models added later and labelled
generic wherever PDK model cards are unavailable. Nonlinear models route their
`exp()`/`log()` calls through the pinned `libm` crate so native and wasm stay bit
identical.
