//! Geometric benchmark checkers beyond the primitive `rect_present` / `drc_clean` /
//! `intent` set.
//!
//! These decide *structured* layout tasks: how many shapes on a layer, how much area,
//! a contact/via stack that actually bridges two conductors, a via chain, an
//! interdigitated comb, a closed guard ring, and a compound cell that places
//! sub-cells. Each is built from a [`ParsedChecker`] so the layer, counts, and
//! thresholds come from the task's `checker` string (see [`crate::params`]); each is
//! unit-tested in both directions.
//!
//! # Connectivity
//!
//! The connectivity-flavoured checkers (contact stack, via chain, comb, compound
//! cell) reuse `reticle_extract`'s engine rather than re-deriving nets: they flatten
//! the target cell, run [`reticle_extract::build_components`] over the SKY130 via
//! stack, and reason about the resulting disjoint set. This is the same extraction
//! the intent checker uses, so "connected" means exactly what it means everywhere
//! else in the workspace (same-layer touch plus a via that overlaps both conductors).

use reticle_agent_api::Transcript;
use reticle_extract::{DisjointSet, build_components, sky130_connection_rules};
use reticle_geometry::{LayerId, Point, Rect, Shape as _};
use reticle_model::{Document, DrawShape, ShapeKind};

use crate::checker::{CheckFailure, CheckResult, Checker};
use crate::params::{ParamError, ParsedChecker};

/// The cell a checker inspects: the first declared top cell, or any cell if none is
/// marked top. Mirrors [`crate::checkers`]'s own `target_cell` and the agent
/// session's top-cell choice, so document-level checkers agree on which cell to read.
fn target_cell(doc: &Document) -> Option<String> {
    doc.top_cells()
        .first()
        .cloned()
        .or_else(|| doc.cells().next().map(|c| c.name.clone()))
}

/// The bounding box of a shape on the DBU grid (rectangles are exact; polygons and
/// paths use their bounding box, which is what the connectivity engine also uses).
fn shape_bbox(shape: &DrawShape) -> Rect {
    shape.bounding_box()
}

/// The area a shape contributes to a per-layer area total.
///
/// A rectangle contributes its exact area; a polygon its shoelace area (rounded down
/// to an integer DBU²); a path its bounding-box area (a conservative stand-in, paths
/// are not used by the area tasks). Kept simple and non-negative.
fn shape_area(shape: &DrawShape) -> i64 {
    match &shape.kind {
        ShapeKind::Rect(r) => r.area(),
        // `Polygon::area` is an f64 magnitude; floor to an exact DBU² count. The
        // cast is saturating on the platforms we target and areas here are small.
        #[allow(clippy::cast_possible_truncation)]
        ShapeKind::Polygon(p) => p.area() as i64,
        ShapeKind::Path(p) => p.bounding_box().area(),
    }
}

/// Every shape on `layer` in `cell`, in document order.
fn shapes_on_layer<'a>(
    doc: &'a Document,
    cell: &str,
    layer: LayerId,
) -> impl Iterator<Item = &'a DrawShape> {
    doc.cell(cell)
        .into_iter()
        .flat_map(|c| c.shapes.iter())
        .filter(move |s| s.layer == layer)
}

/// Builds one of the geometric checkers from a parsed checker string, or returns the
/// parameter error that makes the task fail to compile.
///
/// The recognized names are `shape_count`, `layer_area`, `contact_stack`,
/// `via_chain`, `comb`, `guard_ring`, and `compound_cell`. An unrecognized name
/// yields `Ok(None)` so the caller can fall through to the built-in registry.
///
/// # Errors
///
/// Returns a [`ParamError`] when a recognized checker is missing a required parameter
/// or a parameter does not parse.
pub fn build(parsed: &ParsedChecker) -> Result<Option<Box<dyn Checker>>, ParamError> {
    let checker: Box<dyn Checker> = match parsed.name() {
        "shape_count" => Box::new(ShapeCount::from_params(parsed)?),
        "layer_area" => Box::new(LayerArea::from_params(parsed)?),
        "contact_stack" => Box::new(ContactStack::from_params(parsed)?),
        "via_chain" => Box::new(ViaChain::from_params(parsed)?),
        "comb" => Box::new(Comb::from_params(parsed)?),
        "guard_ring" => Box::new(GuardRing::from_params(parsed)?),
        "compound_cell" => Box::new(CompoundCell::from_params(parsed)?),
        _ => return Ok(None),
    };
    Ok(Some(checker))
}

// --------------------------------------------------------------------------------
// shape_count
// --------------------------------------------------------------------------------

/// Passes iff the number of shapes on a layer meets a count constraint.
///
/// Parameters (`shape_count:layer=68/20,min=3` and so on): `layer` (required),
/// `min`, `max`, and `exact`. `exact` sets both bounds. With no bound the check
/// degenerates to "at least one shape on the layer".
#[derive(Clone, Copy, Debug)]
pub struct ShapeCount {
    /// Layer the shapes must be on.
    layer: LayerId,
    /// Inclusive lower bound on the count.
    min: u32,
    /// Inclusive upper bound on the count, if any.
    max: Option<u32>,
}

