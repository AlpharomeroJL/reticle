# 0112, xschem interop: a local probe-list format and a temporary export bridge

## Context

The v8.2 Phase 3 `xschem` lane implements `file.export_spice` and `xschem.import_probe`
against the committed SPICE exchange contract (`crates/reticle-extract/tests/fixtures/
contracts/spice_exchange_inverter.{spice,json}`), fixture-first, so it does not block on
the parallel `netlist` lane (which owns the real `DeviceNetlist` to SPICE-text writer in
`reticle_extract::spice`) or the `waveform-ui` lane (which owns wiring the `reticle-sim`
workspace dependency into `reticle-app`'s `Cargo.toml`). Two design questions follow from
that boundary: what format an "xschem-style probe list" actually is, since xschem's native
schematic file format is explicitly out of this lane's scope, and how `file.export_spice`
can do something real today when the netlist lane's writer is not yet merged into this
worktree.

## Decision

`xschem.import_probe` reads Reticle's own minimal probe-list interchange subset, not a
byte-for-byte port of any xschem file: one probe per line, `<id> <node> <quantity>`
separated by whitespace, `#` full-line comments, blank lines skipped. `quantity` is
`voltage` / `current` / `charge`, the same three variants and the same wire strings as
`reticle_sim::Quantity`, defined locally in `crates/reticle-app/src/xschem.rs` as
`ProbeQuantity` rather than taking a dependency on `reticle-sim` (that `Cargo.toml` edit is
`waveform-ui`'s owned path). The parser caps input size and probe count before doing any
per-line work and returns a structured error on anything malformed, never a panic.

`file.export_spice` runs the real, already-merged `reticle_extract::extract_devices` over
the open document, then bridges the resulting `DeviceNetlist` into the contract's
`SpiceNetlist` shape with a small, local `spice_netlist_from_devices`, and writes it with
`write_spice`. That bridge is temporary: its two-entry `DeviceKind` to PDK model name table
and its decimal-micron formatter stand in for the `netlist` lane's real tech table and
DBU-to-micron conversion. The formatter itself is exact (integer long division, never a
`f64`, so it cannot carry float-formatting drift), but the model names only cover what the
committed contract fixture names.

## Consequences

The two commands are real and tested today: `xschem.import_probe` parses a genuine probe
list into `XschemState`, and `file.export_spice` emits real SPICE from whatever devices are
recognised in the open design, both without waiting on either sibling lane. The cost is a
small, deliberate duplication: once `reticle_extract::spice` merges, `spice_netlist_from_devices`
in `xschem.rs` should be replaced by a direct call to it, and the model-name table retired.
Until then, a technology beyond what the fixture names exports with an honestly-wrong model
name rather than a fabricated one; a future change should either widen the bridge's table or
retire it in favour of the merged writer, whichever lands first.
