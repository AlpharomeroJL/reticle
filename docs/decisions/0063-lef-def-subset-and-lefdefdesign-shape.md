# 0063, LEF/DEF import: the supported subset and the LefDefDesign shape

## Context

The v8 run viewer (lane 5B) needs to render an OpenROAD/OpenLane run: the die, the
rows, the placed cells, and the routed nets, with slots for report-derived overlays.
That means importing LEF (technology plus macro cell abstracts) and DEF (placement
plus routing) into the `reticle-model` `Document`. LEF/DEF are large, old text
formats with many features no viewer needs; a full implementation would be a large
surface with little payoff. This work is a new crate, `reticle-lefdef`, because
`reticle-io` is hardened and frozen-adjacent (ADR 0034) and LEF/DEF are a different
concern from the GDSII/OASIS layout binaries it owns. Its output type, `LefDefDesign`,
freezes at the 5A merge and gates 5B (ADR 0061), so both the parsed subset and the
type shape are decisions worth recording.

## Decision

**Subset.** Import the smallest LEF/DEF that renders a run. LEF: `UNITS`, `LAYER`
(name, `TYPE`, `WIDTH`), `SITE` (`CLASS`, `SIZE`), and `MACRO` (`CLASS`, `SIZE`,
`PIN` with `PORT`/`LAYER`/`RECT`, `OBS`). DEF: `DESIGN`, `UNITS`, `DIEAREA`, `ROW`,
`COMPONENTS` (`PLACED`/`FIXED` location and orientation), `PINS` (`NET`, `DIRECTION`,
a `LAYER` rectangle, `PLACED`), and `NETS` (`ROUTED`/`FIXED` wires and vias, with
`NEW` breaks and the `*` repeated coordinate). Everything else (via geometry, spacing
and antenna tables, `SPECIALNETS`, `GROUPS`, `REGIONS`, `BLOCKAGES`, non-`RECT` port
geometry) is skipped with a warning, never an error. LEF layer names are interned to
`LayerId`s in declaration order; LEF microns and DEF DBU are placed on one shared
resolution (DEF `UNITS` when present, else LEF, else 1000 DBU/micron) so cells and
routing line up. DEF orientations map to `reticle-geometry`'s reflect-then-rotate
`Orientation` by an exact table, verified pointwise in a test.

**Shape.** `LefDefDesign` is flat and owned: the lowered `Document` in one field, and
run-level metadata beside it (`design_name`, `die_area`, `sites`, `rows`, `nets` with
per-net wire and via segments, `pins`, and a `warnings` list). Report-derived
overlays (`utilization`, `congestion`, `timing_critical_nets`) are owned fields on a
`ReportOverlays` companion, present but empty after a LEF/DEF import: the report
parsing that fills them is a 5B concern, and giving 5B a named, owned slot now is what
lets the type freeze here. The layout stays in `document` because the renderer already
draws it; the metadata stays out because a viewer treats it differently (rows and die
are chrome, nets are selectable, overlays come from reports).

## Consequences

Lane 5B builds against a frozen `LefDefDesign`: it renders `document`, overlays the
die/rows/nets/pins, and fills the `ReportOverlays` slots from the run's report files
without changing the type. The subset is honest and enforced by the warning path, so a
real foundry LEF/DEF opens (with a list of what was skipped) rather than failing on
the first unmodeled keyword. Growing the subset later (via geometry, special nets,
polygon ports) is additive: new fields or new `NetSegment`/warning variants, not a
reshape. The crate carries no external dependency and no native-only code, so it holds
the wasm-clean line 5B relies on. The end-to-end "a real OpenROAD run renders" check
depends on fetching a platform LEF and a design DEF; where that fetch is unavailable
the parser and its synthetic-fixture tests still ship, and the render is recorded as
not-run with the exact fetch command (see the crate `NOTICE.md`).
