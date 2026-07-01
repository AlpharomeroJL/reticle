# 0011, Incremental per-wave dependency resolution

## Context

The Appendix A crates include fast-moving stacks (`wgpu`, `egui`, `egui-wgpu`,
`glyphon`) whose versions must be mutually compatible, plus more stable crates
(`rstar`, `gds21`, `prost`, `rhai`). Guessing exact versions up front risks
resolution failures and API-drift bugs during an unattended run, exactly the
failure the spec warns about.

## Decision

Do not hand-write external version numbers in Wave 0. Add each external dependency
with `cargo add` in the wave/lane that first uses it, so cargo resolves a compatible
version, and lock the whole graph in a committed `Cargo.lock`. Before using an
unfamiliar API surface, consult docs.rs / the `crate-docs` MCP for the resolved
version's signatures. The Wave 0 skeleton is std-only precisely so it does not
depend on any external version being correct.

## Consequences

The workspace always resolves and the GPU/UI crates stay mutually compatible
because they are added together in the render/app lanes. `Cargo.lock` is committed
so builds are reproducible. The trade-off is that the full dependency set is not
visible in `Cargo.toml` until later waves; `docs/PLAN.md` and this ADR record the
intended crate-to-subsystem mapping in the meantime.
