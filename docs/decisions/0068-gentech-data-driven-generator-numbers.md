# 0068, GenTech: the generator process numbers become data, threaded through the tech argument

## Context

The `reticle-gen` generators were written against the SKY130 subset with the process
numbers - conductor min width/spacing/area, cut sizes, enclosures, the substrate-tap
contact, the conservative cut pitch - baked in as constants scattered through the
topology code (the private `sky130` module, read directly as `sky130::MET1` and
friends). That made every generator SKY130-only and coupled the numbers to the
algorithms. The `Generator::generate` trait already threads a `&Technology` argument
that was unused: the designed-for refactor seam. The v8 roadmap wants a second PDK
proven by running the existing cleanliness proptests over it, which the baked constants
made impossible.

## Decision

Introduce `GenTech`, a value gathering exactly the numbers the generators need: four
stacked interconnect conductors (index 0 = base), three cuts where `cut[i]` bridges
`conductor[i]`/`conductor[i+1]`, one substrate-tap cut, and a conservative cut pitch.
Generators read it via `GenTech::for_technology(tech)` - selecting a built-in by the
`Technology` name and **defaulting to SKY130** for an empty/unrecognized name, so every
caller that passes `Technology::default()` (the app, the tests, the agent) is
byte-for-byte unaffected. The enums address conductors/cuts by *role* (level index),
not physical name. The ring/serpentine/fill/array topology, the parameter schemas, and
`validate` stay code; only the numbers become data.

This is an **internal-only, signature-preserving** amendment to the frozen `reticle-gen`
(ADR 0061 authorizes the second-PDK amendment at this wave). The `sky130` module was
already private, and no generator trait, enum, or `Registry` signature changes - the
public API only *grows* (`Conductor`, `Cut`, `GenTech`, `Residue`, `derive_gentech`),
so `reticle-app`/`reticle-mcp`/`reticle-bench`/`reticle-agent` are untouched.
`validate` keeps the reference (SKY130) bounds; the `generate` path uses the active
technology, and the contact chain gained a portability fix (it encloses each contact by
both bridging conductor levels).

`GenTech::sky130()` is an authored constant whose fidelity to the committed deck is a
tested invariant: `derive_gentech(Technology, Residue)` reconstructs it from a parsed
technology's rules (cross-checking the stack z-order), and a test asserts
`derive_gentech(committed SKY130 deck) == GenTech::sky130()`. A small per-PDK **residue**
supplies only what a rule deck cannot: the role assignment, the conservative enclosure
for a cut the deck gives no rule, and the cut pitch.

## Consequences

The same generator code runs against any process that supplies a `GenTech`, proven by
the both-PDK cleanliness oracle (ADR 0069). The residue and the authored constant carry
a maintenance cost - two representations tied by tests - accepted because it keeps the
runtime path a plain constant (no file I/O, `wasm32`-clean) while the derivation proves
faithfulness. `GenTech::sky130()` is the default, so the change is invisible to existing
behavior; a future non-orthogonal-transform-style limitation would surface in the same
tests rather than silently.
