# 0031, editor productivity ops are lifted to the agent command surface

## Context

The Wave 2 editor gained productivity operations (boolean combines, align,
distribute, offset/size, and a via-stack builder) implemented in `reticle-app`
(`ops.rs`, `productivity.rs`). Those operations existed only behind the egui panels,
so an agent driving Reticle through `reticle-agent-api` (and thus through
`reticle-mcp`) could add and transform individual shapes but could not restructure
geometry the way a person can: it could not union two rectangles, align a row of
shapes, or drop a design-rule-correct via stack. `reticle-agent-api` is a frozen
surface, and `reticle-app` is deliberately not one of its dependencies, so the panel
code could not simply be called.

## Decision

Extend the frozen `AgentCommand` enum additively (it is `#[non_exhaustive]`, so this
is a compatible change) with five new variants, and implement them at the command
layer against the same `reticle-geometry` primitives the editor uses, rather than
depending on `reticle-app`:

- `BooleanCombine { cell, bool_op, ids, layer }` union / intersection / difference /
  xor over a set of shapes, writing the result to a target layer and deleting the
  inputs. The boolean is named `bool_op` because `op` is the enum's serde tag.
- `AlignShapes { ids, align }` align shapes within their combined bounding box.
- `DistributeShapes { ids, axis }` respace three or more shapes to equal gaps.
- `OffsetShapes { ids, delta }` grow (positive) or shrink (negative) shapes.
- `BuildViaStack { cell, lower_layer, upper_layer, cut_layer, center, cut_size,
  default_enclosure }` a square cut plus a lower and upper enclosure, sized from the
  technology's `RuleKind::Enclosure` rules (falling back to `default_enclosure`).

The apply-layer logic mirrors the editor's (`reticle_geometry::polygon_boolean` and
`offset`, the same fold and per-layer semantics, the same enclosure lookup), so an
agent-built result and a hand-built one are bit-for-bit identical. Each variant is
exposed as one `reticle-mcp` tool with a tight model-facing description and a
hand-written JSON schema, and is covered by the two-way schema test and the stdio
integration test. This is an authorized amendment to the otherwise-frozen surface.

## Consequences

An agent can now perform the same structural edits a person can, which the benchmark
tasks and the demo loop can exercise. The cost is a second, parallel implementation
of the boolean/align/distribute/offset/via logic: the apply-layer helpers duplicate
the shape of `reticle-app`'s `ops.rs` and `productivity.rs` rather than sharing code,
because the layering forbids the dependency. They are kept in sync by both running on
the shared `reticle-geometry` engine and by unit tests that pin the exact geometry
(a union's bounding box, an enclosure's margins). If the two drift, the fix is to
extract the shared logic into a lower crate both can depend on; until then the
duplication is the deliberate price of keeping `reticle-agent-api` free of the app.
