# 0034, GDSII import hardening: contain panics, degrade to structured warnings

## Context

The browser viewer wedge rests on one promise: a person opens a real IC layout
file and never sees a crash. The GDSII importer in `reticle-io` leans on the
`gds21` crate, which has real panic vectors on crafted input (a zero-length string
record indexes `data[len - 1]`, and an out-of-range date field in a BGNLIB or
BGNSTR record panics chrono deep inside the parser), and its frozen
`Importer::import` returns only `Result<Document>`, so a boundary with too few
vertices or an out-of-range magnification had nowhere to report itself: it was
either silently materialized as junk geometry or lost. We needed no panics, no
hangs, and no `unwrap`/`expect` on untrusted bytes, plus a way to keep a mostly-good
file open while telling the user what in it was dropped.

## Decision

Keep the frozen `Importer::import` trait method exactly as is (it now delegates and
discards warnings) and add an inherent `Gds::import_with_warnings(&self, bytes) ->
Result<GdsImport>` that carries a `Vec<ImportWarning>` alongside the document.
Guard the parse with three layers: a `MAX_INPUT_BYTES` (256 MiB) size check before
any allocation so a hostile length cannot force an out-of-memory abort; the
existing `std::panic::catch_unwind` (safe Rust, no `unsafe`) around
`GdsLibrary::from_bytes` to contain every `gds21` panic vector and return it as a
clean `IoError`; and post-parse validation that skips degenerate geometry (a
boundary ring under three vertices, a path under two points) or clamps out-of-range
values (a negative array count, an unrepresentable magnification), recording one
`ImportWarning` per problem rather than failing. `gds21` reads from an in-memory
cursor where every record consumes at least four bytes, so parsing a finite slice
provably terminates and allocates O(input) memory; that is why bounded parsing plus
the panic guard is sufficient and no timeout is needed. Warnings deduplicate by a
small stable `WarningKind` so a pathological file yields one representative warning
per category with a count, not a second memory hazard.

## Consequences

Every input now returns a well-formed document or a clean, human-readable error,
and a real production file with a stray bad element still opens with the bad element
skipped and noted. The corpus under `corpus/tinytapeout/` proves it: five malformed
samples return `Err`, two pathological-but-parseable ones (cyclic and dangling
structure references) import as valid documents that the model's own cycle guards
keep finite, and a degenerate-boundary sample imports with exactly one warning. The
cost is a second import entry point to keep in step with the first, and a
defense-in-depth vertex ceiling (`MAX_SHAPE_VERTICES`) that a conformant single XY
record can never reach, kept as a guard against re-encoded or crafted streams rather
than as a live limit.
