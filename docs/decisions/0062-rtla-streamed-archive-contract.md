# 0062 — The `.rtla` streamed-archive format, the `TileSource` seam, and the `gds_stream` reader

## Context

Wave 2 makes silicon stream over HTTP Range requests: a builder writes a tiled archive,
native and wasm sources read tiles by byte range, and a consumer drives residency as the
camera moves. Three lanes build these in parallel (2A the builder and streaming reader,
2B the sources, 2C the residency consumer, 2D the converter), so the shared interfaces
must be frozen first or the lanes block on each other. The existing `TiledPayload`
(`reticle-index/src/streaming.rs`) stores only `(bbox, id)` and has no level-of-detail
structure or renderable payload, so it cannot be the transport unit. The existing GDSII
importer reads a whole library into memory under a 256 MiB cap, which cannot build an
archive from a multi-gigabyte die or run inside a browser worker.

## Decision

Freeze a new container and two seams as the Wave 2 contract, committed to main before
the lanes fan out. `.rtla` v1 (`reticle-index/src/archive.rs`): an rkyv-archived
`RtlaHeader` (magic, version, world bbox, `dbu_per_micron`, per-level grid dims), an
rkyv-archived tile directory of `(offset, len)` entries in level-major row-major order,
then byte-contiguous rkyv-archived `TilePayload` tiles, each independently fetchable by
one Range request and independently validated by `rkyv::access` exactly as the mmap path
already is. The finest level is exact; coarser levels are paint-only approximations and
queries always resolve against the finest level. The `TileSource` trait (async, because
the wasm implementation fetches over the network) is the read seam; it has no mutation
path, encoding the read-mostly scope of streamed documents. The `gds_stream` module
(`reticle-io/src/gds_stream.rs`) freezes a small flat `GdsEvent` vocabulary and a
forward-only `GdsRecordReader<R: Read>` over any byte source. To keep lanes 2A and 2B on
disjoint files, all three implementation modules (`archive_build`, `tile_source`,
`gds_stream`) are pre-declared now, with `archive.rs`'s types and trait complete and the
builder/sources/reader left as honest contract stubs (a clear error, never a fake
success or a `todo!`) that the lanes replace before the Wave 2 gate.

## Consequences

Lanes 2A, 2B, 2C, and 2D code against frozen signatures immediately and never edit the
same file: 2A owns `archive_build.rs` and `gds_stream.rs`, 2B owns `tile_source.rs`, and
neither touches `lib.rs`. The read-mostly scope is structural (no mutation on
`TileSource`), which lane 2C reinforces at the app layer. The standing hardening lesson
is carried into the contract docs: every count or length field from a stream or header is
untrusted, so no reservation exceeds the remaining input (ADR follows commit 1b1b56b), and
the streaming reader's fuzz target seeds from the committed GDS crash fixtures so it cannot
reintroduce a fixed panic class. Compression and the finest-level polygon vertex list are
named follow-ups, additive to v1. A future format change bumps `RTLA_VERSION` with a
reader that refuses an unknown version, the same discipline the proto schema uses.
