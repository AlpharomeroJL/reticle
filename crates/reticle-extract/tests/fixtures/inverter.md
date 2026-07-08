# Golden fixture: minimal SKY130 inverter

This is the hand-verified reference for the device-extraction tests. It is a
**synthetic** layout (coordinates written by hand, not derived from any external
design), so it carries no third-party provenance. It is built in code by
`inverter()` in `tests/common/mod.rs`; this file is the annotated record of what
that geometry is and why the expected device list is what it is.

A byte-for-byte copy of the real production cell lives at
`crates/reticle-app/assets/sky130_fd_sc_hd__inv_1.gds` (and the io corpus); the
container oracle (`scripts/device-oracle.ps1`) runs Magic over *that* file. This
synthetic fixture is deliberately minimal so the expected netlist is obvious by
inspection.

## Layers (SKY130 GDS layer/datatype)

| Layer  | GDS     | Role                                    |
| ------ | ------- | --------------------------------------- |
| nwell  | 64/20   | n-well (under the PMOS)                  |
| diff   | 65/20   | active (source/drain/channel diffusion) |
| tap    | 65/44   | well/substrate body-tie diffusion       |
| poly   | 66/20   | gate polysilicon                        |
| licon1 | 66/44   | local-interconnect contact              |
| li1    | 67/20   | local interconnect (metal-0)            |
| nsdm   | 93/44   | n+ select/implant                       |
| psdm   | 94/20   | p+ select/implant                       |

## Geometry (database units = nm)

A single vertical poly stripe (input `A`) at x [40, 60] crosses both diffusions.

```
 y
200 +--------------------- nwell [-10,90]-[110,210] ------------------+
    |                                                                |
190 |   n-tap (VPWR) [0,165]-[100,190]   nsdm [-5,160]-[105,195]      |
    |        licon1 [12,172]-[28,185]                                 |
140 |   PMOS diff [0,100]-[100,140]      psdm [-5,95]-[105,145]        |
    |   src lobe [0,40]   | A |  drn lobe [60,100]                     |
100 |   licon1 [12,112]-[28,128]         licon1 [72,112]-[88,128]      |
    |   li1 VPWR [8,108]-[32,188]        li1 Y [68,8]-[92,132]         |
 40 |   NMOS diff [0,100]-[100,40]       nsdm [-5,-5]-[105,45]         |
    |   src lobe [0,40]   | A |  drn lobe [60,100]                     |
  0 |   licon1 [12,12]-[28,28]           licon1 [72,12]-[88,28]        |
    |   li1 VGND [8,-35]-[32,32]                                       |
-15 |   p-tap (VGND) [0,-40]-[100,-15]   psdm [-5,-45]-[105,-10]        |
    |        licon1 [12,-32]-[28,-20]                                  |
    +----------------------------------------------------------------+ x
        0        40   60        100
```

- **Poly `A`** at x [40, 60], y [-10, 150] is vertical (taller than wide), so
  current flows in x: gate **length L = 20** (poly width), gate **width W = 40**
  (diffusion height) for both transistors.
- The channel (poly ∩ diff) splits each diffusion into a low-x **source** lobe and
  a high-x **drain** lobe.

## Expected device list (hand-verified)

| Device | Kind | Gate | Source | Drain | Bulk | W  | L  |
| ------ | ---- | ---- | ------ | ----- | ---- | -- | -- |
| M0     | NMOS | A    | VGND   | Y     | VGND | 40 | 20 |
| M1     | PMOS | A    | VPWR   | Y     | VPWR | 40 | 20 |

Reasoning:

- **NMOS** (bottom): n+ select over the active, no n-well → NMOS. Source lobe ties
  to the `VGND` li1 rail (through licon1), drain lobe to the `Y` strap. Its p-tap
  (p+ implant) ties the substrate to `VGND`, so bulk = `VGND`.
- **PMOS** (top): p+ select over the active, inside the n-well → PMOS. Source lobe
  ties to `VPWR`, drain lobe to the shared `Y` strap. Its n-tap ties the n-well to
  `VPWR`, so bulk = `VPWR`.
- Both gates are the same poly → the shared input `A`. Both drains are strapped to
  the same `Y` li1 → the output. This is exactly an inverter: `1 NMOS + 1 PMOS`.

Note the key point device recognition adds over pure connectivity: each single
diffusion rectangle would be **one** net under connectivity extraction (source
shorted to drain), because a plain wire and a transistor look identical to a
same-layer union-find. Splitting the diffusion by its gate is what recovers the
distinct source and drain nets asserted in the tests.

## Oracle agreement (Magic)

`inverter_magic.spice` in this directory is Magic's own device extraction of the
**real** production cell `sky130_fd_sc_hd__inv_1.gds`, produced by
`scripts/device-oracle.ps1` inside the pinned `hpretl/iic-osic-tools:2025.01`
container (Magic 8.3.513, sky130A PDK). Provenance flows from the source GDS,
whose Apache-2.0 attribution is in `crates/reticle-app/assets/NOTICE.md`.

Magic's netlist is:

```
X0 Y A VGND VNB sky130_fd_pr__nfet_01v8      w=0.65 l=0.15
X1 Y A VPWR VPB sky130_fd_pr__pfet_01v8_hvt  w=1    l=0.15
```

That is **1 NMOS + 1 PMOS**, gate = `A`, drains = `Y`, sources on `VGND` / `VPWR`,
bodies on `VNB` / `VPB` (the substrate/well ties). This is the same device count,
the same kinds, and the same terminal connectivity our extractor recovers from the
synthetic fixture above, an independent-tool agreement at the device level. Magic's
device order (drain, gate, source, body) and its W/L in microns are the SPICE
convention; our L = 0.15 um = 150 nm matches, and our synthetic W is not intended
to reproduce the production cell's.
