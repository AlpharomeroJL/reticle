# 0110, Waveform viewer: normalized-point plotting, an honest banner, exact CSV

## Context

Phase 3 (Depth) adds a waveform viewer to the Inspector: a panel that renders the F4
`WaveformSet` records a bounded circuit simulator produces (ADR 0104), fixture-first, so it
shows real waveforms before that solver exists. Which solver route ships (a vendored
ngspice-WASM build, a pinned emscripten toolchain, or a pure-Rust modified-nodal-analysis
solver) is a separate, still-open decision the `oracle-feasibility` lane makes; the viewer
must not assume any one of them, and must not claim to run a simulation it does not.

Three questions need an answer before the panel can be written: how the plotted-point
geometry stays unit-tested without a GPU (the project's stated architecture, `crate::app`'s
module docs), how the panel stays honest about the fixture standing in for a real run, and
what a "CSV export" of an integer-scaled record should actually contain.

## Decision

**Plotted geometry is normalized and egui-free.** `crate::waveform_panel` (mirroring
`crate::trace_panel`) never imports `egui`. Its `transient_trace`/`operating_point_value`
functions map a probe's samples into `NormPoint { x, y }` in `0.0..=1.0` on both axes, scaled
by the set's `Bounds`, with no notion of pixels or panel size. `App::waveform_section` (the
thin glue in `crate::app`) is the only place a `NormPoint` becomes a `Pos2`: it multiplies by
the allocated plot rect's width/height, the same custom-painter pattern the replay theater's
`replay_canvas` already uses. A degenerate or inverted bounds range (`max <= min`, which a
flat probe or a single-instant operating point both produce) normalizes to `0.5` rather than
dividing by zero, so a boring set never NaNs the plot.

A transient set plots the *selected* probe's polyline (one probe at a time, chosen from a
clickable probe list, matching how the PCell and Trace sections already work one-selection-
at-a-time). An operating point has no time axis to plot against, so instead of forcing a
single point onto a line chart, every probe's value is spread evenly across the plot width by
probe index and plotted as a marker on the shared value axis, with a text readout listing
every probe's exact value below. This asymmetry is deliberate: a transient's story is "how do I
change over time" (one probe, inspected in detail); an operating point's story is "what is
everyone's value right now" (every probe, at a glance).

**The fixture-first banner is honest and testable text, not a comment.** `waveform.run_oracle`
loads the committed F4 fixture (`crates/reticle-sim/tests/fixtures/contracts/f4_rc_transient.json`,
an analytic RC charging transient) and `waveform_section` renders a warning-toned banner
naming this plainly, with hover text spelling out the fixture path and that no simulation
runs. This is the same reticle-claims honesty seam the agent panel's banner already
establishes (`agent_banner`/`agent_section`): the label must match whichever path is
actually active, and until Gate 3 only the fixture path exists.

**Operating-point rendering is covered by a synthetic set, not a second fixture.** The
committed F4 fixture is transient-only. Rather than invent a second contract fixture this
lane does not own, `waveform_panel`'s tests build a small operating-point `WaveformSet`
directly (two probes, one sample each), mirroring the shape `reticle-sim`'s own
`f4_operating_point_shape_is_distinct` test already constructs. A committed OP fixture,
should one become useful, is left for whichever phase needs a producer to build against.

**CSV export dumps the raw integer samples, not the display-divided floats.** `to_csv` writes
`time_fs` and one `<probe id>_n<unit>` column per probe (e.g. `out_nV` for a nanovolt-scaled
voltage probe), with the exact `i64` values from the record. This follows the F4 contract's
own byte-stability rationale (ADR 0104): a spreadsheet import that round-trips exactly is more
useful than one that has already lost precision to float formatting, and the unit-suffixed
header keeps the nano-scale legible without a conversion step in the exporter itself. An
operating point (empty `time_fs`) writes one row from each probe's lone sample; a probe
shorter than the time axis (a shape `WaveformSet::is_well_formed` never allows) leaves that
cell blank rather than panicking.

## Consequences

The panel builds and is fully unit-tested (`waveform_panel`'s own suite, plus
`App::waveform_section`/`waveform_plot` headless-render tests) against nothing but the
committed fixture and in-test synthetic data; no other Phase 3 lane needs to land first. When
the bounded solver ships, `waveform_run_oracle` is the one call site that swaps
`fixture_transient()` for the real query, and the banner's condition flips the same way
`agent_section`'s does; nothing else in `waveform_panel` or `App::waveform_section` changes.
The CSV's raw-integer convention means a reader must know to divide by `1e9` (or `1e6` for the
time column) themselves; the column header names the unit prefix so this is discoverable
without reading source, but it is not self-converting, a deliberate trade for exactness over
convenience.
