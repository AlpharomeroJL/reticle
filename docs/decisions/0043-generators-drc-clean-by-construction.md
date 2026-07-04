# 0043, Generators are DRC-clean by construction, proven by the real DRC engine

## Context

A parameterized generator is only trustworthy if what it emits passes DRC. The two
obvious ways to get there are both unsatisfying. Running the generator's output back
through the DRC engine and *repairing* violations turns a pure geometry function into a
solver and makes its output hard to reason about. Trusting a hand-argument that "this is
clean" with no check lets a wrong number, or a rule the author forgot, ship silently.

The generators target the committed SKY130 subset in `tech/sky130-drc-subset.toml`
(min width, spacing, the contact/via sizes, three enclosures, two minimum-area rules for
the digital metal stack), so they need the exact numbers from that deck: `li1` width 170
and area 56100, `met1` width 140 and area 83000, `mcon`/`licon` size 170 enclosed by 30/80,
and so on. Those numbers have to live somewhere the generator can read them, and that copy
must not drift from the deck the app actually checks against.

## Decision

Make the generators DRC-clean *by construction*: each emits only geometry that satisfies
the cited rules, with no post-hoc repair pass. Two mechanisms back the claim.

First, a `sky130` numbers module holds the subset values as named `Conductor` and `Cut`
constants transcribed from the deck, and a unit test (`constants_match_committed_subset`)
ties every one back to `reticle_drc::sky130_drc_rules()`, so a change to the deck that the
generators target fails the test rather than silently diverging. Generators emit only
axis-aligned `Rect` shapes, which the bounding-box DRC engine checks *exactly*, and pick
geometry that clears each rule: ring strips at least the layer min width, ring openings at
least the layer min spacing, cut plates grown from the cut array by the enclosure margin
and then bumped up to the layer minimum area where one exists. Where the subset carries no
rule (cut-to-cut spacing on the contact/via layers), the generator picks a conservative
pitch and says so at the call site rather than pretending a rule constrains it.

Second, the oracle is the production checker, not a reimplementation. For each generator a
proptest (`guard_ring_is_drc_clean`, `via_farm_is_drc_clean`, plus
`registry_generate_matches_typed` for the erased path) sweeps randomized in-range
parameters, generates into a fresh cell, runs `DrcEngine::new(sky130_drc_rules())` over it,
and asserts zero violations, at 400 cases each. `validate` is covered two-directionally:
in-range samples must validate, out-of-range or contextually-invalid samples must be
rejected with a `GenError` naming the offending field.

## Consequences

- "DRC-clean by construction" is a proven property over the whole valid parameter space,
  against the same engine the app runs, not a comment. A wrong constant, a forgotten rule,
  or a bad edge case surfaces as a failing property with a minimized counterexample.
- The generators cover the subset honestly and no further: the guard ring draws on the
  interconnect layers the subset carries rules for (`li1`/`met1`/`met2`/`met3`) and lines an
  `li1` ring with `licon` taps (the one cut with an interconnect enclosure in the subset);
  the via farm bridges the adjacent metal-stack pairs with cut and enclosure rules
  (`mcon`/`via`/`via2`). Passing the subset is not tape-out clean, and the crate docs say so.
- The minimum-area rules force plate growth: a 1x1 `mcon` farm's bare enclosure plate
  (230x230 = 52900) is below the `met1` minimum area (83000), so plates are grown to a
  square of side `ceil(sqrt(min_area))`. This is load-bearing, not cosmetic, and has its own
  regression test.
- Because the numbers are baked in rather than read from the technology at run time, a user
  loading a *different* technology does not retarget the generators; they remain SKY130-subset
  generators. Generalizing to arbitrary technologies is future work, deliberately out of scope
  for this lane.
