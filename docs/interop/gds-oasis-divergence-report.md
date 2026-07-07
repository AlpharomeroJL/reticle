# GDS / OASIS interop divergence report

This report records how Reticle's GDS round-trip compares against two independent
external tools, and whether KLayout can read Reticle's conformant-OASIS writer
(`oasis_std`). It is produced by the harness in `scripts/interop/`, which runs headless
inside the pinned container and is re-runnable with `scripts/interop/run-interop.ps1`.

## Environment

- **Container:** `hpretl/iic-osic-tools:2025.01` (the pinned image already on disk).
- **KLayout:** 0.29.10 (via `klayout.db`, the authoritative geometric reader).
- **gdspy:** 1.6.13 (the fixture source and the second independent tool).
- **gdstk:** the packet named gdstk, but it is **not preinstalled** in this image and
  PEP 668 blocks a clean in-container `pip install`. We use **gdspy** (its same-author
  predecessor, preinstalled) as the second tool, so the harness is reproducible with no
  network. To add gdstk instead: `pip install --break-system-packages gdstk` in the
  container, then extend `interop.py`'s `roundtrip` with a `gdstk` arm.

## Method

1. gdspy writes two fixtures (`clean.gds`, `odd.gds`), 1 dbu = 1 nm.
2. Each tool reads a fixture and re-exports it (a round-trip): Reticle via the
   `reticle-roundtrip` binary on the host; KLayout and gdspy in the container.
3. Every output GDS is normalized by a **single** reader (KLayout `klayout.db`): each
   shape is rendered to an integer-dbu polygon (so path-vs-box representation
   differences are neutralized and only real geometric divergence shows), labels are
   compared by `(layer, datatype, position, text)`, and **instance transforms are read
   with gdspy** — it exposes the raw GDS `STRANS` rotation/magnification/reflection
   directly, whereas KLayout's composed `ICplxTrans` angle can differ from the stored
   field for magnified references. Because one reader normalizes every writer's output,
   a divergence is attributable to the tool that **wrote** the file.
4. Reticle exports each fixture as conformant OASIS (`oasis_std`); KLayout reads it.

## Fixtures

- **`clean.gds`** — well-formed and rectilinear-plus-45°: a rectangle, a Manhattan
  polygon, a 45° diamond polygon, a flush-ended path, a text label, a cell reference,
  and a 2×2 array of a sub-cell.
- **`odd.gds`** — seeded quirks that tend to surface writer/reader divergences: a 1 nm
  sliver rectangle, a round-ended path, a custom-extension path, a polygon with a
  duplicate consecutive vertex, a **reference rotated 45° with magnification 2**, and a
  negative-coordinate rectangle.

## Results

### `clean.gds` — no divergence

All three tools round-trip the clean design identically. Element census (box / polygon
/ path / text / sref / aref) is `2 / 3 / 0 / 1 / 1 / 1` for reticle, klayout, and gdspy
alike; rendered geometry, labels, and instances all match. **OASIS read test: PASS** —
KLayout read Reticle's `oasis_std` output (2 cells, 6 shapes, dbu = 0.001).

### `odd.gds` — one documented divergence

Element census matches across all three tools (`4 / 2 / 0 / 0 / 1 / 0`); the sliver
rectangle, the round and custom-extension paths, the degenerate-vertex polygon, and the
negative-coordinate rectangle all round-trip cleanly and geometrically identically.

The seeded divergence is in **instance transform handling**:

| tool | recovered rotation of the `SUB` reference | magnification |
|------|--------------------------------------------|---------------|
| source (gdspy) | 45° | 2× |
| gdspy round-trip | 45° | 2× |
| KLayout round-trip | 45° | 2× |
| **Reticle round-trip** | **90°** | 2× |

**Cause (honest):** Reticle's placement model, `reticle_geometry::Orientation`, encodes
only the eight orthogonal orientations (R0/R90/R180/R270 and their mirrors). A GDS
`STRANS` with a non-orthogonal angle (45°) has no exact representation, so Reticle's
GDS importer snaps it to the nearest orthogonal orientation (90°) on the way in, and
re-exports 90°. KLayout and gdspy carry the arbitrary angle through unchanged. The
magnification (2×) and origin round-trip correctly in all three tools. This is a
**modelling limitation, not a reader/writer bug**: Reticle is a Manhattan-plus-45°-fill
layout tool whose instance transforms are orthogonal by design; arbitrary-angle
instance rotation is out of its scope and is snapped rather than silently dropped.

## Conformant-OASIS writer (`oasis_std`) — validated

The timeboxed conformant-OASIS writer **passed** its acceptance test on the first real
attempt: KLayout reads Reticle's `oasis_std` output for both fixtures as OASIS
(`OASIS-READ OK`, dbu = 0.001, correct cell and shape counts). The writer emits a
practical SEMI P39 subset — uncompressed (no CBLOCK), `RECTANGLE`/`POLYGON`/`PATH`/
`PLACEMENT`/`TEXT` with fully explicit modal state, `CELLNAME`+`CELL` tables, and
`PLACEMENT` type 18 carrying magnification and angle. Documented subset gaps: arrays are
expanded to individual placements (no OASIS repetition), a label's anchor is dropped
(OASIS `TEXT` is a point), and a path's *round* end cap is written flush (OASIS path
extensions are flush / half-width / explicit only). See `crates/reticle-io/src/oasis_std.rs`.

This is distinct from the in-house `oasis.rs`, which is the **Reticle container format
(OASIS-inspired, ADR 0004)** — a proprietary binary container KLayout cannot read.

## Reproducing

```powershell
powershell -File scripts/interop/run-interop.ps1
```

Requires Docker and the pinned image. The machine-generated comparison is written next
to this file as `gds-roundtrip.generated.md` (with a `.json` of the raw normalized
views). If Docker is unavailable the runner prints the exact skipped command and exits
3, so the KLayout/gdspy comparison can be recorded as not-run while the writer, the
second PDK, and the GenTech refactor still ship.
