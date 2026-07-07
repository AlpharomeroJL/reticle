"""Python bindings for the Reticle document and layout-generator APIs.

The native surface lives in the compiled ``reticle_py._core`` module; this
package re-exports it and adds a small Jupyter inline viewer (:mod:`reticle_py.widget`).

Typical use::

    import reticle_py as rp

    doc = rp.Document.open("design.gds")
    print(doc.summary())
    cell = doc.top_cells()[0]
    doc.place_generator(cell, "guard_ring", "{}")
    rp.show(doc, cell)          # inline PNG in a notebook, or a note if no GPU
"""

from ._core import Document, generators, version
from .widget import LayoutView, render_image, show

__all__ = [
    "Document",
    "generators",
    "version",
    "LayoutView",
    "render_image",
    "show",
]

__version__ = version()
