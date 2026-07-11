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
| 2026-07-10 | 168c892 | v8.2 Gate 1 (Phase 1: open silicon, formats, review) | - | - | 4356173 (4.15 MiB) | +357129 (+348.8 KiB) |
| 2026-07-11 | 88c1d16 | v8.2 Gate 2 (Phase 2: PCell engine, agent, F2/F3 panels) | - | - | 4411071 (4.21 MiB) | +412027 (+402.4 KiB) |
| 2026-07-11 | 71797b3 | v8.2 Gate 3 (Phase 3: simulator, netlist, classroom; native-only rhai) | - | - | 4448990 (4.24 MiB) | +449946 (+439.4 KiB) |
| 2026-07-11 | 367e5b9 | v8.2 Gate 4 (Phase 4: plugins, image underlay, embed, desktop) | 11576449 (11.04 MiB) | 4394446 (4.19 MiB) | 4463844 (4.26 MiB) | +464800 (+453.9 KiB) |

Ceiling amendment (ADR 0122): the delta ceiling was +450 KiB gz through Gate 3; Gate 4's
two browser features (image-underlay browser decode ADR 0118 +6.6 KiB, plugin-manager F5
browse ADR 0120 +6.7 KiB) breach it by +3.9 KiB combined. Ceiling raised to +456 KiB gz,
bounded to the +453.9 KiB measurement (+2.1 KiB margin). `just bundle-gate` asserts 456.
Provisional pending operator review at the release gate (see ADR 0122 for the trim option).
Gate 1-3 raw/gz-wasm cells recorded as delta only; exact splits are in each gate's
scratch/logs bundle log. Gate 3 gz total is baseline plus the measured +439.4 KiB delta.
