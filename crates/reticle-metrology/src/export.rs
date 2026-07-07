//! The combined metrology report and its CSV and Markdown renderings.
//!
//! [`MetrologyReport`] bundles the per-layer area/perimeter table, the
//! connectivity summary, and the antenna screen. [`MetrologyReport::to_csv`] and
//! [`MetrologyReport::to_markdown`] render it deterministically: the same
//! document and threshold always produce byte-identical output (newline-only line
//! endings, fixed number formatting), so the rendering is safe to diff and to pin
//! with a golden test.

use std::fmt::Write as _;

use reticle_model::Document;

use crate::antenna::{self, AntennaCheck};
use crate::area::{self, LayerMetrics};
use crate::connectivity::{self, ConnectivityStats};

/// The three metrology reports for one document, gathered for export.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct MetrologyReport {
    /// Per-layer area and perimeter, ordered by layer.
    pub layers: Vec<LayerMetrics>,
    /// Connectivity statistics.
    pub connectivity: ConnectivityStats,
    /// Antenna screen result at the requested threshold.
    pub antenna: AntennaCheck,
}

impl MetrologyReport {
    /// Computes every report for a document's flattened top cell.
    ///
    /// `antenna_threshold` is the ratio above which the antenna screen flags a
    /// net (see [`crate::antenna`]).
    #[must_use]
    pub fn generate(doc: &Document, antenna_threshold: f64) -> Self {
        Self {
            layers: area::report(doc),
            connectivity: connectivity::stats(doc),
            antenna: antenna::check(doc, antenna_threshold),
        }
    }

    /// Renders the report as CSV: three `#`-commented sections (layers,
    /// connectivity, antenna), newline-terminated lines throughout.
    #[must_use]
    pub fn to_csv(&self) -> String {
        let mut s = String::new();

        s.push_str("# layers\n");
        s.push_str("layer,area,perimeter,shape_count\n");
        for m in &self.layers {
            let _ = writeln!(
                s,
                "{},{},{},{}",
                layer_id(m),
                num(m.area),
                num(m.perimeter),
                m.shape_count
            );
        }

        s.push_str("\n# connectivity\n");
        s.push_str("net_count,total_shapes,max_fanout\n");
        let c = &self.connectivity;
        let _ = writeln!(s, "{},{},{}", c.net_count, c.total_shapes, c.max_fanout);

        let _ = write!(
            s,
            "\n# antenna (threshold={})\n",
            num(self.antenna.threshold)
        );
        s.push_str("net,gate_area,metal_area,ratio\n");
        for v in &self.antenna.violations {
            let _ = writeln!(
                s,
                "{},{},{},{}",
                v.net,
                num(v.gate_area),
                num(v.metal_area),
                num(v.ratio)
            );
        }

        s
    }

    /// Renders the report as Markdown with a heading and a table per section.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut s = String::new();
        s.push_str("# Metrology report\n\n");

        s.push_str("## Layers\n\n");
        s.push_str("| Layer | Area | Perimeter | Shapes |\n");
        s.push_str("| --- | ---: | ---: | ---: |\n");
        for m in &self.layers {
            let _ = writeln!(
                s,
                "| {} | {} | {} | {} |",
                layer_id(m),
                num(m.area),
                num(m.perimeter),
                m.shape_count
            );
        }

        let c = &self.connectivity;
        s.push_str("\n## Connectivity\n\n");
        s.push_str("| Nets | Total shapes | Max fanout |\n");
        s.push_str("| ---: | ---: | ---: |\n");
        let _ = writeln!(
            s,
            "| {} | {} | {} |",
            c.net_count, c.total_shapes, c.max_fanout
        );

        let _ = write!(
            s,
            "\n## Antenna (threshold {})\n\n",
            num(self.antenna.threshold)
        );
        if self.antenna.violations.is_empty() {
            s.push_str("No nets exceed the threshold.\n");
        } else {
            s.push_str("| Net | Gate area | Metal area | Ratio |\n");
            s.push_str("| --- | ---: | ---: | ---: |\n");
            for v in &self.antenna.violations {
                let _ = writeln!(
                    s,
                    "| {} | {} | {} | {} |",
                    v.net,
                    num(v.gate_area),
                    num(v.metal_area),
                    num(v.ratio)
                );
            }
        }

        s
    }
}

/// Formats a layer as `layer/datatype`.
fn layer_id(m: &LayerMetrics) -> String {
    format!("{}/{}", m.layer.layer, m.layer.datatype)
}

/// Formats a metric deterministically: integral values print without a decimal
/// point, others with four decimals. Keeps golden output byte-stable.
fn num(x: f64) -> String {
    if x.is_finite() && x.fract() == 0.0 {
        format!("{}", x as i64)
    } else {
        format!("{x:.4}")
    }
}

#[cfg(test)]
mod tests {
    use super::MetrologyReport;
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{Cell, Document, DrawShape, ShapeKind};

    const POLY: LayerId = LayerId::new(66, 20);
    const LICON1: LayerId = LayerId::new(66, 44);
    const LI1: LayerId = LayerId::new(67, 20);

    fn rect(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    }

    /// A fixture with one high-antenna net: a small poly gate connected through
    /// licon1 to a large li1 sheet.
    fn fixture() -> Document {
        let mut cell = Cell::new("top");
        cell.shapes = vec![
            rect(POLY, 0, 0, 2, 2),
            rect(LICON1, 0, 0, 1, 1),
            rect(LI1, 0, 0, 100, 100),
        ];
        let mut doc = Document::new();
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["top".into()]);
        doc
    }

    #[test]
    fn csv_is_byte_stable() {
        let report = MetrologyReport::generate(&fixture(), 400.0);
        let expected = "\
# layers
layer,area,perimeter,shape_count
66/20,4,8,1
66/44,1,4,1
67/20,10000,400,1

# connectivity
net_count,total_shapes,max_fanout
1,3,3

# antenna (threshold=400)
net,gate_area,metal_area,ratio
net_0,4,10000,2500
";
        assert_eq!(report.to_csv(), expected);
    }

    #[test]
    fn markdown_is_byte_stable() {
        let report = MetrologyReport::generate(&fixture(), 400.0);
        let expected = "\
# Metrology report

## Layers

| Layer | Area | Perimeter | Shapes |
| --- | ---: | ---: | ---: |
| 66/20 | 4 | 8 | 1 |
| 66/44 | 1 | 4 | 1 |
| 67/20 | 10000 | 400 | 1 |

## Connectivity

| Nets | Total shapes | Max fanout |
| ---: | ---: | ---: |
| 1 | 3 | 3 |

## Antenna (threshold 400)

| Net | Gate area | Metal area | Ratio |
| --- | ---: | ---: | ---: |
| net_0 | 4 | 10000 | 2500 |
";
        assert_eq!(report.to_markdown(), expected);
    }
}
