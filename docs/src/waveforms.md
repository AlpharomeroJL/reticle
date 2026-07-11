# Waveform viewer

Scaffolded in the v8.2 campaign Phase 3. The waveform panel renders F4
waveform records ([`WaveformSet`](https://docs.rs/reticle-sim): a shared
femtosecond time axis, integer nano-unit probe series, and axis bounds) produced
by the bounded simulator.

The panel builds fixture-first against the committed F4 contract fixture
`crates/reticle-sim/tests/fixtures/contracts/f4_rc_transient.json`, so it renders
real waveforms before the solver exists; the fixture is swapped for live
simulator output at Gate 3 if the `oracle-feasibility` route delivers. This
chapter is filled by the `waveform-ui` lane.
