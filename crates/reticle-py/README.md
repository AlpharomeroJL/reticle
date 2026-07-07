# reticle-py

Python bindings for the Reticle document and layout-generator APIs, built with
[PyO3](https://pyo3.rs) as a stable-ABI (`abi3`) extension module. One wheel
covers every CPython 3.9 and later.

This crate is native only and is not part of the default Cargo workspace, so the
repository's `just ci` gate never needs a Python toolchain. See
`docs/decisions/0068-python-bindings-abi3-nondefault.md`.

## API surface

```python
import reticle_py as rp

rp.version()                       # -> str
rp.generators()                    # -> list[dict] of {id, title, description}

doc = rp.Document.open("design.gds")   # GDSII or the OASIS subset, by extension
doc.cell_names()                   # -> sorted list[str]
doc.top_cells()                    # -> list[str]
doc.cell_count()                   # -> int
doc.summary()                      # -> dict of counts, top cells, and layers
doc.shapes("TOP")                  # -> list[dict] of {layer, datatype, kind, bbox}

doc.place_generator("TOP", "guard_ring", "{}")   # -> {shapes_added, bbox}
png = doc.render_png("TOP", 800, 600)            # -> bytes, or None if no GPU
doc.save("out.gds")                              # format inferred, or pass "gds"/"oasis"

rp.show(doc, "TOP")                # inline PNG in a Jupyter notebook
```

Every count and size crossing the boundary is validated: render dimensions are
bounded, cell names are checked against the document, and generator parameters
are parsed and range-checked by the registry, which reports the offending field.

## Building the wheel

The build uses [maturin](https://www.maturin.rs) under [uv](https://docs.astral.sh/uv).

```powershell
uv tool install maturin
cd crates/reticle-py
maturin build --release            # writes a wheel under target/wheels
# or, for local development against the current interpreter:
maturin develop
```

## Example notebook

`examples/reticle_intro.ipynb` opens the bundled `examples/basic.gds`, lists the
generators, places a guard ring, and renders the cell inline. Smoke it with:

```powershell
uv run --with nbconvert --with ipython jupyter nbconvert --to notebook --execute `
    --output executed.ipynb examples/reticle_intro.ipynb
```
