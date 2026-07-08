# 0087. Python bindings: PyO3 `abi3` in a non-default, excluded crate

Status: accepted

## Context

The v8 packet asks for Python access to the document and generator APIs: open a
GDSII layout, inspect it, place a generator by id with JSON parameters, render a
cell to an image, and drive all of that from a Jupyter notebook. Two decisions
had to be made up front, because both shape how the crate is wired into the
repository rather than what it does.

First, which wheel to build. A native extension can be compiled per Python
minor version (`cp311`, `cp312`, ...), producing one wheel per interpreter, or
against CPython's stable ABI (`abi3`), producing a single wheel that loads on
every CPython 3.x from a chosen floor upward. The host runs CPython 3.14, but a
binding meant to be useful should not be pinned to whatever interpreter happened
to build it.

Second, how the crate joins the workspace. The repository's sole gate is
`just ci`, which is deliberately Python-free and Node-free so it can run on any
checkout without extra toolchains. Its Rust steps (`clippy`, `test`, `doctest`,
`doc-build`, `build`) all pass `--workspace`. A crate that needs a Python
toolchain to build cannot sit in that `--workspace` set.

## Decision

Build against the stable ABI. The `pyo3` dependency carries `abi3-py39`, so one
wheel (`reticle_py-7.0.0-cp39-abi3-win_amd64.whl`) loads on CPython 3.9 and
later. The cost is that the binding may use only the stable-ABI subset of the
CPython C API, which is ample for this surface.

Keep the crate out of the default build set by putting it in the workspace
`exclude` list, not by using `default-members`. `--workspace` overrides
`default-members`, so a crate listed as a member (even one omitted from
`default-members`) is still built by every gate step. Only `exclude` removes a
crate from `--workspace` entirely. This is the same mechanism the MCP servers
and the fuzz crate already use (see 0005 and 0006), for the same reason: the
gate must not depend on a toolchain the crate needs.

Because an excluded crate cannot inherit `[workspace.package]` or
`[workspace.lints]`, `reticle-py` sets its package fields and lints locally and
references the internal crates by relative path, exactly as `fuzz` does. Its
gate is run from inside the crate directory (`cargo clippy --all-targets -- -D
warnings`, `cargo doc --no-deps`), which the shared `CARGO_TARGET_DIR` keeps
warm alongside the rest of the lane.

## Consequences

- One wheel covers every supported CPython; verified by building against 3.14
  and importing the same wheel under 3.13.
- `just ci` stays Python-free: `cargo metadata` confirms `reticle-py` is not a
  workspace member, so no gate step compiles it or needs `pyo3`.
- The binding depends on `reticle-cli` (open, save, render, summarize) and
  `reticle-render`, reusing the exact offscreen render path the CLI uses rather
  than duplicating it. The Python surface adds only validation at the boundary
  and Pythonic error mapping.
- A missing GPU is reported to Python as `render_png(...) -> None`, mirroring the
  CLI's `RenderOutcome::NoGpu`, so a headless caller degrades instead of failing.
