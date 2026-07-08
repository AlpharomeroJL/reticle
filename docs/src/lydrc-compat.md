# KLayout `.lydrc` compatibility

Reticle can run a documented subset of a real [KLayout](https://www.klayout.de/) `.lydrc`
DRC deck. `reticle_drc::parse_lydrc` compiles the deck into the same `Vec<Rule>` the
[design-rule engine](drc.md) already runs, so a KLayout rule deck (restricted to the subset
below) produces Reticle violations. Anything outside the subset fails with a clear error that
names the construct and the line; the parser never panics on untrusted input.

The syntax is pinned to the current KLayout DRC reference, not to memory of the DSL:
[DRC Layer reference](https://www.klayout.de/doc/about/drc_ref_layer.html) and
[DRC global functions](https://www.klayout.de/doc/about/drc_ref_global.html), read against
KLayout 0.29.10 (the version in the pinned `hpretl/iic-osic-tools:2025.01` container). See
[ADR 0083](../decisions/0083-lydrc-drc-deck-compatibility-subset.md) for the scope decision.

## The file format

A `.lydrc` file is KLayout's DRC *macro* format: an XML wrapper whose `<text>` element holds
the Ruby DRC DSL script.

```xml
<klayout-macro>
 <category>drc</category>
 <interpreter>dsl</interpreter>
 <dsl-interpreter-name>drc-dsl-xml</dsl-interpreter-name>
 <text>
met1 = input(68, 20)
met1.width(0.14).output("m1.1", "met1 minimum width")
 </text>
</klayout-macro>
```

The parser extracts and XML-unescapes the `<text>` body; a bare `.drc` script (no wrapper)
is also accepted.

## Units

Following the DSL, a **floating-point** dimension is micrometres and an **integer** dimension
is database units; an explicit `.um` or `.dbu` suffix overrides. Reticle's database units are
nanometres (1 dbu = 1 nm, as in the SKY130 table), so a micrometre value is scaled by 1000
(areas by `1_000_000`) into dbu. Thus `width(0.14)`, `width(0.14.um)`, and
`width(140)` all mean 140 dbu.

## Supported constructs

| `.lydrc` construct | Compiles to | Notes |
|---|---|---|
| `name = input(layer)` / `input(layer, datatype)` | a layer binding | datatype defaults to 0 |
| `source(...)`, `report(...)` | ignored | KLayout I/O header, not a rule |
| `layer.width(v)` | `RuleKind::Width` | single layer |
| `layer.space(v)` | `RuleKind::Spacing` | single layer |
| `layer.notch(v)` | `RuleKind::Notch` | single layer (see divergence note) |
| `layer.separation(other, v)` / `layer.sep(other, v)` | `RuleKind::Spacing` | `other_layer = other` |
| `outer.enclosing(inner, v)` | `RuleKind::Enclosure` | receiver is the enclosing layer; the parser swaps it into `other_layer` and the argument into `layer` |
| `layer.with_area(0, v)` / `with_area(0.0, v)` / `with_area(nil, v)` | `RuleKind::Area` | below-threshold selection only |
| trailing `.output("name"[, "desc"])` | rule name | optional; names the reported rule |

The **enclosing swap** matters: KLayout writes `outer.enclosing(inner, v)` with the receiver
being the enclosing (outer) layer, while the engine's `Rule.layer` is the enclosed (inner)
shape and `Rule.other_layer` is the enclosing (outer) one. The parser exchanges the two so
the verdicts agree.

## Not supported

Everything else, which fails with an `UnsupportedConstruct` error naming the construct and
line:

- **Extension, density, and angle** rules. No `.lydrc` construct maps to these unambiguously
  within a bounding-box engine, so a deck using them is out of the supported subset.
- **Boolean layer algebra** (`&`, `|`, `-`, `^`), **sizing** (`sized`), **merging**, and the
  **universal `drc` expression**. The engine checks declarative rules, not derived layers.
- **Connectivity** (`connect`, `netter`, antenna). The engine is connectivity-free.
- **Two-sided `with_area(min, max)` bands** and the aggregate scalar `layer.area`. Only a
  minimum-area (below-threshold) selection maps to `RuleKind::Area`.

## Divergence note

The committed fixture (`crates/reticle-drc/tests/fixtures/subset.lydrc` over `subset.gds`) is
run both through `parse_lydrc` + `DrcEngine` and through KLayout headless, and their verdicts
are compared. The comparison is at the **layout-level verdict** granularity: for each rule,
did the tool report at least one violation, or none. The two engines agree on every
supported-subset rule of the fixture:

| Rule | Kind | Reticle count | KLayout count | Verdict agrees |
|---|---|---:|---:|:---:|
| `m1.1` | Width | 1 | 1 | yes |
| `m1.2` | Spacing | 1 | 1 | yes |
| `m1.4` | Enclosure | 1 | 4 | yes |
| `li.6` | Area | 1 | 1 | yes |
| `m2.1` | Width (clean) | 0 | 0 | yes |

Raw marker counts are **not** required to match, and `m1.4` shows why: KLayout emits one
edge-pair marker per offending edge (four, one per side of the under-enclosed cut), while the
engine emits one violation per offending shape. The fired/not-fired verdict is what both
tools agree on.

Two constructs are deliberately excluded from the verdict comparison:

- **`notch`.** KLayout's `notch` is an *intra-polygon* concavity check (a narrow gap within
  one polygon). The engine's `Notch` is an *inter-shape* same-layer gap measured on bounding
  boxes, and a bounding-box engine cannot see an intra-polygon notch at all. The parser
  accepts `notch` and maps it to `RuleKind::Notch`, but the two tools measure different
  things, so the fixture does not compare `notch` verdicts.
- **Non-rectilinear geometry.** The engine reduces every shape to its bounding box, exact for
  rectangles and conservative (never under-reporting) for polygons and paths. The fixture is
  rectangles only, so bounding-box area equals true area and the comparison is exact.

## Reproducing the comparison

The reticle side runs in the normal gate:

```
cargo nextest run -p reticle-drc --test lydrc_engine
```

The KLayout side needs Docker and the pinned container (the same image and invocation as
`just tt-precheck`). It mirrors the container run, parses KLayout's report database, and
asserts each supported-subset rule's fired verdict matches the reticle side:

```
powershell -File scripts/lydrc-compare.ps1
```

The exact underlying command, for a WSL or Linux fallback, is:

```
klayout -b -r crates/reticle-drc/tests/fixtures/subset.lydrc \
  -rd input=crates/reticle-drc/tests/fixtures/subset.gds \
  -rd report=out.lyrdb
```
