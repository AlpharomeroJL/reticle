//! A deterministic hash of a document, for transcript replay verification.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::{Cell, Document, DrawShape, ShapeKind};

/// A deterministic hash of the document's content.
///
/// Cells are hashed in name order (the document stores them in a hash map), and
/// within each cell its shapes, instances, arrays, labels, and pins are hashed in
/// stored order, along with the top-cell list. Re-executing an identical command
/// sequence rebuilds the same document and so reproduces this hash, which is the
/// transcript replay contract. It uses the standard library's fixed-key
/// `DefaultHasher`, so it is stable across processes for the same input.
#[must_use]
pub fn document_hash(doc: &Document) -> u64 {
    let mut h = DefaultHasher::new();
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
