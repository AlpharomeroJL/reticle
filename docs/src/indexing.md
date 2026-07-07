# Spatial indexing

`reticle-index` answers the queries that make browsing a massive layout
interactive: which shapes fall in the view, which shape is nearest the cursor, and
which tiles to stream at the current zoom.

## Indices

- **R-tree.** A bulk-loaded R-tree (`rstar`) is the primary index for rectangle,
  nearest-edge, and k-nearest queries. Bulk loading packs the tree in one pass,
  which is far faster than repeated insertion and yields better query performance.
- **Uniform grid.** For uniformly distributed geometry, a uniform grid buckets
  shapes by cell and answers rectangle queries by scanning the covered cells. It is
  cheap to build and update.
- **Tile and LOD pyramid.** Shapes are bucketed into tiles at several levels of
  detail. A renderer requests only the tiles inside the view at the level appropriate
  to the zoom, so memory and draw work scale with what is on screen rather than with
  the size of the design.

All indices implement the shared `SpatialIndex` trait, so callers are generic over
the structure. A brute-force `LinearIndex` implements the same trait and serves as
the oracle the fast indices are property-tested against.

## Zero-copy archive (the building block for out-of-core)

An index payload serializes to a zero-copy `rkyv` archive laid out exactly as its
in-memory form, so a caller can read shape rectangles, and index a single entry -
straight from the bytes with no parsing or allocation, validated by `rkyv`'s
`bytecheck`. This is the primitive a memory-mapped, larger-than-RAM layout would sit
on. The disk/mmap paging layer and its renderer integration are **not yet wired up**
today (the API is exercised over in-memory buffers); the out-of-core streaming ADR and
`STATUS.md` record this precisely.

## Streaming a `.rtla` archive over a `TileSource`

The `.rtla` container (ADR 0062) turns a layout into a network transport for renderable
silicon: a header with the world box and per-level grid dimensions, a tile directory of
byte ranges, then byte-contiguous tiles, each an independently-validated `rkyv`
`TilePayload`. A `TileSource` is the read seam over one such archive: fetch the header,
fetch one tile's bytes by address. Three sources implement it. `MmapTileSource` maps a
local file and serves a tile by slicing the mapped range, reusing the zero-copy archive
discipline above (the one documented `unsafe`, every block validated). `HttpRangeTileSource`
runs in the browser: it fetches the header with two ranged GETs, then a tile per `fetch`
with a `Range: bytes=offset-end` header, in front of a byte-budgeted in-memory LRU and an
OPFS persistent cache so revisiting an archive is instant (ADR 0063). `MemTileSource` is an
in-memory double for tests.

A small query layer resolves a viewport against the archive's finest level (coarser levels
are paint-only), fetches the overlapping tiles, and returns the records that intersect the
view. That streamed result is proven equal to the in-RAM R-tree's answer over 600
randomized layouts. Every count read from a header or directory is untrusted: tile counts
are summed with checked arithmetic, a header inconsistent with its directory is rejected,
and the header fetch is capped, so a directory claiming billions of tiles errors rather
than exhausting memory. The physical byte framing and the OPFS/LRU cache policy are fixed
in ADR 0063.

## Targets

The bulk index build of one million shapes should complete in well under a second,
and point or rubber-band picking over a million shapes should return in under a
millisecond. Measured numbers are in the [performance chapter](performance.md).
