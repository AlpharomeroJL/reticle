# 0109, the simulator route is a pure-Rust MNA solver, not ngspice-in-WASM

## Context

Phase 3 (Depth) gives Reticle a bounded circuit simulator (crate `reticle-sim`)
that turns an extracted small circuit into the F4 waveform record already frozen
by ADR 0104 (`WaveformSet`, integer-scaled, byte-stable). This ADR decides how
that record gets produced. It does not implement the solver; that is the
`sim-engine` lane.

Three routes were on the table:

1. vendor ngspice and compile it to WASM,
2. pin an emscripten toolchain and build a SPICE engine to WASM ourselves,
3. write a pure-Rust modified-nodal-analysis (MNA) transient/DC solver.

Four constraints decide it, each with a way to check it:

- Bundle. The app is one Rust codebase compiled to native and wasm32; the site
  ships one wasm module under a measured budget. `just bundle-gate` asserts the
  gz delta stays under +450 KiB against the v8.0 baseline (ADR 0098,
  `docs/design/bundle-ledger.md`). The ledger's last row (v8.1-wave2) is
  +307.61 KiB, so 142.39 KiB of gz headroom remains for all of Phase 3; the
  orchestrator allocates roughly 47.6 KiB of that to sim. Command:
  `cargo run -p xtask -- bundle-size`.
- License. Standing rule: no GPL linking (a file or subprocess boundary is
  allowed, static linking is not). A wasm module statically links everything it
  contains, so any GPL/LGPL code inside the wasm is a linking event.
- Determinism. The F4 contract is byte-stable and the replay/diff story depends
  on reproducibility; the producer must give the same bytes on native and in the
  browser.
- Effort, inside a timebox. The route must be buildable by one follow-up lane,
  not a multi-week toolchain project.

## Route probes (evidence)

### Route 1: vendor ngspice, compile to WASM

- Availability: real. Multiple projects compile ngspice to WASM via emscripten,
  e.g. `wokwi/ngspice-wasm`, `danchitnis/ngspice`, and `eelab-dev/EEcircuit`
  (published as the npm package `eecircuit-engine`).
  Source: https://github.com/wokwi/ngspice-wasm ,
  https://github.com/eelab-dev/EEcircuit .
- License: mixed, and the mix is the problem. The ngspice core is New BSD
  (BSD-3-Clause), but the tree ships LGPL and GPL pieces: `frontend/numparam`
  (LGPL, the netlist parameter parser), `spicelib/dev/adms` scripts (LGPL),
  `tclspice` (LGPLv2), and `xspice/icm/table` (GPLv2 per the search summary).
  The COPYING file states its own policy verbatim: "GPL is not suitable for code
  to be directly linked into ngspice, but may be used in shared object libraries
  only." A stock ngspice build (what the WASM projects above compile) statically
  links numparam (LGPL) into the module. LGPL static linking into a wasm blob
  carries a relink obligation that our no-GPL-linking rule exists to avoid, and a
  BSD-only carve-out (dropping numparam and friends) is extra work that no
  existing WASM build does. Source:
  https://raw.githubusercontent.com/ngspice/ngspice/master/COPYING .
- Bundle: disqualifying. `eecircuit-engine@1.7.0` ships its engine module at
  20,424,332 bytes with the ngspice wasm base64-embedded in the JS glue (its two
  build variants are 20.4 MB and 20.3 MB; unpacked package ~40 MB). Source:
  https://data.jsdelivr.com/v1/packages/npm/eecircuit-engine@1.7.0 and the npm
  registry `unpackedSize`. Even after gzip a compiled ngspice.wasm is on the
  order of megabytes (order-of-magnitude estimate from the 20 MB raw anchor, not
  a measured gz), which is one to two orders of magnitude over the 142 KiB of
  total Phase 3 headroom, and ~400x the raw size of the 47.6 KiB sim slice.
- Determinism: not guaranteed. ngspice uses adaptive timestep control and a
  sparse solver with dynamic pivoting, built here against emscripten's libm; none
  of the WASM projects claim bit-reproducible output across platforms.
- Verdict: rejected on bundle alone (by ~400x), with license and determinism as
  independent blockers.

### Route 2: pin an emscripten toolchain, build a SPICE engine to WASM ourselves

This is the do-it-yourself form of Route 1 and inherits every one of its
problems with none of the maturity:

- Bundle: a C SPICE engine compiled to wasm is megabyte-scale whether we build it
  or borrow it; same ~400x overrun.
- Toolchain: it bolts a second, non-Rust wasm toolchain (emscripten) onto a
  workspace whose whole shape is "one Rust codebase to native and wasm32," plus a
  second wasm module and its JS glue to load and keep in sync. That is a large CI
  and maintenance cost the campaign explicitly does not want.
- License: if the engine is ngspice, the Route 1 LGPL/GPL analysis applies
  unchanged; if it is bare Spice3f5 (New BSD), we would be maintaining decades-old
  C by hand.
- Determinism: still built on emscripten libm and a C solver, so no better than
  Route 1.
- Verdict: rejected; strictly dominated by Route 1.

### Route 3: pure-Rust modified-nodal-analysis solver

