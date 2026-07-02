//! The properties inspector: a read-only summary of the current selection.
//!
//! Given the selected shape indices and the flattened scene, this module builds a
//! plain-data [`Inspection`] describing either a single shape (layer, bounding box,
//! width, height, area) or a multi-shape aggregate (count plus combined bounding
//! box). The app panel then renders the fields as labels. The summarization is a free
//! function so the formatting is unit-tested without an egui context; layer names are
//! looked up through a small [`LayerNamer`] trait so the tests do not need a full
//! [`crate::layers::LayerState`].

use reticle_geometry::{LayerId, Rect, Shape};
use reticle_model::DrawShape;

/// Resolves a layer id to its display name, if the technology knows one.
///
/// Implemented by [`crate::layers::LayerState`] in the app; the inspector depends on
/// this narrow trait rather than the whole layer table so it stays testable.
pub trait LayerNamer {
    /// The human-readable name of `layer`, or `None` if it is not in the table.
    fn layer_name(&self, layer: LayerId) -> Option<String>;
}

/// A read-only summary of what is selected, ready to render as panel rows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Inspection {
    /// Nothing is selected.
    Empty,
    /// Exactly one shape is selected.
    Single(ShapeInfo),
    /// More than one shape is selected: their count and combined bounding box.
    Multiple {
        /// How many shapes are selected.
        count: usize,
        /// The bounding box enclosing all selected shapes.
        bounds: Rect,
    },
}

/// The per-shape facts shown when a single shape is selected.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ShapeInfo {
    /// The shape's layer/datatype.
    pub layer: LayerId,
    /// The layer's display name, if the technology names it.
    pub layer_name: Option<String>,
    /// The shape's bounding box in DBU.
    pub bounds: Rect,
}

impl ShapeInfo {
    /// The bounding-box width in DBU.
    #[must_use]
    pub fn width(&self) -> i64 {
        self.bounds.width()
    }

    /// The bounding-box height in DBU.
    #[must_use]
    pub fn height(&self) -> i64 {
        self.bounds.height()
    }

    /// The bounding-box area in DBU squared.
    #[must_use]
    pub fn area(&self) -> i64 {
        self.bounds.area()
    }

    /// The layer formatted as `name (layer/datatype)`, or `layer/datatype` alone.
    #[must_use]
    pub fn layer_label(&self) -> String {
        let coords = format!("{}/{}", self.layer.layer, self.layer.datatype);
        match &self.layer_name {
            Some(name) if !name.is_empty() => format!("{name} ({coords})"),
            _ => coords,
        }
    }
}

/// Builds an [`Inspection`] for the shapes at `indices` in `shapes`.
///
/// `indices` are indices into the flattened scene shape list (the selection model's
/// contents). Out-of-range indices are skipped, so a stale selection after an edit
/// degrades gracefully rather than panicking. `namer` supplies layer names.
#[must_use]
pub fn inspect<N: LayerNamer>(shapes: &[DrawShape], indices: &[usize], namer: &N) -> Inspection {
    // Collect the valid selected shapes once.
    let selected: Vec<&DrawShape> = indices.iter().filter_map(|&i| shapes.get(i)).collect();

    match selected.as_slice() {
        [] => Inspection::Empty,
        [shape] => {
            let layer = shape.layer;
            Inspection::Single(ShapeInfo {
                layer,
                layer_name: namer.layer_name(layer),
                bounds: shape.bounding_box(),
            })
        }
        many => {
            let bounds = many
                .iter()
                .map(|s| s.bounding_box())
                .reduce(|a, b| a.union(&b))
                .unwrap_or_default();
            Inspection::Multiple {
                count: many.len(),
                bounds,
            }
        }
    }
}

/// Formats a bounding box as `min (x, y)  max (x, y)` for the inspector rows.
#[must_use]
pub fn format_bounds(r: &Rect) -> String {
    format!(
        "min ({}, {})  max ({}, {})",
        r.min.x, r.min.y, r.max.x, r.max.y
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::Point;
    use reticle_model::ShapeKind;
    use std::collections::HashMap;

    /// A tiny in-memory namer for tests.
    struct MapNamer(HashMap<LayerId, String>);

    impl LayerNamer for MapNamer {
        fn layer_name(&self, layer: LayerId) -> Option<String> {
            self.0.get(&layer).cloned()
        }
    }

    fn namer() -> MapNamer {
        let mut m = HashMap::new();
        m.insert(LayerId::new(4, 0), "METAL1".to_owned());
        MapNamer(m)
    }

    fn rect_on(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    }

    #[test]
    fn empty_selection_is_empty() {
        let shapes = vec![rect_on(LayerId::new(4, 0), 0, 0, 10, 10)];
        assert_eq!(inspect(&shapes, &[], &namer()), Inspection::Empty);
    }

    #[test]
    fn single_shape_reports_bbox_dimensions_and_area() {
        // A 400 x 300 rectangle at a known origin.
        let shapes = vec![rect_on(LayerId::new(4, 0), 100, 200, 500, 500)];
        let Inspection::Single(info) = inspect(&shapes, &[0], &namer()) else {
            panic!("expected a single-shape inspection");
        };
        assert_eq!(info.bounds.min, Point::new(100, 200));
        assert_eq!(info.bounds.max, Point::new(500, 500));
        assert_eq!(info.width(), 400);
        assert_eq!(info.height(), 300);
        assert_eq!(info.area(), 400 * 300);
        assert_eq!(info.layer_label(), "METAL1 (4/0)");
    }

    #[test]
    fn single_shape_without_name_falls_back_to_coords() {
        // Layer 7/2 is not in the namer.
        let shapes = vec![rect_on(LayerId::new(7, 2), 0, 0, 10, 10)];
        let Inspection::Single(info) = inspect(&shapes, &[0], &namer()) else {
            panic!("expected single");
        };
        assert_eq!(info.layer_name, None);
        assert_eq!(info.layer_label(), "7/2");
    }

    #[test]
    fn multiple_selection_reports_count_and_combined_bounds() {
        let shapes = vec![
            rect_on(LayerId::new(4, 0), 0, 0, 100, 100),
            rect_on(LayerId::new(5, 0), 900, 800, 1000, 1000),
        ];
        let insp = inspect(&shapes, &[0, 1], &namer());
        let Inspection::Multiple { count, bounds } = insp else {
            panic!("expected a multi-shape inspection");
        };
        assert_eq!(count, 2);
        assert_eq!(bounds.min, Point::new(0, 0));
        assert_eq!(bounds.max, Point::new(1000, 1000));
    }

    #[test]
    fn out_of_range_indices_are_skipped() {
        let shapes = vec![rect_on(LayerId::new(4, 0), 0, 0, 10, 10)];
        // Index 5 does not exist; only index 0 counts, so this is a single shape.
        assert!(matches!(
            inspect(&shapes, &[0, 5], &namer()),
            Inspection::Single(_)
        ));
        // All-invalid indices collapse to Empty.
        assert_eq!(inspect(&shapes, &[5, 6], &namer()), Inspection::Empty);
    }

    #[test]
    fn format_bounds_shows_min_and_max() {
        let r = Rect::new(Point::new(1, 2), Point::new(3, 4));
        assert_eq!(format_bounds(&r), "min (1, 2)  max (3, 4)");
    }
}
