//! Connectivity statistics from geometric net extraction.
//!
//! [`stats`] extracts nets from the flattened top cell with [`reticle_extract`]
//! and summarizes them: how many nets there are, how many shapes each holds, and
//! the largest net's shape count. Extraction uses the SKY130 via/contact stack
//! ([`reticle_extract::sky130_connection_rules`]) so shapes on different
//! conductor layers joined by a via count as one net; every other layer connects
//! same-layer only.

use reticle_extract::{Extractor, sky130_connection_rules};
use reticle_model::Document;

/// Summary of a document's extracted connectivity.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ConnectivityStats {
    /// Number of distinct nets (connected components).
    pub net_count: usize,
    /// Shapes per net, sorted from largest to smallest.
    pub net_sizes: Vec<usize>,
    /// Total shapes across all nets.
    pub total_shapes: usize,
    /// Shape count of the largest net; zero when there are no nets.
    pub max_fanout: usize,
}

/// Extracts connectivity for a document's flattened top cell and summarizes it.
///
/// A document with no declared top cell yields an all-zero, empty summary.
#[must_use]
pub fn stats(doc: &Document) -> ConnectivityStats {
    let Some(top) = crate::top_cell(doc) else {
        return ConnectivityStats {
            net_count: 0,
            net_sizes: Vec::new(),
            total_shapes: 0,
            max_fanout: 0,
        };
    };
    let flat = doc.flatten(top);
    let netlist = Extractor::new()
        .with_rules(sky130_connection_rules())
        .extract_shapes(&flat);

    let mut net_sizes: Vec<usize> = netlist.nets.iter().map(|n| n.shape_count).collect();
    net_sizes.sort_unstable_by(|a, b| b.cmp(a));
    let total_shapes = net_sizes.iter().sum();
    let max_fanout = net_sizes.first().copied().unwrap_or(0);

    ConnectivityStats {
        net_count: netlist.nets.len(),
        net_sizes,
        total_shapes,
        max_fanout,
    }
}

#[cfg(test)]
mod tests {
    use super::stats;
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{Cell, Document, DrawShape, ShapeKind};

    const MET1: LayerId = LayerId::new(68, 20);
    const VIA1: LayerId = LayerId::new(68, 44);
    const MET2: LayerId = LayerId::new(69, 20);

    fn rect(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    }

    fn doc_of(shapes: Vec<DrawShape>) -> Document {
        let mut cell = Cell::new("top");
        cell.shapes = shapes;
        let mut doc = Document::new();
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["top".into()]);
        doc
    }

    #[test]
    fn empty_document_has_no_nets() {
        let s = stats(&Document::new());
        assert_eq!(s.net_count, 0);
        assert_eq!(s.max_fanout, 0);
        assert!(s.net_sizes.is_empty());
    }

    #[test]
    fn two_disjoint_islands_are_two_nets() {
        let doc = doc_of(vec![
            rect(MET1, 0, 0, 10, 10),
            rect(MET1, 100, 100, 110, 110),
        ]);
        let s = stats(&doc);
        assert_eq!(s.net_count, 2);
        assert_eq!(s.net_sizes, vec![1, 1]);
        assert_eq!(s.total_shapes, 2);
        assert_eq!(s.max_fanout, 1);
    }

    #[test]
    fn overlapping_same_layer_shapes_merge() {
        let doc = doc_of(vec![rect(MET1, 0, 0, 10, 10), rect(MET1, 5, 5, 15, 15)]);
        let s = stats(&doc);
        assert_eq!(s.net_count, 1);
        assert_eq!(s.max_fanout, 2);
    }

    #[test]
    fn via_stack_joins_layers_into_one_net() {
        // met1 -> via1 -> met2 all overlapping: the via bridges the two metals.
        let doc = doc_of(vec![
            rect(MET1, 0, 0, 10, 10),
            rect(VIA1, 2, 2, 4, 4),
            rect(MET2, 0, 0, 10, 10),
        ]);
        let s = stats(&doc);
        assert_eq!(s.net_count, 1);
        assert_eq!(s.net_sizes, vec![3]);
        assert_eq!(s.max_fanout, 3);
    }
}
