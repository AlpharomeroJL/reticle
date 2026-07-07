# 0063, `.rtla` physical framing, and the wasm tile-source LRU and OPFS cache policy

## Context

ADR 0062 freezes the `.rtla` logical layout (a header block, a tile directory, then
byte-contiguous tiles) and the frozen types (`RtlaHeader`, `TileDirEntry`, `TilePayload`,
the `TileSource` trait), but not two things a concrete reader needs and a builder must
match: the exact byte framing that lets a reader locate each block, and, for the wasm
`HttpRangeTileSource`, the caching policy that makes revisiting an archive fast. Lane 2B
implements the readers against the frozen surface, so it fixes both here as the concrete
realization of the 0062 contract, in one place both the reader (lane 2B) and the external
builder (lane 2A) can converge on. Two forces shape the framing: `rkyv` zero-copy access
requires the mapped bytes of a block to be aligned, and an HTTP `Range` request must be
able to fetch exactly one tile. Two forces shape the cache: a browser tab has a bounded
memory budget, and OPFS persists across page loads but must never serve a stale tile when
an archive is rebuilt at the same URL.

## Decision

**Framing.** A `.rtla` file opens with a fixed 32-byte preamble: `RTLA_MAGIC` (8 bytes,
checkable before any parse), a little-endian `u32` version (a reader refuses an unknown
one), 4 reserved flag bytes, then the `u64` archived byte lengths of the header and the
directory. The header block starts at offset 32; the directory and every tile start at the
next 16-byte-aligned offset. 16-byte alignment means a memory-mapped block validates
zero-copy off the page-aligned map and a freshly allocated fetched block is aligned enough
for every archived type (they need at most 8). A `TileDirEntry`'s `offset` is an absolute,
16-aligned file offset and `len` is the tile's exact archived length, so one `Range`
request over `[offset, offset+len)` returns exactly one independently-validatable tile.

**Untrusted counts.** Every count or length from a header or directory is untrusted. Tile
counts are summed with checked `u64` arithmetic (overflow is an error, never a panic or a
giant allocation), a header whose level dimensions disagree with the actual directory
length is rejected as `Malformed`, and the header-plus-directory fetch is capped at 128
MiB. No `Vec` is ever reserved from such a count. A directory claiming billions of tiles
therefore errors in bounded time and memory, carrying the OASIS out-of-memory lesson
(commit 1b1b56b) into the reader; it is proven two ways (an inconsistent header, and a
truncated directory) in the crate's tests.

**Caches (wasm source).** In front of the network sit two caches. An in-memory
`LruByteCache` holds tiles under a fixed byte budget, evicting least-recently-used first;
a tile larger than the whole budget is simply not retained. Its eviction is pure,
target-independent logic, unit-tested off-wasm. Behind it, OPFS persists tiles per archive
under a directory named by a stable hash of the archive URL and its `ETag`: a rebuilt
archive (new `ETag`) derives a fresh key, so a stale tile is never served, and a revisit to
an unchanged archive reads tiles straight from disk. The cache-key derivation is pure and
unit-tested; the OPFS read/write and the `fetch` glue are the only `cfg(target_arch =
"wasm32")` code.

## Consequences

Lane 2A's builder writes exactly this framing so its archives read through lane 2B's
sources; the framing table lives in the `tile_source` module docs and here. The read seam
carries no allocation driven by an untrusted count, so a hostile or corrupt archive is a
clean error, not a crash. The wasm source's correctness rests on the pure LRU and cache-key
logic (unit-tested) plus the shared framing parsers (exercised natively through
`MmapTileSource` and the headline proptest); the browser end-to-end pass over real `fetch`
and OPFS is lane 2E/8's, noted honestly as the one unproven seam. Compression and the
finest-level polygon vertex list remain the 0062 follow-ups, additive to this framing (a
future change bumps `RTLA_VERSION`, which readers already refuse when unknown).
