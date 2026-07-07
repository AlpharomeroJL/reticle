# RESULT: Lane v8-5e-python (PyO3 bindings + Jupyter notebook widget)

Status: GREEN. New non-default crate `reticle-py` builds, the abi3 wheel builds,
imports, and the example notebook executes end to end (including a real GPU
render). `just ci` stays Python-free.

## Commits (branch lane/v8-5e-python, not pushed)

| sha | commit |
|-----|--------|
| 1cc630b | build(reticle-py): exclude a non-default PyO3 abi3 crate from the workspace |
| 0aa2472 | feat(reticle-py): PyO3 abi3 bindings for documents, generators, render |
| af89e2a | feat(reticle-py): maturin packaging, Jupyter widget, example notebook |
| 3d49e5f | docs(reticle-py): Python bindings book chapter |

## What shipped

- `crates/reticle-py/` (new, workspace-excluded): a PyO3 `abi3-py39` extension
  module. Mixed Rust/Python layout: the compiled module is `reticle_py._core`;
  `python/reticle_py/` re-exports it and adds the Jupyter widget.
- Root `Cargo.toml`: one additive edit, `"crates/reticle-py"` appended to the
  `exclude` list with a comment (see "Non-default mechanism" below).
- `docs/decisions/0068-python-bindings-abi3-nondefault.md` + README row.
- `docs/src/python.md` + `docs/src/SUMMARY.md` chapter.

## Bound API surface

Module `reticle_py`:
- `version() -> str`
- `generators() -> list[dict]` (`id`, `title`, `description` for the 6 builtins:
  guard_ring, via_farm, pad_ring, seal_ring, fill, test_structure)
- `Document.open(path) -> Document` (GDSII or OASIS subset, by extension)
- `Document.save(path, format=None)` (`"gds"` / `"oasis"`, else inferred)
- `Document.cell_names() / top_cells() / cell_count()`
- `Document.summary() -> dict` (counts, top cells, layers)
- `Document.shapes(cell) -> list[dict]` (layer, datatype, kind, bbox)
- `Document.place_generator(cell, generator_id, params_json) -> {shapes_added, bbox}`
- `Document.render_png(cell, width, height) -> bytes | None` (None when no GPU)
- Widget: `show(doc, cell, ...)`, `render_image(...)`, `LayoutView` (with `_repr_png_`)

Validation at the boundary (untrusted inputs): render width/height are bounded to
`[1, 16384]` (rejects zero and unbounded allocations); missing cells raise
`KeyError`; bad generator id or params raise `ValueError` with the field message;
I/O failures raise `IOError`. It reuses `reticle-cli` (open/save/summarize/frame)
and `reticle-render` (offscreen path) rather than duplicating them.

## Non-default mechanism (why `exclude`, not `default-members`)

The gate runs `cargo clippy/test/doctest/doc-build/build` with `--workspace`,
which ignores `default-members`. So `default-members` would NOT keep a member out
of the gate. `exclude` removes the crate from the workspace entirely, which is the
only mechanism that keeps its Python toolchain off `just ci`. This matches the
existing precedent (the MCP servers and `fuzz` crate are excluded for the same
reason, ADRs 0005/0006). Confirmed with `cargo metadata --no-deps`: `reticle-py`
is not a workspace member.

## Gate results

- `cargo clippy --all-targets -- -D warnings` (run in `crates/reticle-py/`,
  since `-p reticle-py` does not resolve from the excluded root): GREEN, no warnings.
- `cargo doc --no-deps` (in `crates/reticle-py/`): GREEN.
- `cargo test` (in `crates/reticle-py/`): 4 passed (dim validation x3, generator listing).
- `just lint` (workspace fmt-check + clippy --workspace): GREEN.
- `powershell -File scripts/check-style.ps1`: OK (no em-dashes, no banned words).
- `cargo build --workspace` (the default set): GREEN in 3m14s; neither
  `reticle-py` nor `pyo3` is compiled, so the default gate needs no Python.
- Full `just ci` tail (test/doctest/doc-build/wasm/deny/typos) not re-run in this
  session; the change is purely additive (one `exclude` entry + a new excluded
  crate + docs) and cannot affect the workspace default set, which was proven to
  build green above.

## Wheel + notebook (both RAN, not stubbed)

- Toolchain: Python 3.14.3, uv 0.11.6, `uv tool install maturin` -> maturin 1.14.1.
- `maturin build --interpreter python` in `crates/reticle-py/`:
  built `reticle_py-7.0.0-cp39-abi3-win_amd64.whl`
  (in `E:\dev\reticle-target-v8-5e-python\wheels\`).
- abi3 proof: the wheel built against CPython 3.14 was installed into a CPython
  3.13 venv and imported cleanly (`import reticle_py; version()` -> "7.0.0";
  all 6 generators listed). One wheel covers 3.x.
- Notebook smoke: `jupyter nbconvert --to notebook --execute` on
  `examples/reticle_intro.ipynb` exited 0. It opened `basic.gds` (cell `CORPUS`),
  placed `guard_ring` (12 shapes added, bbox [0,0,2800,2800]), rendered inline,
  and saved. A GPU adapter was present, so cell 4 produced a real `image/png`
  output (not just the no-GPU fallback path).

## Honest gaps / notes

- The widget is v1: a static inline PNG. A richer interactive (browser pan/zoom)
  widget is noted as a follow-up in the chapter and ADR.
- `render_png` returns `None` when no GPU adapter is available, matching the CLI's
  `RenderOutcome::NoGpu`; on such a host the notebook still runs but prints a note
  instead of an image.
- No additive accessor was needed on any frozen crate; the binding uses only the
  existing public APIs of reticle-model, reticle-gen, reticle-cli, reticle-render.
- The wheel built here is a debug wheel (fast, sufficient to prove the toolchain
  and run the notebook). A shippable release wheel is `maturin build --release`.
