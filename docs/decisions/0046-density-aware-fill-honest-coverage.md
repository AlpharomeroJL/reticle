# 0046, Density-aware fill: approach a target, report the honest coverage

## Context

Fill is the layout task of covering empty area on a layer with a regular pattern so the
foundry's density windows are satisfied, while leaving keep-outs (over sensitive
devices, seal rings, alignment marks) clear. It fits the generator framework (ADR 0042)
cleanly: a region, a fill layer, a target density, and a list of keep-out rectangles in,
DRC-clean geometry out.

Two things about the committed SKY130 subset (`tech/sky130-drc-subset.toml`) shape the
honest scope. First, the subset is a min-width, min-spacing, min-area, and enclosure
deck: it carries **no maximum-density rule** for any layer. So "fill up to a density" is
a coverage *objective* here, not a rule the fill must respect, and there is nothing to
over-fill against. Second, the DRC engine reduces every shape to its bounding box and
never flags two shapes that touch or overlap (gap zero); a spacing violation is a
strictly positive gap below the minimum. That means a grid of separate square tiles is
clean exactly when each tile clears the layer width and area on its own and the pitch
keeps neighbours at least the minimum spacing apart.

The hard question is honesty about the density number. A whole-DBU pitch does not divide
an arbitrary region evenly, so the count of whole tiles that fit is not a smooth function
of the target, and the achieved coverage can land a little above or below the requested
value. Claiming the generator "hits" a density would be a lie the first time a region is
not an exact multiple of the pitch.

## Decision

Ship a `fill` generator (`FillGen`, id `"fill"`) that tiles a `region_width` by
`region_height` rectangle with `tile`-sized squares on a chosen interconnect layer
(`li1`, `met1`, `met2`, `met3`), skipping any tile that touches a keep-out, and
**approaches** a `target_density_permille` rather than pretending to hit it.

Cleanliness is by construction, proven by the real engine. `tile` is validated to be at
least the layer minimum width and, where the layer has an area rule, at least the side
whose square meets that area (`ceil(sqrt(min_area))`), so every tile clears width and
area on its own bounding box. The pitch is `round(tile / sqrt(target))` clamped up to at
least `tile + min_spacing`, so adjacent tiles are never closer than the layer minimum
spacing; that clamp also caps the achievable density at `tile^2 / (tile + min_spacing)^2`,
and asking for more yields that ceiling. Keep-outs are honored by dropping any tile that
overlaps or merely touches one, and only whole tiles are placed.

Honesty about the number is a first-class method, not a comment. `GenOutput` cannot carry
a density (its shape is frozen by ADR 0042), so `FillGen::achieved_density_permille`
computes the true achieved coverage (placed tile area over region area, in per-mille) from
the same grid that is drawn. The module docs state plainly that the value approaches the
target and can be slightly above or below it, and that keep-outs and edge clipping only
reduce it. The keep-out list is a `Vec<KeepOut>` on the parameter struct; the schema's
field types are scalar (int, bool, enum) with no array widget, so the keep-out list is
deliberately absent from the schema fields (which describe the form's scalar inputs) and
travels on the JSON parameter path instead, defaulting to empty.

## Consequences

- The 400-case cleanliness proptest (`fill_is_drc_clean`) sweeps random layers, regions,
  tiles, densities, and keep-out sets, runs `DrcEngine::new(sky130_drc_rules())`, and
  asserts zero violations, and in the same test asserts no tile ever overlaps or touches a
  keep-out. A separate 400-case property (`fill_density_is_honest`) asserts the achieved
  density tracks the target within a two-sided tolerance band over a large unobstructed
  region, and that keep-outs only reduce it. The claim "approaches the target" is a proven
  property, not marketing.
- The honest ceiling is load-bearing. A very high target does not silently pack tiles
  sub-spacing; the pitch clamp holds the grid at the min-spacing ceiling and the reported
  density reflects that, so a caller asking for the impossible is told what it actually
  got.
- Because the subset has no density rule, this generator cannot claim to make a layout
  pass a real fill deck; it makes a clean, honest, keep-out-respecting tile grid on the
  subset. Targeting a foundry density window with a checked max-density rule is future
  work, out of scope for this lane, and the docs say so.
- Keeping the keep-out list off the schema is a small honesty cost: a generated form
  built from the schema exposes the scalar knobs but not the keep-out list, which a caller
  supplies through JSON. This avoided extending the frozen schema API (ADR 0042) with an
  array field type for one generator; if more generators need list parameters, adding an
  array `FieldType` is the right follow-up.
