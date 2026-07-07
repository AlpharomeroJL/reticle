# 0063 - `.rtla` on-disk framing and the external two-pass builder

## Context

ADR 0062 froze the `.rtla` v1 record *types* (`RtlaHeader`, the `Vec<TileDirEntry>`
directory, and the per-tile `TilePayload` blocks) and stated that tiles are
byte-contiguous and addressed by `(offset, len)` from the start of the file. It did
not say how a reader *locates* the two variable-length blocks that precede the tiles:
the header (which carries a `Vec<LevelDims>`) and the directory (whose length is the
tile count). Because each block is an independent rkyv archive whose root sits at the
end of its own byte range, a reader cannot find a block without knowing that range,
and rkyv `access` needs each block to start at an aligned offset. Lane 2A writes the
archive and lane 2B (`tile_source.rs`) reads it; they are built in parallel, so the
framing must be pinned in writing, not left implicit.

Separately, ADR 0016's in-RAM index builder holds the whole archive at once. A `.rtla`
exists to stream dies of several gigabytes and to be built inside a browser worker, so
its builder must be external: bounded memory regardless of layout size.

## Decision

**Framing.** A `.rtla` file is a fixed 32-byte little-endian preamble of four `u64`s
(`header_off`, `header_len`, `dir_off`, `dir_len`), then the rkyv `RtlaHeader` block,
then the rkyv `Vec<TileDirEntry>` directory block, then the tiles. Every block starts
on a 16-byte boundary (the preamble is 32 bytes; padding follows each block), so a
reader memory-maps the file, reads the preamble, and `rkyv::access`es the header and
directory in place, then each tile by its directory `(offset, len)`. `TileDirEntry`
offsets stay absolute from the file start, exactly as ADR 0062 specifies. Magic and
version live inside the archived header and are checked after access, the same
discipline `RTLA_MAGIC`/`RTLA_VERSION` already imply.

**External build.** `build_rtla(header, records, out_path)` takes the record source as
an `IntoIterator<Item = TileRecord>` consumed once, lazily. Pass 1 streams the records,
assigns each to the tiles it overlaps at every level (the LOD tile-span math), and
spills fixed-size `(tile_index, record)` entries as sorted runs, holding at most one
sort chunk in memory. Pass 2 `k`-way merges the runs into global tile order and emits
each tile's contiguous run as a `TilePayload`, holding at most one tile's records.
Peak memory is a sort chunk (~128 MiB) plus one tile, independent of the record count.

**Coarse-level decimation.** The finest level (last in `RtlaHeader::levels`) is exact:
every record reaches it, so it round-trips. Coarser levels are the paint-only
approximations ADR 0062 describes; the builder subsamples them with a per-level stride
that keeps their per-tile density near the finest level's, which bounds memory
uniformly and keeps the spill near-linear in the record count instead of multiplying
it by the level count. A per-tile safety cap bounds memory even under a pathological
distribution.

## Consequences

Lane 2B's `MmapTileSource` (and the wasm `HttpRangeTileSource`) read to this framing:
fetch the 32-byte preamble, then the header block, then the directory, then individual
tiles by range. The round-trip is proved in `reticle-index/tests/rtla_build.rs`, which
builds an archive and reads it back exactly this way over an mmap. Measured on the 30M
generated layout, peak RSS is 127 MiB (well under the ~2 GiB budget) and a 120M-record
build produces a 2.42 GB archive to completion under the same bound. The stride
decimation means only the finest level is exact; recovering a coarse level yields a
subsample, which is what a paint-only tile needs. Uncompressed tiles and the
finest-level polygon vertex list remain the ADR 0062 follow-ups; a format change bumps
`RTLA_VERSION` and the preamble is where a future reader learns the block extents
before trusting them.
