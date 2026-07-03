//! Saved selection sets, select-similar, and the cell/instance outline tree.
//!
//! These are the three depth features that sit on top of the flat `Selection`:
//!
//! * [`SavedSets`] names the current selection and restores it later. It stores a
//!   plain snapshot of the selected indices per name, so a round-trip is exact as
//!   long as the scene has not been rebuilt under it.
//! * [`select_similar`] grows a selection by adding every shape that shares a
//!   *seed* shape's layer and has a similar bounding-box area (within a tolerance
//!   band). It is the "select all the vias like this one" gesture.
//! * [`OutlineTree`] flattens a [`Document`]'s cell hierarchy into a list of
//!   clickable [`OutlineNode`]s, each carrying the world [`Rect`] the camera
//!   should frame when the node is clicked (the "locate" gesture).
//!
//! Every function here is window-free and unit-tested; the egui panel only draws
//! the resulting rows and forwards clicks.

use reticle_geometry::{LayerId, Point, Rect, Shape, Transform};
use reticle_model::{Document, DrawShape};
use std::collections::BTreeSet;

/// A named store of selection snapshots.
///
/// A snapshot is just the set of selected shape indices at save time. Restoring
/// returns that set for the app to install into its live `Selection`. Indices
/// are into the flattened scene, so a snapshot is only meaningful against the same
/// scene it was captured from; the panel saves and restores within one session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavedSets {
    sets: Vec<SavedSet>,
}

/// One named selection snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedSet {
    /// The user-supplied name.
    pub name: String,
    /// The selected indices captured when the set was saved.
    pub indices: Vec<usize>,
}

impl SavedSets {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Saves `indices` under `name`, replacing any existing set with that name.
    ///
    /// The name is trimmed; saving under a blank name is refused and returns
    /// `false`. Indices are stored sorted and deduplicated so a restore is
    /// order-stable regardless of how the selection was built.
    pub fn save<I: IntoIterator<Item = usize>>(&mut self, name: &str, indices: I) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        let sorted: BTreeSet<usize> = indices.into_iter().collect();
        let set = SavedSet {
            name: name.to_owned(),
            indices: sorted.into_iter().collect(),
        };
        match self.sets.iter_mut().find(|s| s.name == name) {
            Some(existing) => *existing = set,
            None => self.sets.push(set),
        }
        true
    }

    /// Returns the indices saved under `name`, if any.
    #[must_use]
    pub fn restore(&self, name: &str) -> Option<&[usize]> {
        self.sets
            .iter()
            .find(|s| s.name == name.trim())
            .map(|s| s.indices.as_slice())
    }

    /// Removes the set named `name`, returning whether one was removed.
    pub fn remove(&mut self, name: &str) -> bool {
        let name = name.trim();
        let before = self.sets.len();
        self.sets.retain(|s| s.name != name);
        self.sets.len() != before
    }

    /// All saved sets, in insertion order (name plus selection size).
    #[must_use]
    pub fn sets(&self) -> &[SavedSet] {
        &self.sets
    }

    /// The number of saved sets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sets.len()
    }

    /// Whether no sets are saved.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }
}

/// The default half-width of the select-similar area tolerance band, as a
/// fraction of a seed shape's area (0.25 = accept areas within ±25%).
pub const DEFAULT_SIMILAR_TOLERANCE: f64 = 0.25;

/// Grows `seed` into the set of shape indices that are *similar* to any seed shape.
///
/// A shape is similar to a seed when it is on the same [`LayerId`] and its
/// bounding-box area is within `tolerance` (a fraction, e.g. `0.25` for ±25%) of
/// that seed's area. The returned set always contains the original seed indices,
/// so this only ever adds to a selection. `tolerance` is clamped to be
/// non-negative; a zero tolerance keeps only shapes with exactly a seed's area on
/// a seed's layer.
///
/// With no seed indices (an empty selection) the result is empty: there is nothing
/// to be similar to.
#[must_use]
pub fn select_similar(
    shapes: &[DrawShape],
    seed: &BTreeSet<usize>,
    tolerance: f64,
) -> BTreeSet<usize> {
    let mut result = seed.clone();
    if seed.is_empty() {
        return result;
    }
    let tol = tolerance.max(0.0);
    // The (layer, area) signature of every seed shape.
    let seeds: Vec<(LayerId, i64)> = seed
        .iter()
        .filter_map(|&i| shapes.get(i))
        .map(|s| (s.layer(), s.bounding_box().area()))
        .collect();
    for (i, shape) in shapes.iter().enumerate() {
        if result.contains(&i) {
            continue;
        }
        let layer = shape.layer();
        let area = shape.bounding_box().area();
        if seeds
            .iter()
            .any(|&(seed_layer, seed_area)| seed_layer == layer && within(area, seed_area, tol))
        {
            result.insert(i);
        }
    }
    result
}

