# LEF/DEF import

The `reticle-lefdef` crate imports the two text formats an OpenROAD/OpenLane run
emits, LEF (the technology and the macro cell abstracts) and DEF (the placed,
routed design), and lowers them into a `reticle-model` `Document` plus the run-level
metadata a viewer overlays. The single public result is `LefDefDesign`, the contract
the run viewer consumes.

It is a new crate, not an extension of `reticle-io`: `reticle-io` is hardened and
frozen-adjacent, and LEF/DEF are a different concern (a design-and-technology
interchange, not a layout binary). The crate has no external dependencies and no
native-only code, so it builds for `wasm32-unknown-unknown` alongside the rest.

## Entry points

```rust
pub fn import_lef_def(lef: &[u8], def: &[u8]) -> Result<LefDefDesign, LefDefError>;
pub fn import_run_dir(dir: &Path) -> Result<LefDefDesign, LefDefError>;
```

`import_lef_def` takes a LEF byte slice (or several LEF files concatenated, which is
valid) and a DEF byte slice. `import_run_dir` walks a flow output directory (bounded
in depth and file count), concatenates the `*.lef` files it finds, and imports the
`*.def` whose name sorts last, which selects the later flow stage (for example
`6_final.def` over `2_floorplan.def`).

## The `LefDefDesign` contract

`LefDefDesign` keeps the lowered layout separate from the run metadata a viewer
overlays:

| field | source | meaning |
|-------|--------|---------|
| `document` | LEF macros + DEF placement/routing | the lowered `Document`: a cell per macro, a top cell of placed instances and routed shapes, and the layer table |
| `design_name` | DEF `DESIGN` | the top cell name |
| `die_area` | DEF `DIEAREA` | the die outline (a rectilinear outline is reduced to its bounding box) |
| `sites` | LEF `SITE` | placement site definitions |
| `rows` | DEF `ROW` | placement rows |
| `nets` | DEF `NETS` | the routed net list, each with its wire and via segments |
| `pins` | DEF `PINS` | external I/O pins |
| `overlays` | reports (viewer) | congestion, utilization, and timing-critical-net slots, empty after a LEF/DEF import |
| `warnings` | import | non-fatal problems (skipped keywords, dropped degenerate shapes, unresolved references) |

The layout lives in `document` because the renderer already draws cells, instances,
and shapes. The rest is run-level metadata a viewer treats differently: rows and the
die area are chrome, nets are selectable by name, and the `overlays` slots are filled
in later from a run's report files, which is a separate concern from the layout
import.

## Supported subset

This is a deliberate subset, chosen to render an OpenROAD run, not a full LEF/DEF
implementation. ADR 0082 records the scope and the reasons.

**LEF**

- `UNITS DATABASE MICRONS` sets the database resolution.
- `LAYER <name>` with `TYPE` (`ROUTING`, `CUT`, or other) and `WIDTH`. Each layer
  name is interned to a `LayerId` in declaration order and given a palette color;
  routing widths become the default wire width for that layer.
- `SITE <name>` with `CLASS` and `SIZE`.
- `MACRO <name>` with `CLASS`, `SIZE`, `PIN` (with `DIRECTION` and `PORT`/`LAYER`/
  `RECT` geometry), and `OBS`. Each macro becomes a `Cell`; each pin becomes a model
  `Pin` and its port rectangles are drawn on their layers; obstructions are drawn.

Via geometry, spacing and antenna tables, and property definitions are skipped with
a warning. Only `RECT` geometry is lowered from ports and obstructions; a `POLYGON`
is skipped with a warning.

**DEF**

- `DESIGN`, `UNITS DISTANCE MICRONS`, `DIEAREA`.
- `ROW`, including the `DO`/`BY`/`STEP` repeat.
- `COMPONENTS`: each `PLACED` or `FIXED` component becomes an `Instance` at its
  location and orientation. A component whose macro the LEF never defined is skipped
  with a warning.
- `PINS`: `NET`, `DIRECTION`, a `LAYER` rectangle, and `PLACED` location.
- `NETS`: `ROUTED` (and `FIXED`) wires and vias, including `NEW` layer breaks and the
  `*` repeated-coordinate shorthand. Each wire is drawn into the top cell as a path
  and recorded in the net list.

`SPECIALNETS`, `GROUPS`, `REGIONS`, and `BLOCKAGES` are skipped with a warning.
Coordinates are read as they appear: DEF coordinates are already DBU, LEF microns are
converted to DBU on the shared resolution.

## Orientation mapping

DEF names the eight placements `N`, `S`, `E`, `W` and their flipped forms `FN`, `FS`,
`FE`, `FW`. The DEF flip is a mirror about the Y axis applied before the rotation;
Reticle's `Orientation` models a reflect-about-X-then-rotate (the GDSII convention).
The exact correspondence, verified in the crate's tests by comparing the point
transforms directly, is:

| DEF | Reticle | DEF | Reticle |
|-----|---------|-----|---------|
| `N` | `R0`   | `FN` | `MirrorX180` |
| `W` | `R90`  | `FW` | `MirrorX270` |
| `S` | `R180` | `FS` | `MirrorX`    |
| `E` | `R270` | `FE` | `MirrorX90`  |

## Robustness

LEF and DEF are untrusted input, so import never panics or hangs on any byte
sequence. Inputs over 256 MiB are refused before parsing, so a hostile length cannot
force a large allocation (the OASIS out-of-memory lesson). Bytes are decoded lossily,
so invalid UTF-8 never panics. The tokenizer and parsers advance by at least one
token per step over a finite stream, so no parse loops forever, and no collection is
ever pre-sized from a count read out of the input. A statement that cannot be parsed
is a clean `LefDefError` naming its line; a recoverable problem is a `LefDefWarning`
and the rest of the design still imports.
