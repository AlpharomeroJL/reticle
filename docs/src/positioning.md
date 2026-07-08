# Positioning

This chapter is an honest map of where Reticle sits among layout tools. It states
what Reticle is, what the established tools do that Reticle does not, and the full
list of things Reticle deliberately is not. The goal is that a reader from the EDA
world knows within a page whether this project is relevant to them and can trust
that nothing here is oversold.

## The field

Physical IC layout has a mature, mostly open or commercial tooling landscape.
Reticle overlaps a narrow slice of it.

- **KLayout** is the reference open-source layout viewer and editor. It reads and
  writes GDSII and OASIS at production scale, has a full DRC and LVS scripting
  engine (Ruby and Python), a rich GUI, and a large plugin ecosystem. It is the tool
  most engineers reach for to open, inspect, and script mask data. Reticle does not
  approach KLayout's format coverage, DRC-language maturity, or feature breadth.
- **Magic** is the classic open-source layout editor from Berkeley, still widely
  used, with interactive layout, a built-in continuous DRC, and its own extraction
  to a SPICE netlist. Magic's device-level extraction (recognizing transistors,
  resistors, and capacitors from geometry and emitting a netlist a simulator can
  read) is a capability Reticle does not have.
- **Commercial place-and-route and signoff flows** (the tools from Cadence,
  Synopsys, and Siemens EDA) cover the parts of the flow that turn a netlist into a
  manufacturable, verified mask set: synthesis, floorplanning, placement, clock-tree
  synthesis, detailed routing, parasitic extraction, static timing analysis,
  signoff DRC and LVS against a foundry-qualified rule deck, and tape-out. These are
  the tools a design must pass through to be fabricated. Reticle does none of this
  and is not on that path.
- **The open SKY130 flow** (OpenLane / OpenROAD on the SkyWater SKY130 PDK) is the
  open reference for taking RTL to a GDSII that a shuttle can fabricate. Reticle uses
  the same SKY130 PDK data as a source of real, cited numbers (layers, a stack, and a
  DRC rule subset), but it is not part of the OpenLane flow and produces nothing that
  flow consumes.

## Where Reticle sits

Reticle is a browser-native, GPU-accelerated **viewer and editor** for very large
hierarchical layout scenes, with a checker layer and an agent layer on top. Its
distinct emphasis is threefold:

1. **Interactive rendering of very large scenes in a browser.** Reticle renders
   hierarchical IC geometry through a retained GPU scene on `wgpu` (WebGPU natively
   and in the browser, with a WebGL2 fallback), never flattening the hierarchy for
   browsing, so an arrayed cell with billions of effective leaf shapes costs only what
   is on screen. The measured retained path holds interactive frame rates on scenes of
   ten million leaf shapes (see [Performance methodology](performance.md) and
   `PERF.md`). Running this in a plain browser tab, with no install, is the part of the
   problem Reticle pushes hardest on.
2. **Objective checkers as a first-class engine.** The design-rule checker, router,
   and connectivity extractor are each pinned to an independent reference oracle by
   property tests, so their correctness is demonstrated rather than asserted. They are
   built to be driven programmatically, not only from a GUI. The same discipline covers
   the six parameterized generators (guard ring, via farm, pad ring, seal ring, density
   fill, test structure): a property test runs each over 400 random valid parameter sets
   and asserts zero DRC violations, so the structures they emit are clean by construction.
3. **An agent surface graded by those checkers.** Reticle exposes its whole editing
   engine as a serializable command API and drives a model through a
   propose-verify-correct loop whose success is decided by the DRC subset and the
   connectivity checker, not by the model's own claim. A benchmark suite scores that
   loop; an MCP server offers the surface (including one tool per generator) to any model
   host. When the surface is driven by a bare local model, Reticle supplies the loop and
   grades the result; an agent system such as Claude Code brings its own loop, so its
   result is not head-to-head comparable with a bare model. This agent-plus-objective-
   checker angle is where Reticle does something the established tools do not package.

