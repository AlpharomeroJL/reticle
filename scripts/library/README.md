# The library pipeline

`fetch-convert-verify.ps1` is the rerunnable machinery that turns a table of source
dies (`dies.json`) into the committed sample under `library/`: one `.rtla` archive and
one `.rtla.NOTICE` per die, a CHECKED license verdict for each, and the F1
`GalleryManifest` JSON the start-screen gallery reads (ADR 0101).

This lane (`pipeline-manifest`) proves the pipeline on a **tiny, already-committed
sample**: two dies, both network-light. It does not fetch the multi-gigabyte real
shuttle archives that would populate a real gallery -- that is a separate, later step.
See "What is deferred" below.

## Files

- `dies.json`: the die table. Each entry names a die's id, editorial metadata (name,
  technology, a curated landmark), its provenance (repo, commit, url), a source GDS
  (a path already committed elsewhere in this repo, or `null` to synthesize one with
  `xtask gen-layout`), and an `spdx` identifier (or `null`, which produces a NOTICE
  with no SPDX line at all -- the license gate then excludes that die, proving the
  fail-closed path rather than claiming anything false about a real license).
- `fetch-convert-verify.ps1`: reads `dies.json` and, per die, resolves the source GDS,
  converts it with the existing `reticle-cli convert` command, writes the sibling
  NOTICE, then runs `xtask verify-licenses` (informational) and `xtask
  library-manifest` (gates the script) to produce `library/gallery-manifest.json`.

## Running it

From the repo root, with `CARGO_TARGET_DIR` set as your environment requires:

```
powershell -File scripts/library/fetch-convert-verify.ps1
```

Optional parameters: `-DiesMeta <path>` (default `scripts/library/dies.json`),
`-LibraryDir <dir>` (default `library`), `-ScratchDir <dir>` (default
`scratch/library-pipeline`, for synthesized intermediate GDS files only -- gitignored,
never committed).

The run is idempotent: converting is byte-deterministic (ADR 0072) and the manifest is
a pure function of the die table plus the archives, so re-running reproduces the same
output (other than `gallery-manifest.json`'s `fetched_utc`, which reflects the run
time by design).

## Adding a die

Add an entry to `dies.json` naming a `source_gds` already committed somewhere in this
repo (or `null` to synthesize one) and its provenance, then re-run the script. The
`xtask library-manifest` step fails closed on a mismatch: every archive in the library
directory must have a matching `dies.json` entry by id, and vice versa, so a stale
entry or a forgotten archive is a hard error, never a silently incomplete manifest.

## What is deferred to the valley-queue bulk fetch

This lane's sample is deliberately tiny and network-light (a few KB, entirely from
GDS files already committed in this repo's own test corpora) so it is provable without
a real download. A real gallery needs the actual open-silicon shuttle archives
(TinyTapeout shuttles, full sky130 standard-cell sets, and similar), which run from
`gh` to multiple gigabytes -- too large to fetch or commit from a lane worktree, and
explicitly out of scope for this lane per its brief. That bulk fetch is a separate,
later orchestrator step (the "valley queue"): it reuses this exact same machinery
(`dies.json` grows more entries; `fetch-convert-verify.ps1` runs unchanged) but adds a
real network fetch stage in front (in the shape of the existing
`scripts/fetch-sky130-cells.ps1` / `scripts/fetch-tinytapeout-gds.ps1` scripts) and
runs from outside a lane worktree, with its output staged and uploaded by the archive
Worker's deploy step (ADR 0070), not committed to git.
