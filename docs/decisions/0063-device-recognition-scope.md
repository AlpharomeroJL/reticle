# 0063, Device recognition and device-level LVS-lite: scope and limits

## Context

`reticle-extract` recovered geometric connectivity (nets) and a connectivity-only
LVS-lite. A net is not a device: pure connectivity sees a single diffusion
rectangle as one net, so a MOSFET's source and drain come out shorted, and there is
no notion of a transistor at all. The v8 roadmap wants device-level extraction (recognize
SKY130 MOSFETs, emit a device netlist, compare at the device level) without
disturbing the frozen connectivity contract (ADR 0061 keeps `Netlist`, `Net`,
`compare_netlists`, and the intent types read-only). The open questions were how
far "device-level" should go, and how to bind terminals correctly given that the
raw diffusion shorts source to drain.

## Decision

Device recognition ships as a new `device` module, a sibling that builds on the
connectivity extractor and adds nothing to and changes nothing in the frozen types;
`lib.rs` only appends `pub mod device` and additive re-exports. A gate is a poly
shape that fully crosses a diffusion shape; NMOS versus PMOS is read from the
implant/well (`nsdm`/`psdm`/`nwell`). Terminal nets are bound by **splitting each
diffusion by its gates** and extracting connectivity over the cut geometry (reusing
`Extractor` with the SKY130 contact stack plus a tap body-tie path), so source and
drain are distinct nets exactly when the layout wires them apart. The device-level
compare (`compare_devices`) matches devices by kind and terminal-net connectivity
(gate, unordered source/drain, bulk, by net name) and reports device-count and
terminal-net mismatches; it is additive and leaves `compare_netlists` untouched.

The scope is deliberately "lite" and stated in the book: NMOS/PMOS only (no
parasitic devices, JFETs, or ESD), no device-parameter matching beyond reporting
W/L (compare is connectivity-level, not W/L-tolerance or model matching), no
series/parallel folding, flattened (non-hierarchical) extraction, and best-effort
bulk binding from the nearest matching body tap. Source and drain use a stable
low/high geometric convention, not a functional one.

The oracle is Magic's own device extraction inside the pinned
`hpretl/iic-osic-tools` container (`scripts/device-oracle.ps1`). On the production
cell `sky130_fd_sc_hd__inv_1` Magic reports 1 NMOS + 1 PMOS with matching terminal
connectivity, agreeing with our extractor on the golden fixture; the extracted
SPICE is committed. When Docker or the image is absent the tests fall back to the
committed hand-verified golden fixture and say so.

## Consequences

Reticle now recognizes transistors and does a device-level LVS-lite, so the README
non-goal "no device-level LVS / extraction is not device recognition" is updated to
the honest new boundary rather than deleted. The connectivity contract is intact,
so no other lane is affected. Because terminal binding depends on splitting
diffusion by the gate, the device netlist's `nets` differ from the plain
connectivity netlist on any cell with transistors; that is the intended, correct
behaviour (a channel is not a wire) and is documented. A fuller LVS (parameters,
parasitics, hierarchy, a netgen-driven compare) and a multi-cell oracle-agreement
table are left to a later lane; this one draws the scope line and proves the device
count and kinds against Magic.
