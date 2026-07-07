//! The `.rtla` streamed-archive format (v1) and the [`TileSource`] transport seam.
//!
//! This module is the **frozen Wave 2 contract** (ADR 0062): it defines the byte
//! layout and the read seam that lane 2A (the builder, [`crate::archive_build`])
//! and lane 2B (the sources, [`crate::tile_source`]) implement against so they can
//! be developed in parallel without either blocking on the other. The types here
//! are complete; the builder and the concrete sources are not (see those modules).
//!
//! # Why a new container, not [`TiledPayload`](crate::streaming::TiledPayload)
//!
//! [`TiledPayload`](crate::streaming::TiledPayload) stores only `(bbox, u32 id)` per
//! entry and has no level-of-detail
//! structure or renderable payload: it answers a spatial query, it does not carry the
//! geometry to draw. A `.rtla` archive is a network transport for *renderable* silicon
//! at many zoom levels, so it needs a multi-level tile pyramid whose tiles carry packed
//! shape records, each tile independently fetchable and independently validatable.
//!
//! # Layout (`.rtla` v1)
//!
//! ```text
//! [ header block         ] rkyv-archived RtlaHeader (magic, version, world bbox,
//!                          technology blob, per-level grid dims)
//! [ tile directory        ] rkyv-archived Vec<TileDirEntry> (offset,len per tile,
//!                          level-major then row-major within a level)
//! [ tile 0 ][ tile 1 ] ... each an rkyv-archived TilePayload, byte-contiguous, so a
//!                          single HTTP Range request over [offset, offset+len) yields
//!                          exactly one independently-validatable tile.
//! ```
//!
//! The finest level holds exact geometry; coarser levels hold decimated,
//! paint-only approximations. Spatial queries always resolve against the finest
//! level; coarse tiles are for drawing while fine tiles stream in. Uncompressed in
//! v1 (Range requests and block compression do not compose trivially); compression
//! is a documented follow-up.
//!
//! # Read-mostly scope
//!
//! A streamed archive is browse/measure/query/share only. Editing stays on in-RAM
//! documents; there is deliberately no mutation path through a [`TileSource`]. That
//! scope line is enforced at the app layer ([`crate`] has no opinion) and stated in
//! the book.

use reticle_geometry::Rect;
use rkyv::{Archive, Deserialize, Serialize};

use crate::streaming::ArchivableRect;

/// Magic bytes at the start of the archived [`RtlaHeader`], so a fetched header can
/// be sanity-checked before trusting its fields. ASCII `"RTLA1\0\0\0"`.
pub const RTLA_MAGIC: [u8; 8] = *b"RTLA1\0\0\0";

/// The `.rtla` format version this build reads and writes.
pub const RTLA_VERSION: u32 = 1;

/// One packed, renderable rectangle inside a tile: a layer/datatype pair plus the
/// four integer corners. The transport unit of a streamed archive; a polygon is
/// carried as its bounding-box record at coarse levels and by a companion vertex
/// list (a follow-up field) at the finest level.
#[derive(Archive, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[rkyv(derive(Debug))]
pub struct TileRecord {
    /// GDSII layer number.
    pub layer: u16,
    /// GDSII datatype number.
    pub datatype: u16,
    /// The rectangle, in database units.
    pub rect: ArchivableRect,
}

/// The payload of a single tile: the records that fall in it at one level. Each tile
/// is archived independently so one Range fetch validates standalone.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[rkyv(derive(Debug))]
pub struct TilePayload {
    /// The renderable records in this tile.
    pub records: Vec<TileRecord>,
}

/// The grid dimensions of one level of the pyramid: a `cols x rows` tiling of the
/// world bounding box. Level 0 is the coarsest (few tiles); the finest level has the
/// most.
#[derive(Archive, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[rkyv(derive(Debug))]
pub struct LevelDims {
    /// Tile columns across the world bbox at this level.
    pub cols: u32,
    /// Tile rows down the world bbox at this level.
    pub rows: u32,
}

/// The archive header: everything a client needs to map a viewport to the tiles that
/// cover it, without reading any tile.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[rkyv(derive(Debug))]
pub struct RtlaHeader {
    /// [`RTLA_MAGIC`]; checked on read.
    pub magic: [u8; 8],
    /// [`RTLA_VERSION`]; a reader refuses a version it does not understand.
    pub version: u32,
    /// The world bounding box every level tiles, in database units.
    pub world: ArchivableRect,
    /// Database units per micron, carried so a streamed document has a scale without
    /// a separate technology fetch.
    pub dbu_per_micron: i64,
    /// Grid dimensions per level, coarsest (index 0) to finest (last).
    pub levels: Vec<LevelDims>,
}

