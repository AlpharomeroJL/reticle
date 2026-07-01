# 0005, MCP servers in Rust, outside the workspace

## Context

Section 5 asks for two stdio MCP servers: `reticle-dev` (build/test/lint/ci/bench/
wasm/media/gen-layout/drc/route/perf-check) and `crate-docs` (docs.rs + crates.io
lookups). They can be written in any language. They must not slow the `just ci`
gate, and the build must never block waiting for an MCP to load.

## Decision

Write both as small Rust stdio MCP servers under `tools/`, `exclude`d from the main
workspace so they never enter `cargo build --workspace` or the `just ci` path.
Every `reticle-dev` operation is a thin wrapper over the corresponding `justfile`
recipe, so `just` is always the authoritative fallback. The servers are registered
in `.mcp.json` but only hot-load on the next session launch; this session drives
everything through `just` directly.

## Consequences

The repository stays pure-Rust and the MCP servers reuse the same recipes as the
gate, so there is a single source of truth. Keeping them out of the workspace means
their (heavier, network-capable) dependencies never affect core build times. The
cost is a second, smaller lockfile per tool and that the MCP convenience is only
available from the following session onward, acceptable because `just` covers the
current run.
