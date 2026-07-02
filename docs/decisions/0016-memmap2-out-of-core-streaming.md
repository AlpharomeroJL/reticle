# 0016, Out-of-core streaming via memmap2: the workspace's one unsafe block

## Context

ADR 0013 deferred the disk/mmap layer of out-of-core streaming to preserve a fully
safe-Rust workspace: `streaming.rs` shipped only the zero-copy `rkyv` read primitive
over an in-memory byte slice. v4.0.0 calls for the real thing: browse an archived
layout from disk without reading the whole file into memory, paging in only the tiles
a query touches.

Memory-mapping a file cannot be done in safe Rust. `memmap2::Mmap::map` is `unsafe`
because the mapped bytes are backed by the kernel, not Rust's allocator: another
process truncating or writing the file while the map is live is undefined behaviour,
and no library API can rule that out. That obligation is real but narrow, and it is
the standard contract every memory-mapped-IO program accepts.

## Decision

Accept exactly one `unsafe` block in the workspace: the `Mmap::map` call inside
`StreamingIndex::open` (crates/reticle-index/src/streaming.rs), annotated
`#[allow(unsafe_code)]` against the workspace `unsafe_code = "warn"` lint and carrying
a SAFETY comment that states the obligation and what we uphold: the file is opened
read-only, owned by the `StreamingIndex` so it outlives the map, never exposed for
writing, and the mapped bytes are validated by `rkyv` `access` (bytecheck) immediately
after mapping so a corrupt or truncated-at-open file is an error, not undefined
behaviour.

On top of the map, a `TiledPayload` organizes entries tile-contiguously behind a
per-tile directory over a uniform grid, so `StreamingIndex::query_region` walks only
the directory plus the tiles the region overlaps. The OS demand-pages exactly those
regions of the file; the entries array is never materialized in memory.

This supersedes the "stay fully safe" stance of ADR 0013.

## Consequences

- The workspace is no longer zero-`unsafe`; it is exactly-one-`unsafe`, documented,
  reviewed, and confined to a single line whose failure mode is well understood. The
  demo example reads its working set via a spawned PowerShell query specifically so it
  would not add a second unsafe site.
- Miri cannot execute a real file mmap, so the mmap integration tests are gated
  `#![cfg(not(miri))]` (crates/reticle-index/tests/streaming_mmap.rs). The tests cover
  oracle-checked region queries over random regions, partial-touch accounting (a small
  viewport touches few tiles), rejection of non-archive files, and empty regions.
- `examples/stream_demo.rs` measures the behaviour honestly: cold query time, tiles
  and entries touched versus totals, a brute-force cross-check, and the process
  working set versus the on-disk file size.
- The archive builder currently constructs the payload in memory before writing it,
  so the largest archive this tooling can produce is bounded by RAM even though
  reading is not. A streaming writer is the natural follow-up if truly RAM-exceeding
  archives need to be authored on this machine.
