# 0104, F4: the waveform-record contract is integer-scaled and byte-stable

## Context

The v8.2 campaign runs UI lanes fixture-first: the waveform panel (Phase 3) must build
against a frozen record shape before the bounded solver that produces those records
exists. A cross-lane contract needs a byte-level fixture and a cross-test both the
producer (the solver) and the consumer (the UI) run, so the two lanes cannot drift.

A simulation is naturally floating point (a 0.7 V node, an exponential decay). But a
contract fixture must be deterministic and byte-stable: an `f64` JSON round-trip is
neither, because float formatting drifts across serializers and platforms, and the
project is otherwise integer-exact everywhere (geometry in DBU). A record that carries a
float cannot be hashed or diffed exactly, which the replay and determinism story relies
on elsewhere.

## Decision

F4 records store values as `i64` in fixed nano-units (nanovolts, nanoamperes,
nanocoulombs) and time as `i64` femtoseconds. The schema lives in `reticle-sim`
(`waveform.rs`): `WaveformSet { analysis, time_fs, probes, bounds }`, `Probe { id, node,
quantity, samples_nano }`, `AnalysisKind { Transient, OperatingPoint }`, `Quantity {
Voltage, Current, Charge }`, `Bounds`. A transient's axis is non-empty and every probe has
one sample per time point; an operating point has an empty axis and one sample per probe
(`WaveformSet::is_well_formed`). The UI divides by `1e9` for display; the records never
carry a float.

The fixture is `crates/reticle-sim/tests/fixtures/contracts/f4_rc_transient.json`: a
first-order RC charging transient (`V(t) = 1 - exp(-t/RC)`, `RC = 1 ns`). The cross-test
(`tests/f4_waveform.rs`) checks the invariant, confirms the samples match the analytic
curve within 1 nV (so the fixture has a defined physical meaning), and round-trips through
serde. Amendments only at a phase gate, by a follow-up ADR, with the fixture updated in the
same commit.

## Consequences

Records hash and diff exactly, so a recorded waveform is reproducible and comparable.
Display precision is capped at nano-units, which is far finer than any panel renders and
adequate for the bounded small-circuit scope. When the solver lands (Phase 3) it emits
this shape directly and swaps its output in for the fixture at the gate; if the shape must
change, the ADR and fixture change together and every consumer re-verifies.
