# 0070, A conformant-OASIS writer subset (oasis_std), and renaming the in-house format honestly

## Context

Reticle's `Oasis` type (ADR 0004) has always been an in-house, OASIS-*inspired* binary
container, not conformant OASIS - no third-party tool can read it. Some docs drifted
into implying interoperability ("sufficient to import typical files"), which is not
true. Separately, the v8 roadmap asked for one honest, timeboxed attempt at a genuine
conformant-OASIS **writer** that KLayout can actually read, to see how far a practical
subset gets.

## Decision

Two things.

**Rename for honesty.** Across the docs, the in-house format is named "the Reticle
container format (OASIS-inspired, ADR 0004)" and stated plainly to be unreadable by
KLayout/gdstk. The `Oasis` type and `oasis.rs` are unchanged in behavior - only the
naming and claims are corrected - so no round-trip or format guarantee moves.

**A new writer.** Add `crates/reticle-io/src/oasis_std.rs` (`OasisStd`), a genuine
SEMI P39 OASIS **writer** for a practical subset, alongside the internal container (the
one `reticle-io/lib.rs` edit this batch). It is **export-only** - a reader is out of
scope - and uncompressed (no `CBLOCK`). It emits `RECTANGLE`, `POLYGON`, `PATH`,
`PLACEMENT`, and `TEXT` with **fully explicit modal state** (every element carries its
own layer, datatype, coordinates, and dimensions/point-list/extension), which is the
single most common OASIS-conformance pitfall dispatched by construction. Cells use
`CELLNAME`+`CELL` tables; placements use `PLACEMENT` type 18 carrying magnification and
angle so any Reticle transform round-trips; the `END` record is padded to exactly 256
bytes. Since the input is Reticle's trusted internal model and there is no parser, the
UNTRUSTED-INPUT rules have no new surface here.

Documented subset gaps, stated rather than hidden: arrays are **expanded** to individual
placements (no OASIS repetition), a label's anchor is dropped (OASIS `TEXT` is a point),
and a path's *round* end cap is written flush (OASIS path extensions are flush /
half-width / explicit only).

## Consequences

The timebox **succeeded**: KLayout reads `OasisStd` output as OASIS, verified
in-container by the interop harness (both fixtures, correct cells/shapes/dbu). The
writer is a real, if minimal, interop path out of Reticle that no in-house format could
be. Its subset gaps mean it is not a general OASIS exporter - large arrays inflate, and
round/anchored features degrade - acceptable for a first honest writer and documented at
the call site and in the interop report. The rename removes an overstatement that could
have misled a user into expecting KLayout to open a `.oas` written by the internal
container.
