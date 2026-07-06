# 0053, TinyTapeout tile template: a technology plus a built document, validated against the published frame

## Context

Reticle needs a "New TinyTapeout tile" action on the Start screen that opens a
correctly framed SKY130 GDS-mode (analog / custom-layout) tile: the tile boundary,
the analog pins, and the power straps placed where TinyTapeout requires, so a user
can fill in the interior and submit. The geometry is not ours to invent; it is fixed
by TinyTapeout's own template. Three questions had non-obvious answers: where the
authoritative geometry lives, how to encode a fixed "frame" in a model that has no
per-shape lock, and how to validate the result against a real submission without
committing a large third-party design.

## Decision

**Source of truth.** The tile boundary and the six `ua[0]`..`ua[5]` analog pins are
transcribed from TinyTapeout's canonical analog DEF template
`tech/sky130A/def/analog/tt_analog_1x2.def` in `TinyTapeout/tt-support-tools`
(`DIEAREA ( 0 0 ) ( 161000 225760 )`; each `ua[n]` a met4 PORT `( -450 -500 )
( 450 500 )` `PLACED ( x 500 )`). The power straps come from the same directory's
official init script `magic_init_project.tcl` (met4 vertical stripes, y 5 um to
220.76 um, 2 um wide, `VDPWR` at 1 um, `VGND` at 4 um, `VAPWR` at 7 um). The DEF
carries eight `ua` pins physically, but the analog spec caps usable analog pins at
six, used in order, so the template exposes `ua[0]`..`ua[5]`. Numbers were fetched
2026-07-06 for the TTSKY26c shuttle.

**Two-part bundle, both wasm-safe.** The technology half is a committed text file
`tech/tinytapeout-sky130.tech` (the SKY130 met4 drawing/pin/label purposes, a
`tt_boundary` marker on SKY130 areaid.sc `81/4`, and the met5 keep-out on record).
The geometry half is a pure-Rust builder `reticle_app::tinytapeout::tile_document()`
that constructs the `tt_um_reticle_tile` cell. The technology is embedded with
`include_str!` and the document is built in code, so both work on `wasm32` where the
Start screen runs and there is no filesystem, exactly as `crate::usecases` embeds
its SKY130 cell.

**No faked lock.** The Reticle model has no per-shape lock, and the met technology
format has no keep-out rule kind. Rather than fake either, the frame shapes are
documented (in the module and the technology file) as the fixed part the user must
not move, and the met5 prohibition is a documented rule enforced by tests, not a
model constraint.

**Start-screen entry.** A new `UseCase::NewTinyTapeoutTile` variant returns
`Scenario::LoadDocument` through the existing document-load path; the Start screen's
scenario loop renders it as a fifth card automatically. The `app.rs` change is
additive (doc-comment wording only); no routing was rewritten.

**Validation.** Coordinates extracted from TinyTapeout's own files are committed as
tiny text fixtures under `crates/reticle-app/tests/fixtures/tinytapeout/` (with a
`NOTICE.md` giving each source URL), and an integration test asserts the template's
die area and six pin rectangles match the canonical DEF exactly and its straps match
the init script exactly. A real published GDS-mode submission, `tt_um_analog_mux`,
is the external cross-check: the template shares its exact 225.76 um height, sits
within a 10 um width tolerance of it, and obeys the same met4-top / no-met5 rule its
`config.json` states. No whole DEF or GDS is committed; only the numbers the checks
need.

## Consequences

- "New TinyTapeout tile" opens a document whose frame is provably TinyTapeout's own,
  not an approximation. The internal-consistency tests (pins on met4, in order,
  inside the tile; straps span bottom-10 to top-10; nothing on met5) and the external
  tests against the published frame both gate the geometry.
- The bundle is a template, not a lock: nothing stops a user moving a frame shape,
  and Reticle's DRC subset is not TinyTapeout's precheck. Submission-readiness still
  depends on the precheck oracle (Lane 4B), as `docs/src/tapeout.md` states.
- The template tracks a specific shuttle's numbers. When TinyTapeout revises the
  template, the fixtures and the builder constants must be re-fetched together; the
  fixtures' source URLs make that a mechanical update, and the exact-match tests will
  flag any drift.
- The `ua` pitch (19_320 nm) and the older `VPWR` naming still present in the DEF's
  `SPECIALNETS` (versus the current `VDPWR`/`VGND`/`VAPWR` in the init script) are a
  known inconsistency in TinyTapeout's own files; the template follows the init
  script's current net names, which is the more recent source.
