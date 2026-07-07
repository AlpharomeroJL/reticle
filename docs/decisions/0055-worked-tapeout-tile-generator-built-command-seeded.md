# 0055, The worked TinyTapeout tile: generator-built, command-seeded through GDS import, DRC-subset-clean and precheck-deferred

## Context

Lane 4A built the TinyTapeout tile template (the `tt_um_reticle_tile` frame: the 1x2
die outline, the six `ua[0]`..`ua[5]` met4 analog pins, and the VDPWR/VGND/VAPWR met4
power straps, met5 kept clear). Lane 4B built the precheck oracle wrapper (`just
tt-precheck`). This lane produces the packet's proof artifact: a real, committed tile
made by Reticle's own generators, framed by that template, DRC-clean against the SKY130
subset, and committed with a replayable transcript. Three questions had non-obvious
answers.

**How is the build a transcript-replayable command run when the frame carries pins and
labels?** The objective prefers driving the placement through the Wave 2D `RunGenerator`
`AgentCommand` applied to a `Session` seeded with the template document, so the run
produces a real, replayable `Transcript` under the `document_hash` replay contract. But
a `Session` starts empty and the frozen command vocabulary has no command that creates a
`Pin` or a `Label`, and the frame is built in pure code with both. Reconstructing the
frame from commands alone would silently drop the pins and labels, so the seeded
document would not be the template.

**Where does the generated structure go, given `RunGenerator` cannot place?** The
`test_structure` generator emits at its own origin `(0,0)`, and `RunGenerator` carries no
transform. Left there, a large structure would collide with the analog-pin strip (y
`0`..`1000`) and the power straps (x `0`..`8000`).

**What may this tile honestly claim?** The authoritative bar is the TinyTapeout precheck
(Lane 4B), which needs a multi-GB Docker image and the SKY130 PDK and is the operator's
live step. Reticle's SKY130 DRC is a cited subset, not that precheck.

## Decision

**Build the tile through the command surface, seeding the frame by GDS import.** The
build (`reticle_app::tinytapeout_example`) runs three `AgentCommand`s against a `Session`:
`ImportGds` of the Lane 4A frame (exported to GDSII in code), then `RunGenerator`
`test_structure`, then `TransformShapes`. Seeding by import is the honest way to put the
whole frame into a session the transcript can replay: the alternative, reconstructing the
frame from `AddRect` commands, cannot recreate the `Pin`/`Label` records the frame holds.
GDSII has no pin element, so the import is where the frame's `Pin` objects are dropped:
the pin **metal** (the met4 `ua[*]` and strap rectangles on `71/20`) and the labels
survive; the `Pin` terminal records on the met4 pin purpose (`71/16`) do not. This is a
property of GDSII, not a shortcut, and the committed GDS is *defined* to be exactly what
this session exports, so the artifact and the transcript agree by construction (the same
`document_hash` on both). The physical met4 landing pads a probe or the shuttle needs are
the drawing metal, which is present; the pins remain documented in the Lane 4A template
and the technology file.

**Place with a second command.** `RunGenerator` returns the new shapes' ids; a single
`TransformShapes` translates them by `(12000, 88000)` DBU into the interior, 4 um to the
right of the rightmost power strap and far above the analog-pin strip, wholly inside the
die. A pure translation keeps every rectangle a rectangle, so the structure stays
DRC-clean. Both commands are recorded, so replaying the transcript reproduces the placed
structure exactly.

**Use a serpentine on met2, sized to fill the interior.** The `test_structure`
serpentine (`feature_width = 1000`, `feature_length = 140000`, `count = 40`) is a
continuous boustrophedon trace whose end-to-end resistance a probe station reads. It is
on `met2` (a subset layer with width/spacing rules), never met4 or the forbidden met5,
and its ~140 um by ~45.5 um footprint fills a substantial band of the interior.

**Drive and verify from an `xtask` subcommand and a committed test.** `xtask
tapeout-example` writes `examples/tapeout/tt_um_reticle_tile.gds` and
`tt_um_reticle_tile.transcript.jsonl` (one `CommandRecord` per line plus a `{"final_hash":
...}` trailer, the format the replay theater loads), refusing to write unless the tile is
DRC-subset-clean and the transcript replays to the exported document's hash. A committed
test (`worked_tile_is_drc_subset_clean` and its siblings) regenerates the tile
deterministically in memory and asserts the same properties, so a regression fails the
gate without depending on the checked-in files.

**State the precheck as deferred, plainly.** The tile is DRC-clean against the SKY130
**subset** only. It is a **generator-driven, deterministic build, not a Claude Code run**
(the CLI is unauthenticated in this environment, so nothing here was authored by a model).
It is **not** verified through the real TinyTapeout precheck; the exact operator step is
`just tt-precheck examples/tapeout/tt_um_reticle_tile.gds`.

## Consequences

- The committed transcript is fully self-contained and replayable: the `ImportGds`
  command embeds the frame GDS inline, so a fresh session replays the entire build with no
  external input, and `document_hash` verifies it. The transcript records a real command
  run over the same frozen surface an agent uses, exercising `RunGenerator` (Wave 2D) and
  `TransformShapes` together.
- The build driver lives in the app crate next to the Lane 4A template (which already has
  every dependency it needs) and is invoked by a thin `xtask` subcommand, so no core crate
  gains a dependency.
- The honesty boundary is explicit in three places (the module docs, the book section, and
  the subcommand's own output): DRC-subset-clean, not precheck-verified; generator-built,
  not agent-authored.
- The GDSII pin-drop is a real limitation of the artifact format: the committed GDS carries
  the frame as drawing metal and labels, not as Reticle `Pin` objects. This matches every
  other GDSII export in the project and is stated rather than hidden.