impl ShapeCount {
    /// Builds a shape-count checker from parsed parameters.
    ///
    /// # Errors
    ///
    /// [`ParamError`] if `layer` is missing/malformed or a bound does not parse.
    pub fn from_params(p: &ParsedChecker) -> Result<Self, ParamError> {
        let layer = p.layer("layer")?;
        if let Some(exact) = optional_u32(p, "exact")? {
            return Ok(Self {
                layer,
                min: exact,
                max: Some(exact),
            });
        }
        let min = p.u32_or("min", 1)?;
        let max = optional_u32(p, "max")?;
        Ok(Self { layer, min, max })
    }
}

impl Checker for ShapeCount {
    fn check(&self, doc: &Document, _transcript: &Transcript) -> CheckResult {
        let Some(cell) = target_cell(doc) else {
            return fail("document has no cell to check");
        };
        let count = shapes_on_layer(doc, &cell, self.layer).count() as u32;
        if count < self.min {
            return fail(format!(
                "layer {}/{} has {count} shapes, expected at least {}",
                self.layer.layer, self.layer.datatype, self.min
            ));
        }
        if let Some(max) = self.max
            && count > max
        {
            return fail(format!(
                "layer {}/{} has {count} shapes, expected at most {max}",
                self.layer.layer, self.layer.datatype
            ));
        }
        CheckResult::Pass
    }
}

// --------------------------------------------------------------------------------
// layer_area
// --------------------------------------------------------------------------------

/// Passes iff the total drawn area on a layer meets an area constraint (in DBU²).
///
/// Parameters (`layer_area:layer=68/20,min_area=83000`): `layer` (required),
/// `min_area`, `max_area`. Area is summed over each shape's own area, so overlapping
/// shapes are counted twice; the tasks that use this draw disjoint geometry.
#[derive(Clone, Copy, Debug)]
pub struct LayerArea {
    /// Layer whose area is summed.
    layer: LayerId,
    /// Inclusive minimum total area in DBU².
    min_area: i64,
    /// Inclusive maximum total area in DBU², if any.
    max_area: Option<i64>,
}

impl LayerArea {
    /// Builds a layer-area checker from parsed parameters.
    ///
    /// # Errors
    ///
    /// [`ParamError`] if `layer` is missing/malformed or a bound does not parse.
    pub fn from_params(p: &ParsedChecker) -> Result<Self, ParamError> {
        let layer = p.layer("layer")?;
        let min_area = p.i64_or("min_area", 0)?;
        let max_area = optional_i64(p, "max_area")?;
        Ok(Self {
            layer,
            min_area,
            max_area,
        })
    }
}

impl Checker for LayerArea {
    fn check(&self, doc: &Document, _transcript: &Transcript) -> CheckResult {
        let Some(cell) = target_cell(doc) else {
            return fail("document has no cell to check");
        };
        let total: i64 = shapes_on_layer(doc, &cell, self.layer)
            .map(shape_area)
            .sum();
        if total < self.min_area {
            return fail(format!(
                "layer {}/{} total area {total} < minimum {}",
                self.layer.layer, self.layer.datatype, self.min_area
            ));
        }
        if let Some(max) = self.max_area
            && total > max
        {
            return fail(format!(
                "layer {}/{} total area {total} > maximum {max}",
                self.layer.layer, self.layer.datatype
            ));
        }
        CheckResult::Pass
    }
}

// --------------------------------------------------------------------------------
// contact_stack (single via/contact bridging two conductors)
// --------------------------------------------------------------------------------

/// Passes iff a via/contact on the given layer bridges a bottom-conductor shape and a
/// top-conductor shape into one net, and (optionally) both conductors enclose the via
/// by at least `min_enclosure` DBU.
///
/// Parameters (`contact_stack:via=68/44,bottom=68/20,top=69/20,min_enclosure=55`):
/// `via` (required), `bottom`/`top` (the conductor layers; default to the SKY130
/// stack pair for the via), and `min_enclosure` (default `0`, i.e. topology only).
///
/// The topology is judged by the shared connectivity engine: the via's net must
/// contain at least one shape on each conductor layer. This is exactly "the contact
/// actually connects the two metals", the thing a placement task gets wrong by
/// leaving a gap.
#[derive(Clone, Copy, Debug)]
pub struct ContactStack {
    /// The via/contact layer.
    via: LayerId,
    /// The lower conductor layer.
    bottom: LayerId,
    /// The upper conductor layer.
    top: LayerId,
    /// Minimum enclosure of the via by each conductor, in DBU (`0` disables it).
    min_enclosure: i64,
}

impl ContactStack {
    /// Builds a contact-stack checker from parsed parameters, defaulting the
    /// conductor layers from the SKY130 stack when only the via is given.
    ///
    /// # Errors
    ///
    /// [`ParamError`] if `via` is missing/malformed or a parameter does not parse.
    pub fn from_params(p: &ParsedChecker) -> Result<Self, ParamError> {
        let via = p.layer("via")?;
        let (default_bottom, default_top) = sky130_stack_pair(via);
        let bottom = p.layer_or("bottom", default_bottom)?;
        let top = p.layer_or("top", default_top)?;
        let min_enclosure = p.i64_or("min_enclosure", 0)?;
        Ok(Self {
            via,
            bottom,
            top,
            min_enclosure,
        })
    }
}

