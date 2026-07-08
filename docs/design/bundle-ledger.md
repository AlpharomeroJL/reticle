# Bundle ledger

Measured by 'cargo run -p xtask -- bundle-size' over crates/web/dist (trunk release build,
wasm-opt=z, content-hashed artifacts). Gzip is flate2 at best compression: it approximates
but does not equal GitHub Pages' on-the-wire compression; the +450 KB gz budget gate
(just bundle-gate) is self-consistent against the v8.0-baseline row below.

| date | commit | label | raw wasm | gz wasm | gz total | delta gz vs v8.0-baseline |
|---|---|---|---|---|---|---|
| 2026-07-08 | ec9a851 | v8.0-baseline | 10404970 (9.92 MiB) | 3932592 (3.75 MiB) | 3999044 (3.81 MiB) | - |
