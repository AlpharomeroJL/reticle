//! Zero-copy access to archived index payloads with [`rkyv`] 0.8, the building
//! block for out-of-core streaming.
//!
//! An [`IndexPayload`] serializes to a byte buffer laid out exactly as its in-memory
//! representation, so a caller reads shape rectangles straight from those bytes with
//! no parsing or heap allocation, and can index a single archived entry in place. That
//! is precisely what a memory-mapped, larger-than-RAM layout would sit on: map a file,
//! hand these bytes in, and the OS pages in only the regions actually touched.
//!
//! What is **not** yet wired up: nothing in the workspace memory-maps a file into this
//! path or streams from disk, and no renderer consumes it, today the API is exercised
//! over in-memory buffers only. Full out-of-core browsing (mmap plus on-demand tile
//! paging) is a documented follow-up, see `docs/STATUS.md` and
//! `docs/decisions/0013-out-of-core-streaming-scope.md`. The zero-copy read primitive
//! itself is real and validated (below).
//!
//! # Safety
//!
//! This module is entirely safe Rust. It uses `rkyv`'s validated
//! [`access`](rkyv::access) entry point (enabled by `rkyv`'s default `bytecheck`
//! feature), which checks the byte buffer before handing back a reference, so a
//! truncated or corrupt file yields a [`StreamError`] rather than undefined
//! behaviour. No `unsafe` and no `access_unchecked` are used.

use reticle_geometry::{Point, Rect};
use rkyv::{Archive, Deserialize, Serialize, rancor};

/// A rectangle in a form `rkyv` can archive, mirroring [`Rect`] as four `i32`
/// corners. Convert with [`ArchivableRect::from_rect`] / [`ArchivableRect::to_rect`].
#[derive(Archive, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[rkyv(derive(Debug))]
pub struct ArchivableRect {
    /// Minimum x corner, in DBU.
    pub min_x: i32,
    /// Minimum y corner, in DBU.
    pub min_y: i32,
    /// Maximum x corner, in DBU.
    pub max_x: i32,
    /// Maximum y corner, in DBU.
    pub max_y: i32,
}

impl ArchivableRect {
    /// Converts a [`Rect`] into its archivable form.
    #[must_use]
    pub fn from_rect(r: Rect) -> Self {
        Self {
            min_x: r.min.x,
            min_y: r.min.y,
            max_x: r.max.x,
            max_y: r.max.y,
        }
    }

    /// Reconstructs the [`Rect`] this was built from.
    #[must_use]
    pub fn to_rect(self) -> Rect {
        Rect::new(
            Point::new(self.min_x, self.min_y),
            Point::new(self.max_x, self.max_y),
        )
    }
}

/// A serializable index payload: a flat list of `(bounding box, item id)` entries.
///
/// The `u32` item id is a handle into the caller's shape table. This is the unit
/// that gets archived to disk and memory-mapped; see the [module docs](self).
#[derive(Archive, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[rkyv(derive(Debug))]
pub struct IndexPayload {
    /// The indexed entries, each a rectangle paired with an item id.
    pub entries: Vec<(ArchivableRect, u32)>,
}

impl IndexPayload {
    /// Creates a payload from `(bbox, id)` pairs.
    pub fn from_entries(entries: impl IntoIterator<Item = (Rect, u32)>) -> Self {
        Self {
            entries: entries
                .into_iter()
                .map(|(r, id)| (ArchivableRect::from_rect(r), id))
                .collect(),
        }
    }

    /// The entries as `(Rect, u32)` pairs (allocates a fresh vector).
    #[must_use]
    pub fn to_entries(&self) -> Vec<(Rect, u32)> {
        self.entries
            .iter()
            .map(|(r, id)| (r.to_rect(), *id))
            .collect()
    }
}

/// An error from serializing or accessing a streamed index payload.
#[derive(Debug)]
pub struct StreamError(rancor::Error);

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "index streaming error: {}", self.0)
    }
}

impl std::error::Error for StreamError {}

impl From<rancor::Error> for StreamError {
    fn from(e: rancor::Error) -> Self {
        Self(e)
    }
}

/// Serializes a payload to an aligned byte buffer suitable for writing to a file
/// and later memory-mapping.
///
/// The returned bytes begin at an alignment `rkyv` requires for zero-copy access;
/// write them to disk verbatim and map the file at a matching alignment.
pub fn serialize(payload: &IndexPayload) -> Result<Vec<u8>, StreamError> {
    let bytes = rkyv::to_bytes::<rancor::Error>(payload)?;
    Ok(bytes.to_vec())
}

/// Validates `bytes` and returns a zero-copy reference to the archived payload.
///
/// No data is copied or deserialized: entries are read in place from the buffer,
/// which may be a memory-mapped file. The buffer must be aligned as produced by
/// [`serialize`]. Returns [`StreamError`] if the bytes are not a valid archive.
pub fn access(bytes: &[u8]) -> Result<&ArchivedIndexPayload, StreamError> {
    Ok(rkyv::access::<ArchivedIndexPayload, rancor::Error>(bytes)?)
}

/// Reads the `index`-th entry of an archived payload as a `(Rect, u32)` pair
/// without deserializing the whole payload, or `None` if out of range.
///
/// This is the zero-copy fast path: the archived integers are decoded directly
/// from the mapped bytes.
#[must_use]
pub fn entry_at(archived: &ArchivedIndexPayload, index: usize) -> Option<(Rect, u32)> {
    // Archived tuples become `ArchivedTuple2` with fields `.0` and `.1`, and
    // archived integers are little-endian wrappers convertible with `From`.
    let entry = archived.entries.get(index)?;
    let rect = &entry.0;
    let bbox = Rect::new(
        Point::new(i32::from(rect.min_x), i32::from(rect.min_y)),
        Point::new(i32::from(rect.max_x), i32::from(rect.max_y)),
    );
    Some((bbox, u32::from(entry.1)))
}

/// The number of entries in an archived payload, read without deserializing.
#[must_use]
pub fn len(archived: &ArchivedIndexPayload) -> usize {
    archived.entries.len()
}

/// Validates and fully deserializes `bytes` back into an owned [`IndexPayload`].
///
/// Use this when you need an owned, mutable copy; prefer [`access`] + [`entry_at`]
/// for read-only streaming, which avoids the allocation.
pub fn load(bytes: &[u8]) -> Result<IndexPayload, StreamError> {
    Ok(rkyv::from_bytes::<IndexPayload, rancor::Error>(bytes)?)
}
