# 0004 — `gds21` for GDSII; in-house OASIS subset

## Context

The spec requires GDSII and OASIS import/export with round-trip fidelity. GDSII has
a mature Rust library (`gds21`, part of Layout21). OASIS (the newer, compressed
successor) has no mature Rust reader/writer, and the full specification is large
(strict/relaxed modes, CBLOCK compression, many record types, modal state).

## Decision

Use `gds21` for GDSII read/write in `reticle-io`. Implement an **in-house OASIS
subset** covering the records Reticle itself emits plus the common records seen in
practice (START/END, CELL, PLACEMENT, RECTANGLE, POLYGON, PATH, layer/datatype,
repetition, and the modal-state machinery), sufficient for lossless round-trips of
our own exports and for importing typical foundry-style files. Document the
supported record set and known limitations in the IO chapter and gate anything
unsupported behind a clear error rather than silent data loss.

## Consequences

GDSII is fully supported with little owned code. OASIS is genuinely supported for
the common path and round-trips our exports, but is not a complete implementation
of the standard; unsupported records produce explicit errors. The OASIS parser is
fuzzed like the GDSII one. If a complete OASIS implementation is ever needed it can
grow incrementally behind the same `Importer`/`Exporter` traits. This is the one
subsystem where hand-rolling is unavoidable because no suitable crate exists.
