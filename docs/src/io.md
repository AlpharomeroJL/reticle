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

## OASIS

OASIS is the newer, compressed successor to GDSII. It has no mature Rust library,
so Reticle implements a focused in-house subset covering the records it emits plus
the common records seen in practice, sufficient to round-trip its own exports and
import typical files. The supported record set and its limits are documented, and
anything unsupported is a clear error rather than silent data loss. See ADR 0004.

## Technology files

A technology file describes the process: the database resolution, the layer table
(numbers, datatypes, names, and display colors), and the design rules. Reticle
parses a simple, readable text format into the model's `Technology`, which then
drives layer display and the [design-rule checker](drc.md).

## Robustness

The parsers are fuzzed. A parser must never panic or hang on malformed input; it
either produces a document or returns an error.
