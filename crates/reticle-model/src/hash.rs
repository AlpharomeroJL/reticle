//! A deterministic hash of a document, for transcript replay verification.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::{Cell, Document, DrawShape, ShapeKind};

/// A [`Hasher`] adapter that writes every pointer-width integer (`usize`/`isize`)
/// as a fixed 64-bit value, so [`document_hash`] is identical on 64-bit native and
/// 32-bit wasm.
///
/// The document hash must match across the platforms that record and replay a
/// transcript: a run is recorded natively, but the public web build replays it on
/// wasm32. `usize`/`isize` are 8 bytes on `x86_64` and 4 bytes on wasm32, and the
/// standard `Hash` impls feed a pointer-width integer to the hasher for every
/// length (a slice or `Vec` writes a length prefix; an explicit `.len()` hashes a
/// `usize`). That made the byte stream, and therefore the final hash, differ
/// between native and wasm, so a run recorded on native replayed to a MISMATCH in
/// the browser. Forcing those writes to 64 bits removes the only platform
/// dependency (`Dbu` is `i32`, `LayerId` is `u16`, magnification is `u32/u32`, so
/// every other hashed field is already fixed width).
///
/// On a 64-bit host `write_usize(x)` already emits the same eight little-endian
/// bytes as `write_u64(x)`, so this adapter is a no-op for the native byte stream
/// and preserves every hash value recorded on native (the replay contract). On
/// wasm it widens the four-byte length writes to eight, so wasm now agrees.
struct FixedWidth<H>(H);

impl<H: Hasher> Hasher for FixedWidth<H> {
    fn finish(&self) -> u64 {
        self.0.finish()
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0.write(bytes);
    }

    fn write_usize(&mut self, i: usize) {
        self.0.write_u64(i as u64);
    }

    fn write_isize(&mut self, i: isize) {
        self.0.write_i64(i as i64);
    }
}

/// A deterministic hash of the document's content.
///
/// Cells are hashed in name order (the document stores them in a hash map), and
/// within each cell its shapes, instances, arrays, labels, and pins are hashed in
/// stored order, along with the top-cell list. Re-executing an identical command
/// sequence rebuilds the same document and so reproduces this hash, which is the
/// transcript replay contract. It uses the standard library's fixed-key
/// `DefaultHasher` behind a fixed-width adapter, so it is stable across processes AND
/// across 64-bit native and 32-bit wasm for the same input.
#[must_use]
pub fn document_hash(doc: &Document) -> u64 {
    let mut h = FixedWidth(DefaultHasher::new());
    let mut cells: Vec<&Cell> = doc.cells().collect();
    cells.sort_by(|a, b| a.name.cmp(&b.name));
    for cell in cells {
        cell.name.hash(&mut h);
        cell.shapes.len().hash(&mut h);
        for s in &cell.shapes {
            hash_shape(s, &mut h);
        }
        cell.instances.len().hash(&mut h);
        for inst in &cell.instances {
            inst.cell.hash(&mut h);
            inst.transform.hash(&mut h);
        }
        cell.arrays.len().hash(&mut h);
        for a in &cell.arrays {
            a.cell.hash(&mut h);
            a.transform.hash(&mut h);
            a.columns.hash(&mut h);
            a.rows.hash(&mut h);
            a.column_pitch.hash(&mut h);
            a.row_pitch.hash(&mut h);
        }
        cell.labels.hash(&mut h);
        cell.pins.hash(&mut h);
    }
    doc.top_cells().hash(&mut h);
    h.finish()
}

/// Feeds a shape's layer and geometry to the hasher, tagged by kind so a rectangle
/// and a polygon with the same coordinates hash differently.
fn hash_shape(shape: &DrawShape, h: &mut impl Hasher) {
    shape.layer.hash(h);
    match &shape.kind {
        ShapeKind::Rect(r) => {
            0u8.hash(h);
            r.hash(h);
        }
        ShapeKind::Polygon(p) => {
            1u8.hash(h);
            p.hash(h);
        }
        ShapeKind::Path(p) => {
            2u8.hash(h);
            p.hash(h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `FixedWidth` adapter must widen a `usize` write to the same eight bytes a
    /// `u64` write produces, so a length hashes identically to its 64-bit value.
    /// This is the mechanism that makes wasm (4-byte usize) agree with native
    /// (8-byte usize): the document hash's only pointer-width inputs are lengths,
    /// and this normalizes them.
    #[test]
    fn fixed_width_hashes_usize_as_u64() {
        let mut via_usize = FixedWidth(DefaultHasher::new());
        7usize.hash(&mut via_usize);
        let mut via_u64 = FixedWidth(DefaultHasher::new());
        7u64.hash(&mut via_u64);
        assert_eq!(via_usize.finish(), via_u64.finish());

        // A slice's hidden length prefix normalizes the same way, so a Vec/slice
        // hashes to the same value regardless of pointer width.
        let mut slice_hasher = FixedWidth(DefaultHasher::new());
        [10i32, 20, 30].hash(&mut slice_hasher);
        let mut manual = FixedWidth(DefaultHasher::new());
        3u64.hash(&mut manual);
        10i32.hash(&mut manual);
        20i32.hash(&mut manual);
        30i32.hash(&mut manual);
        assert_eq!(slice_hasher.finish(), manual.finish());
    }

    /// On this 64-bit host the adapter must be a pure no-op for the byte stream:
    /// `write_usize(x)` already equals `write_u64(x)`, so wrapping a `DefaultHasher`
    /// changes no native hash value (the recorded-transcript contract holds).
    #[test]
    fn fixed_width_is_identity_on_the_native_byte_stream() {
        let sample: &[usize] = &[0, 1, 42, 12345];
        let mut wrapped = FixedWidth(DefaultHasher::new());
        let mut bare = DefaultHasher::new();
        for &v in sample {
            v.hash(&mut wrapped);
            // Bare hasher: usize hashes as its native width, which is 64-bit here.
            v.hash(&mut bare);
        }
        assert_eq!(wrapped.finish(), bare.finish());
    }
}
