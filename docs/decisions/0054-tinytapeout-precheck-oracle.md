# 0054, Tiny Tapeout's precheck as an external oracle: a pinned Docker container, a structured-failure parser, and the agent-loop seam

## Context

A Tiny Tapeout GDS-mode submission is authoritative only when it passes Tiny Tapeout's
own precheck, the `precheck` module of `TinyTapeout/tt-support-tools`, which runs Magic
DRC and a set of KLayout checks over the GDS plus structural checks (pins against the
analog template, the tile boundary, the layer whitelist, forbidden layers, and the
top-cell name). Reticle's own SKY130 DRC subset is a fast in-tool approximation, not the
precheck; ADR 0053 and `docs/src/tapeout.md` already draw that line. The precheck is
Linux-native and needs Magic, KLayout, gdstk, and the SKY130 PDK, none of which the
Windows workspace or the wasm build has. Three questions had non-obvious answers: how to
run a Linux-native, PDK-heavy tool reproducibly from this repo; how to turn its output
into something the propose-verify-correct loop can act on; and how to prove the oracle
both ways without the multi-GB image being present in the gate.

## Decision

**Run the precheck in a pinned container, additively.** `just tt-precheck <gds>` calls
`scripts/tt-precheck.ps1`, which runs `python precheck/precheck.py --gds <gds> --tech
sky130A` inside `hpretl/iic-osic-tools`, the all-in-one image that bundles Magic, KLayout,
gdstk, and the SKY130 PDK (its PDK lives at `/foss/pdks`, so `PDK_ROOT` is set for us). The
image is pinned to a dated tag, `2025.01` (amd64 digest
`sha256:a51257b7d85fc75d5a690317539f9787a401d6dd28583d73dceab174ccc9e78f`, 3.94 GB
compressed across 48 layers, measured from the registry manifest on 2026-07-06), never
`latest`. The wrapper stages a minimal Tiny Tapeout project (an `info.yaml` whose
`top_module` equals the GDS filename stem, which the precheck requires and asserts, plus
the GDS under `gds/`), mounts that project and a pinned `tt-support-tools` checkout, runs
the precheck, and copies its reports directory (`results.md`, `results.xml`,
`magic_drc.txt`, `drc_*.xml`) back to an out directory. Its exit code is the precheck's
own. WSL is a documented fallback (the same precheck command against a distro that has the
tools and PDK installed). The recipe is deliberately outside `just ci`, like the
nightly-only `fuzz`/`miri` recipes, because it needs Docker and a multi-GB image.

**Parse the output into a small, agent-consumable type, in `reticle-cli`.**
`reticle_cli::tt_precheck` (standard-library-only, no new dependency) parses the reports
into `PrecheckReport { passed: bool, failures: Vec<PrecheckFailure> }`, where
`PrecheckFailure { rule, layer, location, message }` is modeled on a
`reticle_model::Violation`: `parse_results_md` turns each failed Markdown row into a
structural failure carrying the precheck's own exception string, and `parse_magic_drc`
turns each Magic DRC rectangle (four micron floats) into a located failure at its bounding
box in DBU. `PrecheckReport::feedback_lines()` returns exactly the `Vec<String>` the loop
folds into its model context (`reticle_bench::model::Context::feedback`), the same channel
the DRC verifier uses, so a precheck failure reaches the model on the next proposal like a
DRC violation does. `passed` is the precheck's own verdict, not `failures.is_empty()`, and
a missing `results.md` is an error rather than a silent pass.

## Consequences

The parser, the wrapper, and the agent-loop seam are real and committed, and none of them
needs Docker or the PDK to compile or to be unit-tested: the two-way oracle test
(`tests/tt_precheck_oracle.rs`) reads committed fixtures under
`tests/fixtures/tt-precheck/{pass,fail}/` and asserts a known-good run parses as
`passed = true` with no failures and a seeded-violation run parses as a failing report
with parsed, actionable failures (a Magic DRC rectangle located in DBU, a boundary failure
with its message) plus the feedback lines that carry them. Those fixtures are **synthesized
from the precheck's real output format** (transcribed from `precheck.py`, `magic_drc.tcl`,
and `pin_check.py`, fetched 2026-07-06) and are labeled as synthesized in their
`NOTICE.md`; they are not captured from a live run. The **live Docker precheck was
attempted but deliberately not run to completion in this lane**: the wrapper ran end to end
through the real path (it validated the GDS, cloned `tt-support-tools`, staged the minimal
project, assembled the exact `docker run`, and started the pull, with real image layers
observed downloading from the `desktop-linux` context), so the daemon is reachable and the
invocation is correct; the pull was stopped because the 3.94 GB compressed image expands to
well over 10 GB uncompressed (plus the PDK) against 39.5 GB free disk, and the
pull-plus-precheck is slow, so completing it here was out of scope. The image tag and
digest, the exact `docker run` invocation, and the WSL fallback are recorded here and in
the wrapper so a live run is an operator step, not a fabricated pass: no tile is claimed to
have passed the precheck.
When a real run is captured, its `results.md` and `magic_drc.txt` drop in beside (or over)
the synthesized fixtures and the same parser and test cover the real output unchanged, and
the e2e test can then be wired to the live run with its measured tag and timing.
