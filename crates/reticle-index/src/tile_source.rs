//! Concrete [`TileSource`](crate::archive::TileSource) implementations (Wave 2 lane 2B).
//!
//! **Contract stub.** ADR 0062 freezes the [`TileSource`](crate::archive::TileSource)
//! trait in [`crate::archive`]; this module holds its implementations, which lane 2B
//! fills:
//!
//! - `MmapTileSource` (native): wraps a memory-mapped `.rtla` file, reusing the
//!   [`crate::streaming`] mmap discipline (validated `rkyv` access, the one `unsafe`).
//! - `HttpRangeTileSource` (wasm): `fetch` with a `Range` header per tile, an
//!   in-memory LRU tile cache under a byte budget, and OPFS as the persistent cache
//!   so revisiting an archive is instant.
//! - `MemTileSource` (tests): an in-memory map of tiles, the property-test double
//!   that proves a streamed query equals the in-RAM R-tree query.
//!
//! Requirement carried from the contract: every count or length field read from the
//! header or a tile is untrusted; never reserve capacity from it beyond what the
//! remaining input can hold (the OASIS OOM lesson, commit 1b1b56b).
//!
//! This file is intentionally empty of items at the contract commit so that lane 2B
//! owns it exclusively and lane 2A owns [`crate::archive_build`] exclusively; neither
//! edits `lib.rs`, so the two lanes never touch the same file.
