"""A Jupyter inline viewer for Reticle layouts.

The v1 widget is deliberately simple: it renders a cell to a PNG with the same
offscreen GPU path the ``reticle`` CLI uses and hands the bytes to IPython for
inline display. A richer interactive widget (pan and zoom in the browser) is a
follow-up; an image is enough to see a layout in a notebook today.

Rendering needs a GPU adapter. On a headless machine without one, the render
returns ``None`` and the helpers here degrade gracefully rather than raising.
"""

from __future__ import annotations

from ._core import Document

DEFAULT_WIDTH = 800
DEFAULT_HEIGHT = 600


def render_image(doc: Document, cell: str, width: int = DEFAULT_WIDTH, height: int = DEFAULT_HEIGHT):
    """Render ``cell`` and return an ``IPython.display.Image``.

    Returns ``None`` when no GPU adapter is available so the caller can decide
    how to present that. Requires IPython (install the ``notebook`` extra).
    """
    png = doc.render_png(cell, width, height)
    if png is None:
        return None
    from IPython.display import Image

    return Image(data=png, format="png")


def show(doc: Document, cell: str, width: int = DEFAULT_WIDTH, height: int = DEFAULT_HEIGHT):
    """Display ``cell`` inline in a notebook.

    Returns the ``IPython.display.Image`` (so ``show(...)`` as a cell's last
    expression renders it), or prints a short note and returns ``None`` when no
    GPU adapter is available.
    """
    img = render_image(doc, cell, width, height)
    if img is None:
        print(f"[reticle] no GPU adapter available; cannot render cell {cell!r}")
    return img


class LayoutView:
    """A lazy inline view of one cell.

    Returning a ``LayoutView`` as a notebook cell's last expression renders the
    cell as a PNG through ``_repr_png_``; Jupyter falls back to the text
    ``__repr__`` when no GPU is available.
    """

    def __init__(
        self,
        doc: Document,
        cell: str,
        width: int = DEFAULT_WIDTH,
        height: int = DEFAULT_HEIGHT,
    ) -> None:
        self.doc = doc
        self.cell = cell
        self.width = width
        self.height = height

    def _repr_png_(self):
        # Jupyter calls this for rich display; returning None makes it fall back
        # to __repr__, which is exactly what we want when no GPU is present.
        return self.doc.render_png(self.cell, self.width, self.height)

    def __repr__(self) -> str:
        return f"LayoutView(cell={self.cell!r}, size={self.width}x{self.height})"
