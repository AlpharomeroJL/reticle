# Library sample (pipeline-manifest lane)

This directory is the tiny, committed **proof sample** for the open-silicon library
pipeline (`scripts/library/`), not a shipped gallery. It contains:

- `<id>.rtla`: one streamable tile archive per die, built by `reticle-cli convert`.
- `<id>.rtla.NOTICE`: the sibling provenance/license record the redistribution gate
  (`xtask verify-licenses`, ADR 0070) reads.
- `gallery-manifest.json`: the F1 `GalleryManifest` (ADR 0101) `xtask library-manifest`
  generated from the verdicts above plus each archive's own real geometry.

Regenerate everything here with `scripts/library/fetch-convert-verify.ps1`; see
`scripts/library/README.md` for the full pipeline and what a real (bulk) library run
adds on top of this sample.

## Why one entry is `Excluded`

`demo.unverified-fixture` is a first-party, synthetic die (`xtask gen-layout` output,
not third-party content) whose NOTICE deliberately carries no
`SPDX-License-Identifier` line. It exists to prove, end to end, that the license gate
and the manifest generator correctly propagate an unverified license into
`License::Excluded { reason }` with no streaming archive and no landmark -- the
fail-closed path -- not to claim anything about a real project's licensing. Because of
this entry, running `xtask verify-licenses library` over this directory exits non-zero
by design (see `scripts/library/fetch-convert-verify.ps1`, which treats that exit code
as informational for exactly this reason). A real staging run would only ever ship the
verified subset.

`sky130.inv-1` is real: the SkyWater `sky130_fd_sc_hd__inv_1` standard cell, already
committed at `crates/reticle-io/tests/corpus/sky130/sky130_fd_sc_hd__inv_1.gds`
(Apache-2.0; see that directory's own `NOTICE.md` for the exact upstream commit). It
verifies and carries a streaming badge and one curated landmark.
