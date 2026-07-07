# 0059, The live TinyTapeout precheck run: making the wrapper work end to end, and the real prBoundary bug it caught

## Context

Lane 4B (ADR 0054) built `just tt-precheck` around TinyTapeout's own Magic+KLayout precheck
in a pinned Docker image, but deliberately never ran the container to a verdict (the image
is multi-GB and disk was tight). The worked tile (ADR 0055) was therefore labeled
"DRC-subset-clean, precheck-deferred". The v7 finish pulled the image and ran the real
precheck on `examples/tapeout/tt_um_reticle_tile.gds`. Running it to completion surfaced
problems the never-completed path had hidden, in two categories: the wrapper's own
invocation, and one real bug in the tile.

**The wrapper did not actually invoke the precheck.** Five distinct, correctly-diagnosed
blockers, each a permanent fix to `scripts/tt-precheck.ps1`, stood between the assembled
`docker run` and the precheck actually executing:

1. The `hpretl/iic-osic-tools` image ships an entrypoint launcher (X11/VNC bootstrap) that
   treats the container command as its own options and rejects a bare `bash`
   (`Unexpected option "bash"`). It needs `--skip` as the first argument after the image.
2. `precheck.py` imports `gdstk`, which the image does not bundle (it bundles Magic,
   KLayout, and the PDK). It is a pinned precheck requirement, installed with pip.
3. The image's Python is PEP 668 externally-managed, so pip needs `--break-system-packages`.
4. `gdstk 0.9.52` is built against NumPy 1.x while the image ships NumPy 2.2.1, so the
   pinned `numpy==1.26.4` must come too (this is why tt-support-tools pins it).
5. The image entrypoint sets `PYTHONPATH` with system `dist-packages` (NumPy 2) ahead of
   the user site, so the user-site NumPy 1.26.4 is shadowed until the user site is
   prepended to `PYTHONPATH`. And `precheck.py` resolves `tech-files/` and
   `../tech/<tech>/def` relative to its own directory, so it must run from
   `/support/precheck`, not `/support`. The staged `info.yaml` also has to record the tile
   footprint (`tiles: "1x2"`) so the precheck selects the matching DEF template.

**The precheck then caught a real bug.** With the wrapper fixed, the precheck ran cleanly
and reported the tile as failing its `prBoundary.boundary (235/4)` check: the tile drew its
outline only on `areaid.sc (81/4)`, the marker Magic reads, but TinyTapeout's KLayout
checks delimit the project area from `prBoundary.boundary (235/4)`, which the tile lacked.
That also cascaded into the boundary check ("shapes outside project area"), since without
the layer KLayout could not establish the area.

## Decision

**Fix the wrapper permanently** with the five changes above, so `just tt-precheck` runs the
real precheck end to end on this host (and any host with the pinned image). A `-Tiles`
parameter (default `1x2`) records the footprint in the staged minimal `info.yaml`.

**Fix the tile.** Add the `prBoundary.boundary (235/4)` layer to
`tech/tinytapeout-sky130.tech` and draw the die outline on it as well as on `areaid.sc`, so
Magic and KLayout agree on the boundary. The tile now carries both markers. The worked
example and its transcript were regenerated (`xtask tapeout-example`), staying
DRC-subset-clean and replay-stable.

**Record the honest verdict.** After the fix, the tile passes every one of TinyTapeout's
geometry, DRC, and structural checks against their own decks: Magic DRC, KLayout FEOL,
BEOL, offgrid, pin-label-overlap, zero-area, the prBoundary/KLayout checks, the boundary
check, the layer whitelist, the cell-name check, and the urpm/nwell check. The four
remaining failures are not geometry and not DRC; they are submission artifacts a
GDS-geometry generator does not produce:

- **Pin check**: needs a `.lef` pin abstract (Reticle exports GDS, not LEF).
- **Power pin check** and **Verilog syntax check**: need a `.v` Verilog view (and
  `yowasp-yosys`); a GDS-mode tile still needs a Verilog interface stub.
- **Analog pin check**: the six `ua[*]` pins are met4 landing pads, not wired to the
  interior test structure, because the worked tile is a template plus an isolated
  probe-able structure, not a wired design. TinyTapeout requires analog pins wired or
  `analog_pins: 0`.

## Consequences

- `just tt-precheck` now produces a real TinyTapeout precheck verdict on this host. The
  captured `results.md` is committed at `examples/tapeout/precheck-results.md` as evidence.
- The worked tile's claim is upgraded from "DRC-clean against the SKY130 subset" to "passes
  all of TinyTapeout's own Magic+KLayout DRC and geometry checks", which is stronger and
  now measured, not asserted. ADR 0055's "precheck-deferred" is superseded on the geometry
  axis by this run; the submission-completeness axis (LEF, Verilog, wired pins) is the
  honest remaining boundary.
- The real precheck earned its keep on the first real run: it caught a wrong-layer boundary
  bug (`81/4` where `235/4` was required) that the SKY130 subset DRC could not, exactly the
  reason for keeping an external oracle distinct from our own fast subset (ADR 0054).
- Reaching a fully green precheck (a submittable tile) would require a LEF writer, a Verilog
  interface view, and wiring the analog pins to a design. Those are outside a geometry
  generator's scope and are not faked here; the tile is a DRC-clean geometry demonstration,
  and the missing pieces are named plainly.
