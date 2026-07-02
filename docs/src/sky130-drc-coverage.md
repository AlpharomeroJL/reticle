# SKY130 rule coverage

This deck is a SUBSET of the SKY130 design rules: passing it is NOT tape-out
clean, and it does not cover antenna, density, latch-up, or most implant and
well rules. It exists so the editor, the agent harness, and the benchmark
suite can check the everyday geometry mistakes (too narrow, too close, too
small, under-enclosed, short endcap) against real, cited SKY130 values, not so
a design can be signed off.

The rules live in `tech/sky130-drc-subset.toml`, transcribed from the
[SkyWater SKY130 periphery rules](https://skywater-pdk.readthedocs.io/en/main/rules/periphery.html)
for the digital metal stack. `reticle_drc::sky130_drc_rules()` embeds that file
at compile time and returns it as engine rules ready for `DrcEngine::new`; see
[Design-rule checking](drc.md) for how the engine evaluates each kind. Values
are database units (1 dbu = 1 nm); areas are dbu squared.

## Checked rules

Layer names follow `tech/sky130.tech` (GDS `layer/datatype`).

| Rule id | Kind | Layer(s) | Value | Meaning |
|---------|------|----------|-------|---------|
| `li.1` | width | li1 (67/20) | 170 (0.17 um) | Min width of li1 |
| `li.3` | spacing | li1 (67/20) | 170 (0.17 um) | Min li1 to li1 spacing |
| `li.5` | enclosure | licon1 (66/44) in li1 (67/20) | 80 (0.08 um) | licon1 enclosed by li1 |
| `li.6` | area | li1 (67/20) | 56100 (0.0561 um²) | Min li1 area |
| `m1.1` | width | met1 (68/20) | 140 (0.14 um) | Min width of met1 |
| `m1.2` | spacing | met1 (68/20) | 140 (0.14 um) | Min met1 to met1 spacing |
| `m1.4` | enclosure | mcon (67/44) in met1 (68/20) | 30 (0.03 um) | mcon enclosed by met1 |
| `m1.6` | area | met1 (68/20) | 83000 (0.083 um²) | Min met1 area |
| `m2.1` | width | met2 (69/20) | 140 (0.14 um) | Min width of met2 |
| `m2.2` | spacing | met2 (69/20) | 140 (0.14 um) | Min met2 to met2 spacing |
| `m2.4` | enclosure | via (68/44) in met2 (69/20) | 55 (0.055 um) | via enclosed by met2 |
| `m3.1` | width | met3 (70/20) | 300 (0.3 um) | Min width of met3 |
| `m3.2` | spacing | met3 (70/20) | 300 (0.3 um) | Min met3 to met3 spacing |
| `m4.1` | width | met4 (71/20) | 300 (0.3 um) | Min width of met4 |
| `m4.2` | spacing | met4 (71/20) | 300 (0.3 um) | Min met4 to met4 spacing |
| `m5.1` | width | met5 (72/20) | 1600 (1.6 um) | Min width of met5 |
| `m5.2` | spacing | met5 (72/20) | 1600 (1.6 um) | Min met5 to met5 spacing |
| `poly.1a` | width | poly (66/20) | 150 (0.15 um) | Min width of poly |
| `poly.2` | spacing | poly (66/20) | 210 (0.21 um) | Min poly to poly spacing |
| `poly.8` | extension | poly (66/20) past diff (65/20) | 130 (0.13 um) | Poly endcap past diff |
| `difftap.1` | width | diff (65/20) | 150 (0.15 um) | Min width of diff or tap |
| `difftap.3` | spacing | diff (65/20) | 270 (0.27 um) | Min diff to diff spacing |
| `licon.1` | width | licon1 (66/44) | 170 (0.17 um) | licon1 size, as min width |
| `ct.1` | width | mcon (67/44) | 170 (0.17 um) | mcon size, as min width |
| `via.1a` | width | via (68/44) | 150 (0.15 um) | via size, as min width |
| `via2.1a` | width | via2 (69/44) | 200 (0.2 um) | via2 size, as min width |

26 rules: 12 width, 8 spacing, 3 enclosure, 2 area, 1 extension. The loader's
tests pin this count and the per-kind distribution, so the table above and the
committed data cannot drift apart silently.

## Not covered

Everything not in the table, including but not limited to:

- **Antenna rules.** No charge-accumulation checks at all.
- **Density rules.** No metal fill or min/max density windows (the engine has a
  density check, but this deck defines none).
- **Latch-up and well rules.** No nwell spacing/width, no tap distance rules
  (`tap.*`, `nwell.*`), no butting rules.
- **Implant and marker layers.** `nsdm`/`psdm`/`npc` and friends are in the
  layer map but carry no rules here; most implant enclosure/spacing rules
  (`nsd.*`, `psd.*`, `npc.*`) are absent.
- **Most contact/via rules.** Sizes are encoded as min width only (the real
  rules are exact-size), and only three enclosure directions are present; end-of-line,
  differential enclosure (`m1.5`, `m2.5`), array spacing, and licon-on-poly vs
  licon-on-diff distinctions are absent.
- **Transistor-level rules.** Gate spacing to licon, diff extension past poly
  (`difftap.2`, `poly.7`), and everything hvi/hv related.
- **Resistors, capacitors, SRAM, sealring, pad** special-case rules.

Two engine caveats also apply (see [Design-rule checking](drc.md)): shapes are
reduced to axis-aligned bounding boxes, which is exact for rectangles but
conservative (may over-report, never under-report) for polygons and paths; and
same-layer spacing treats touching or overlapping shapes as merged rather than
as violations.

If a layout must be manufacturable, run the full SkyWater deck in a sign-off
tool. This subset is a fast, honest first filter, nothing more.