impl RtlaHeader {
    /// The number of pyramid levels.
    #[must_use]
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// The world bounding box as a geometry [`Rect`].
    #[must_use]
    pub fn world_rect(&self) -> Rect {
        self.world.to_rect()
    }
}

/// One entry in the tile directory: where a tile's archived [`TilePayload`] lives in
/// the file, as a byte range `[offset, offset + len)`. Directory order is level-major
/// (all of level 0's tiles, then level 1's, ...) and row-major within a level.
#[derive(Archive, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[rkyv(derive(Debug))]
pub struct TileDirEntry {
    /// Byte offset of the tile's archived payload from the start of the file.
    pub offset: u64,
    /// Byte length of the tile's archived payload.
    pub len: u64,
}

/// A tile's address in the pyramid: `(level, column, row)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileCoord {
    /// Pyramid level, 0 = coarsest.
    pub level: u32,
    /// Tile column at this level.
    pub col: u32,
    /// Tile row at this level.
    pub row: u32,
}

/// The transport seam over a `.rtla` archive: fetch the header, and fetch one tile's
/// raw archived bytes by address. Native mmap and wasm HTTP-Range sources implement
/// it (lane 2B, [`crate::tile_source`]); a consumer (lane 2C) drives residency
/// through it and stays agnostic to native-vs-wasm.
///
/// It is deliberately **async**: the wasm implementation fetches over the network,
/// which cannot block. The native mmap implementation completes immediately. The
/// `async_fn_in_trait` lint is allowed because sources are always used as concrete or
/// generic types (never `dyn`), and `Send`-ness is not required on the single-threaded
/// wasm target.
///
/// A returned tile's bytes are an independently-archived [`TilePayload`]; the caller
/// validates them with `rkyv::access` exactly as [`crate::streaming`] does, so a
/// corrupt or truncated tile yields an error rather than undefined behaviour.
#[allow(async_fn_in_trait)]
pub trait TileSource {
    /// Fetch and return the archive header. Called once when a document opens.
    async fn header(&self) -> Result<RtlaHeader, TileSourceError>;

    /// Fetch the raw archived bytes of the tile at `coord`. The bytes are a complete,
    /// independently-validatable [`TilePayload`] archive.
    async fn tile_bytes(&self, coord: TileCoord) -> Result<Vec<u8>, TileSourceError>;
}

/// Why a [`TileSource`] fetch failed.
#[derive(Debug)]
pub enum TileSourceError {
    /// The header or a tile was not a valid archive (bad magic, unknown version, or a
    /// `rkyv` validation failure on truncated or corrupt bytes).
    Malformed(String),
    /// The transport itself failed (I/O on native, a fetch/Range error on wasm).
    Transport(String),
    /// The requested tile address is outside the archive's directory.
    OutOfRange(TileCoord),
}

impl std::fmt::Display for TileSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(m) => write!(f, "rtla archive malformed: {m}"),
            Self::Transport(m) => write!(f, "rtla transport error: {m}"),
            Self::OutOfRange(c) => write!(
                f,
                "rtla tile out of range: level {} ({}, {})",
                c.level, c.col, c.row
            ),
        }
    }
}

impl std::error::Error for TileSourceError {}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::Point;

    #[test]
    fn header_accessors_reflect_fields() {
        let header = RtlaHeader {
            magic: RTLA_MAGIC,
            version: RTLA_VERSION,
            world: ArchivableRect::from_rect(Rect::new(Point::new(0, 0), Point::new(100, 200))),
            dbu_per_micron: 1000,
            levels: vec![
                LevelDims { cols: 1, rows: 1 },
                LevelDims { cols: 4, rows: 4 },
            ],
        };
        assert_eq!(header.level_count(), 2);
        assert_eq!(
            header.world_rect(),
            Rect::new(Point::new(0, 0), Point::new(100, 200))
        );
        assert_eq!(header.magic, RTLA_MAGIC);
    }

    #[test]
    fn tile_payload_round_trips_through_rkyv() {
        // Prove the frozen tile format archives and validates, so 2A (writer) and 2B
        // (reader) share a working interop unit from the contract commit.
        let payload = TilePayload {
            records: vec![TileRecord {
                layer: 68,
                datatype: 20,
                rect: ArchivableRect::from_rect(Rect::new(Point::new(1, 2), Point::new(3, 4))),
            }],
        };
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&payload).expect("archive");
        let archived =
            rkyv::access::<ArchivedTilePayload, rkyv::rancor::Error>(&bytes).expect("validate");
        assert_eq!(archived.records.len(), 1);
        assert_eq!(archived.records[0].layer, 68);
    }

    #[test]
    fn tile_source_error_displays() {
        let e = TileSourceError::OutOfRange(TileCoord {
            level: 2,
            col: 5,
            row: 1,
        });
        assert!(e.to_string().contains("level 2"));
    }
}
