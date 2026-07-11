# 0114: the MNA solver integrates trapezoidally and pivots by magnitude

## Context

ADR 0109 chose a pure-Rust dense modified-nodal-analysis (MNA) solver for the
bounded simulation route: linear R/C/L and independent sources, DC operating point
plus fixed-step transient, a hand-rolled dense solve over `f64` using only
`+ - * /`, emitting the F4 `WaveformSet` directly and reproducing
`f4_rc_transient.json` within 1 nV. That leaves three numerical sub-decisions ADR
0109 deferred to the implementation: the integration method, the linear-solve pivot
strategy, and how the `t = 0` state is seeded. Two of them are forced by hard
constraints rather than preference.

The fixture is the analytic first-order step `V(t) = 1 - exp(-t/RC)` (`RC = 1 ns`)
rounded to nano-volts. The solver must land within 1 nV of it while staying
bit-identical between the native and `wasm32` targets.

## Decision

**Trapezoidal integration, not backward Euler.** For the reference RC step the
per-step decay factor of the trapezoidal companion is `(1-a)/(1+a)` with
`a = h/(2RC)`, whose error against the exact `exp(-2a)` is `O(a^3)` per step and
`O(h^2)` globally; the peak absolute error is about `a^2/(3e)`. At the chosen 20 fs
internal step (`a = 1e-5`) that is ~0.012 nV, so the quantised curve equals the
fixture exactly. Backward Euler is only first order (`O(h)` global); reaching the
same nano-volt bound would need a step near `5e-9 * RC`, on the order of 1e8 steps
for the 3 ns window instead of the 1.5e5 steps trapezoidal uses. Trapezoidal is the
only fixed-step method in the MVP that meets the bar at a bounded step count.

**Gaussian elimination with partial (magnitude) pivoting, not a fixed natural
order.** MNA appends a branch-current unknown per voltage source whose diagonal
entry is structurally zero, so natural-order elimination hits a zero pivot on every
circuit with a source. Partial pivoting reorders rows by an exact `f64` magnitude
comparison. That comparison is deterministic and its result is identical on native
and `wasm32` (IEEE-754 compare), and the arithmetic remains pure `+ - * /` with no
fused multiply-add, so pivoting keeps the solve both solvable and bit-identical
across targets. This is the intended reading of the ADR 0109 "fixed pivot order"
note: a deterministic pivot rule, not literal natural order.

**Consistent initial-condition start.** Before stepping, each capacitor is pinned to
its IC voltage (stamped as a voltage source) and each inductor to its IC current
(stamped as a current source); the resulting solve yields the branch currents and
voltages the trapezoidal history term needs at step 1. Seeding the history current
to zero instead would corrupt the first step and lose the exact-match property.

**Deterministic rounding.** Samples are quantised to nano-units with
`(x + 0.5) as i64` (half up), which uses only addition and a truncating cast; both
are bit-identical native and `wasm32`, avoiding `f64::round`, whose lowering can
differ across targets.

No `exp`/`log` appears anywhere in the linear path, so the pinned `libm` crate is
not needed for this MVP; a future nonlinear device model that needs transcendentals
must route them through `libm` and add a native-vs-`wasm32` check, per ADR 0109.

## Consequences

The solver reproduces `f4_rc_transient.json` to the exact nano-unit
(`cargo nextest run -p reticle-sim`, test
`rc_transient_reproduces_the_f4_fixture_exactly`) and is byte-for-byte deterministic
across repeated solves (`transient_is_deterministic_byte_for_byte`). The internal
step is a caller option; 20 fs is the value the RC cross-test pins because it clears
the half-nano rounding boundary with ~80x margin. The dense solve is `O(n^3)` in the
unknown count and capped at `MAX_UNKNOWNS`, which suits the bounded small-circuit
scope but not large netlists; a sparse factorisation is the follow-on if the scope
grows. Nonlinear devices (MOSFET, diode) remain out of the MVP and are a
labelled-generic follow-on.
