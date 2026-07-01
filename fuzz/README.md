# Fuzzing

`cargo-fuzz` targets for the parsers and the geometry boolean engine. Each target
must never panic, hang, or exhibit undefined behavior on arbitrary input; it either
produces a result or returns an error.

## Targets

- `gds_import` — the GDSII importer (`reticle_io::Gds`).
- `oasis_import` — the OASIS importer (`reticle_io::Oasis`).
- `geometry_boolean` — the polygon boolean engine (`reticle_geometry::polygon_boolean`).

## Running

```sh
cargo +nightly fuzz run gds_import
cargo +nightly fuzz run oasis_import
cargo +nightly fuzz run geometry_boolean
```

Seed corpora live under `fuzz/corpus/<target>/` and are committed; crash artifacts
and coverage are not.

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
