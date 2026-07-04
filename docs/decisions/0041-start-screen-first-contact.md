# 0041, Product-grade first contact: gallery, drag-drop, and a tour that covers open

## Context

The v6 Start screen offered the four worked scenarios and nothing else, so a
first-time visitor had no obvious way to open their own design or to try a real chip,
and the browser build, which is the whole no-install story, had no filesystem to open
from at all. The first-run tour walked the editor's panels but started at the canvas
and never mentioned how a design gets in or how a session is shared, the two things a
newcomer most needs to know. These gaps sat exactly at first contact, where a missing
affordance reads as a missing capability.

## Decision

Extend the existing Start screen and tour rather than replace them. The Start screen
now shows, in one scroll: an "open a file" section with a drag-and-drop target hint
and the supported formats; a recent-files section rendered from a minimal
`recent_files` list on the `App` (a persistence backend, IndexedDB on the web, feeds
it through `set_recent_files`; this lane only displays it); an example-chip gallery of
redistribution-cleared designs; and the four worked scenarios as before. The gallery
designs are compiled in with `include_bytes!` (the minimized Apache-2.0 Tiny Tapeout
sample and the bundled SKY130 inverter cell) and opened through the existing
document-open seam, so the gallery works on wasm where there is no filesystem and has
no privileged load path. Drag-and-drop is handled for both platforms from egui's
dropped-files (bytes on wasm, a path read on native), classified by extension, with
every failure routed to the notification surface (ADR 0040). A toolbar "Open" control
returns to the Start screen so the open affordance is reachable from the editor and is
the target the tour points at. The tour gains two core steps: a first "open" step on
the open affordance and a "share" step on the Share section, so the walkthrough now
opens with how a design gets in and closes the core chapter with how a session gets
out.

## Consequences

A visitor to the public web build can open a real chip in one click and drop their own
GDSII or OASIS file onto the window, and the tour teaches both open and share without
either lane duplicating the other's UI. The gallery and drag-drop share the same
hardened seam and the same error surface as every other open, so "load a real design
and never crash" stays proven in one place; the Start-screen model (the example chips
and the recent-file shape) is unit-tested for enumerability, clean opening through the
seam, and the exact top cells framed, and the new tour steps are asserted to lead with
open and cover share. The compiled-in designs add a few kilobytes to every binary
(native and wasm), which is the deliberate price of a filesystem-free gallery. The
`recent_files` list is display-only here; its persistence is a sibling concern that
fills the same shape at merge, so on a fresh session the section shows a placeholder
rather than real history.
