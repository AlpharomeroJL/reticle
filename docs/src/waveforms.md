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

## Where it lives

The Inspector's Automate group gains a "Waveform" section, alongside Agent,
Generate, and PCell (the other panels that run something and show a result).
Two commands drive it, both palette-reachable and both unbound by default:

- `waveform.run_oracle` loads (or reloads) the waveform set and reveals the
  section.
- `waveform.export_csv` dumps the loaded set to a CSV file.

## Fixture-first, said plainly

The panel never pretends to simulate. Every time `waveform.run_oracle` runs, a
warning-toned banner across the top of the section states that the data is the
committed F4 fixture, not a live run, with the exact fixture path in the hover
text. When a bounded solver ships, the one call this banner's condition and
`waveform.run_oracle` both key off flips from the fixture to the real query;
nothing else in the panel changes (see
[0110](../decisions/0110-waveform-viewer.md)).

## What gets rendered

`WaveformSet::analysis` selects one of two views:

- **Transient.** A probe list shows every recorded node; clicking one plots its
  polyline, scaled into the plot rect by the set's `Bounds` on both axes (time
  on x, value on y). The axis label under the plot names the quantity and its
  unit (`Voltage (V)`, `Current (A)`, `Charge (C)`), and the value shown is
  always the display form: `samples_nano` divided by `1e9`, time in nanoseconds
  (`time_fs` divided by `1e6`).
- **Operating point.** There is no time axis to plot against a single sample,
  so every probe is shown at once: a marker per probe spread evenly across the
  plot's value axis, plus an exact readout line per probe below it (`vdd
  (n_vdd)  1.800000 V`). A committed operating-point fixture has not landed
  yet; the panel's own tests build a small synthetic one to prove this path
  renders correctly (ledgered in [0110](../decisions/0110-waveform-viewer.md)).

The plotted-point geometry (`crate::waveform_panel::transient_trace` and
`operating_point_value`) is plain, egui-free math, unit-tested without a
window, mirroring the net-trace panel (`crate::trace_panel`); only the thin
glue in `App::waveform_section` touches `egui`.

## CSV export

`waveform.export_csv` writes the raw integer record, not the display-divided
floats: a header row of `time_fs` followed by one `<probe id>_n<unit>` column
per probe (`out_nV` for a nanovolt-scaled probe named `out`), and one data row
per time sample (or a single row, for an operating point). Every value is the
exact `i64` from the record, so round-tripping the file reproduces the
original samples bit for bit, the same byte-stability the F4 contract itself is
built on. A spreadsheet user divides by `1e9` (or `1e6` for `time_fs`)
themselves; the column header names the unit so that step is discoverable
without reading source.

## Testing

`crates/reticle-app/src/waveform_panel.rs` unit-tests the plotted-point
geometry, the probe-list and operating-point formatting, and the CSV
round-trip against the committed fixture, all without a window. The thin
`App::waveform_section`/`waveform_plot` glue is covered by a headless
render test (`crates/reticle-app/src/app.rs`, mirroring
`trace_section_renders_without_panic`) that builds a real `egui` pass with no
GPU, exercising both the empty state and the loaded fixture.
