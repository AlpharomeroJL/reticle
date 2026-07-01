# 0010, `puffin` + `criterion` for profiling and benchmarks

## Context

The spec requires continuous profiling of CPU/GPU/memory and a committed benchmark
history, backing every performance claim with a real number. Options for in-app
profiling include `tracing-tracy` (needs the external Tracy viewer) and `puffin`
(pure-Rust, in-process flamegraph UI via `puffin_egui`).

## Decision

Use `criterion` for microbenchmarks and the committed benchmark history, and
`puffin` for in-app frame/span profiling surfaced through the egui UI. Instrument
with `tracing` spans as the common vocabulary; a lightweight bridge feeds the same
spans into `puffin`. This keeps profiling self-contained (no external viewer needed
for the demo) while `criterion` provides statistically rigorous, regression-guarded
numbers for `perf-check`.

## Consequences

Benchmarks are reproducible and comparable over time; `reticle-dev perf-check`
compares against the committed history and fails on regression beyond a threshold.
In-app profiling ships with the binary and needs no extra tooling. If deep GPU
timeline capture is later required, a `tracing-tracy` layer can be added alongside
without changing instrumentation, since both consume `tracing` spans.
