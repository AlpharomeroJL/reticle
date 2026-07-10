# 0101, F1: the gallery-manifest contract, licence-checked and streamable-only

## Context

The v8.2 campaign ships a start-screen library gallery (Phase 1) whose cards render from a
manifest the content pipeline produces. The gallery UI lane must build before the pipeline
has fetched a single die (fixture-first), and the pipeline and UI must agree on one shape
or they drift. Two campaign rules bear on this: no die is ever uploaded or shown without a
CHECKED licence (mirroring the `xtask verify-licenses` redistribution gate, ADR 0070), and
the project is integer-exact and byte-stable (no floats in a committed record).

## Decision

F1 lives in `reticle-index` (`gallery_manifest.rs`), the crate that already owns the
`.rtla` archive a die streams from. `GalleryManifest { version, dies }`; each `DieEntry`
carries id, name, technology, `width_dbu`/`height_dbu`, `source` (repo, commit, url),
`license`, `streaming`, `landmarks`, and `provenance`.

The licence is always present and is exactly one of two states: `License::Verified { spdx,
text_sha256 }` (an identified SPDX id plus the SHA-256 of the licence text) or
`License::Excluded { reason }`. A verified die is streamable and carries `Some(Streaming
{ archive_key, tile_count, total_bytes })` with a content-hash R2 key; an excluded die
carries `None` (no archive is ever uploaded for an unverified licence) and is a ledger of
what was skipped and why. `GalleryManifest::validate` enforces sorted unique ids, a
64-char lowercase-hex text hash on every verified die, and the verified-streamable /
excluded-no-archive invariant. Landmark camera views are integer DBU with milli-scaled
zoom, so a landmark deep link round-trips exactly through a permalink.

The fixture is `crates/reticle-index/tests/fixtures/contracts/f1_manifest.json` (two
synthetic dies, one verified and streamable, one excluded); the cross-test
(`tests/f1_manifest.rs`) validates it, confirms the gallery shows only the verified die,
and checks the validator rejects each contract violation.

## Consequences

The gallery UI renders `N` manifest entries generically and filters to verified dies,
built entirely against the fixture until the pipeline lands. At Gate 1 the real manifest
(generated from verified, uploaded dies) swaps in for the fixture and the gallery's success
bar re-runs against it. Because an excluded die can appear only without an archive, the
"no unverified uploads" rule is a type-level invariant the validator enforces, not a
convention a lane can forget.
