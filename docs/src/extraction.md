# Connectivity extraction

`reticle-extract` recovers the electrical connectivity implied by the geometry.

## Nets from geometry

Two shapes on the same layer that touch or overlap are connected; shapes on
different layers are connected where a contact or via joins them. Extraction walks
the geometry with the spatial index and a union-find structure to group all
connected shapes into nets. Each net can be highlighted in the renderer so a
designer can trace a signal by eye.

## Compare against expectation

Given an expected netlist, extraction reports where the geometry and the intent
disagree: shapes that should be connected but are not, and shapes that are
connected but should not be. This is the geometric half of a layout-versus-schematic
check.

## Testing

Extraction is validated against an independent union-find oracle over randomized
geometry, so the optimized traversal cannot disagree with the definition of
connectivity.
