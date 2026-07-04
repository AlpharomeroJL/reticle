# 0035, The document-open seam: bytes plus a format hint, warnings alongside

## Context

Several parts of the app need to load a layout file into the editor: a Start
screen's "open a file" button, an example gallery, a drag-and-drop handler, and the
browser build where a file arrives as an `ArrayBuffer` with no filesystem behind it.
Each of these is owned by a different lane, and without a shared contract each would
reach into `reticle-io` and the editor's document model on its own, duplicating the
import wiring and the "replace the live document and reframe" dance, and each would
have to invent its own way to surface the non-fatal warnings the hardened importer
now produces. The editor already had a private `install_document` that rebuilds all
derived state, but nothing public that took untrusted bytes, imported them safely,
and reported what happened.

## Decision

Add one public, platform-neutral seam in `reticle-app` (`crate::open`) that takes
bytes and a `DocFormat` hint, never a path, so the same call works on native and
wasm: `open_document_bytes(bytes: &[u8], format: DocFormat) -> Result<OpenOutcome,
OpenError>`. It imports through the hardened `reticle-io` path (GDSII via
`import_with_warnings`, OASIS via its bounded reader), returns an `OpenOutcome`
carrying the opened `Document`, the top cell to frame, and a `Vec<OpenWarning>` of
structured non-fatal problems, or a clean `OpenError` (`Import` for bad bytes,
`Empty` for a well-formed but cell-less document). The `App` wraps it with
`open_document_bytes`/`open_outcome`, which install the document, frame its top,
dismiss the Start screen, and stash the warnings for a minimal, non-panicking
warnings window. The seam is kept free of browser specifics and any Start-screen or
rich-error UI on purpose: those belong to the lane that owns the Start experience,
which routes its file-open and example-gallery flows through this seam and reads
`App::open_warnings` for its own comprehensive surface.

## Consequences

Every file-open path in the app now shares one hardened, unit-tested entry point, so
"open a real file and never crash" is proven once in plain code (the seam's tests
open valid GDSII and OASIS, reject non-GDS bytes, and treat empty input as a clean
error) rather than re-proven per caller. The `OpenOutcome`/`OpenError`/`OpenWarning`
types are the frozen contract other lanes build against; `OpenError` is
`#[non_exhaustive]` so finer categories can be added later without breaking them. The
cost is that the seam owns a small amount of policy (which cell to frame when a
document declares no top, mapping importer warnings onto `OpenWarning`) that those
callers must not duplicate, and the OASIS path carries no warnings until that
importer grows a warning channel of its own.
