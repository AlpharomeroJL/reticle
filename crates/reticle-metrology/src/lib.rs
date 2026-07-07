//! Exact CPU metrology reports over a Reticle [`Document`](reticle_model::Document).
//!
//! This crate turns a laid-out document into a small set of quantitative reports,
//! all computed on the CPU with exact integer geometry (no GPU, no sampling).
//! Reports operate on the flattened top cell and never mutate the document.
//!
//! The reports land incrementally (see `docs/src/metrology.md`):
//!
//! - Area and perimeter per layer (union area and union boundary length).
//! - Connectivity statistics (net count, shapes per net, maximum fanout).
//! - A simplified per-net antenna ratio over a SKY130 layer subset.
//! - Export of the combined report to CSV and Markdown.
//!
//! Everything here reads the public APIs of `reticle-model`, `reticle-geometry`,
//! and `reticle-extract`.
//!
//! ```
//! use reticle_geometry::{LayerId, Point, Rect};
//! use reticle_model::{Cell, Document, DrawShape, ShapeKind};
//!
//! let metal = LayerId::new(68, 20);
//! let mut cell = Cell::new("top");
//! cell.shapes.push(DrawShape::new(
//!     metal,
//!     ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(10, 10))),
//! ));
//! let mut doc = Document::new();
//! doc.insert_cell(cell);
//! doc.set_top_cells(vec!["top".into()]);
//!
//! let layers = reticle_metrology::area::report(&doc);
//! assert_eq!(layers.len(), 1);
//! assert_eq!(layers[0].area, 100.0);
//! assert_eq!(layers[0].perimeter, 40.0);
//! ```

pub mod antenna;
pub mod area;
pub mod connectivity;

mod polyize;

pub use antenna::{AntennaCheck, AntennaViolation};
pub use area::{LayerMetrics, report as area_report};
pub use connectivity::ConnectivityStats;

/// Returns the name of the first top cell, if the document declares one.
///
/// Metrology reports operate on the flattened top cell. When a document declares
/// no top cell there is nothing to measure and reports return empty results.
#[must_use]
pub fn top_cell(doc: &reticle_model::Document) -> Option<&str> {
    doc.top_cells().first().map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::top_cell;
    use reticle_model::{Cell, Document};

    #[test]
    fn top_cell_is_none_without_a_declared_top() {
        let doc = Document::new();
        assert_eq!(top_cell(&doc), None);
    }

    #[test]
    fn top_cell_returns_the_first_declared_top() {
        let mut doc = Document::new();
        doc.insert_cell(Cell::new("top"));
        doc.set_top_cells(vec!["top".into()]);
        assert_eq!(top_cell(&doc), Some("top"));
    }
}
