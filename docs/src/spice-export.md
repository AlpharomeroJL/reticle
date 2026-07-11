# SPICE netlist export

Reticle recognises transistors from layout geometry ([device
recognition](device-extraction.md): `DeviceKind`, terminal nets, and channel
W/L in DBU) and writes that as a SPICE subcircuit, for exchange with external
simulators and schematic-capture tools. The writer lives in `reticle-extract`
(`spice.rs`), a sibling of device recognition, not a change to it: it consumes
an already-built `DeviceNetlist` and adds no new extraction logic.

## The exchange subset

The interchange subset is fixed by the committed contract fixture
`crates/reticle-extract/tests/fixtures/contracts/spice_exchange_inverter.spice`
(with its structural companion `.json`):

- `*` full-line comments and blank lines, skipped on read, optional on write.
- One `.subckt NAME <ports...>` / `.ends` wrapper.
- One `X` device-instance card per device: `Xn <drain> <gate> <source> <bulk>
  <model> w=<W> l=<L>`, `n` the device's index in `DeviceNetlist::devices`.
- Decimal-micron `w=`/`l=` params, and `.end`.

The `netlist` lane's writer (`to_spice_subckt`, `format_spice`, `write_spice`)
emits this subset; the `xschem` lane reads it back. Both lanes build against
the committed fixture before the other lane's half exists (see [ADR
0108](../decisions/0108-spice-netlist-export.md)), so neither blocks on the
other.

## What is written, and from where

- **Ports** are the nets any device terminal references, kept in the
  extracted netlist's own stable (lowest-member-index) order -- the writer
  invents no ordering of its own.
- **W/L** convert from the `Device` DBU fields to decimal microns by exact
  integer long division, never a float: `650` DBU at `1000` DBU/micron is the
  literal string `"0.65"`, with no `0.6500000001`-style formatting drift.
- **The model name** comes from `SpiceTech`, a small table the caller
  supplies, keyed only on `DeviceKind` (NMOS/PMOS). Device recognition reads
  no threshold-voltage flavour, body-bias variant, or other model-selection
  detail from geometry, so the table cannot honestly derive one either; see
  [ADR 0108](../decisions/0108-spice-netlist-export.md) for why this is
  caller-supplied rather than baked into a single "sky130" default.

## What is honestly absent

- **Area/perimeter parameters** (`ad`/`pd`/`as`/`ps`): `Device` carries no
  diffusion-area data, so these are never invented.
- **A guessed node name**: a terminal `extract_devices` could not bind to a
  net (`Option::None`) is written as the documented placeholder node `NC`.
- **A general SPICE importer**: `parse_spice` reads back only the subset
  `format_spice` writes, enough to round-trip the writer's own output and
  validate the committed contract fixture. It never panics on malformed
  input -- every failure is a `SpiceParseError` -- but it is not a hardened
  importer for arbitrary, untrusted decks, and has not been through a fuzz
  campaign the way Reticle's binary-format readers (GDS, OASIS) have.

## Cross-test

`crates/reticle-extract/tests/spice_writer.rs` extracts a hand-built SKY130
inverter (separate `VPB`/`VNB` body-tie straps, so the bulk terminals bind to
their own nets rather than shorting into the power rails the way the smaller
device-recognition fixture does), writes it, and checks the result three ways:
the structured output matches the contract JSON's kind/terminals/model/W/L;
the emitted text parses back to the same structure; and the *committed*
`.spice` fixture itself parses and agrees with the writer's structural output.

