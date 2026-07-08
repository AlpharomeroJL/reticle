# Bundle ledger

Measured by 'cargo run -p xtask -- bundle-size' over crates/web/dist (trunk release build,
wasm-opt=z, content-hashed artifacts). Gzip is flate2 at best compression: it approximates
but does not equal GitHub Pages' on-the-wire compression; the +450 KB gz budget gate
(just bundle-gate) is self-consistent against the v8.0-baseline row below.

| date | commit | label | raw wasm | gz wasm | gz total | delta gz vs v8.0-baseline |
|---|---|---|---|---|---|---|
| 2026-07-08 | ec9a851 | v8.0-baseline | 10404970 (9.92 MiB) | 3932592 (3.75 MiB) | 3999044 (3.81 MiB) | - |
| 2026-07-08 | d0ca61c | v8.1-wave1 | 10599359 (10.11 MiB) | 4038939 (3.85 MiB) | 4105392 (3.92 MiB) | +106348 (+103.86 KiB) |
| 2026-07-08 | a2493e3 | v8.1-wave2 | 11172720 (10.66 MiB) | 4246091 (4.05 MiB) | 4314041 (4.11 MiB) | +314997 (+307.61 KiB) |