- Availability: a pure-Rust SPICE-style MNA solver is proven prior art, twice
  over. `sindr` v0.1.0-alpha.6 is "Rust circuit simulator: SPICE-style MNA solver
  with built-in semiconductor device models. Transient, AC, DC sweep, temperature
  sweep" (MIT OR Apache-2.0; ~12.6k LOC across 26 files; depends on `nalgebra`).
  `tiny-spice-rs` is a transient MNA simulator (Unlicense). Sources:
  https://crates.io/api/v1/crates/sindr ,
  https://github.com/Harnesser/tiny-spice-rs . For the bounded small-circuit
  scope a compact in-house dense-MNA solver is enough (linear R, C, L, and V/I
  sources via companion models for DC and transient), so we do not need to pull
  `nalgebra`; `reticle-sim` today depends only on `serde`
  (`crates/reticle-sim/Cargo.toml`).
- License: clean. In-house code takes the workspace license, MIT OR Apache-2.0
  (`grep license Cargo.toml` -> `license = "MIT OR Apache-2.0"`; `LICENSE-MIT`,
  `LICENSE-APACHE` present). No GPL or LGPL anywhere. Per ADR 0104's consumers and
  the claims rule, this must be labelled a pure-Rust MNA solver everywhere and
  never presented as ngspice; generic device models are labelled generic wherever
  PDK model cards are unavailable.
- Bundle: fits. The solver compiles into the existing wasm module and adds only
  Rust source (no second payload, no glue). A hand-rolled dense MNA over `f64`
  with a fixed pivot order is a few hundred lines using only `+ - * /`; the added
  gz cost is estimated in the single-digit KiB range, well inside 47.6 KiB. This
  figure is an estimate and is labelled as such; the `sim-engine` lane confirms it
  with `just bundle-gate` when the solver lands. The decision does not rest on the
  exact number: Route 3 adds Rust to one module, Routes 1 and 2 add a multi-MB
  second payload.
- Determinism: supported by the platform. WebAssembly arithmetic is strict
  IEEE-754, round-to-nearest ties-to-even, with no non-default rounding and no
  implicit fused-multiply-add (Rust emits `fma` only through an explicit
  `mul_add`), so for non-NaN inputs the result is identical across browsers, CPUs,
  and native. Source: the WebAssembly numerics spec,
  https://webassembly.github.io/spec/core/exec/numerics.html . A linear-network
  companion-model solve uses only `+ - * /`, so native and wasm agree bit for bit,
  and F4 quantises the `f64` result to `i64` nano-units (ADR 0104), absorbing any
  last-ULP difference; the F4 cross-test already asserts the fixture curve to
  within 1 nV. The one caveat, flagged for the `sim-engine` lane: nonlinear
  devices solved by Newton iteration use `exp()`, whose result can differ across
  platform libm, so those models should call the pinned `libm` crate rather than
  the system math library to stay deterministic.
- Effort: bounded and timeboxable; emits the F4 `WaveformSet` directly, adds no
  toolchain, and has two reference implementations to check against.
- Verdict: feasible and recommended.

## Decision

Take Route 3: a pure-Rust modified-nodal-analysis transient/DC solver in
`reticle-sim`, licensed MIT OR Apache-2.0 (workspace license), labelled a
pure-Rust MNA solver everywhere and never as ngspice. It is the only route that
fits the ~47.6 KiB gz sim headroom, the only one that keeps GPL/LGPL code out of
the linked wasm, and the only one with a platform-guaranteed determinism story
against the F4 contract.

Fallback, recorded but not needed on this evidence: if the `sim-engine` lane
measures the built solver over the 47.6 KiB headroom, or the bounded scope proves
too thin to be useful, PARK the in-bundle live solver. The waveform UI still ships
against the frozen F4 fixture (ADR 0104,
`crates/reticle-sim/tests/fixtures/contracts/f4_rc_transient.json`), and netlist
export still delivers a SPICE deck for xschem and external simulators (the
`spice_exchange_inverter` contract in `reticle-extract`, `docs/src/spice-export.md`).
That keeps the demo honest with no solver in the bundle.

## Consequences

- The `sim-engine` lane builds an in-house dense-MNA solver, not an FFI or wasm
  wrapper, and holds it to the F4 shape. Its scope is small circuits (linear R,
  C, L, and sources for DC and transient) with any nonlinear device models added
  later and labelled generic.
- No new toolchain enters CI; the wasm build stays a single Rust module.
- The claims ledger and `docs/src/simulation.md` describe the simulator as a
  pure-Rust MNA solver for bounded small circuits, never as ngspice, with generic
  device models labelled as generic.
- The determinism guarantee holds for linear networks out of the box; the
  `sim-engine` lane must route any `exp()`/`log()` in device models through the
  pinned `libm` crate to keep native and wasm bit-identical, and should add a
  native-vs-wasm cross-check when nonlinear models land.
- If a future need outgrows a bounded in-browser solver (large nets, full BSIM
  models), the honest answer is the existing SPICE export to an external
  simulator, not shipping ngspice in the bundle. Revisit only with a new ADR.
