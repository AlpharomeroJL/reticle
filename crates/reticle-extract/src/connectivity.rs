//! The geometric connectivity engine: touch predicates, footprints, and the
//! spatial-index-driven union-find pass that groups shapes into nets.
//!
//! # Connectivity model
//!
//! - **Same layer.** Two shapes on the same layer are connected when their
//!   footprints touch or overlap. For axis-aligned rectangles this is decided
//!   exactly by [`rects_touch`] (a *closed*-box intersection, so shapes that share
//!   only an edge or a corner still count, layout wires abutting end-to-end are
//!   electrically one). Shapes involving polygons or paths use their bounding box
//!   as a candidate filter and confirm a positive-area overlap with an exact
//!   integer polygon boolean.
//! - **Different layers.** Never connected directly. A via/contact shape on a
//!   layer named by a [`ConnectionRule`](crate::ConnectionRule) connects a
//!   bottom-layer and a top-layer shape wherever the via overlaps both.
//!
//! # Efficiency
//!
//! Candidate touching pairs are found by querying a bulk-loaded R-tree with each
//! shape's (slightly grown) bounding box, rather than testing all `O(n²)` pairs.
//! Each confirmed connection is a `union` in the disjoint set.

use reticle_geometry::{
    BooleanOp, LayerId, Point, Polygon, Rect, Shape, SpatialIndex, polygon_boolean,
};
use reticle_index::RTreeIndex;
use reticle_model::{DrawShape, ShapeKind};

use crate::rules::ConnectionRules;
use crate::union_find::DisjointSet;

/// Returns `true` if two axis-aligned rectangles touch or overlap, treating them
/// as *closed* boxes `[min, max]`.
///
/// Unlike [`Rect::intersects`](reticle_geometry::Rect::intersects), which
/// requires positive-area overlap, this returns `true` when the rectangles share
/// only an edge or a single corner. That is the correct rule for connectivity:
/// two wire segments that abut end-to-end are electrically one net.
#[must_use]
pub fn rects_touch(a: &Rect, b: &Rect) -> bool {
    a.min.x <= b.max.x && b.min.x <= a.max.x && a.min.y <= b.max.y && b.min.y <= a.max.y
}

/// The polygonal footprint of a shape, used for exact overlap tests.
///
/// Rectangles and polygons map to their exact outline; a path maps to its
/// (conservative) bounding-box rectangle, which is sufficient for the
/// bounding-box-plus-boolean confirmation used here.
fn footprint(shape: &DrawShape) -> Polygon {
    match &shape.kind {
        ShapeKind::Rect(r) => Polygon::from_rect(*r),
        ShapeKind::Polygon(p) => p.clone(),
        ShapeKind::Path(p) => Polygon::from_rect(p.bounding_box()),
    }
}

/// Returns `true` if both shapes are axis-aligned rectangles.
fn both_rects(a: &DrawShape, b: &DrawShape) -> bool {
    matches!(a.kind, ShapeKind::Rect(_)) && matches!(b.kind, ShapeKind::Rect(_))
}

/// Decides whether two shapes' geometry touches or overlaps.
///
/// Rectangle/rectangle pairs are decided exactly by closed-box touch. Any pair
/// involving a polygon or path first requires their bounding boxes to touch, then
/// confirms with a polygon-boolean intersection; if the boxes only share an edge
/// or corner (zero-area overlap) the boolean cannot report area, so bounding-box
/// contact is accepted as the connection.
#[must_use]
pub fn shapes_touch(a: &DrawShape, b: &DrawShape) -> bool {
    let bbox_a = a.bounding_box();
    let bbox_b = b.bounding_box();
    if !rects_touch(&bbox_a, &bbox_b) {
        return false;
    }
    if both_rects(a, b) {
        // Exact for rectangles.
        return true;
    }
    // If the bounding boxes overlap with positive area, confirm a real geometric
    // overlap with an exact integer boolean; otherwise the shapes merely abut and
    // bounding-box contact already established touching.
    if bbox_a.intersects(&bbox_b) {
        polygons_overlap(&footprint(a), &footprint(b))
    } else {
        true
    }
}

