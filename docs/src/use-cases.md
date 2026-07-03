# Worked use cases

Reticle opens on a Start screen that offers four worked examples. Each one drops
you straight into a prepared starting point for a different part of the tool, so a
capability is one click away rather than behind a blank document. Pick one to begin,
or skip to a blank editor.

The scenarios are built by the `reticle_app::usecases` module. Everything they set
up is deterministic and self-contained: the SKY130 cell and its technology are
compiled into the binary, so the scenarios behave identically in the native
application and in the browser, where there is no filesystem. The chooser itself is
skipped for the deployed web replay build, which drops a public visitor straight
into the theater instead.

## 1. Inspect a SKY130 cell

Loads a real SkyWater SKY130 high-density standard cell, the `inv_1` inverter, from
a bundled GDSII stream. The imported geometry is given the committed SKY130
technology, so its layers arrive named and colored (nwell, diff, poly, li1, met1,
and their contacts and vias) rather than as anonymous layer numbers, and the
physical layer stack comes along so the 3D view has real elevations to extrude.

From here you can:

- Toggle layers in the layer manager to isolate the poly gate, the local
  interconnect, or metal 1.
- Measure feature widths and spacings with the measure tool, reading values in both
  database units and microns.
- Open the 3D layer-stack window to see the metals and contacts extruded to their
  process heights, and take a cross-section across the cell.

This is the fastest way to see that the importer handles a production layout, not
just fixtures, and that the layer, measurement, and 3D machinery all read a real
technology. See the [file formats](io.md) and [rendering](rendering.md) chapters for
the underlying import and 3D-stack details.

## 2. Find and fix a violation

Opens a small layout that deliberately breaks a design rule. The top cell holds two
metal-1 wires: one comfortably wider than the SKY130 minimum, and one only 80 nm
wide, which violates the SKY130 `m1.1` rule (minimum met1 width of 0.14 um, i.e.
140 nm). The SKY130 rule subset is carried in the document's technology, so a check
resolves the real periphery rules rather than a generic fallback.

The intended loop is:

1. Run the design-rule checker. The violation overlay marks the narrow wire and the
   error browser lists the `m1.1` violation with its measured-versus-required width.
2. Zoom to the violation, select the offending wire, and widen it to at least 140 nm
   (for example with the productivity panel's move-by-delta, or by editing its
   vertices).
3. Re-run the checker. With both wires meeting the minimum, the `m1.1` violation
   clears.

This exercises the whole DRC path end to end on rules that mean something. See the
[design-rule checking](drc.md) chapter and the [SKY130 rule
coverage](sky130-drc-coverage.md) table for exactly which rules the subset checks.

## 3. Watch the agent work

Opens the replay theater and plays a recorded agent run. The transcript is the
bundled model-free scripted run in which the agent places a clean met1 wire; it
drives a real engine replay, with step, play, pause, and speed controls and a live
narration feed alongside the canvas. Each verify step the run crosses feeds its
design-rule results back to the overlay, so you watch the check clear as the run
progresses.

Because the theater plays a compiled-in transcript, it needs no model, network, or
API key, and it runs identically on native and in the browser. See the [in-app agent
UX](agent-ux.md) chapter for the theater and narration, and the [agent API and
harness](agent.md) chapter for how such transcripts are produced.

## 4. Build with the new tools

Loads a small starter layout, sparse on purpose, so there is real geometry to work
with but plenty of room to build. It seeds two short metal-1 wires and a metal-2
landing pad on the SKY130 metal layers.

It is a sandbox for the newer editing tools:

- **Draw** additional shapes on the active layer, snapping to the existing geometry.
- **Boolean** the two met1 wires together, or subtract one shape from another.
- **Array** a shape into a repeated block to see hierarchy build up.
- **Via stack** from met1 up to the met2 pad, generating the enclosure and cut
  geometry between the layers.

Every edit is undo-integrated, so you can experiment freely. See the [drawing and
vertex editing](draw.md), [boolean and transform operations](boolean-transform.md),
and [productivity editing](productivity.md) chapters for the tools themselves.
