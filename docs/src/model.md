# Document model

`reticle-model` is the hierarchical document that everything edits and renders.

## Hierarchy is the source of scale

A document is a set of named **cells**. A cell owns flat geometry (shapes on
layers) and placements of other cells: single **instances** and regular **arrays**
(rows and columns with a pitch). Placements carry a transform (orientation,
magnification, translation), and they nest, so a modest cell arrayed thousands of
times expands to effectively billions of leaf shapes.

Crucially the hierarchy is never flattened for browsing. Each cell caches the
bounding box of its own geometry, so the renderer can cull whole instances and
arrays that fall outside the view and pay only for what is visible. Flattening is
available when a tool genuinely needs the expanded geometry, and so is the inverse.

## Transactional editing

Edits are expressed as a small vocabulary of reversible operations (add or remove a
shape, add an instance or array, add or remove a cell). Applying an edit records
its inverse on an undo stack, so undo and redo are exact and unbounded. This same
operation log is what the collaboration layer replicates; see
[Collaboration](collaboration.md).

## The trait surface

The model also defines the stable traits the higher subsystems implement:
`DocumentStore` for editable access, `RuleSet` for design-rule checking, `Router`
for routing, `Importer` and `Exporter` for file formats, and `Renderer` for
drawing. Keeping these here lets the core stay free of the GPU, IO, and async
stacks while still describing how they plug in.