/// Returns `true` if `point` lies within the closed bounding box of `shape`.
///
/// Edges and corners are included. This is exact for rectangles and rectilinear
/// layout, and a safe over-approximation for arbitrary polygons/paths, exact
/// point-in-polygon is unnecessary for the label-seeding use-case. Used to attach
/// a [`NetLabel`](crate::NetLabel) to the net covering its point.
#[must_use]
pub fn shape_covers_point(shape: &DrawShape, point: Point) -> bool {
    let b = shape.bounding_box();
    point.x >= b.min.x && point.x <= b.max.x && point.y >= b.min.y && point.y <= b.max.y
}

/// Returns `true` if two polygon footprints overlap in a region of positive area.
fn polygons_overlap(a: &Polygon, b: &Polygon) -> bool {
    let a = std::slice::from_ref(a);
    let b = std::slice::from_ref(b);
    match polygon_boolean(BooleanOp::Intersection, a, b) {
        // A non-empty intersection with any polygon of >= 3 vertices means overlap.
        Ok(result) => result.iter().any(|p| p.len() >= 3),
        Err(_) => false,
    }
}

/// Builds a disjoint set whose members are the indices `0..shapes.len()`, unioned
/// so that each set is one connected net.
///
/// `shapes` is the flat shape list of a cell (already expanded if the caller wants
/// hierarchy flattened). `rules` supplies the via/contact connections.
pub fn build_components(shapes: &[DrawShape], rules: &ConnectionRules) -> DisjointSet {
    let mut dsu = DisjointSet::new(shapes.len());
    if shapes.is_empty() {
        return dsu;
    }

    // One R-tree over every shape's bounding box. Querying it with a shape's box
    // (grown by 1 DBU so edge/corner-touching neighbours are returned) yields the
    // candidate set to test, replacing the O(n²) all-pairs scan.
    let index: RTreeIndex<usize> = RTreeIndex::bulk_load(
        shapes
            .iter()
            .enumerate()
            .map(|(i, s)| (s.bounding_box(), i)),
    );

    connect_same_layer(shapes, &index, &mut dsu);
    connect_vias(shapes, &index, rules, &mut dsu);
    dsu
}

/// Unions every same-layer touching pair, found via the spatial index.
fn connect_same_layer(shapes: &[DrawShape], index: &RTreeIndex<usize>, dsu: &mut DisjointSet) {
    for (i, shape) in shapes.iter().enumerate() {
        // Grow by 1 DBU so neighbours sharing only an edge/corner are candidates
        // (the R-tree post-filters to positive-area overlap of the *query* box).
        let query = shape.bounding_box().expanded(1);
        for &j in index.query_rect(query) {
            // Order the pair so each is considered once; skip self.
            if j <= i {
                continue;
            }
            let other = &shapes[j];
            if shape.layer == other.layer && shapes_touch(shape, other) {
                dsu.union(i, j);
            }
        }
    }
}

/// For each via shape, unions the bottom- and top-layer shapes it overlaps.
///
/// A via connects a pair only if it overlaps *both* conductors, so a lone via, or
/// one missing either landing pad, leaves the nets separate.
fn connect_vias(
    shapes: &[DrawShape],
    index: &RTreeIndex<usize>,
    rules: &ConnectionRules,
    dsu: &mut DisjointSet,
) {
    if rules.is_empty() {
        return;
    }
    for (vi, via) in shapes.iter().enumerate() {
        // Collect the conductor layer pairs this shape can bridge (if any).
        let pairs: Vec<(LayerId, LayerId)> = rules.conductor_pairs_for_via(via.layer).collect();
        if pairs.is_empty() {
            continue;
        }
        // Candidate conductors overlapping the via's (grown) box.
        let query = via.bounding_box().expanded(1);
        let candidates: Vec<usize> = index.query_rect(query).into_iter().copied().collect();

        for (bottom_layer, top_layer) in pairs {
            // Shapes on the bottom/top conductor layers that the via actually
            // touches. A via must overlap both to bridge them.
            let bottoms: Vec<usize> = candidates
                .iter()
                .copied()
                .filter(|&c| {
                    c != vi && shapes[c].layer == bottom_layer && shapes_touch(via, &shapes[c])
                })
                .collect();
            let tops: Vec<usize> = candidates
                .iter()
                .copied()
                .filter(|&c| {
                    c != vi && shapes[c].layer == top_layer && shapes_touch(via, &shapes[c])
                })
                .collect();
            // Union the via itself with every conductor it lands on, and thereby
            // every bottom with every top through the shared via node.
            for &b in &bottoms {
                dsu.union(vi, b);
            }
            for &t in &tops {
                dsu.union(vi, t);
            }
        }
    }
}
