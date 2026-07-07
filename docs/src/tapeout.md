# TinyTapeout submission

Reticle exists to open, inspect, share, and generate real IC layout. TinyTapeout is
the cheapest real path from a GDS file to a physical chip, so it is the natural proof
that Reticle produces layout a fabricator will actually accept. This chapter is the
honest plan for a **GDS-mode** (custom-layout) TinyTapeout submission built with
Reticle, grounded in TinyTapeout's live specifications, plus a clear statement of what
Reticle has today and what remains to build.

All TinyTapeout facts below were fetched from `tinytapeout.com` on 2026-07-06. Their
specs move between shuttles, so treat their site and repositories as the source of
truth over this page.

## Status: what is built, what is planned

- **Built and shipped** (earlier waves): the SKY130 technology grounding, GDSII and
  OASIS import/export, the DRC subset over the cited SKY130 rules, and the
  parameterized generators (guard ring, via farm, pad ring, seal ring, density fill,
  probe-able test structures) that are DRC-clean by construction against that subset.
  Those are what a tile's content would be made of.
- **Built in this wave**: a Reticle technology-plus-template bundle that frames a
  correctly-shaped TinyTapeout tile (Lane 4A). "New TinyTapeout tile" on the Start
  screen now opens the `tt_um_reticle_tile` template: the 1x2 die outline, the six
  `ua[0]`..`ua[5]` analog pins on met4, and the VDPWR/VGND/VAPWR power straps, all at
  coordinates transcribed from TinyTapeout's own analog DEF template and init script
  (see below).
