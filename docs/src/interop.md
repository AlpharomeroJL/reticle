# GDS / OASIS interop

Reticle's interchange formats are only as good as what other tools make of them.
This chapter records how Reticle's GDS round-trip compares against two independent
external tools, and whether KLayout can read Reticle's conformant-OASIS writer.

## The harness

`scripts/interop/` drives a comparison headless inside the pinned
`hpretl/iic-osic-tools` container (KLayout 0.29.x + gdspy 1.6), re-runnable with
`scripts/interop/run-interop.ps1`. It:

1. generates two fixtures with gdspy (1 dbu = 1 nm);
2. round-trips each fixture (read then re-export) through **Reticle** (a standalone
   `reticle-roundtrip` driver on the host), **KLayout**, and **gdspy**;
3. normalizes every output with a single authoritative reader - KLayout renders each
   shape to an integer-dbu polygon (so path-vs-box representation differences are
   neutralized), and instance transforms are read with gdspy, which exposes the raw
   GDS `STRANS` rotation/magnification directly; and
4. exports each fixture as conformant OASIS (`oasis_std`) and has KLayout read it.

Because one reader normalizes every writer's output, a divergence is attributable to
the tool that *wrote* the file. The committed report and its machine-generated
companion live under `docs/interop/` in the repository.

> **Tooling note.** The packet named gdstk; it is not preinstalled in the image and
> PEP 668 blocks a clean in-container `pip install`, so the harness uses gdspy (its
> same-author predecessor, preinstalled) as the second tool and stays reproducible
> with no network. If Docker is unavailable the runner prints the exact skipped
> command and exits 3, so the comparison can be recorded as not-run while the writer,
> the second PDK, and the GenTech refactor still ship.

## What the round-trip preserves

On the **clean** fixture (a rectangle, a Manhattan polygon, a 45° polygon, a path, a
label, a cell reference, and a 2×2 array), Reticle, KLayout, and gdspy round-trip the
design **identically** - rendered geometry, labels, and instances all match, and
KLayout reads Reticle's OASIS output.

## The documented divergence

On the **odd** fixture (seeded with a 1 nm sliver, round and custom-extension paths, a
degenerate-vertex polygon, and a **reference rotated 45° with magnification 2×**), the
geometry all round-trips cleanly, but one divergence surfaces in instance transforms:

| tool | recovered rotation | magnification |
|------|--------------------|---------------|
| source / gdspy / KLayout | 45° | 2× |
| **Reticle** | **90°** | 2× |

Reticle's placement model, `Orientation`, encodes only the eight orthogonal
orientations (R0/R90/R180/R270 and mirrors). A non-orthogonal `STRANS` angle has no
exact representation, so Reticle's GDS importer snaps 45° to the nearest orthogonal
orientation (90°) rather than dropping it silently; the magnification and origin
survive. This is a **modelling limitation, not a reader/writer bug** - Reticle is a
Manhattan-plus-45°-fill layout tool whose instance transforms are orthogonal by
design. It is called out here and in the committed report rather than hidden.

See the full report at `docs/interop/gds-oasis-divergence-report.md`.
