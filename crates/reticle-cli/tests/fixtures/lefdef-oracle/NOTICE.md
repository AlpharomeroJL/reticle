# LEF/DEF oracle fixture provenance

These fixtures back the LEF/DEF import oracle cross-check (`tests/lefdef_oracle.rs`,
ADR 0083). They are original, hand-written, and released under the same license as the
repository. None is copied from a third party.

## The design fixtures

- `faithful.lef` and `faithful.def` are a small synthetic standard-cell design (two
  macros `INV`/`BUF`, three placed components, two I/O pins, a `20000 x 20000` DBU die).
  They are authored to parse cleanly under BOTH `reticle-lefdef` and `OpenROAD`, so the
  two tools can be compared on the exact same input. To stay inside what `OpenROAD`'s
  strict LEF/DEF reader accepts, they omit the optional `BUSBITCHARS`/`DIVIDERCHAR` header
  lines and keep the net routing via-free (an undefined via is a hard `OpenROAD` error,
  while the compared facts, macros/components/pins/die area, do not involve nets).
- `corrupt.def` is `faithful.def` with one component (`u3 INV`) deleted and the
  `COMPONENTS` count corrected to `2`. It is the negative control: the oracle reports two
  components where the faithful import has three, so the counts DISAGREE, which proves the
  oracle actually discriminates a faithful import from a corrupted one.

## The oracle-output fixtures

- `oracle_faithful.txt` and `oracle_corrupt.txt` are the real stdout of `OpenROAD`
  reading `faithful.lef` with `faithful.def` and `corrupt.def` respectively, captured from
  the pinned image `hpretl/iic-osic-tools:2025.01`
  (digest `sha256:a51257b7d85fc75d5a690317539f9787a401d6dd28583d73dceab174ccc9e78f`) on
  2026-07-07. They let the parser and the two-way discrimination be proven in the ordinary
  gate with no Docker and no image: `parse_oracle_output` reads the `ORACLE <key>=<value>`
  lines back and ignores the surrounding banner and log lines.

The live container cross-check (`tests/lefdef_oracle.rs`) additionally runs `OpenROAD`
over `faithful.def` and `corrupt.def` when Docker and the pinned image are present, and
skips honestly otherwise. See ADR 0083 for the tool choice, the pinned digest, the
compared facts, and the tolerance and subset limits.