impl Checker for ContactStack {
    fn check(&self, doc: &Document, _transcript: &Transcript) -> CheckResult {
        let Some(cell) = target_cell(doc) else {
            return fail("document has no cell to check");
        };
        let shapes = doc.flatten(&cell);
        let mut dsu = build_components(&shapes, &sky130_connection_rules());

        // Find a via on the target layer whose net includes both conductors.
        let vias: Vec<usize> = indices_on_layer(&shapes, self.via);
        if vias.is_empty() {
            return fail(format!(
                "no via/contact shape on layer {}/{}",
                self.via.layer, self.via.datatype
            ));
        }
        for &vi in &vias {
            let bottom_hit = any_connected_on_layer(&shapes, &mut dsu, vi, self.bottom);
            let top_hit = any_connected_on_layer(&shapes, &mut dsu, vi, self.top);
            if let (Some(bi), Some(ti)) = (bottom_hit, top_hit) {
                if self.min_enclosure > 0
                    && let Some(reason) = self.enclosure_shortfall(&shapes, vi, bi, ti)
                {
                    return CheckResult::Fail(vec![reason]);
                }
                return CheckResult::Pass;
            }
        }
        fail(format!(
            "no via on {}/{} connects a {}/{} shape to a {}/{} shape",
            self.via.layer,
            self.via.datatype,
            self.bottom.layer,
            self.bottom.datatype,
            self.top.layer,
            self.top.datatype
        ))
    }
}

impl ContactStack {
    /// Returns a failure if either conductor fails to enclose the via by
    /// `min_enclosure` on all four sides, else `None`.
    fn enclosure_shortfall(
        &self,
        shapes: &[DrawShape],
        vi: usize,
        bi: usize,
        ti: usize,
    ) -> Option<CheckFailure> {
        let via = shape_bbox(&shapes[vi]);
        for (name, ci) in [("bottom", bi), ("top", ti)] {
            let cond = shape_bbox(&shapes[ci]);
            let margin = enclosure_margin(&cond, &via);
            if margin < self.min_enclosure {
                return Some(CheckFailure::new(format!(
                    "{name} conductor encloses the via by {margin} < required {}",
                    self.min_enclosure
                )));
            }
        }
        None
    }
}

// --------------------------------------------------------------------------------
// via_chain (a ladder of vias joining a stack of conductors into one net)
// --------------------------------------------------------------------------------

/// Passes iff at least `vias` shapes on the via layer all end up on a single net that
/// also spans both conductor layers, i.e. the whole chain is electrically continuous.
///
/// Parameters (`via_chain:via=68/44,vias=4`): `via` (required), `vias` (required
/// count), `bottom`/`top` (default from the SKY130 stack).
///
/// A broken chain (a missing landing pad, a via that misses its metal) splits the net
/// so no single net carries all `vias` vias, and the check fails.
#[derive(Clone, Copy, Debug)]
pub struct ViaChain {
    /// The via/contact layer.
    via: LayerId,
    /// The lower conductor layer.
    bottom: LayerId,
    /// The upper conductor layer.
    top: LayerId,
    /// Minimum number of via shapes that must share one net.
    vias: u32,
}

impl ViaChain {
    /// Builds a via-chain checker from parsed parameters.
    ///
    /// # Errors
    ///
    /// [`ParamError`] if `via`/`vias` are missing/malformed.
    pub fn from_params(p: &ParsedChecker) -> Result<Self, ParamError> {
        let via = p.layer("via")?;
        let (default_bottom, default_top) = sky130_stack_pair(via);
        let bottom = p.layer_or("bottom", default_bottom)?;
        let top = p.layer_or("top", default_top)?;
        let vias = p.u32("vias")?;
        Ok(Self {
            via,
            bottom,
            top,
            vias,
        })
    }
}

impl Checker for ViaChain {
    fn check(&self, doc: &Document, _transcript: &Transcript) -> CheckResult {
        let Some(cell) = target_cell(doc) else {
            return fail("document has no cell to check");
        };
        let shapes = doc.flatten(&cell);
        let mut dsu = build_components(&shapes, &sky130_connection_rules());

        let vias = indices_on_layer(&shapes, self.via);
        if (vias.len() as u32) < self.vias {
            return fail(format!(
                "found {} vias on {}/{}, need {}",
                vias.len(),
                self.via.layer,
                self.via.datatype,
                self.vias
            ));
        }
        // Root -> number of chain vias on that net.
        let mut per_root: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();
        for &vi in &vias {
            *per_root.entry(dsu.find(vi)).or_default() += 1;
        }
        let Some((&root, &best)) = per_root.iter().max_by_key(|&(_, n)| *n) else {
            return fail("no via net found");
        };
        if best < self.vias {
            return fail(format!(
                "the largest via net carries {best} of {} vias; the chain is broken",
                self.vias
            ));
        }
        // The continuous net must reach both conductor layers.
        let reaches_bottom = shapes
            .iter()
            .enumerate()
            .any(|(i, s)| s.layer == self.bottom && dsu.find(i) == root);
        let reaches_top = shapes
            .iter()
            .enumerate()
            .any(|(i, s)| s.layer == self.top && dsu.find(i) == root);
        if !reaches_bottom || !reaches_top {
            return fail(format!(
                "the via chain does not span both {}/{} and {}/{}",
                self.bottom.layer, self.bottom.datatype, self.top.layer, self.top.datatype
            ));
        }
        CheckResult::Pass
    }
}

