# 0115, the rhai PCell producer is native-only (a browser-frugal runtime budget)

## Context

Phase 3's `f2f3-wiring` lane wired the PCell Inspector's Regenerate to the real
sandboxed producer (`reticle_script::produce`, the rhai engine; ADR 0107/0102),
adding `reticle-script` as an unconditional `reticle-app` dependency so live
produce ran in the browser too. That pulled the whole rhai scripting engine into
the wasm bundle, and `just bundle-gate` failed: the gz total reached +963.1 KiB
over the v8.0 baseline, 2.1x the +450 KiB budget (ADR 0098).

A byte-attribution (twiggy on the names-retained raw cargo wasm; the release and
gz wasm are name-stripped, defeating name-based tools) put rhai at 1,630,420 raw
bytes, the single largest crate in the module, larger than egui (906 KB) or all
of reticle-app (902 KB). The bounded MNA solver (`reticle-sim`) was about 571
bytes; the Phase-3 UI panels were a small fraction of reticle-app. rhai was the
dominant addition by an order of magnitude.

## Decision

`reticle-script` is a native-only dependency of `reticle-app`
(`[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`), exactly as the live
agent runner's `reticle-agent` / `reticle-bench` are. The browser PCell Inspector
shows the *predicted* F2 provenance (`selected_produce_meta`, a local hash of the
effective parameters, computed without running any script) plus an honest in-UI
disclaimer that live produce runs in the desktop app; it never claims to run a
live sandboxed produce it cannot. Native builds keep the real produce.

Measured result: removing rhai from the wasm graph cut the gz total from 4,985,287
to 4,442,442, a 530.1 KiB gz saving, landing Phase 3 at +433.0 KiB over the v8.0
baseline, back under the +450 KiB budget (`just bundle-gate` PASS; see the
bundle-ledger row).

## Consequences

- The browser is honest about the boundary: predicted provenance is real, live
  sandboxed produce is desktop-only. The panel and the claims ledger say so; the
  browser never presents fixture-backed parameter editing as a live run.
- The bundle stays browser-frugal: +433.0 KiB leaves about 17 KiB of headroom
  under the +450 ceiling. This is tight.
- PRECEDENT for Phase 4: the WASM plugin runtime goes native-first for the same
  reason. A general script or plugin runtime (rhai here; a wasmi-class plugin host
  later) is hundreds of KB to megabytes of wasm, and shipping one in the browser
  bundle is the wrong default for a browser-native editor whose positioning is a
  small, fast bundle. Phase 4's plugin work inherits this: native-first, the
  browser path ledgered, the moat claim worded to the native capability, revisited
  only with a measured budget-amendment ADR if a browser runtime is ever judged
  worth the bytes.
- If browser live-produce is ever wanted, the honest path is a measured budget
  amendment (a new baseline row), not an unmeasured ceiling bump; that decision is
  the operator's with the numbers in hand.
