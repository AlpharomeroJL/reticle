# Fuzzing

`cargo-fuzz` targets for the parsers and the geometry boolean engine. Each target
must never panic, hang, or exhibit undefined behavior on arbitrary input; it either
produces a result or returns an error.

## Targets

- `gds_import`, the GDSII importer (`reticle_io::Gds`).
- `oasis_import`, the OASIS importer (`reticle_io::Oasis`).
- `geometry_boolean`, the polygon boolean engine (`reticle_geometry::polygon_boolean`).

## Running

```sh
cargo +nightly fuzz run gds_import
cargo +nightly fuzz run oasis_import
cargo +nightly fuzz run geometry_boolean
```

Seed corpora live under `fuzz/corpus/<target>/` and are committed (a curated set of
the smallest coverage-preserving inputs per target); crash artifacts and coverage are
not. Crash and OOM regression inputs are pinned separately under
`crates/reticle-io/tests/fuzz-regressions/` and asserted panic-free by the normal test
suite, so they run in `just ci` on every platform.

## Campaign history

The v8.0.0 Wave 0 campaign (WSL Ubuntu, `cargo-fuzz` 0.13.2, `-fork=4`) ran all three
targets and found three real `reticle-io` defects, each fixed with a committed
regression fixture: an out-of-range-date panic and a zero-length-string panic in the
GDSII importer (both would abort a wasm tab, where `catch_unwind` does not help), and
an unbounded-allocation OOM in the OASIS importer. A clean-rebuilt confirmation pass
produced zero surviving artifacts on all three targets. See `docs/STATUS.md` for the
full accounting.

Gotcha for re-runs: the `/mnt/d` 9p mount defeats cargo's incremental rebuild, so a
fuzz build after editing source must use a fresh `CARGO_TARGET_DIR` (or it silently
reuses a stale binary and reports already-fixed crashes). Confirm any fix by also
running the crash artifact through the native importer, which links normally.

## Platform note

libFuzzer needs the LLVM `compiler-rt` runtime (SanitizerCoverage, and
AddressSanitizer when enabled). That runtime ships with Linux toolchains and with a
clang install; the default Windows/MSVC toolchain does not provide it, so on Windows
the targets compile but the fuzz binary fails to link (`__stop___sancov_pcs`,
`clang_rt.asan`). Run the fuzzers on Linux, or on Windows with clang's `compiler-rt`
on the library path.

Parser robustness against arbitrary and truncated bytes is *additionally* covered by
property tests in `reticle-io` (2048 randomized cases per parser) that run in the
normal `just ci` gate on every platform, so the no-panic guarantee is enforced
continuously regardless of whether the libFuzzer harness can be built locally.