- **Planned, not yet built** (the rest of this wave): a wrapper that runs
  TinyTapeout's own precheck as an external oracle, `just tt-precheck <gds>` (Lane 4B);
  and the worked in-repo example, an agent-generated test-structure tile in the TT
  template that passes `just tt-precheck` clean, committed with its transcript (the
  packet's proof artifact). Those are the remaining work.

No shuttle purchase is in scope for this project. A paid submission is the operator's
own decision, which the tooling above is meant to make straightforward at any time.

## What a GDS-mode submission is, and is not

TinyTapeout accepts two kinds of design. The common one is the **digital** flow: you
write HDL (Verilog), and TinyTapeout's hardened flow (OpenLane and friends) places and
routes it into a tile for you. GDS-mode, also called the **analog** or custom-layout
path, is different: **you** provide the finished GDS layout for the tile, and
TinyTapeout only checks it and drops it into the shuttle. That is the path Reticle
serves, because Reticle is a layout tool, not an HDL synthesis flow. A GDS-mode tile
is therefore fully your geometry: standard cells, generators, hand layout, or a mix,
as long as it satisfies the template and passes the checks below.

## TTSKY26c: the current open SKY130 shuttle

The current open SKY130 shuttle at the time of writing is **TTSKY26c**: it launched
2026-05-26, submissions **close 2026-09-07**, and estimated delivery is 2027-03-27 to
2027-05-12. Designs can be revised up to the closing date; nothing new is accepted
after it. (The prior shuttle, TTSKY26b, closed 2026-05-18 with delivery in late 2026.)

## Cost

TinyTapeout prices per tile through its live calculator rather than a fixed figure, so
the honest answer is to read the current number from
`https://app.tinytapeout.com/calculator`. A submission gets you the design on the
shuttle plus a devkit: two boards (a demo board and a breakout board) and one physical
chip; extra chips mean extra devkit boards.

## The tile template a GDS must satisfy

For a SKY130 GDS-mode tile (numbers approximate, per TinyTapeout's analog spec; 3.3 V
designs are slightly narrower):

- **Footprint:** one of two standard sizes, a **1x2 tile at about 160x225 um** or a
  **2x2 tile at about 334x225 um**. Larger designs are billed as multiple tiles.
- **Pins:** up to six analog pins `ua[0]` through `ua[5]`, used in order from 0, placed
  on **metal 4** at locations matching a TinyTapeout DEF template. Each pin's path must
  stay under 500 ohm, under 5 pF, and 4 mA maximum.
- **Power:** `VGND`, `VDPWR` (1.8 V digital), and optional `VAPWR` (3.3 V analog),
  brought in as **vertical met4 stripes** at least 1.2 um wide, running from within the
  bottom 10 um of the tile to within the top 10 um.
- **Forbidden:** **metal 5 is off limits**, TinyTapeout uses it for the power grid. No
  floating digital output pins.
- **Naming:** the top macro name must start with `tt_um_` and be unique on the shuttle.

## The template bundle (Lane 4A)

"New TinyTapeout tile" on the Start screen loads a document whose frame is
transcribed from TinyTapeout's own files, not this summary. The bundle has two
wasm-safe halves:

- a technology file, `tech/tinytapeout-sky130.tech`, that names met4 and its
  pin/label purposes, adds a `tt_boundary` marker (SKY130 areaid.sc, `81/4`) for the
  tile outline, and puts the met5 prohibition on record; and
- a pure-Rust builder, `reticle_app::tinytapeout::tile_document()`, that constructs
  the `tt_um_reticle_tile` cell: the 1x2 die outline
  (`( 0 0 )`..`( 161000 225760 )` DBU), the six `ua[0]`..`ua[5]` analog pins on met4
  (each a `( -450 -500 )( 450 500 )` port at the DEF's `PLACED` x centers), and the
  VDPWR/VGND/VAPWR met4 power straps (y 5 um to 220.76 um, 2 um wide, at x = 1, 4,
  and 7 um).

The coordinates come from `tt_analog_1x2.def` and `magic_init_project.tcl` in
`TinyTapeout/tt-support-tools`. The Reticle model has no per-shape lock, so the frame
is documented as the fixed part the user must not move rather than mechanically
locked. The bundle is validated by a test that matches the die area, the six pin
rectangles, and the strap geometry against numbers extracted from those TinyTapeout
files (committed as small fixtures with their source URLs), and cross-checked against
a real published GDS-mode submission, `tt_um_analog_mux`, for the shared 1x2 height
and the met4-top / no-met5 rule.

## Submission mechanics and the precheck oracle

The submission steps are: build the tile GDS, run TinyTapeout's precheck locally until
it is clean, then submit through the TinyTapeout app before the shuttle closes.

The **precheck** is the gate that matters. It lives in TinyTapeout's own
`TinyTapeout/tt-support-tools` repository (its `precheck` module) and runs Magic and
KLayout checks over the submitted GDS: DRC, the required layers and power straps, pin
placement against the template, the top-cell name, and related structural rules. It is
Linux-native, so Reticle will run it via a pinned Docker container (WSL is a documented
fallback), wrapped as `just tt-precheck <gds>` (Lane 4B). The plan is to wire its
structured failures back into the agent loop like DRC violations, so a tile can be
generated, prechecked, and corrected in the same loop, and to prove it with an
end-to-end test where a known-good example passes and a seeded violation fails with a
parsed, actionable report.

Reticle's own SKY130 DRC subset is a fast, in-tool approximation, useful while
authoring, but it is **not** the precheck: the precheck is the authoritative external
oracle, and only a clean precheck run means a tile is submission-ready. Keeping the two
distinct, our subset for speed and their precheck for truth, is the honest arrangement.

## The precheck oracle (Lane 4B)

The precheck wrapper and its structured-failure parser are built and committed (ADR
0054). Three pieces:

- **`just tt-precheck <gds>`** runs Tiny Tapeout's own precheck over a GDS inside a
  **pinned** Docker container. The image is `hpretl/iic-osic-tools:2025.01` (amd64
  digest `sha256:a51257b7d85fc75d5a690317539f9787a401d6dd28583d73dceab174ccc9e78f`,
  measured at 3.94 GB compressed on 2026-07-06), which bundles Magic, KLayout, gdstk,
  and the SKY130 PDK (`PDK_ROOT=/foss/pdks`). The recipe calls
  `scripts/tt-precheck.ps1`, which stages a minimal Tiny Tapeout project (an
  `info.yaml` whose `top_module` equals the GDS filename stem, which the precheck
  requires and asserts), mounts that project and a pinned `tt-support-tools` checkout,
  runs `python precheck/precheck.py --gds <gds> --tech sky130A` in the container, and
  copies the reports (`results.md`, `results.xml`, `magic_drc.txt`, `drc_*.xml`) to an
  out directory. The container exit code is the precheck's own (`0` = passed). WSL is a
  documented fallback (the same precheck command against a distro that has the tools and
  PDK installed). The recipe is additive and **not** part of `just ci`, like the
  nightly-only `fuzz`/`miri` recipes, because it needs Docker and a multi-GB image.
- **A structured-failure parser** in `reticle_cli::tt_precheck` (standard-library-only,
  no new dependency) turns the reports into
  `PrecheckReport { passed, failures: Vec<PrecheckFailure> }`, where a
  `PrecheckFailure { rule, layer, location, message }` is modeled on a
  `reticle_model::Violation`. `parse_results_md` records each failed Markdown row as a
  structural failure carrying the precheck's own message; `parse_magic_drc` turns each
  Magic DRC rectangle (four micron floats) into a located failure at its bounding box in
  database units. `passed` is the precheck's own verdict, not "no failures parsed", and a
  missing `results.md` is an error, not a silent pass.
- **The agent-loop seam.** `PrecheckReport::feedback_lines()` returns exactly the
  `Vec<String>` the propose-verify-correct loop folds into its model context (the same
  `Context::feedback` channel the DRC verifier uses), so a precheck failure reaches the
  model on the next proposal the way a DRC violation does. A tile can therefore be
  generated, prechecked, and corrected in one loop.

The oracle is proven **both ways** by `tests/tt_precheck_oracle.rs`: a known-good report
parses as `passed = true` with no failures, and a seeded-violation report parses as a
failing report with a Magic DRC rectangle located in database units and a boundary
failure (`Shapes outside project area`) with its message, plus the feedback lines that
carry them. The fixtures under `crates/reticle-cli/tests/fixtures/tt-precheck/` are
**synthesized from the precheck's real output format** (transcribed from `precheck.py`,
`magic_drc.tcl`, and `pin_check.py`, fetched 2026-07-06) and are labeled as synthesized
in their `NOTICE.md`; they are not captured from a live run.

**Live-run status, stated plainly:** the live Docker precheck was **attempted but
deliberately not run to completion in this lane**. The wrapper ran end to end through the
real path (it validated the GDS, cloned `tt-support-tools`, staged the minimal project,
assembled the exact `docker run`, and started the pull, with real image layers observed
downloading from the `desktop-linux` context), so the daemon is reachable and the
invocation is correct. The pull was stopped because the 3.94 GB compressed image expands
to well over 10 GB uncompressed (plus the PDK) against about 39.5 GB free disk, and the
pull-plus-precheck is slow, so completing it here was out of scope. The pinned image tag
and digest, the exact `docker run` invocation, and the WSL fallback are recorded so that
running it to a verdict is an operator step, not a fabricated pass. **No tile is claimed
to have passed the precheck.** When a real run is captured, its `results.md` and
`magic_drc.txt` drop in beside (or over) the synthesized fixtures and the same parser and
test cover the real output unchanged.

## Honest limits

- The tooling makes a submission possible and repeatable; it does not make one. No tile
  is purchased or submitted as part of this project.
- Passing Reticle's DRC subset is necessary but not sufficient; the TinyTapeout
  precheck is the real bar, and until Lane 4B runs it here, "submission-ready" is a
  claim this project has not yet earned for any specific tile.
- The generators are DRC-clean against the cited SKY130 rule subset, not the full
  foundry deck; a real tile still has to clear the precheck's fuller checks.
