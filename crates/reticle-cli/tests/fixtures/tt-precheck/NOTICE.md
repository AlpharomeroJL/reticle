# TinyTapeout precheck fixture provenance

These `results.md` and `magic_drc.txt` files are **synthesized**, not captured from a
live precheck run. They are hand-built to match the exact output format of TinyTapeout's
precheck, transcribed from its source in `TinyTapeout/tt-support-tools` (fetched
2026-07-06):

- `precheck/precheck.py` writes `reports/results.md` as a Markdown table
  `| Check | Result |` with `| {name} | ✅ |` for a passing check and
  `| {name} | ❌ Fail: {str(exception)} |` for a failing one. Source:
  <https://github.com/TinyTapeout/tt-support-tools/blob/main/precheck/precheck.py>
- The failure messages used here are the precheck's own `PrecheckFailure` strings:
  `Shapes outside project area` (boundary check) and the Magic DRC summary. Source: same
  file plus `precheck/pin_check.py`.
- `precheck/magic_drc.tcl` writes the Magic DRC report as a cell-name header, per-rule
  blocks separated by dashed rules, offending rectangles as four micron floats
  (`llx lly urx ury`), and an `[INFO]: COUNT: <n>` summary. Source:
  <https://github.com/TinyTapeout/tt-support-tools/blob/main/precheck/magic_drc.tcl>

Why synthesized and not captured: a live precheck run needs the multi-GB
`hpretl/iic-osic-tools` image, the SKY130 PDK, Magic, and KLayout on Linux (see
`scripts/tt-precheck.ps1` and ADR 0054). Capturing real output is the operator's step;
these fixtures let the parser be proven both ways in the ordinary `just ci` gate with no
Docker, no image, and no PDK. Every coordinate and rule name here is illustrative, not a
measurement of any real tile.

- `pass/` is a clean run: every check green, `COUNT: 0` in Magic DRC.
- `fail/` seeds two failures: a Magic DRC met1 spacing/width violation (three rectangles,
  parsed to real locations) and a boundary-check failure (`Shapes outside project area`,
  a structural failure with no location).

When a real precheck run is captured, drop its `results.md` and `magic_drc.txt` in
alongside these (or replace them) and the same parser and test cover the real output
unchanged.