// --------------------------------------------------------------------------------
// comb (two interdigitated combs on one layer, kept on separate nets)
// --------------------------------------------------------------------------------

/// Passes iff two combs on one layer are each a single connected net of at least
/// `fingers + 1` shapes (a spine plus its fingers) and the two combs are on
/// *different* nets (not shorted together).
///
/// Parameters (`comb:layer=68/20,fingers=4`): `layer` (required), `fingers`
/// (required, the finger count per comb).
///
/// Two interdigitated combs are the canonical capacitor/matching structure; the
/// failure mode is a finger of one comb touching the other comb, shorting them, which
/// collapses the two nets into one.
#[derive(Clone, Copy, Debug)]
pub struct Comb {
    /// Layer the combs are drawn on.
    layer: LayerId,
    /// Fingers per comb.
    fingers: u32,
}

impl Comb {
    /// Builds a comb checker from parsed parameters.
    ///
    /// # Errors
    ///
    /// [`ParamError`] if `layer`/`fingers` are missing/malformed.
    pub fn from_params(p: &ParsedChecker) -> Result<Self, ParamError> {
        let layer = p.layer("layer")?;
        let fingers = p.u32("fingers")?;
        Ok(Self { layer, fingers })
    }
}

impl Checker for Comb {
    fn check(&self, doc: &Document, _transcript: &Transcript) -> CheckResult {
        let Some(cell) = target_cell(doc) else {
            return fail("document has no cell to check");
        };
        let shapes = doc.flatten(&cell);
        let mut dsu = build_components(&shapes, &sky130_connection_rules());

        // Count shapes per net, restricted to the comb layer.
        let on_layer = indices_on_layer(&shapes, self.layer);
        let mut per_root: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();
        for &i in &on_layer {
            *per_root.entry(dsu.find(i)).or_default() += 1;
        }
        // A comb needs spine + fingers shapes on one net.
        let need = self.fingers + 1;
        let mut combs: Vec<u32> = per_root.values().copied().filter(|&n| n >= need).collect();
        combs.sort_unstable();
        if combs.len() < 2 {
            return fail(format!(
                "expected two separate combs of >= {need} shapes on {}/{}, found {} \
                 (a short would merge them into one net)",
                self.layer.layer,
                self.layer.datatype,
                combs.len()
            ));
        }
        CheckResult::Pass
    }
}

// --------------------------------------------------------------------------------
// guard_ring (a closed rectangular loop enclosing a hole)
// --------------------------------------------------------------------------------

/// How far inside each outer edge a guard-ring edge probe sits, in DBU.
///
/// Ring material starts exactly at the outer extent, so one DBU inside is within the
/// band of any ring at least two DBU thick regardless of the ring's thickness.
const RING_PROBE_INSET: i32 = 1;

/// Passes iff the shapes on a layer form a single connected net that encloses a hole,
/// i.e. a closed guard ring rather than an open C.
///
/// Parameters (`guard_ring:layer=67/20`): `layer` (required).
///
/// The test is topological on the DBU grid: the ring's own shapes must be one net
/// (closed, not broken into two arcs), the four inner mid-edge points just inside the
/// ring must be covered (there is material on every side), and the centre of the ring
/// must be *uncovered* (there is a hole to guard). An open ring fails the single-net
/// test; a filled rectangle fails the hole test.
#[derive(Clone, Copy, Debug)]
pub struct GuardRing {
    /// Layer the ring is drawn on.
    layer: LayerId,
}

impl GuardRing {
    /// Builds a guard-ring checker from parsed parameters.
    ///
    /// # Errors
    ///
    /// [`ParamError`] if `layer` is missing/malformed.
    pub fn from_params(p: &ParsedChecker) -> Result<Self, ParamError> {
        Ok(Self {
            layer: p.layer("layer")?,
        })
    }
}

