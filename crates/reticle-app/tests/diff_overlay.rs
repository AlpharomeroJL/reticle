//! App-level layout-diff tests: drive the real [`DiffOverlay`] state headlessly (no
//! GPU, no egui) over two Documents that differ by a known set of shapes, and
//! assert the overlay reports the expected added/removed counts. Also asserts that
//! diffing identical documents yields an empty overlay.
//!
//! This exercises the same two-snapshot plumbing the app's `diff_panel` drives:
//! [`DiffOverlay::snapshot`] captures a baseline document and
//! [`DiffOverlay::compute`] diffs it against the current one.

use reticle_app::diff_overlay::DiffOverlay;
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, Document, DrawShape, ShapeKind};

const METAL1: LayerId = LayerId {
    layer: 68,
    datatype: 20,
};
const METAL2: LayerId = LayerId {
    layer: 69,
    datatype: 20,
};

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect::new(Point::new(x, y), Point::new(x + w, y + h))
}

/// A `TOP`-cell document holding the given `(layer, rect)` shapes.
fn doc(shapes: &[(LayerId, Rect)]) -> Document {
    let mut cell = Cell::new("TOP");
    for &(layer, r) in shapes {
        cell.shapes.push(DrawShape::new(layer, ShapeKind::Rect(r)));
    }
    let mut d = Document::new();
    d.insert_cell(cell);
    d.set_top_cells(vec!["TOP".to_owned()]);
    d
}

#[test]
fn two_documents_report_the_known_added_and_removed_shapes() {
    // The baseline (before) carries three shapes across two layers.
    let kept_m1 = (METAL1, rect(0, 0, 10, 10));
    let dropped = (METAL1, rect(20, 0, 10, 10));
    let kept_m2 = (METAL2, rect(0, 20, 10, 10));
    let before = doc(&[kept_m1, dropped, kept_m2]);

    // The after keeps the two `kept_*` shapes, drops `dropped`, and adds two new.
    let new_m1 = (METAL1, rect(40, 0, 10, 10));
    let new_m2 = (METAL2, rect(40, 40, 5, 5));
    let after = doc(&[kept_m1, kept_m2, new_m1, new_m2]);

    let mut overlay = DiffOverlay::new();
    overlay.snapshot(&before);
    let total = overlay.compute(&after).expect("baseline captured");

    // Two added (new_m1, new_m2), one removed (dropped), none changed.
    assert_eq!(overlay.added_count(), 2, "new_m1 and new_m2 are new");
    assert_eq!(overlay.removed_count(), 1, "dropped was removed");
    assert_eq!(overlay.changed_count(), 0, "changed is deferred in v1");
    assert_eq!(total, 3);
    assert!(
        overlay.should_paint(),
        "a non-empty diff paints when visible"
    );

    // The reported shapes carry the exact geometry and layer that changed.
    let added: Vec<(LayerId, Rect)> = overlay.added().iter().map(|s| (s.layer, s.rect)).collect();
    assert!(added.contains(&new_m1));
    assert!(added.contains(&new_m2));
    let removed: Vec<(LayerId, Rect)> = overlay
        .removed()
        .iter()
        .map(|s| (s.layer, s.rect))
        .collect();
    assert_eq!(removed, vec![dropped]);
}

#[test]
fn reversing_the_snapshot_swaps_added_and_removed() {
    let before = doc(&[(METAL1, rect(0, 0, 10, 10))]);
    let after = doc(&[(METAL1, rect(0, 0, 10, 10)), (METAL1, rect(20, 0, 10, 10))]);

    // before -> after: one added, none removed.
    let mut fwd = DiffOverlay::new();
    fwd.snapshot(&before);
    fwd.compute(&after).expect("baseline captured");
    assert_eq!(fwd.added_count(), 1);
    assert_eq!(fwd.removed_count(), 0);

    // after -> before: the same shape is now removed, none added.
    let mut rev = DiffOverlay::new();
    rev.snapshot(&after);
    rev.compute(&before).expect("baseline captured");
    assert_eq!(rev.added_count(), 0);
    assert_eq!(rev.removed_count(), 1);
}

#[test]
fn diffing_identical_documents_yields_an_empty_overlay() {
    let d = doc(&[(METAL1, rect(0, 0, 10, 10)), (METAL2, rect(20, 20, 4, 4))]);
    let mut overlay = DiffOverlay::new();
    overlay.snapshot(&d);
    let total = overlay.compute(&d).expect("baseline captured");

    assert_eq!(total, 0);
    assert!(overlay.has_run(), "the diff ran, it just found nothing");
    assert!(overlay.is_empty());
    assert_eq!(overlay.added_count(), 0);
    assert_eq!(overlay.removed_count(), 0);
    assert!(overlay.added().is_empty());
    assert!(overlay.removed().is_empty());
}