So the honest one-line placement: Reticle is a fast, browser-native layout viewer and
editor with a verified checker core and a checker-graded agent layer. It is a
portfolio-grade engineering project and a research vehicle for machine-driven layout,
not a production EDA tool.

## What the established tools do that Reticle does not

These are real capabilities of KLayout, Magic, or the commercial and open signoff
flows that Reticle does not have. This list is deliberately blunt.

- **Full-fidelity format coverage.** Reticle reads and writes GDSII with full
  hierarchy (via `gds21`). Its `Oasis` type is **not** interoperable OASIS: it is an
  in-house, OASIS-inspired *container* (ADR 0004) that no other tool reads, used only
  to round-trip Reticle's own geometry. A separate conformant-OASIS **writer**
  (`oasis_std`) emits a practical SEMI P39 subset that KLayout does read - export only,
  verified against KLayout in-container (see the [GDS / OASIS interop](interop.md)
  chapter). Even so, KLayout's format coverage is far broader.
- **A production DRC rule language.** Reticle's DRC is a fixed set of eight rule
  kinds (width, spacing, enclosure, extension, notch, area, density, angle) evaluated
  over indexed geometry. It is not a general rule-scripting language, and its SKY130
  deck is a small cited subset (see [SKY130 grounding](sky130.md)). KLayout's DRC and
  Magic's continuous DRC are far more complete.
- **Layout-versus-schematic (LVS).** Reticle has connectivity extraction and can
  compare an extracted netlist against an expected one (opens and shorts), but it does
  not do LVS in the device-recognition sense: it does not identify transistors,
  resistors, or capacitors from geometry, and it does not match a layout against a
  schematic netlist.

The remaining, larger set of things Reticle does not do is the not-list below.

## The full not-list

Reticle deliberately does **not** include, and does not claim, any of the following.
Each is stated so no reader mistakes the project's scope.

- **No logic or physical synthesis.** Reticle does not turn RTL, a gate netlist, or a
  behavioral description into geometry. There is no synthesis, no technology mapping,
  no floorplanning, no placement of a synthesized netlist. The router routes explicit
  nets on a grid; it is not a place-and-route flow.
- **No timing.** There is no static timing analysis, no parasitic (RC) extraction for
  timing, no delay calculation, and no timing-driven optimization anywhere in the
  project. Reticle knows geometry and connectivity, not timing.
- **No device-level LVS.** Reticle does not recognize devices (transistors,
  resistors, capacitors, diodes) from layout geometry and does not match a layout
  against a schematic. Its extraction is geometric net connectivity (union-find over
  touching shapes and cross-layer vias) with an opens-and-shorts compare against an
  expected netlist. That is net-level connectivity checking, not LVS.
- **No tape-out signoff.** Passing Reticle's checks does not mean a layout is
  manufacturable. The SKY130 DRC subset is a fast first filter over everyday geometry
  mistakes against cited values; it is explicitly **not tape-out clean** and omits
  antenna, density, latch-up, most implant and well rules, and the exact-size and
  differential contact and via rules. There is no signoff DRC, no signoff LVS, no fill
  insertion for manufacturing, no antenna fixing, and no foundry-qualified rule deck.
  See [SKY130 grounding](sky130.md) for exactly which rules are and are not checked.

Beyond those four, Reticle also does not provide: analog or custom device generators
(PCells beyond the built-in array and via-stack helpers), a schematic editor,
simulation of any kind (no SPICE, no field solver), mask data preparation (OPC,
fracturing), a parametric-cell scripting language matching KLayout's PCell system, or
any manufacturing handoff.

## Reading this honestly

Everything above is checkable. The capabilities Reticle does have are audited,
per-feature, with a command you can run yourself, in
[the project status report](https://github.com/AlpharomeroJL/reticle/blob/main/docs/STATUS.md).
The subsystems that carry the "verified" weight (geometry booleans, spatial index,
DRC, routing, extraction, and CRDT convergence) are each pinned to an independent
reference oracle by property tests, which is the strongest evidence in the project
that they compute what they claim. When this chapter says Reticle does something, that
audit backs it; when it says Reticle does not, that absence is deliberate and stated so
the project is not mistaken for a production flow it is not.