impl Checker for GuardRing {
    fn check(&self, doc: &Document, _transcript: &Transcript) -> CheckResult {
        let Some(cell) = target_cell(doc) else {
            return fail("document has no cell to check");
        };
        let shapes = doc.flatten(&cell);
        let ring: Vec<usize> = indices_on_layer(&shapes, self.layer);
        if ring.is_empty() {
            return fail(format!(
                "no shapes on ring layer {}/{}",
                self.layer.layer, self.layer.datatype
            ));
        }
        // The ring must be a single connected net (a break splits it into two arcs).
        let mut dsu = build_components(&shapes, &sky130_connection_rules());
        let root0 = dsu.find(ring[0]);
        if ring.iter().any(|&i| dsu.find(i) != root0) {
            return fail("the ring is not a single connected net; it is open");
        }
        // Outer extent of the ring.
        let Some(outer) = ring
            .iter()
            .map(|&i| shape_bbox(&shapes[i]))
            .reduce(|a, b| a.union(&b))
        else {
            return fail("ring has no extent");
        };
        // The centre must be uncovered (a hole), and each edge's mid-band must be
        // covered (material on every side). Ring material starts exactly at the outer
        // extent, so a probe one DBU inside each outer edge lands within the band of
        // any ring at least two DBU thick, independent of the ring's thickness.
        let cx = i32::midpoint(outer.min.x, outer.max.x);
        let cy = i32::midpoint(outer.min.y, outer.max.y);
        let centre = Point::new(cx, cy);
        if covers_point(&shapes, &ring, centre) {
            return fail("the ring has no hole to guard; the interior is filled");
        }
        let probes = [
            Point::new(cx, outer.min.y + RING_PROBE_INSET), // just inside bottom edge
            Point::new(cx, outer.max.y - RING_PROBE_INSET), // just inside top edge
            Point::new(outer.min.x + RING_PROBE_INSET, cy), // just inside left edge
            Point::new(outer.max.x - RING_PROBE_INSET, cy), // just inside right edge
        ];
        for (side, probe) in ["bottom", "top", "left", "right"].into_iter().zip(probes) {
            if !covers_point(&shapes, &ring, probe) {
                return fail(format!(
                    "the ring is not closed on the {side} side (no material there)"
                ));
            }
        }
        CheckResult::Pass
    }
}

// --------------------------------------------------------------------------------
// compound_cell (a top cell that places sub-cell instances)
// --------------------------------------------------------------------------------

/// Passes iff the target (top) cell places at least `instances` sub-cell instances,
/// counting array placements by their instance count, and optionally the flattened
/// design has at least `min_shapes` leaf shapes.
///
/// Parameters (`compound_cell:instances=2,min_shapes=6`): `instances` (required),
/// `min_shapes` (optional).
///
/// Compound tasks ask the model to build a cell from smaller cells; this verifies the
/// hierarchy was actually assembled (instances placed) rather than drawn flat.
/// Connectivity of the assembled design is checked separately by an `intent` task.
#[derive(Clone, Copy, Debug)]
pub struct CompoundCell {
    /// Minimum number of placed instances (arrays counted by element count).
    instances: u32,
    /// Minimum number of leaf shapes after flattening, if required.
    min_shapes: Option<u32>,
}

impl CompoundCell {
    /// Builds a compound-cell checker from parsed parameters.
    ///
    /// # Errors
    ///
    /// [`ParamError`] if `instances` is missing/malformed.
    pub fn from_params(p: &ParsedChecker) -> Result<Self, ParamError> {
        let instances = p.u32("instances")?;
        let min_shapes = optional_u32(p, "min_shapes")?;
        Ok(Self {
            instances,
            min_shapes,
        })
    }
}

impl Checker for CompoundCell {
    fn check(&self, doc: &Document, _transcript: &Transcript) -> CheckResult {
        let Some(cell_name) = target_cell(doc) else {
            return fail("document has no cell to check");
        };
        let Some(cell) = doc.cell(&cell_name) else {
            return fail("target cell not found");
        };
        let placed: u64 = cell.instances.len() as u64
            + cell
                .arrays
                .iter()
                .map(reticle_model::ArrayInstance::count)
                .sum::<u64>();
        if placed < u64::from(self.instances) {
            return fail(format!(
                "top cell places {placed} instances, expected at least {}",
                self.instances
            ));
        }
        if let Some(min_shapes) = self.min_shapes {
            let flat = doc.flatten(&cell_name).len() as u32;
            if flat < min_shapes {
                return fail(format!(
                    "flattened design has {flat} shapes, expected at least {min_shapes}"
                ));
            }
        }
        CheckResult::Pass
    }
}

// --------------------------------------------------------------------------------
// shared helpers
// --------------------------------------------------------------------------------

/// The SKY130 conductor pair a via/contact layer bridges, bottom to top; falls back
/// to `(via, via)` for an unknown layer (which then matches nothing and fails
/// cleanly). Mirrors [`sky130_connection_rules`].
fn sky130_stack_pair(via: LayerId) -> (LayerId, LayerId) {
    const LICON1: LayerId = LayerId::new(66, 44);
    const MCON: LayerId = LayerId::new(67, 44);
    const VIA1: LayerId = LayerId::new(68, 44);
    const VIA2: LayerId = LayerId::new(69, 44);
    const VIA3: LayerId = LayerId::new(70, 44);
    match via {
        LICON1 => (LayerId::new(66, 20), LayerId::new(67, 20)), // poly -> li1
        MCON => (LayerId::new(67, 20), LayerId::new(68, 20)),   // li1 -> met1
        VIA1 => (LayerId::new(68, 20), LayerId::new(69, 20)),   // met1 -> met2
        VIA2 => (LayerId::new(69, 20), LayerId::new(70, 20)),   // met2 -> met3
        VIA3 => (LayerId::new(70, 20), LayerId::new(71, 20)),   // met3 -> met4
        other => (other, other),
    }
}

