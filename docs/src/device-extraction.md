# Device recognition

Connectivity extraction recovers nets; device recognition goes one level up and
recovers the **transistors** those nets connect. It is a new module in
`reticle-extract` (`device`), a sibling of the connectivity types, not a change to
them.

## A gate is poly over diffusion

A MOSFET is where a poly shape crosses a diffusion shape: that overlap is the
channel (the gate). The extractor:

1. Flattens the cell and finds every poly-over-diffusion overlap that fully
   crosses the diffusion (a partial overlap leaves diffusion on only one side and
   is not a channel).
2. Classifies each gate as **NMOS** or **PMOS** from the surrounding implant and
   well: p+ select (`psdm`) or an `nwell` means PMOS; n+ select (`nsdm`) or bare
   substrate means NMOS.
3. Measures the channel: gate **length** is the poly extent across the channel,
   gate **width** is the diffusion extent under the poly.

## Why a transistor needs more than connectivity

Pure connectivity sees a single diffusion rectangle as **one** net: a plain wire
and a transistor look identical to a same-layer union-find, so source and drain
come out shorted. That is wrong for a transistor, whose channel does not conduct
at DC.

Device recognition fixes this by **splitting the diffusion by its gate** before it
assigns terminal nets. Each diffusion is cut into the lobes on either side of every
channel, and connectivity is extracted over that cut geometry (reusing the same
`Extractor`, with the SKY130 contact/via stack plus a body-tie path). The result is
that a transistor's source and drain land on distinct nets exactly when the layout
wires them apart, and the gate, source, drain, and bulk terminals each bind to a
real net.

## Device-level LVS-lite

`compare_devices` compares an extracted device netlist against an expected one by
**device kind and terminal-net connectivity**: each device reduces to its kind,
gate, unordered source/drain, and bulk net names, and devices are matched by that
signature. It reports the devices each side has that the other does not, catching
device-count and terminal-net mismatches. The match is name-based, so both sides
name their nets (the layout through label geometry, the schematic directly). This
extends the connectivity `compare_netlists` additively; that function is unchanged.

## Scope and limits

This is deliberately "lite" and honest about its edges:

- Recognizes NMOS and PMOS only: no parasitic devices (diodes, capacitors,
  bipolars), no JFETs, no ESD structures.
- No device-parameter matching beyond reporting W/L: `compare_devices` matches on
  connectivity, not on W/L tolerance or model name.
- No series/parallel device folding and no hierarchical device extraction; it works
  over the flattened cell.
- Bulk binding is best-effort from the nearest matching body tap; an untapped body
  is left unbound rather than guessed.
- Source and drain are geometrically symmetric, so their labelling is a stable
  low/high convention, not a claim about circuit function.

## Oracle

The recognition is checked against an independent tool. `scripts/device-oracle.ps1`
runs Magic's own device extraction on a GDS inside the pinned
`hpretl/iic-osic-tools` container (Magic + the sky130A PDK, the same image the
tt-precheck recipe uses). On the production cell `sky130_fd_sc_hd__inv_1` Magic
extracts 1 NMOS + 1 PMOS with gate `A`, drains `Y`, and sources on `VGND` / `VPWR`,
the same device count, kinds, and terminal connectivity the extractor recovers from
the hand-built golden fixture (`crates/reticle-extract/tests/fixtures/inverter.md`).
When Docker or the image is unavailable the tests fall back to that committed golden
fixture and state the limitation.

A full multimodal oracle and an oracle-agreement table across more cells are a
separate lane; this chapter documents only the device recognition and its Magic
device-count agreement.
