# reticle-lefdef test corpus

This crate imports LEF/DEF. Its tests run against a small, hand-authored synthetic
design, not a redistributed foundry release. Nothing under `tests/fixtures/` is
third-party content.

## Files

| file | provenance |
|------|------------|
| `tests/fixtures/tinycore.lef` | synthesized in-repo: a 1000 DBU/micron technology with three layers (`li1`, `mcon`, `met1`), one `SITE` (`unithd`), and two `MACRO`s (`INV`, `BUF`) with pins and one obstruction. Not derived from any foundry LEF. |
| `tests/fixtures/tinycore.def` | synthesized in-repo: a placed, routed `tinycore` design (die area, two rows, three placed components across the N/FN/FS orientations, two external pins, two routed nets with a via and a `*` repeated coordinate). |
| `tests/fixtures/run/lib/tech.lef` | synthesized in-repo: a trimmed technology-plus-macros LEF for the run-directory import test. |
| `tests/fixtures/run/results/2_floorplan.def` | synthesized in-repo: an early-stage DEF (die area only) used to check that the later flow stage is selected. |
| `tests/fixtures/run/results/6_final.def` | synthesized in-repo: the final-stage DEF `import_run_dir` selects. |

The synthetic fixtures were written to exercise the supported LEF/DEF subset
directly (see the interop chapter in the book and ADR 0063), including the parser
hazards the robustness tests target: truncated blocks, non-numeric coordinates,
unknown keywords, oversized input, and invalid UTF-8.

## Real OpenROAD run (not fetched here)

The intended real corpus is a minimized single design from the OpenROAD flow
scripts. Fetching it requires network and container access that this build
environment does not have, so no real LEF/DEF is committed and the end-to-end
"an ORFS run renders in Reticle" check is recorded as not-run in the lane RESULT.
The exact fetch command, to be run where the network and `git` are available, is:

```sh
git clone --depth 1 https://github.com/The-OpenROAD-Project/OpenROAD-flow-scripts
# LEF:  OpenROAD-flow-scripts/flow/platforms/nangate45/lef/NangateOpenCellLibrary.tech.lef
#       OpenROAD-flow-scripts/flow/platforms/nangate45/lef/NangateOpenCellLibrary.macro.lef
# DEF:  a small design's results/nangate45/<design>/base/6_final.def
```

The Nangate45 platform in OpenROAD-flow-scripts is distributed under Apache-2.0.
When a real design is added, only a minimized fixture (a few standard cells under a
small placed top) should be committed, with its source URL and license recorded in
this file, following `corpus/tinytapeout/NOTICE.md`.
