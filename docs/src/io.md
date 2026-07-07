# File formats

`reticle-io` reads and writes the layout interchange formats and the technology
description.

## GDSII

GDSII is the long-standing binary interchange format for IC layout. Reticle reads
and writes it through `gds21`, the de facto Rust GDSII library (part of the
Layout21 project), mapping its structures, boundaries, paths, and references onto
Reticle cells, shapes, instances, and arrays. Round-trip fidelity is tested: a
document exported to GDSII and re-imported preserves its geometry, layers, and
hierarchy.

## The Reticle container format (OASIS-inspired, ADR 0004)

OASIS is the newer, compressed successor to GDSII, but it has no mature Rust
library. Reticle's `Oasis` type is therefore **not** a conformant OASIS reader or
writer: it is an in-house binary **container**, OASIS-*inspired* (it borrows the
spirit - a magic string, a `START`/`END` frame, and `CELL`/`RECTANGLE`/`POLYGON`/
`PATH`/`TEXT`/`PLACEMENT`/`ARRAY` records with explicit layer and datatype) but not
its wire format. **No third-party tool (KLayout, gdstk) can read it.** It exists to
round-trip Reticle's own geometry and hierarchy compactly and losslessly for the
supported record set; anything unsupported is a clear error rather than silent data
loss. See ADR 0004 for the container layout and honest gaps.

## Conformant OASIS writer (`oasis_std`)

Separately, `OasisStd` is a genuine **SEMI P39 OASIS writer** for a practical
subset - its output *is* read by KLayout as OASIS. It is export-only (a reader is
out of scope) and uncompressed (no `CBLOCK`), emitting `RECTANGLE`, `POLYGON`,
`PATH`, `PLACEMENT`, and `TEXT` with fully explicit modal state, `CELLNAME`+`CELL`
tables, and `PLACEMENT` records carrying magnification and angle. Documented subset
gaps: arrays are expanded to individual placements, a label's anchor is dropped
(OASIS `TEXT` is a point), and a path's round end cap is written flush. KLayout
reading `oasis_std` output is verified in-container by the interop harness; see the
[interop chapter](interop.md) and ADR 0070.

## Technology files

A technology file describes the process: the database resolution, the layer table
(numbers, datatypes, names, and display colors), and the design rules. Reticle
parses a simple, readable text format into the model's `Technology`, which then
drives layer display and the [design-rule checker](drc.md).

## Robustness

The parsers are fuzzed. A parser must never panic or hang on malformed input; it
either produces a document or returns an error.
