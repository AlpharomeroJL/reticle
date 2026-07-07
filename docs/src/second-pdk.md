# A second PDK: IHP SG13G2

Reticle's generators were written against the SKY130 subset, but the process numbers
they draw against are **data**, not baked constants. This chapter describes that
data-driven design (`GenTech`) and the second process it enables: IHP's open
SG13G2 (a 130 nm SiGe BiCMOS process).

## GenTech: numbers as data

Every generator needs the same small set of per-process numbers: the interconnect
conductors it may route on (each with a minimum width, spacing, and optional minimum
area), the cut layers that bridge adjacent conductors (each with an exact drawn size
and the enclosure a covering plate owes it), the substrate-tap contact under the base
conductor, and a conservative cut pitch for layers whose deck carries no cut-to-cut
spacing. `GenTech` gathers exactly those into one value.

The generators read `GenTech` from the `Technology` argument threaded into
`generate()` (`GenTech::for_technology`, which selects a built-in by process name and
defaults to SKY130 so every existing caller is unaffected). The ring / serpentine /
fill / via-array **topology**, the parameter **schemas**, and the **validation** stay
code; only the numbers are data. `GenTech::sky130()` is authored from the committed
SKY130 subset and cross-checked against `reticle_drc::sky130_drc_rules()` by test; a
`derive_gentech()` function reconstructs a `GenTech` from any parsed `Technology`,
proving the values are faithful to the deck (and to the stack ordering). See ADR 0068.

## Roles, not layer names

A `GenTech` is four stacked interconnect conductors (index 0 = base), three cuts where
`cut[i]` bridges `conductor[i]` and `conductor[i+1]`, and one substrate-tap cut. The
generators address these by *role* (level index), so a generator enum variant like
`RingLayer::Li1` means "the base interconnect" - `li1` on SKY130 and `Metal1` on
SG13G2. The role binding for the two shipped processes:

| role         | SKY130 | SG13G2  | SG13G2 GDS |
|--------------|--------|---------|------------|
| conductor 0  | li1    | Metal1  | 8/0        |
| conductor 1  | met1   | Metal2  | 10/0       |
| conductor 2  | met2   | Metal3  | 30/0       |
| conductor 3  | met3   | Metal4  | 50/0       |
| cut 0 (0↔1)  | mcon   | Via1    | 19/0       |
| cut 1 (1↔2)  | via    | Via2    | 29/0       |
| cut 2 (2↔3)  | via2   | Via3    | 49/0       |
| substrate tap| licon1 | Cont    | 6/0        |

## The SG13G2 data and its provenance

`tech/ihp-sg13g2.tech` carries the layer table, the physical stack, and the DRC subset
inline; `tech/sg13g2-drc-subset.toml` is the cited source of record. Every number is
transcribed from the open IHP-Open-PDK (github.com/IHP-GmbH/IHP-Open-PDK, branch main,
**Apache-2.0**), with the KLayout DRC rule ids preserved (e.g. `M1.a`, `V1.c`). The
subset mirrors the digital routing stack (Metal1–Metal4 width/spacing, the Cont/Via1–3
sizes and via enclosures) and deliberately omits what the generators do not draw
against: the wide-metal and pattern-density spacing variants (which the DRC engine
cannot express as width-conditional), the FEOL Activ/GatPoly contact enclosures, and
the thick TopMetal stack. Passing it is not tape-out clean. See ADR 0069.

## The proof: both PDKs, clean by construction

The cleanliness oracle runs every generator over **both** processes: it samples random
valid parameters, generates into a fresh cell using each process's `Technology`, and
asserts the real DRC engine finds **zero** violations under that process's own deck.
The same generator code, handed a different technology by name, draws against that
process's own layers and numbers and stays clean. Making this hold for the second
process took exactly one generator change - the contact chain now encloses each contact
by *both* bridging conductor levels, so whichever level a process's deck requires to
enclose the cut (`met1` encloses `mcon` on SKY130, `Metal1` encloses `Via1` on SG13G2)
is satisfied. Everything else was already portable once the numbers became data.