/// Whether `area` is within `tolerance` (a fraction) of `reference`.
///
/// Uses `f64` for the band arithmetic so a large area (up to DBU²) does not
/// overflow; the comparison itself is exact enough for a selection heuristic.
fn within(area: i64, reference: i64, tolerance: f64) -> bool {
    let reference = reference as f64;
    let band = reference.abs() * tolerance;
    let lo = reference - band;
    let hi = reference + band;
    let area = area as f64;
    area >= lo && area <= hi
}

/// What an [`OutlineNode`] refers to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutlineKind {
    /// A cell definition (a row for the cell itself).
    Cell,
    /// A single placement of a child cell inside a parent cell.
    Instance,
    /// An arrayed placement of a child cell inside a parent cell.
    Array,
}

/// One row in the outline tree: a cell or a placement, with where to locate it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OutlineNode {
    /// What this row represents.
    pub kind: OutlineKind,
    /// The label to show (cell name, or `child @ (x, y)` for a placement).
    pub label: String,
    /// Indentation depth for the tree view (0 for top-level cells).
    pub depth: usize,
    /// The world-space rectangle the camera should frame when this row is clicked,
    /// or `None` when the target has no geometry to frame (an empty cell).
    pub locate: Option<Rect>,
}

/// A flattened, clickable view of a document's cell hierarchy.
///
/// The tree is one level deep per cell: each top cell, then each of its instances
/// and arrays as indented children. This is enough to browse the demo document and
/// jump the camera to any cell or placement without a full recursive treeview.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct OutlineTree {
    nodes: Vec<OutlineNode>,
}

impl OutlineTree {
    /// Builds the outline for `doc`, rooted at its top cells.
    ///
    /// Top cells appear in declared order; if the document declares none, every
    /// cell is listed (in name order) so nothing is hidden. Under each cell come
    /// its single instances then its arrays, each labelled with the child cell and
    /// the placement origin, and each carrying the world rectangle to frame.
    #[must_use]
    pub fn build(doc: &Document) -> Self {
        let mut nodes = Vec::new();
        let roots = root_names(doc);
        for name in &roots {
            push_cell(doc, name, 0, &mut nodes);
        }
        Self { nodes }
    }

    /// The outline rows, in display order.
    #[must_use]
    pub fn nodes(&self) -> &[OutlineNode] {
        &self.nodes
    }

