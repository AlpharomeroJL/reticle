# Contributing

## The build gate

There is no hosted CI. A single recipe is the gate and must be green before every
commit:

```sh
just ci
```

`just ci` runs, in order: formatting check, Clippy with warnings denied across all
targets, the test suite (`nextest`) and doctests, a documentation build with broken
links denied, a WebAssembly build, `cargo-deny` for licenses and advisories, and a
spell check. Individual steps are available as their own recipes (`just fmt`,
`just clippy`, `just test`, `just doc-build`, and so on); `just --list` shows them
all.

## Standards

- **Exact integers.** Layout coordinates are database units (`i32`); widen to
  `i64`/`i128` for products and areas. Never introduce floating-point coordinates
  into the geometry or model core.
- **Documented public API.** Every public item carries rustdoc; this is enforced.
- **Safe Rust by default.** Any `unsafe` is isolated, carries a `// SAFETY:`
  justification, and is covered by tests and `miri` (`just miri`).
- **Tests before claims.** New geometry, indices, and CRDT behavior come with
  property tests against a brute-force or reference oracle; parsers come with fuzz
  targets (`just fuzz <target>`); the renderer comes with golden-image tests.
- **Measured performance.** A change that affects performance lands with a
  benchmark and a real number recorded in `PERF.md`; `xtask perf-check` guards
  against regressions.
- **Decisions are recorded.** A choice with real trade-offs gets a short
  architecture decision record under `docs/decisions/`.

## Commits

Commits are small, coherent, and use conventional messages. The pre-commit hook
(`git config core.hooksPath .githooks`) runs the fast formatting and Clippy checks.
