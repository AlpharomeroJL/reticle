# 0083 - KLayout `.lydrc` DRC deck compatibility: the supported subset

## Context

Reticle's `DrcEngine` is driven by a `Vec<Rule>` over eight `RuleKind`s (width, spacing,
enclosure, extension, notch, area, density, angle), reduced to axis-aligned bounding boxes.
KLayout is the reference open-source layout tool, and its DRC decks are written in a
Ruby-based DSL stored in `.lydrc` macro files. Being able to run a real KLayout rule deck
in Reticle makes the engine credible against an external oracle instead of only against its
own SKY130 table. The engine's public types (`Rule`, `RuleKind`, `Violation`) are a frozen
de-facto contract, so a compatibility layer must compile *down* to them, never extend them:
a KLayout construct that needs a rule kind Reticle does not have is out of scope by
definition, not a reason to invent one.

The `.lydrc` syntax must be pinned to the current KLayout reference rather than to memory of
the DSL. It was fetched at build time from
[`drc_ref_layer.html`](https://www.klayout.de/doc/about/drc_ref_layer.html) and
[`drc_ref_global.html`](https://www.klayout.de/doc/about/drc_ref_global.html) (KLayout
0.29.10, the version in the pinned `hpretl/iic-osic-tools:2025.01` container). Two facts from
that reference shape the parser: a `.lydrc` file is an XML *macro* wrapper (`<klayout-macro>`
with `<category>drc</category>`, `<interpreter>dsl</interpreter>`,
`<dsl-interpreter-name>drc-dsl-xml</dsl-interpreter-name>`) whose `<text>` element holds the
DSL; and a dimension argument is micrometres when written as a float and database units when
written as an integer, with `.um`/`.dbu` suffixes to override.

## Decision

Add a new `reticle_drc::lydrc` module (this crate is not touched by any other lane) that
parses the following supported subset of a `.lydrc` deck into engine rules, and reports every
other construct as a clear error naming the construct and the line, never a panic and never a
silently dropped rule:

- **Container**: the `.lydrc` XML macro wrapper (script extracted from `<text>`, XML entities
  unescaped) or a bare `.drc` script.
- **Layer inputs**: `name = input(layer)` / `input(layer, datatype)` (datatype defaults 0).
- **Header directives**: `source(...)` and `report(...)` recognized and ignored (KLayout I/O,
  not rules).
- **Single-layer checks**: `layer.width(v)` -> Width, `layer.space(v)` -> Spacing,
  `layer.notch(v)` -> Notch.
- **Two-layer checks**: `layer.separation(other, v)` / `sep` -> Spacing with `other_layer`;
  `outer.enclosing(inner, v)` -> Enclosure. KLayout's `enclosing` receiver is the *enclosing*
  (outer) layer, whereas the engine's `Rule.layer` is the *enclosed* (inner) shape, so the
  parser swaps the two.
- **Minimum area**: `layer.with_area(0, v)` / `with_area(0.0, v)` / `with_area(nil, v)` -> Area
  (a pure below-threshold selection; a two-sided band has no single-threshold engine
  equivalent and is unsupported).
- **Reporting**: an optional trailing `.output("name"[, "description"])` names the rule.

Units follow the DSL: a float dimension is micrometres, an integer is database units, `.um`
and `.dbu` override; Reticle DBU are nanometres, so micrometres scale by 1000 (areas by
1_000_000). Extension, density, and angle are deliberately **not** mapped: no `.lydrc`
construct maps to them unambiguously within a bounding-box engine, so a deck using them is
out of the supported subset.

A committed fixture (`subset.lydrc` + `subset.gds`, built by `scripts/lydrc-fixture-gen.rb`)
is run both ways: through `parse_lydrc` + `DrcEngine` (`tests/lydrc_engine.rs`) and through
KLayout headless in the pinned container (`scripts/lydrc-compare.ps1`). The comparison is at
the **layout-level verdict** granularity (did a rule fire at all), because KLayout emits one
edge-pair marker per offending edge while the engine emits one violation per offending shape
or pair; raw counts are recorded but not required to match.

## Consequences

A real KLayout deck, restricted to the subset above, runs in Reticle and produces the same
per-rule verdict as KLayout: on the fixture, all five rules agree (four fire, one is clean),
with only `m1.4`'s raw marker count differing (KLayout 4 edge-pairs versus one engine
violation). The subset is honest and narrow: `notch` parses but its verdict is **not**
compared, because KLayout's `notch` is an intra-polygon concavity check while the engine's
Notch is an inter-shape same-layer gap on bounding boxes, and a bounding-box engine cannot
see an intra-polygon notch at all. Anything outside the subset (boolean layer algebra,
sizing, connectivity, the universal `drc` expression, extension/density/angle) fails with an
`UnsupportedConstruct` naming it. The parser treats its input as arbitrary untrusted text: it
never panics or hangs and allocates nothing from a declared count. The engine's frozen
surface is unchanged; the compatibility layer is purely additive. See the book chapter
[KLayout `.lydrc` compatibility](../src/lydrc-compat.md) for the exact grammar, the divergence
note, and the reproduction command.
