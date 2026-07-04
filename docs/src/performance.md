# Performance methodology

Every performance claim in Reticle is a measured number, not an aspiration. This
chapter describes how the numbers are produced so they can be reproduced and
trusted; the numbers themselves live in `docs/PERF.md`.

## How measurements are taken

- **Microbenchmarks** use `criterion`, which runs each benchmark many times, warms
  the caches, and reports a statistically meaningful median with a confidence
  interval. The benchmark inputs come from the deterministic layout generator, so a
  run is reproducible.
- **Frame timing** for the renderer is measured with an in-process profiler
  (`puffin`) fed by `tracing` spans, and with the offscreen render harness, which
  renders scripted camera paths and records per-frame times.
- **The host** is recorded with every number: these are measured on an RTX 4060 Ti
  16 GB. Numbers from other machines are not comparable and are not mixed in.

## Guarding against regressions

The benchmark results are committed as a history. `xtask perf-check` compares a
fresh run against the committed baseline and fails if a result regresses beyond a
threshold, so a performance regression is caught the same way a test failure is.

## Honesty

Where a target is missed, the measured number and the bottleneck are recorded
plainly. A missed target with an honest explanation is more useful than a number
that cannot be reproduced.
