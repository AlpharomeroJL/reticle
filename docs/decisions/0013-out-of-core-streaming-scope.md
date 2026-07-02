# 0013, Out-of-core streaming: zero-copy primitive now, mmap paging deferred

**Superseded by [ADR 0016](0016-memmap2-out-of-core-streaming.md)**: the mmap paging
layer described below as deferred is now implemented, with exactly one documented
`unsafe` block.

## Context

The spec calls for browsing layouts too large to hold in RAM by streaming tiles/LOD
from disk. `reticle-index` ships the foundational piece: `streaming.rs` serializes an
`IndexPayload` to an `rkyv` archive laid out exactly as its in-memory form and reads
entries back **zero-copy** through `rkyv`'s validated `access` entry point (bytecheck
on, no `unsafe`, no `access_unchecked`), including single-entry random access without
deserializing the whole vector. A truncated or corrupt buffer yields a `StreamError`
rather than undefined behaviour.

What does **not** exist: nothing memory-maps a file into that byte slice, nothing pages
tiles from disk on demand, and no renderer consumes the streamed payload. The API is
exercised only over in-memory `Vec<u8>` buffers in its own tests. The module's original
doc comment described the disk + mmap + demand-paging behaviour in the present tense, as
if shipped, an overstatement corrected during the 2026-07-01 audit.

The blocker to finishing it honestly is that true memory-mapping requires an `unsafe`
call (e.g. `memmap2::Mmap::map`; a file mapped read-only can still be mutated by another
process, so the safety obligation is real and cannot be discharged in safe Rust). The
workspace is deliberately 100% safe Rust (`unsafe_code = "warn"`, zero `unsafe` today),
which is itself a quality property we did not want to trade for one feature under an
autonomous pass.

## Decision

Ship the **validated zero-copy read primitive** as the building block, and **defer** the
mmap/disk-paging layer and its renderer integration. Correct the docs (module doc,
README, this ADR, `docs/STATUS.md`) to describe the primitive accurately and mark
end-to-end out-of-core browsing as an explicit follow-up rather than a present-tense
capability. Large **in-memory** layouts continue to browse via the in-RAM LOD pyramid
and cell culling, which are real.

## Consequences

- No dishonest claim: the code does exactly what the docs now say, zero-copy access
  over a byte buffer, and nothing pretends to page from disk.
- The follow-up is well-scoped: add `memmap2` behind a tightly-reviewed
  `#[allow(unsafe_code)]` with a `SAFETY` note (or a safe `seek`+read tile reader if we
  choose to avoid `unsafe` entirely), map the archive file, feed the bytes to the
  existing `access`/`entry_at` path, and have the renderer request tiles by region. The
  primitive is designed so this is additive, not a rewrite.
- If the mmap path is added, `miri` gains a crate with `unsafe` to cover, and the mmap
  test must be `#[cfg(not(miri))]` (miri cannot execute a real file mmap).
- The all-safe-Rust property is preserved for now.
