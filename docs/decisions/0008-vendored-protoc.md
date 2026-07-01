# 0008, `protoc-bin-vendored`, no system protoc

## Context

`reticle-proto` generates Rust types from a `.proto` schema with `prost-build`,
which needs the `protoc` compiler at build time. Requiring a system `protoc`
install makes the build environment-dependent and can stall an unattended run on a
machine without it (as here, `protoc` is not installed).

## Decision

Depend on `protoc-bin-vendored` as a build dependency and point `prost-build` at
the vendored binary. No system `protoc` is required; the build is hermetic and
reproducible across machines.

## Consequences

The proto build works anywhere with no manual setup. The vendored binary adds a
small build-time dependency and pins a specific `protoc` version, which is exactly
the determinism we want. If a newer protobuf feature is ever needed, bump
`protoc-bin-vendored`.