/// Indices of `shapes` that lie on `layer`.
fn indices_on_layer(shapes: &[DrawShape], layer: LayerId) -> Vec<usize> {
    shapes
        .iter()
        .enumerate()
        .filter(|(_, s)| s.layer == layer)
        .map(|(i, _)| i)
        .collect()
}

/// The first shape on `layer` that shares `anchor`'s net, if any.
fn any_connected_on_layer(
    shapes: &[DrawShape],
    dsu: &mut DisjointSet,
    anchor: usize,
    layer: LayerId,
) -> Option<usize> {
    let root = dsu.find(anchor);
    (0..shapes.len()).find(|&i| shapes[i].layer == layer && dsu.find(i) == root)
}

/// The directional enclosure margin of `outer` around `inner`: the smallest of the
/// four gaps (left, right, bottom, top). Negative if `inner` pokes outside `outer`.
fn enclosure_margin(outer: &Rect, inner: &Rect) -> i64 {
    let left = i64::from(inner.min.x) - i64::from(outer.min.x);
    let right = i64::from(outer.max.x) - i64::from(inner.max.x);
    let bottom = i64::from(inner.min.y) - i64::from(outer.min.y);
    let top = i64::from(outer.max.y) - i64::from(inner.max.y);
    left.min(right).min(bottom).min(top)
}

/// Returns `true` if any of the `ring` shapes covers `point` (closed bounding box).
fn covers_point(shapes: &[DrawShape], ring: &[usize], point: Point) -> bool {
    ring.iter().any(|&i| {
        let b = shape_bbox(&shapes[i]);
        point.x >= b.min.x && point.x <= b.max.x && point.y >= b.min.y && point.y <= b.max.y
    })
}

/// Reads an optional `u32` parameter (absent -> `None`).
fn optional_u32(p: &ParsedChecker, key: &str) -> Result<Option<u32>, ParamError> {
    if p.has(key) {
        Ok(Some(p.u32(key)?))
    } else {
        Ok(None)
    }
}

/// Reads an optional `i64` parameter (absent -> `None`).
fn optional_i64(p: &ParsedChecker, key: &str) -> Result<Option<i64>, ParamError> {
    if p.has(key) {
        Ok(Some(p.i64(key)?))
    } else {
        Ok(None)
    }
}

/// Shorthand for a single-reason failure.
fn fail(reason: impl Into<String>) -> CheckResult {
    CheckResult::Fail(vec![CheckFailure::new(reason)])
}

#[cfg(test)]
mod tests {
    use super::build;
    use crate::checker::CheckResult;
    use crate::params::ParsedChecker;
    use reticle_agent_api::Transcript;
    use reticle_geometry::{LayerId, Point, Rect, Transform};
    use reticle_model::{Cell, Document, DrawShape, Instance, ShapeKind};

    // SKY130 layers exercised by the tests.
    const MET1: LayerId = LayerId::new(68, 20);
    const MET2: LayerId = LayerId::new(69, 20);
    const VIA1: LayerId = LayerId::new(68, 44);
    const LI1: LayerId = LayerId::new(67, 20);

    /// A rectangle shape on `layer` spanning `(x0,y0)-(x1,y1)`.
    fn rect(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    }

    /// A one-cell document named `top` holding `shapes`.
    fn doc_with(shapes: Vec<DrawShape>) -> Document {
        let mut cell = Cell::new("top");
        cell.shapes = shapes;
        let mut doc = Document::new();
        doc.insert_cell(cell);
        doc
    }

    /// Builds the checker named by `spec` and runs it over `doc`.
    fn run(spec: &str, doc: &Document) -> CheckResult {
        let parsed = ParsedChecker::parse(spec);
        let checker = build(&parsed)
            .expect("checker params parse")
            .expect("spec names a geometric checker");
        checker.check(doc, &Transcript::default())
    }

    /// Asserts a spec passes on `doc`.
    fn assert_pass(spec: &str, doc: &Document) {
        assert!(
            run(spec, doc).is_pass(),
            "expected `{spec}` to PASS but it failed: {:?}",
            run(spec, doc)
        );
    }

    /// Asserts a spec fails on `doc` (with at least one reason).
    fn assert_fail(spec: &str, doc: &Document) {
        assert!(
            matches!(run(spec, doc), CheckResult::Fail(f) if !f.is_empty()),
            "expected `{spec}` to FAIL but it passed"
        );
    }

    // ---- shape_count ------------------------------------------------------------

