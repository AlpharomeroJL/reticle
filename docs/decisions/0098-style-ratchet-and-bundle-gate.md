# 0098, UI style-lint ratchet and the bundle-size gate

## Context

Two packet requirements need mechanical gates that did not exist. First,
hardcoded UI literals must become impossible outside the theme module, but 89
of them exist until lane 1A drains them, and just ci must stay green at every
commit in between. Second, the redesign carries a bundle budget (at most
450 KB gzipped added), yet the repo recorded only a raw wasm size in prose and
had no measurement tool, no ledger, and no gate.

## Decision

- check-style.ps1 (already CI's first step) gains a third check: banned UI
  patterns (Color32 constructors, RichText size, FontId constructors) in
  crates/reticle-app/src and crates/web/src, excluding the theme module.
  A committed scripts/style-baseline.json maps file to grandfathered count: a
  file exceeding its baseline fails with a file:line list; falling below warns
  that a ratchet is available; just style-ratchet rewrites the baseline
  downward only, and deletes it when every count reaches zero, after which the
  ban is unconditional. New files have no baseline and therefore start banned.
- Bundle measurement lives in xtask (bundle-size): it walks crates/web/dist
  after a release trunk build, records raw and gzip (flate2, best compression)
  bytes per artifact, and appends labeled rows to docs/design/bundle-ledger.md.
  just bundle-gate asserts the current gz total against the committed
  v8.0-baseline row plus 450 KB and fails by exit code. flate2's gzip
  approximates but does not equal GitHub Pages' wire compression; the gate is
  self-consistent against its own baseline, which is what a budget needs.

## Consequences

- Style drift is a CI failure from Wave 0 forward without ever holding CI
  hostage to the legacy literal count; the Gate 1 exit criterion is the
  baseline file's deletion.
- Bundle growth is visible per gate in a committed ledger with real measured
  numbers; claims about bundle cost in docs cite ledger rows instead of
  estimates.
- The ratchet applies to line patterns, not semantics: a literal smuggled
  through a helper would evade it. Review and the one-source-of-truth
  convention carry that residue; the lint exists to make the honest path the
  cheap one.