    /// The number of rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the outline is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// The top-cell names to root the outline at: the declared top cells, or every
/// cell name (sorted) when none are declared.
fn root_names(doc: &Document) -> Vec<String> {
    let tops = doc.top_cells();
    if tops.is_empty() {
        let mut names: Vec<String> = doc.cells().map(|c| c.name.clone()).collect();
        names.sort();
        names
    } else {
        tops.to_vec()
    }
}

/// Appends a cell row and its placement children to `nodes`.
fn push_cell(doc: &Document, name: &str, depth: usize, nodes: &mut Vec<OutlineNode>) {
    nodes.push(OutlineNode {
        kind: OutlineKind::Cell,
        label: name.to_owned(),
        depth,
        locate: doc.cell_bbox(name),
    });
    let Some(cell) = doc.cell(name) else {
        return;
    };
    for inst in &cell.instances {
        let target = doc
            .cell_bbox(&inst.cell)
            .map(|bbox| transform_rect(&inst.transform, bbox));
        nodes.push(OutlineNode {
            kind: OutlineKind::Instance,
            label: placement_label(&inst.cell, inst.transform.translation),
            depth: depth + 1,
            locate: target,
        });
    }
    for array in &cell.arrays {
        let target = doc
            .cell_bbox(&array.cell)
            .map(|bbox| transform_rect(&array.transform, bbox));
        let label = format!(
            "{} [{}x{}] @ ({}, {})",
            array.cell,
            array.columns,
            array.rows,
            array.transform.translation.x,
            array.transform.translation.y
        );
        nodes.push(OutlineNode {
            kind: OutlineKind::Array,
            label,
            depth: depth + 1,
            locate: target,
        });
    }
}

/// A `child @ (x, y)` label for a single placement.
fn placement_label(cell: &str, at: Point) -> String {
    format!("{cell} @ ({}, {})", at.x, at.y)
}

/// Transforms a rectangle by `transform` and returns the bounding box of the
/// result (exact for the dihedral orientations and integer magnifications used by
/// placements). Mirrors the private helper in `reticle_model`.
fn transform_rect(transform: &Transform, rect: Rect) -> Rect {
    let corners = [
        rect.min,
        Point::new(rect.max.x, rect.min.y),
        rect.max,
        Point::new(rect.min.x, rect.max.y),
    ];
    Rect::from_points(corners.into_iter().map(|c| transform.apply(c))).unwrap_or(rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::LayerId;
    use reticle_model::{ArrayInstance, Cell, Instance, ShapeKind};

    const M1: LayerId = LayerId::new(4, 0);
    const M2: LayerId = LayerId::new(5, 0);

    fn rect_on(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    }

    fn set(items: impl IntoIterator<Item = usize>) -> BTreeSet<usize> {
        items.into_iter().collect()
    }

    // ---- SavedSets ------------------------------------------------------

    #[test]
    fn saved_set_round_trips() {
        let mut sets = SavedSets::new();
        assert!(sets.save("vias", [3, 1, 2, 1]));
        // Stored sorted and deduplicated.
        assert_eq!(sets.restore("vias"), Some([1, 2, 3].as_slice()));
    }

    #[test]
    fn saved_set_restore_missing_is_none() {
        let sets = SavedSets::new();
        assert_eq!(sets.restore("nope"), None);
    }

    #[test]
    fn saving_same_name_replaces() {
        let mut sets = SavedSets::new();
        sets.save("a", [1, 2]);
        sets.save("a", [7, 8, 9]);
        assert_eq!(sets.len(), 1);
        assert_eq!(sets.restore("a"), Some([7, 8, 9].as_slice()));
    }

    #[test]
    fn blank_name_is_refused() {
        let mut sets = SavedSets::new();
        assert!(!sets.save("   ", [1, 2]));
        assert!(sets.is_empty());
    }

    #[test]
    fn name_is_trimmed_on_save_and_restore() {
        let mut sets = SavedSets::new();
        assert!(sets.save("  ring  ", [4, 5]));
        assert_eq!(sets.restore("ring"), Some([4, 5].as_slice()));
        assert_eq!(sets.sets()[0].name, "ring");
    }

    #[test]
    fn remove_deletes_the_set() {
        let mut sets = SavedSets::new();
        sets.save("a", [1]);
        sets.save("b", [2]);
        assert!(sets.remove("a"));
        assert!(!sets.remove("a"));
        assert_eq!(sets.len(), 1);
        assert_eq!(sets.restore("b"), Some([2].as_slice()));
    }

    #[test]
    fn empty_selection_can_be_saved_and_restored() {
        let mut sets = SavedSets::new();
        assert!(sets.save("empty", []));
        assert_eq!(sets.restore("empty"), Some([].as_slice()));
    }

    // ---- select_similar -------------------------------------------------

    #[test]
    fn select_similar_matches_layer_and_area() {
        // Index 0 is the seed (METAL1, 100x100 = area 10_000).
        let shapes = vec![
            rect_on(M1, 0, 0, 100, 100),       // 0: seed
            rect_on(M1, 500, 0, 610, 110),     // 1: METAL1, 110x110 area 12_100 (+21%)
            rect_on(M2, 0, 0, 100, 100),       // 2: METAL2 (wrong layer)
            rect_on(M1, 0, 0, 1000, 1000),     // 3: METAL1 but far larger
            rect_on(M1, 900, 900, 1000, 1000), // 4: METAL1, exactly 100x100
        ];
        let result = select_similar(&shapes, &set([0]), DEFAULT_SIMILAR_TOLERANCE);
        // 0 (seed), 1 (+21% within 25%), 4 (exact). Not 2 (layer) or 3 (too big).
        assert_eq!(result, set([0, 1, 4]));
    }

    #[test]
    fn select_similar_only_adds_never_removes() {
        let shapes = vec![rect_on(M1, 0, 0, 100, 100), rect_on(M2, 0, 0, 100, 100)];
        // Seed includes a METAL2 shape; it must survive even though nothing else
        // is similar to it.
        let result = select_similar(&shapes, &set([1]), DEFAULT_SIMILAR_TOLERANCE);
        assert!(result.contains(&1));
    }

    #[test]
    fn select_similar_zero_tolerance_is_exact_area() {
        let shapes = vec![
            rect_on(M1, 0, 0, 100, 100),       // 0: seed area 10_000
            rect_on(M1, 900, 900, 1000, 1000), // 1: exactly 10_000
            rect_on(M1, 0, 0, 101, 100),       // 2: 10_100, off by a hair
        ];
        let result = select_similar(&shapes, &set([0]), 0.0);
        assert_eq!(result, set([0, 1]));
    }

    #[test]
    fn select_similar_empty_seed_is_empty() {
        let shapes = vec![rect_on(M1, 0, 0, 100, 100)];
        assert!(select_similar(&shapes, &set([]), DEFAULT_SIMILAR_TOLERANCE).is_empty());
    }

    #[test]
    fn select_similar_negative_tolerance_clamped_to_zero() {
        let shapes = vec![
            rect_on(M1, 0, 0, 100, 100),
            rect_on(M1, 900, 900, 1000, 1000), // same area
        ];
        // A negative tolerance must behave like 0, not reject the exact match.
        let result = select_similar(&shapes, &set([0]), -1.0);
        assert_eq!(result, set([0, 1]));
    }

    #[test]
    fn select_similar_ignores_out_of_range_seed_index() {
        let shapes = vec![rect_on(M1, 0, 0, 100, 100)];
        // Seed index 99 does not exist; it contributes no signature and is kept as
        // a (dangling) member, but nothing else is added.
        let result = select_similar(&shapes, &set([99]), DEFAULT_SIMILAR_TOLERANCE);
        assert_eq!(result, set([99]));
    }

    // ---- OutlineTree ----------------------------------------------------

    fn doc_with_hierarchy() -> Document {
        let mut doc = Document::new();
        let mut leaf = Cell::new("LEAF");
        leaf.shapes.push(rect_on(M1, 0, 0, 200, 100));
        doc.insert_cell(leaf);

        let mut top = Cell::new("TOP");
        top.shapes.push(rect_on(M2, 0, 0, 50, 50));
        top.instances.push(Instance {
            cell: "LEAF".to_owned(),
            transform: Transform::translate(1000, 2000),
        });
        top.arrays.push(ArrayInstance {
            cell: "LEAF".to_owned(),
            transform: Transform::translate(0, 5000),
            columns: 3,
            rows: 2,
            column_pitch: 400,
            row_pitch: 300,
        });
        doc.insert_cell(top);
        doc.set_top_cells(vec!["TOP".to_owned()]);
        doc
    }

    #[test]
    fn outline_lists_cell_then_placements() {
        let tree = OutlineTree::build(&doc_with_hierarchy());
        let kinds: Vec<_> = tree.nodes().iter().map(|n| n.kind).collect();
        assert_eq!(
            kinds,
            vec![OutlineKind::Cell, OutlineKind::Instance, OutlineKind::Array]
        );
        assert_eq!(tree.nodes()[0].label, "TOP");
        assert_eq!(tree.nodes()[0].depth, 0);
        assert_eq!(tree.nodes()[1].depth, 1);
    }

    #[test]
    fn outline_instance_locate_is_transformed() {
        let tree = OutlineTree::build(&doc_with_hierarchy());
        let inst = tree
            .nodes()
            .iter()
            .find(|n| n.kind == OutlineKind::Instance)
            .unwrap();
        // LEAF bbox is (0,0)-(200,100); translated by (1000, 2000).
        let loc = inst.locate.expect("instance has geometry");
        assert_eq!(loc.min, Point::new(1000, 2000));
        assert_eq!(loc.max, Point::new(1200, 2100));
    }

    #[test]
    fn outline_placement_labels_carry_origin() {
        let tree = OutlineTree::build(&doc_with_hierarchy());
        let inst = tree
            .nodes()
            .iter()
            .find(|n| n.kind == OutlineKind::Instance)
            .unwrap();
        assert_eq!(inst.label, "LEAF @ (1000, 2000)");
        let array = tree
            .nodes()
            .iter()
            .find(|n| n.kind == OutlineKind::Array)
            .unwrap();
        assert_eq!(array.label, "LEAF [3x2] @ (0, 5000)");
    }

    #[test]
    fn outline_falls_back_to_all_cells_without_top_cells() {
        let mut doc = Document::new();
        doc.insert_cell(Cell::new("BBB"));
        doc.insert_cell(Cell::new("AAA"));
        // No top cells declared: list every cell, name-sorted.
        let tree = OutlineTree::build(&doc);
        let cell_labels: Vec<_> = tree
            .nodes()
            .iter()
            .filter(|n| n.kind == OutlineKind::Cell)
            .map(|n| n.label.clone())
            .collect();
        assert_eq!(cell_labels, vec!["AAA".to_owned(), "BBB".to_owned()]);
    }

    #[test]
    fn outline_empty_document_is_empty() {
        assert!(OutlineTree::build(&Document::new()).is_empty());
    }

    #[test]
    fn outline_empty_cell_has_no_locate() {
        let mut doc = Document::new();
        doc.insert_cell(Cell::new("EMPTY"));
        doc.set_top_cells(vec!["EMPTY".to_owned()]);
        let tree = OutlineTree::build(&doc);
        assert_eq!(tree.len(), 1);
        assert!(tree.nodes()[0].locate.is_none());
    }
}
