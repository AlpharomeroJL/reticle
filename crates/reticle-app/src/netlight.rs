//! Net highlighting: connectivity extraction, cached, plus the highlighted set.
//!
//! When the Select tool picks a shape, the app asks this module for the set of shape
//! indices electrically connected to it. Extraction runs [`reticle_extract`] over the
//! *flattened* top cell, whose shape indices line up one-to-one with the flattened
//! shape list the [`crate::culling::SceneIndex`] holds, so a net's member indices are
//! directly usable as canvas highlight indices.
//!
//! Extraction is not free, so the [`Netlight`] caches the [`Netlist`] and re-extracts
//! only when the caller reports a new document generation. The generation is an
//! opaque monotonically-increasing token the app bumps on every edit (the same moment
//! it rebuilds the scene index), so undo/redo and edits all invalidate the cache. The
//! extraction and net-lookup logic are free of egui and unit-tested below.

use reticle_extract::{Extractor, Netlist};
use reticle_model::Document;

/// A monotonically-increasing token identifying a document revision.
///
/// The app increments it whenever the scene is rebuilt after an edit; the cache
/// compares it to decide whether the stored netlist is still valid.
pub type Generation = u64;

/// The net-highlight state: a cached netlist plus the currently highlighted indices.
#[derive(Debug, Default)]
pub struct Netlight {
    /// The cached netlist and the generation it was extracted at, if any.
    cache: Option<(Generation, Netlist)>,
    /// The shape indices of the currently highlighted net (empty when nothing is
    /// highlighted). Kept sorted and de-duplicated by the extractor.
    highlighted: Vec<usize>,
}

impl Netlight {
    /// Creates an empty net-highlight state (no cache, nothing highlighted).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The shape indices of the currently highlighted net.
    #[must_use]
    pub fn highlighted(&self) -> &[usize] {
        &self.highlighted
    }

    /// Whether shape `index` is part of the highlighted net.
    #[must_use]
    pub fn contains(&self, index: usize) -> bool {
        self.highlighted.binary_search(&index).is_ok()
    }

    /// Whether nothing is currently highlighted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.highlighted.is_empty()
    }

    /// Clears the highlighted net (a click on empty space).
    pub fn clear(&mut self) {
        self.highlighted.clear();
    }

    /// Returns the cached netlist for `generation`, extracting it if the cache is
    /// stale or empty.
    ///
    /// The flattened `top` cell of `doc` is extracted; the result is cached against
    /// `generation` so repeated picks at the same revision reuse it.
    fn netlist(&mut self, doc: &Document, top: &str, generation: Generation) -> &Netlist {
        let stale = self.cache.as_ref().is_none_or(|(g, _)| *g != generation);
        if stale {
            let netlist = extract_flattened(doc, top);
            self.cache = Some((generation, netlist));
        }
        &self.cache.as_ref().expect("just populated").1
    }

    /// Highlights the net containing the shape at `shape_index`.
    ///
    /// Extracts (or reuses the cached) connectivity for the flattened `top` cell at
    /// `generation` and stores the connected net's shape indices. Returns the number
    /// of shapes now highlighted; a shape on no net (or an out-of-range index)
    /// highlights just nothing and returns `0`.
    pub fn highlight_shape(
        &mut self,
        doc: &Document,
        top: &str,
        generation: Generation,
        shape_index: usize,
    ) -> usize {
        let net = self.netlist(doc, top, generation).net_of(shape_index);
        match net {
            Some(net) => {
                self.highlighted = net.shapes.clone();
            }
            None => self.highlighted.clear(),
        }
        self.highlighted.len()
    }
}

/// Extracts connectivity over the flattened `top` cell of `doc`.
///
/// Flattening makes the extracted shape indices match the flattened shape list used
/// by the scene index, so a returned net's indices are directly usable as highlight
/// indices. An unknown top cell yields an empty netlist.
#[must_use]
pub fn extract_flattened(doc: &Document, top: &str) -> Netlist {
    let shapes = doc.flatten(top);
    Extractor::new().extract_shapes(&shapes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{Cell, DrawShape, ShapeKind};

    /// A flat document with two touching metal rects (one net) and a separate,
    /// disjoint rect (its own net), so connectivity is easy to reason about.
    fn two_net_doc() -> Document {
        let m = LayerId::new(4, 0);
        let rect = |x0, y0, x1, y1| {
            DrawShape::new(
                m,
                ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
            )
        };
        let mut cell = Cell::new("TOP");
        // Shapes 0 and 1 overlap -> same net. Shape 2 is far away -> its own net.
        cell.shapes.push(rect(0, 0, 100, 100));
        cell.shapes.push(rect(50, 50, 200, 200));
        cell.shapes.push(rect(1000, 1000, 1100, 1100));
        let mut doc = Document::new();
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["TOP".to_owned()]);
        doc
    }

    #[test]
    fn highlight_returns_connected_shapes() {
        let doc = two_net_doc();
        let mut nl = Netlight::new();
        // Clicking shape 0 highlights the {0, 1} net.
        let n = nl.highlight_shape(&doc, "TOP", 1, 0);
        assert_eq!(n, 2);
        assert!(nl.contains(0));
        assert!(nl.contains(1));
        assert!(!nl.contains(2), "disjoint shape must not be highlighted");
    }

    #[test]
    fn highlight_isolated_shape_is_its_own_net() {
        let doc = two_net_doc();
        let mut nl = Netlight::new();
        let n = nl.highlight_shape(&doc, "TOP", 1, 2);
        assert_eq!(n, 1);
        assert!(nl.contains(2));
        assert!(!nl.contains(0));
    }

    #[test]
    fn out_of_range_pick_highlights_nothing() {
        let doc = two_net_doc();
        let mut nl = Netlight::new();
        let n = nl.highlight_shape(&doc, "TOP", 1, 9999);
        assert_eq!(n, 0);
        assert!(nl.is_empty());
    }

    #[test]
    fn clear_empties_highlight() {
        let doc = two_net_doc();
        let mut nl = Netlight::new();
        nl.highlight_shape(&doc, "TOP", 1, 0);
        assert!(!nl.is_empty());
        nl.clear();
        assert!(nl.is_empty());
    }

    #[test]
    fn cache_reused_within_a_generation_and_refreshed_after() {
        let doc = two_net_doc();
        let mut nl = Netlight::new();
        // First pick at generation 1 populates the cache.
        nl.highlight_shape(&doc, "TOP", 1, 0);
        let gen1 = nl.cache.as_ref().map(|(g, _)| *g);
        assert_eq!(gen1, Some(1));
        // A second pick at the same generation keeps the same cached netlist.
        nl.highlight_shape(&doc, "TOP", 1, 2);
        assert_eq!(nl.cache.as_ref().map(|(g, _)| *g), Some(1));
        // A pick at a newer generation re-extracts and re-tags the cache.
        nl.highlight_shape(&doc, "TOP", 5, 0);
        assert_eq!(nl.cache.as_ref().map(|(g, _)| *g), Some(5));
    }

    #[test]
    fn extract_flattened_matches_scene_indices() {
        // The extracted netlist indexes the same flattened shape list the scene uses,
        // so a net's member index is a valid index into `doc.flatten(top)`.
        let doc = two_net_doc();
        let flat = doc.flatten("TOP");
        let netlist = extract_flattened(&doc, "TOP");
        for net in &netlist.nets {
            for &i in &net.shapes {
                assert!(i < flat.len(), "net index {i} out of flattened range");
            }
        }
    }
}
