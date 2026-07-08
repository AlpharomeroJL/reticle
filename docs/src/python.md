# Python bindings

The `reticle-py` crate exposes the document and layout-generator APIs to Python
through [PyO3](https://pyo3.rs). It is a native extension built with
[maturin](https://www.maturin.rs) as a stable-ABI (`abi3`) wheel, so one wheel
loads on every CPython 3.9 and later. The package is a mixed Rust and Python
project: the compiled module is imported as `reticle_py._core`, and a small pure
Python layer re-exports it and adds a Jupyter inline viewer.

`reticle-py` is native only and is deliberately not part of the default Cargo
workspace, so the `just ci` gate never needs a Python toolchain. The reasoning is
recorded in [ADR 0087](decisions/0087-python-bindings-abi3-nondefault.md).

## API surface

```python
import reticle_py as rp

rp.version()                       # crate version string
rp.generators()                    # list of {id, title, description} dicts

doc = rp.Document.open("design.gds")   # GDSII, or the OASIS subset, chosen by extension
doc.cell_names()                   # sorted list of cell names
doc.top_cells()                    # the document's top (root) cells
doc.cell_count()                   # number of cells
doc.summary()                      # dict of counts, top cells, and layers in use
doc.shapes("TOP")                  # list of {layer, datatype, kind, bbox} for a cell

# Place a built-in generator by id with JSON parameters. Empty "{}" uses the
# generator's own defaults. Returns {shapes_added, bbox}.
doc.place_generator("TOP", "guard_ring", "{}")

png = doc.render_png("TOP", 800, 600)   # PNG bytes, or None if no GPU is available
doc.save("out.gds")                     # format inferred, or pass "gds" / "oasis"
```

Everything that crosses the boundary is treated as untrusted and validated
before it reaches native code: render dimensions must be between 1 and 16384 on
each axis, cell names are checked against the document (a miss raises
`KeyError`), and generator parameters are parsed and range-checked by the
registry, which reports the offending field as a `ValueError`.

## The Jupyter widget

The v1 viewer renders a cell to a PNG with the same offscreen GPU path the CLI
uses and displays it inline:

```python
rp.show(doc, "TOP", width=640, height=480)     # inline image, or a note if no GPU
```

`rp.LayoutView(doc, "TOP")` is a lazy alternative whose `_repr_png_` renders when
a notebook displays it, falling back to a text representation when no GPU adapter
is present. A richer interactive widget (browser pan and zoom) is a follow-up; an
image is enough to see a layout today.

The committed example notebook, `crates/reticle-py/examples/reticle_intro.ipynb`,
opens the bundled `basic.gds`, lists the generators, places a guard ring, renders
the cell, and saves the result.

## Building the wheel

The build runs maturin under [uv](https://docs.astral.sh/uv):

```powershell
uv tool install maturin
cd crates/reticle-py
maturin build --release        # writes a wheel under the shared target's wheels/ dir
```

For local development against the current interpreter, `maturin develop` builds
and installs the extension into the active virtual environment in one step.
