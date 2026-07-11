# 0108: the SPICE writer's model table is caller-supplied, not derived

## Context

Phase 3 (Depth) adds circuit simulation interop. The `netlist` lane writes an extracted
`DeviceNetlist` (`reticle-extract/src/device.rs`: recognised NMOS/PMOS devices with terminal
nets and channel W/L in DBU) out as a SPICE subcircuit, so external simulators and the
`xschem` lane's schematic-capture tooling can read a layout's transistor-level netlist. The
`xschem` lane reads the same exchange subset back, fixture-first
(`tests/fixtures/contracts/spice_exchange_inverter.{spice,json}`, scaffolded before either
lane started, per the Phase 3 pre-fan-out commit), so the two lanes do not block on each
other.

The committed contract fixture models `sky130_fd_sc_hd__inv_1`, whose real PDK subcircuit
names its PMOS device `sky130_fd_pr__pfet_01v8_hvt` (a high-Vt primitive) and its NMOS
`sky130_fd_pr__nfet_01v8` (standard-Vt). `DeviceKind` (device.rs) recognises only two device
kinds, `Nmos` and `Pmos`, from poly-over-diffusion geometry plus the surrounding
implant/well; it carries no threshold-voltage flavour, body-bias variant, or any other
model-selection detail, because nothing in the extracted geometry encodes that (SKY130 does
not mark Vt flavour with a distinguishing layer the way it marks implant/well). A model-name
table keyed only on `DeviceKind` therefore cannot honestly special-case one specific cell's
PMOS as high-Vt without also mislabelling every standard-Vt PMOS in every other extracted
layout as high-Vt.

## Decision

`SpiceTech { nmos_model, pmos_model }` (`reticle-extract/src/spice.rs`) is a plain data
struct the caller supplies, mirroring how `DeviceTech` already supplies fixed SKY130 layer
numbers that device recognition has no way to derive from first principles either.
`SpiceTech::sky130()` is a convenience default using the plain, standard-Vt primitive names
(`sky130_fd_pr__nfet_01v8` / `sky130_fd_pr__pfet_01v8`); it does not claim to detect Vt
flavour. A caller that independently knows a specific extracted device is a high-Vt part (as
the `spice_writer.rs` cross-test does, for the `inv_1` contract fixture) builds its own
`SpiceTech` with that model name.

This keeps the committed contract fixture unchanged: `writer_matches_the_spice_exchange_contract_structure`
(`tests/spice_writer.rs`) builds a hand-geometry inverter with separated body-tie straps (so
`VPB`/`VNB` bind to their own nets rather than shorting into the power rails the way the
smaller `common::inverter()` device-recognition fixture does), extracts it, and passes an
explicit `SpiceTech` naming the PMOS model `sky130_fd_pr__pfet_01v8_hvt` to match. The
extracted structure (kind, terminal net names, model, and W/L converted from the hand-built
geometry's DBU) matches `spice_exchange_inverter.json` exactly, and `parse_spice` round-trips
both the writer's own output and the committed `.spice` file's text. No fixture edit was
needed.

W/L convert from DBU to decimal microns by exact integer long division (`format_microns`),
never a float, so `650` DBU at `1000` DBU/micron is the literal string `"0.65"`, not a
float-formatted approximation. A terminal `extract_devices` could not bind to a net
(`Option::None`) writes the documented placeholder node `NC`, never a guessed name.
Area/perimeter params (`ad`/`pd`/`as`/`ps`) are not written: `Device` carries no diffusion-area
data, and the exchange contract's `_comment` field already documents their absence.

`parse_spice` exists only to round-trip this writer's own output (and validate the committed
fixture) for the cross-test; it is explicitly documented as not a hardened importer for
arbitrary SPICE decks, and has not been through a fuzz campaign the way Reticle's binary
format readers (GDS, OASIS) have. It still never panics on malformed input: every parse
failure returns a `SpiceParseError` variant.

## Consequences

- The `xschem` lane's reader builds against the unchanged committed fixture; no coordinated
  fixture swap was needed at the gate.
- A future cell whose extracted device is a different Vt flavour, body-bias variant, or any
  other model distinction geometry does not carry is handled the same way: the caller passes
  a `SpiceTech` with the right name. Adding real Vt-flavour recognition to `DeviceKind` itself
  (a marker-layer read, analogous to the existing implant/well classification) is a separate,
  future decision, not assumed here.
- The SPICE writer adds no new dependency (`serde`/`serde_json` were already present in
  `reticle-extract` for the F3 query contract); `spice.rs` itself does not use serde at all,
  since the cross-test compares structurally against `serde_json::Value` rather than deriving
  `Deserialize` for a JSON-shaped type, keeping the writer's own types independent of the
  fixture's exact JSON nesting.