    #[test]
    fn shape_count_two_way() {
        // Good: exactly 3 met1 rects satisfies min=3.
        let good = doc_with(vec![
            rect(MET1, 0, 0, 200, 200),
            rect(MET1, 300, 0, 500, 200),
            rect(MET1, 600, 0, 800, 200),
        ]);
        assert_pass("shape_count:layer=68/20,min=3", &good);
        assert_pass("shape_count:layer=68/20,exact=3", &good);
        // Bad: only 2 met1 rects, but min=3 required.
        let bad = doc_with(vec![
            rect(MET1, 0, 0, 200, 200),
            rect(MET1, 300, 0, 500, 200),
        ]);
        assert_fail("shape_count:layer=68/20,min=3", &bad);
        // Bad the other way: exact=2 but there are 3.
        assert_fail("shape_count:layer=68/20,exact=2", &good);
        // Wrong layer counts zero.
        assert_fail("shape_count:layer=69/20,min=1", &good);
    }

    // ---- layer_area -------------------------------------------------------------

    #[test]
    fn layer_area_two_way() {
        // A 500x500 met1 rect has area 250000, above the 83000 min-area rule.
        let good = doc_with(vec![rect(MET1, 0, 0, 500, 500)]);
        assert_pass("layer_area:layer=68/20,min_area=83000", &good);
        // Bad: a 100x100 rect is only 10000, below the threshold.
        let bad = doc_with(vec![rect(MET1, 0, 0, 100, 100)]);
        assert_fail("layer_area:layer=68/20,min_area=83000", &bad);
        // max_area is enforced too: 250000 exceeds a 100000 cap.
        assert_fail("layer_area:layer=68/20,max_area=100000", &good);
    }

    // ---- contact_stack ----------------------------------------------------------

    #[test]
    fn contact_stack_two_way() {
        // Good: met1 and met2 both cover the same region, a via bridges them.
        let good = doc_with(vec![
            rect(MET1, 0, 0, 300, 300),
            rect(MET2, 0, 0, 300, 300),
            rect(VIA1, 55, 55, 245, 245), // 55 DBU enclosure on all sides
        ]);
        assert_pass("contact_stack:via=68/44", &good);
        assert_pass("contact_stack:via=68/44,min_enclosure=55", &good);
        // Bad: the via lands on met1 but met2 is elsewhere, so nothing bridges.
        let no_top = doc_with(vec![
            rect(MET1, 0, 0, 300, 300),
            rect(MET2, 1000, 1000, 1300, 1300),
            rect(VIA1, 100, 100, 250, 250),
        ]);
        assert_fail("contact_stack:via=68/44", &no_top);
        // Bad: no via shape at all.
        let no_via = doc_with(vec![rect(MET1, 0, 0, 300, 300), rect(MET2, 0, 0, 300, 300)]);
        assert_fail("contact_stack:via=68/44", &no_via);
        // Bad on enclosure: the via reaches the met1 edge (0 margin) though it connects.
        let tight = doc_with(vec![
            rect(MET1, 0, 0, 300, 300),
            rect(MET2, 0, 0, 300, 300),
            rect(VIA1, 0, 0, 150, 150),
        ]);
        assert_pass("contact_stack:via=68/44", &tight); // topology is fine
        assert_fail("contact_stack:via=68/44,min_enclosure=30", &tight); // enclosure is not
    }

    // ---- via_chain --------------------------------------------------------------

    #[test]
    fn via_chain_two_way() {
        // Good: one met1 and one met2 plane, four vias all landing on both, so all
        // four vias share the single met1+met2 net that spans both layers.
        let good = doc_with(vec![
            rect(MET1, 0, 0, 1000, 200),
            rect(MET2, 0, 0, 1000, 200),
            rect(VIA1, 50, 50, 150, 150),
            rect(VIA1, 250, 50, 350, 150),
            rect(VIA1, 450, 50, 550, 150),
            rect(VIA1, 650, 50, 750, 150),
        ]);
        assert_pass("via_chain:via=68/44,vias=4", &good);
        // Bad: one of the four vias floats off the metal, so only three vias are on
        // the continuous net.
        let broken = doc_with(vec![
            rect(MET1, 0, 0, 1000, 200),
            rect(MET2, 0, 0, 1000, 200),
            rect(VIA1, 50, 50, 150, 150),
            rect(VIA1, 250, 50, 350, 150),
            rect(VIA1, 450, 50, 550, 150),
            rect(VIA1, 5000, 5000, 5100, 5100), // off in the corner, connects nothing
        ]);
        assert_fail("via_chain:via=68/44,vias=4", &broken);
        // Bad: too few vias entirely.
        let few = doc_with(vec![
            rect(MET1, 0, 0, 400, 200),
            rect(MET2, 0, 0, 400, 200),
            rect(VIA1, 50, 50, 150, 150),
        ]);
        assert_fail("via_chain:via=68/44,vias=4", &few);
    }

    // ---- comb -------------------------------------------------------------------

    #[test]
    fn comb_two_way() {
        // Two combs on met1, each a spine plus two fingers, kept apart. Comb A near
        // the bottom, comb B near the top, fingers interdigitated in x but not
        // touching across the two nets.
        let two_combs = doc_with(vec![
            // Comb A: vertical spine at x[0,20], fingers reaching right.
            rect(MET1, 0, 0, 20, 400),
            rect(MET1, 20, 40, 300, 80),
            rect(MET1, 20, 240, 300, 280),
            // Comb B: vertical spine at x[380,400], fingers reaching left, offset in y
            // so they interleave with A's fingers without touching.
            rect(MET1, 380, 0, 400, 400),
            rect(MET1, 100, 140, 380, 180),
            rect(MET1, 100, 340, 380, 380),
        ]);
        assert_pass("comb:layer=68/20,fingers=2", &two_combs);
        // Bad: comb B's finger is extended left to touch comb A's spine, shorting the
        // two nets into one; now there is only a single comb net.
        let shorted = doc_with(vec![
            rect(MET1, 0, 0, 20, 400),
            rect(MET1, 20, 40, 300, 80),
            rect(MET1, 20, 240, 300, 280),
            rect(MET1, 380, 0, 400, 400),
            rect(MET1, 0, 140, 380, 180), // reaches x=0, touches comb A spine
            rect(MET1, 100, 340, 380, 380),
        ]);
        assert_fail("comb:layer=68/20,fingers=2", &shorted);
        // Bad: only one comb present.
        let one_comb = doc_with(vec![
            rect(MET1, 0, 0, 20, 400),
            rect(MET1, 20, 40, 300, 80),
            rect(MET1, 20, 240, 300, 280),
        ]);
        assert_fail("comb:layer=68/20,fingers=2", &one_comb);
    }

    // ---- guard_ring -------------------------------------------------------------

    #[test]
    fn guard_ring_two_way() {
        // Good: four li1 rectangles forming a closed square loop around a hollow
        // centre. Adjacent segments share corners (closed-box touch), so it is one
        // connected net; the centre is uncovered.
        let closed = doc_with(vec![
            rect(LI1, 0, 0, 1000, 100),    // bottom
            rect(LI1, 0, 900, 1000, 1000), // top
            rect(LI1, 0, 0, 100, 1000),    // left
            rect(LI1, 900, 0, 1000, 1000), // right
        ]);
        assert_pass("guard_ring:layer=67/20", &closed);
        // Bad: drop the right edge, leaving an open C. The right side has no material.
        let open_c = doc_with(vec![
            rect(LI1, 0, 0, 1000, 100),
            rect(LI1, 0, 900, 1000, 1000),
            rect(LI1, 0, 0, 100, 1000),
        ]);
        assert_fail("guard_ring:layer=67/20", &open_c);
        // Bad: a solid filled square has no hole to guard.
        let filled = doc_with(vec![rect(LI1, 0, 0, 1000, 1000)]);
        assert_fail("guard_ring:layer=67/20", &filled);
        // Bad: the ring is split into two disconnected L-shaped arcs (a gap on both
        // the top-right and bottom-left corners), so it is not a single net.
        let two_arcs = doc_with(vec![
            rect(LI1, 0, 0, 500, 100),       // bottom-left arc: bottom part
            rect(LI1, 0, 0, 100, 500),       // bottom-left arc: left part
            rect(LI1, 900, 500, 1000, 1000), // top-right arc: right part
            rect(LI1, 500, 900, 1000, 1000), // top-right arc: top part
        ]);
        assert_fail("guard_ring:layer=67/20", &two_arcs);
    }

    // ---- compound_cell ----------------------------------------------------------

    /// A document whose top cell places `n` instances of an (empty) leaf cell, each
    /// translated so they do not stack.
    fn doc_with_instances(n: usize) -> Document {
        let mut leaf = Cell::new("leaf");
        leaf.shapes.push(rect(MET1, 0, 0, 200, 200));
        let mut top = Cell::new("top");
        for i in 0..n {
            top.instances.push(Instance {
                cell: "leaf".into(),
                transform: Transform::translate(i as i32 * 400, 0),
            });
        }
        let mut doc = Document::new();
        doc.insert_cell(leaf);
        doc.insert_cell(top);
        doc.set_top_cells(vec!["top".into()]);
        doc
    }

    #[test]
    fn compound_cell_two_way() {
        // Good: the top cell places two instances.
        let good = doc_with_instances(2);
        assert_pass("compound_cell:instances=2", &good);
        // With min_shapes: two leaves of one shape each flatten to two shapes.
        assert_pass("compound_cell:instances=2,min_shapes=2", &good);
        // Bad: only one instance placed, but two required.
        let bad = doc_with_instances(1);
        assert_fail("compound_cell:instances=2", &bad);
        // Bad: instances placed but the flattened shape count falls short.
        assert_fail("compound_cell:instances=2,min_shapes=5", &good);
    }

    // ---- dispatch / param errors ------------------------------------------------

    #[test]
    fn unknown_name_is_not_a_geometric_checker() {
        // `build` returns Ok(None) for names it does not own, so the registry can
        // fall through to its built-ins.
        assert!(
            build(&ParsedChecker::parse("drc_clean"))
                .expect("no param error")
                .is_none()
        );
    }

    #[test]
    fn missing_required_param_is_an_error() {
        // shape_count needs a layer; omitting it is a build error, not a silent pass.
        assert!(build(&ParsedChecker::parse("shape_count:min=3")).is_err());
        assert!(build(&ParsedChecker::parse("contact_stack")).is_err());
        assert!(build(&ParsedChecker::parse("via_chain:via=68/44")).is_err());
    }
}
