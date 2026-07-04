//! Geometric benchmark checkers beyond the primitive `rect_present` / `drc_clean` /
//! `intent` set.
//!
//! These decide *structured* layout tasks: how many shapes on a layer, how much area,
//! the area a planar boolean wrote to a layer (which pins union vs intersection vs
//! difference), a regular array placed at an expected pitch, a contact/via stack that
//! actually bridges two conductors, a via chain, an interdigitated comb, a closed guard
//! ring, and a compound cell that places sub-cells. Each is built from a
//! [`ParsedChecker`] so the layer, counts, and thresholds come from the task's
//! `checker` string (see [`crate::params`]); each is unit-tested in both directions.
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
use reticle_model::{Document, DrawShape, RuleSet, ShapeKind};

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
/// The recognized names are `shape_count`, `layer_area`, `boolean_result`,
/// `array_pitch`, `contact_stack`, `via_chain`, `comb`, `guard_ring`,
/// `compound_cell`, and `generator`. An unrecognized name yields `Ok(None)` so the
/// caller can fall through to the built-in registry.
///
/// # Errors
///
/// Returns a [`ParamError`] when a recognized checker is missing a required parameter
/// or a parameter does not parse.
pub fn build(parsed: &ParsedChecker) -> Result<Option<Box<dyn Checker>>, ParamError> {
    let checker: Box<dyn Checker> = match parsed.name() {
        "shape_count" => Box::new(ShapeCount::from_params(parsed)?),
        "layer_area" => Box::new(LayerArea::from_params(parsed)?),
        "boolean_result" => Box::new(BooleanResult::from_params(parsed)?),
        "array_pitch" => Box::new(ArrayPitch::from_params(parsed)?),
        "contact_stack" => Box::new(ContactStack::from_params(parsed)?),
        "via_chain" => Box::new(ViaChain::from_params(parsed)?),
        "comb" => Box::new(Comb::from_params(parsed)?),
        "guard_ring" => Box::new(GuardRing::from_params(parsed)?),
        "compound_cell" => Box::new(CompoundCell::from_params(parsed)?),
        "generator" => Box::new(GeneratorCheck::from_params(parsed)?),
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
// boolean_result (a planar boolean wrote its result to a layer)
// --------------------------------------------------------------------------------

/// Passes iff a planar boolean's result landed on a layer with an area inside an
/// expected window, and (optionally) the input layer was consumed (left empty).
///
/// Parameters (`boolean_result:layer=68/20,min_area=175000,max_area=175000,cleared=69/20`):
/// `layer` (required, where the result polygons are written), `min_area`/`max_area`
/// (the inclusive DBU² window the result area must fall in; a union, an intersection,
/// and a difference of the same two inputs produce three distinct areas, so a tight
/// window pins which boolean was performed), and `cleared` (optional: a layer that must
/// hold zero shapes afterward, proving the boolean *consumed* its inputs rather than
/// leaving them lying beside a copied result).
///
/// `boolean_combine` deletes its inputs and writes result polygons to the target layer,
/// so the discriminating failure mode is "drew the wrong op" (area lands outside the
/// window) or "did not run a boolean at all" (inputs still present on `cleared`, or no
/// area on `layer`). Area is summed with the same per-shape area used by
/// [`LayerArea`], so a polygon result is measured by its shoelace area.
#[derive(Clone, Copy, Debug)]
pub struct BooleanResult {
    /// Layer the boolean result polygons must be on.
    layer: LayerId,
    /// Inclusive minimum result area in DBU².
    min_area: i64,
    /// Inclusive maximum result area in DBU², if any.
    max_area: Option<i64>,
    /// A layer that must be empty afterward (the consumed inputs), if required.
    cleared: Option<LayerId>,
}

impl BooleanResult {
    /// Builds a boolean-result checker from parsed parameters.
    ///
    /// # Errors
    ///
    /// [`ParamError`] if `layer` is missing/malformed or a bound/`cleared` does not parse.
    pub fn from_params(p: &ParsedChecker) -> Result<Self, ParamError> {
        let layer = p.layer("layer")?;
        let min_area = p.i64_or("min_area", 1)?;
        let max_area = optional_i64(p, "max_area")?;
        let cleared = if p.has("cleared") {
            Some(p.layer("cleared")?)
        } else {
            None
        };
        Ok(Self {
            layer,
            min_area,
            max_area,
            cleared,
        })
    }
}

impl Checker for BooleanResult {
    fn check(&self, doc: &Document, _transcript: &Transcript) -> CheckResult {
        let Some(cell) = target_cell(doc) else {
            return fail("document has no cell to check");
        };
        let total: i64 = shapes_on_layer(doc, &cell, self.layer)
            .map(shape_area)
            .sum();
        if total < self.min_area {
            return fail(format!(
                "boolean result on {}/{} has area {total} < expected minimum {}",
                self.layer.layer, self.layer.datatype, self.min_area
            ));
        }
        if let Some(max) = self.max_area
            && total > max
        {
            return fail(format!(
                "boolean result on {}/{} has area {total} > expected maximum {max} \
                 (the wrong boolean op, or stray input geometry, inflates the area)",
                self.layer.layer, self.layer.datatype
            ));
        }
        if let Some(cleared) = self.cleared {
            let remaining = shapes_on_layer(doc, &cell, cleared).count();
            if remaining > 0 {
                return fail(format!(
                    "input layer {}/{} still holds {remaining} shape(s); the boolean did \
                     not consume its inputs",
                    cleared.layer, cleared.datatype
                ));
            }
        }
        CheckResult::Pass
    }
}

// --------------------------------------------------------------------------------
// array_pitch (a regular array placed at an expected pitch)
// --------------------------------------------------------------------------------

/// Passes iff the target cell holds an array of at least `instances` placements whose
/// column and row pitch match an expected pitch.
///
/// Parameters (`array_pitch:instances=4,pitch=800` or
/// `array_pitch:instances=6,col_pitch=800,row_pitch=600`): `instances` (required, the
/// minimum `columns * rows`), and the pitch, given either as one `pitch` applied to
/// both axes or as separate `col_pitch`/`row_pitch`. A single-axis array (a row) has a
/// span of one along the other axis, so its pitch on that axis is unconstrained and is
/// only checked when that axis actually repeats (span > 1).
///
/// `place_array` records `columns`, `rows`, `column_pitch`, and `row_pitch` on an
/// [`ArrayInstance`](reticle_model::ArrayInstance); this reads them back. Unlike
/// [`CompoundCell`], which only counts placements, this pins the *step*: an array
/// placed at the wrong pitch (overlapping, or too sparse) is rejected even though it
/// has the right instance count.
#[derive(Clone, Copy, Debug)]
pub struct ArrayPitch {
    /// Minimum number of placements (`columns * rows`) the array must carry.
    instances: u32,
    /// Expected column pitch in DBU, if constrained.
    col_pitch: Option<i32>,
    /// Expected row pitch in DBU, if constrained.
    row_pitch: Option<i32>,
}

impl ArrayPitch {
    /// Builds an array-pitch checker from parsed parameters.
    ///
    /// A lone `pitch` sets both axes; `col_pitch`/`row_pitch` override per axis.
    ///
    /// # Errors
    ///
    /// [`ParamError`] if `instances` is missing/malformed or a pitch does not parse.
    pub fn from_params(p: &ParsedChecker) -> Result<Self, ParamError> {
        let instances = p.u32("instances")?;
        let both = optional_i32(p, "pitch")?;
        let col_pitch = optional_i32(p, "col_pitch")?.or(both);
        let row_pitch = optional_i32(p, "row_pitch")?.or(both);
        Ok(Self {
            instances,
            col_pitch,
            row_pitch,
        })
    }
}

impl Checker for ArrayPitch {
    fn check(&self, doc: &Document, _transcript: &Transcript) -> CheckResult {
        let Some(cell_name) = target_cell(doc) else {
            return fail("document has no cell to check");
        };
        let Some(cell) = doc.cell(&cell_name) else {
            return fail("target cell not found");
        };
        if cell.arrays.is_empty() {
            return fail("target cell places no array");
        }
        // Accept the cell if *any* array satisfies count and pitch; report the closest
        // miss otherwise.
        let mut last_reason = String::from("no array in the target cell meets the count and pitch");
        for array in &cell.arrays {
            if array.count() < u64::from(self.instances) {
                last_reason = format!(
                    "array places {} instances ({}x{}), expected at least {}",
                    array.count(),
                    array.columns,
                    array.rows,
                    self.instances
                );
                continue;
            }
            if let Some(reason) = self.pitch_mismatch(array) {
                last_reason = reason;
                continue;
            }
            return CheckResult::Pass;
        }
        fail(last_reason)
    }
}

impl ArrayPitch {
    /// Returns a mismatch reason if a constrained, actually-repeating axis has the wrong
    /// pitch, else `None`. A pitch on an axis that does not repeat (span 1) is ignored.
    fn pitch_mismatch(&self, array: &reticle_model::ArrayInstance) -> Option<String> {
        if let Some(want) = self.col_pitch
            && array.columns > 1
            && i64::from(array.column_pitch) != i64::from(want)
        {
            return Some(format!(
                "array column pitch is {}, expected {want}",
                array.column_pitch
            ));
        }
        if let Some(want) = self.row_pitch
            && array.rows > 1
            && i64::from(array.row_pitch) != i64::from(want)
        {
            return Some(format!(
                "array row pitch is {}, expected {want}",
                array.row_pitch
            ));
        }
        None
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
// generator (a parameterized-generator task, verified by re-running the generator)
// --------------------------------------------------------------------------------

/// Passes iff the graded document contains the geometry a named
/// [`reticle_gen`] generator emits for the task's parameters, and the target cell is
/// DRC-clean under the SKY130 subset.
///
/// Parameters (`generator:id=guard_ring,layer=li1,region_width=2000,...`): `id`
/// (required, the generator id), plus any of that generator's own parameters by
/// their schema field names. Any parameter the checker string omits takes the
/// generator's schema default, so a task only needs to pin the parameters its prompt
/// asks for (for example a `4x4` via farm pins `rows`/`cols`).
///
/// The test is generator-driven rather than hand-coded per structure: the checker
/// re-runs the named generator with the resolved parameters into a scratch cell to
/// get a *reference*, reduces it to a per-layer shape-count histogram (its
/// fingerprint), and requires the graded document's target cell to carry at least
/// that many shapes on each of those layers. Because generators are DRC-clean by
/// construction, a model that reproduced the structure lands the same fingerprint;
/// extra shapes (a larger design, labels) never make the check fail. The
/// DRC-cleanliness half reuses the same SKY130 subset engine as [`crate::checkers::DrcClean`],
/// so "clean" means exactly what it means for every other task.
#[derive(Clone, Debug)]
pub struct GeneratorCheck {
    /// The generator id (matches the registry key and the `RunGenerator` id).
    id: String,
    /// The resolved generator parameters, as the JSON the registry consumes.
    params: serde_json::Value,
}

impl GeneratorCheck {
    /// Builds a generator checker from parsed parameters.
    ///
    /// Reads `id`, then resolves the generator's parameter object from the checker's
    /// key/value pairs against that generator's [`reticle_gen::ParamSchema`]: each
    /// schema field is taken from the checker string when present (parsed by its
    /// field type) and from the schema default otherwise.
    ///
    /// # Errors
    ///
    /// [`ParamError`] if `id` is absent, names no registered generator, or a supplied
    /// parameter does not parse as its field type.
    pub fn from_params(p: &ParsedChecker) -> Result<Self, ParamError> {
        let id = p
            .get("id")
            .ok_or_else(|| ParamError::Missing {
                key: "id".to_owned(),
            })?
            .to_owned();
        let registry = reticle_gen::Registry::with_builtins();
        let schema = registry.schema(&id).ok_or_else(|| ParamError::Invalid {
            key: "id".to_owned(),
            value: id.clone(),
            expected: "a registered generator id",
        })?;
        let params = resolve_generator_params(p, &schema)?;
        Ok(Self { id, params })
    }
}

impl Checker for GeneratorCheck {
    fn check(&self, doc: &Document, _transcript: &Transcript) -> CheckResult {
        let Some(cell_name) = target_cell(doc) else {
            return fail("document has no cell to check");
        };

        // Build the reference geometry by running the generator itself, then reduce
        // it to a per-layer shape-count fingerprint. The generator uses baked SKY130
        // numbers, so a default technology is all it needs.
        let registry = reticle_gen::Registry::with_builtins();
        let tech = reticle_model::Technology::default();
        let mut reference = reticle_model::Cell::new("__reference");
        if let Err(e) = registry.generate(&self.id, &self.params, &tech, &mut reference) {
            // A checker that cannot build its own reference is a task-authoring bug,
            // surfaced as a failure rather than a panic.
            return fail(format!(
                "generator `{}` could not build a reference: {e}",
                self.id
            ));
        }
        let want = layer_histogram(&reference.shapes);

        // The graded document must carry at least the reference count on every layer
        // the generator touches (flattened, so a hierarchical answer still counts).
        let have = layer_histogram(&doc.flatten(&cell_name));
        let mut failures = Vec::new();
        for (layer, want_count) in &want {
            let have_count = have.get(layer).copied().unwrap_or(0);
            if have_count < *want_count {
                failures.push(CheckFailure::new(format!(
                    "generator `{}` expects at least {want_count} shapes on layer {}/{}, found {have_count}",
                    self.id, layer.layer, layer.datatype
                )));
            }
        }

        // The structure must also be DRC-clean under the SKY130 subset (the same
        // engine the drc_clean checker uses).
        let engine = reticle_drc::DrcEngine::new(reticle_drc::sky130_drc_rules());
        for v in engine.check_cell(doc, &cell_name) {
            failures.push(CheckFailure::new(format!(
                "generated structure is not DRC-clean: {}: {}",
                v.rule, v.message
            )));
        }

        if failures.is_empty() {
            CheckResult::Pass
        } else {
            CheckResult::Fail(failures)
        }
    }
}

/// Resolves a generator's full parameter object from a checker string plus its
/// schema: each schema field is taken from the checker's key/value pairs when
/// present (parsed by the field type) and from the schema default otherwise.
fn resolve_generator_params(
    p: &ParsedChecker,
    schema: &reticle_gen::ParamSchema,
) -> Result<serde_json::Value, ParamError> {
    use reticle_gen::FieldType;
    let mut obj = serde_json::Map::new();
    for field in &schema.fields {
        let value = match p.get(&field.name) {
            None => field.default.clone(),
            Some(raw) => match &field.ty {
                FieldType::Int { .. } => {
                    let n = raw.parse::<i64>().map_err(|_| ParamError::Invalid {
                        key: field.name.clone(),
                        value: raw.to_owned(),
                        expected: "i64",
                    })?;
                    serde_json::Value::from(n)
                }
                FieldType::Bool => {
                    let b = raw.parse::<bool>().map_err(|_| ParamError::Invalid {
                        key: field.name.clone(),
                        value: raw.to_owned(),
                        expected: "bool",
                    })?;
                    serde_json::Value::from(b)
                }
                FieldType::Enum { .. } => serde_json::Value::from(raw.to_owned()),
            },
        };
        obj.insert(field.name.clone(), value);
    }
    Ok(serde_json::Value::Object(obj))
}

/// A per-layer shape-count histogram of `shapes`.
fn layer_histogram(shapes: &[DrawShape]) -> std::collections::BTreeMap<LayerId, u32> {
    let mut hist = std::collections::BTreeMap::new();
    for s in shapes {
        *hist.entry(s.layer).or_insert(0) += 1;
    }
    hist
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

/// Reads an optional `i32` parameter (absent -> `None`).
///
/// Parses through the checker string's `i64` accessor and narrows to `i32`, so an
/// out-of-range value is reported as a malformed parameter rather than silently
/// wrapping. Pitches and DBU deltas are `i32` on the model side.
fn optional_i32(p: &ParsedChecker, key: &str) -> Result<Option<i32>, ParamError> {
    match optional_i64(p, key)? {
        None => Ok(None),
        Some(v) => i32::try_from(v).map(Some).map_err(|_| ParamError::Invalid {
            key: key.to_owned(),
            value: v.to_string(),
            expected: "i32",
        }),
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
    use reticle_geometry::{LayerId, Point, Polygon, Rect, Transform};
    use reticle_model::{ArrayInstance, Cell, Document, DrawShape, Instance, ShapeKind};
    use serde_json::json;

    // SKY130 layers exercised by the tests.
    const MET1: LayerId = LayerId::new(68, 20);
    const MET2: LayerId = LayerId::new(69, 20);
    const VIA1: LayerId = LayerId::new(68, 44);
    const LI1: LayerId = LayerId::new(67, 20);

    /// A polygon shape on `layer` for the rectangle spanning `(x0,y0)-(x1,y1)`, used to
    /// stand in for a planar boolean's polygon output (booleans write polygons).
    fn poly_rect(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            layer,
            ShapeKind::Polygon(Polygon::from_rect(Rect::new(
                Point::new(x0, y0),
                Point::new(x1, y1),
            ))),
        )
    }

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

    // ---- boolean_result ---------------------------------------------------------

    #[test]
    fn boolean_result_two_way() {
        // Two 300x300 squares overlapping in a 100x300 strip.
        //   A = [0,300]x[0,300]   B = [200,500]x[0,300]   overlap = [200,300]x[0,300]
        // Areas: union = 90000 + 90000 - 30000 = 150000; intersection = 30000;
        //        difference (A - B) = 60000. A boolean writes one polygon on met1 and
        //        consumes the met2 inputs.

        // Good (union): a 150000-area polygon on met1, met2 cleared.
        let union = doc_with(vec![poly_rect(MET1, 0, 0, 500, 300)]); // 500*300 = 150000
        assert_pass(
            "boolean_result:layer=68/20,min_area=150000,max_area=150000",
            &union,
        );
        assert_pass(
            "boolean_result:layer=68/20,min_area=150000,max_area=150000,cleared=69/20",
            &union,
        );

        // Good (intersection): a 30000-area polygon; a tight window pins the op.
        let intersection = doc_with(vec![poly_rect(MET1, 200, 0, 300, 300)]); // 100*300
        assert_pass(
            "boolean_result:layer=68/20,min_area=30000,max_area=30000",
            &intersection,
        );
        // The union window rejects the intersection result: the wrong op is caught.
        assert_fail(
            "boolean_result:layer=68/20,min_area=150000,max_area=150000",
            &intersection,
        );

        // Bad: no result geometry at all on the target layer.
        let empty = doc_with(vec![poly_rect(MET2, 0, 0, 500, 300)]);
        assert_fail("boolean_result:layer=68/20,min_area=150000", &empty);

        // Bad (inputs not consumed): the right-area result on met1 is present, but a met2
        // input is still lying around, so `cleared` fails even though the area matches.
        let not_consumed = doc_with(vec![
            poly_rect(MET1, 0, 0, 500, 300),
            rect(MET2, 0, 0, 300, 300),
        ]);
        assert_pass(
            "boolean_result:layer=68/20,min_area=150000,max_area=150000",
            &not_consumed,
        );
        assert_fail(
            "boolean_result:layer=68/20,min_area=150000,max_area=150000,cleared=69/20",
            &not_consumed,
        );
    }

    // ---- array_pitch ------------------------------------------------------------

    /// A one-cell document whose top cell places a single array of `columns`x`rows`
    /// instances of an (empty) leaf cell at the given pitches.
    fn doc_with_array(columns: u32, rows: u32, column_pitch: i32, row_pitch: i32) -> Document {
        let mut leaf = Cell::new("leaf");
        leaf.shapes.push(rect(MET1, 0, 0, 200, 200));
        let mut top = Cell::new("top");
        top.arrays.push(ArrayInstance {
            cell: "leaf".into(),
            transform: Transform::IDENTITY,
            columns,
            rows,
            column_pitch,
            row_pitch,
        });
        let mut doc = Document::new();
        doc.insert_cell(leaf);
        doc.insert_cell(top);
        doc.set_top_cells(vec!["top".into()]);
        doc
    }

    #[test]
    fn array_pitch_two_way() {
        // Good: a 1x4 row at pitch 800 satisfies instances=4 and the column pitch. The
        // row axis has span 1, so its pitch is unconstrained.
        let row4 = doc_with_array(4, 1, 800, 0);
        assert_pass("array_pitch:instances=4,pitch=800", &row4);
        assert_pass("array_pitch:instances=4,col_pitch=800", &row4);
        // Bad: same count, wrong column pitch (500, not 800).
        let row4_wrong = doc_with_array(4, 1, 500, 0);
        assert_fail("array_pitch:instances=4,pitch=800", &row4_wrong);
        // Bad: right pitch but too few instances (a 1x2 row, need 4).
        let row2 = doc_with_array(2, 1, 800, 0);
        assert_fail("array_pitch:instances=4,pitch=800", &row2);

        // Good: a 3x2 grid at 800x600 meets instances=6 and both pitches.
        let grid = doc_with_array(3, 2, 800, 600);
        assert_pass("array_pitch:instances=6,col_pitch=800,row_pitch=600", &grid);
        // Bad: the row pitch is wrong (700, not 600), and that axis actually repeats.
        let grid_wrong_row = doc_with_array(3, 2, 800, 700);
        assert_fail(
            "array_pitch:instances=6,col_pitch=800,row_pitch=600",
            &grid_wrong_row,
        );

        // Bad: the target cell places no array at all.
        let no_array = doc_with(vec![rect(MET1, 0, 0, 200, 200)]);
        assert_fail("array_pitch:instances=4,pitch=800", &no_array);
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

    // ---- generator --------------------------------------------------------------

    /// A one-cell document named `top` holding exactly what generator `id` emits for
    /// `params` (the reference geometry), so it is the canonical correct answer. Takes
    /// `params` by value so call sites pass a `json!(...)` literal directly.
    #[allow(clippy::needless_pass_by_value)]
    fn doc_from_generator(id: &str, params: serde_json::Value) -> Document {
        let registry = reticle_gen::Registry::with_builtins();
        let tech = reticle_model::Technology::default();
        let mut cell = Cell::new("top");
        registry
            .generate(id, &params, &tech, &mut cell)
            .expect("generator produces reference geometry");
        let mut doc = Document::new();
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["top".into()]);
        doc
    }

    #[test]
    fn generator_two_way_via_farm() {
        // Good: the document holds exactly the via farm the checker asks for, so its
        // per-layer fingerprint matches and the geometry is DRC-clean.
        let good = doc_from_generator("via_farm", json!({ "cut": "mcon", "rows": 4, "cols": 4 }));
        assert_pass("generator:id=via_farm,cut=mcon,rows=4,cols=4", &good);

        // Extra shapes never hurt: a 4x4 farm satisfies a checker that only asks for a
        // 3x3 (the graded document has at least the reference count on every layer).
        assert_pass("generator:id=via_farm,cut=mcon,rows=3,cols=3", &good);

        // Bad: an empty document has none of the farm's cuts or plates.
        let empty = doc_with(vec![]);
        assert_fail("generator:id=via_farm,cut=mcon,rows=4,cols=4", &empty);

        // Bad: a 3x3 farm falls short of a checker that pins a 4x4 (fewer cuts than the
        // reference on the cut layer).
        let smaller =
            doc_from_generator("via_farm", json!({ "cut": "mcon", "rows": 3, "cols": 3 }));
        assert_fail("generator:id=via_farm,cut=mcon,rows=4,cols=4", &smaller);
    }

    #[test]
    fn generator_two_way_guard_ring() {
        // Good: the exact guard ring the checker names.
        let params = json!({
            "layer": "li1", "region_width": 2000, "region_height": 2000,
            "ring_width": 400, "taps": true,
        });
        let good = doc_from_generator("guard_ring", params.clone());
        assert_pass(
            "generator:id=guard_ring,layer=li1,region_width=2000,region_height=2000,ring_width=400,taps=true",
            &good,
        );

        // Bad: the four ring strips are present but the li1 tap contacts (on the licon
        // layer) are missing, so the taps=true fingerprint is not met. Build a ring
        // with taps=false and grade it against the taps=true checker.
        let no_taps = doc_from_generator(
            "guard_ring",
            json!({
                "layer": "li1", "region_width": 2000, "region_height": 2000,
                "ring_width": 400, "taps": false,
            }),
        );
        assert_fail(
            "generator:id=guard_ring,layer=li1,region_width=2000,region_height=2000,ring_width=400,taps=true",
            &no_taps,
        );

        // Bad: a document with the right layer coverage but a DRC-dirty shape fails the
        // cleanliness half. A single 100-wide li1 rect (below the li.1 min width) is
        // added to an otherwise-correct ring; DRC flags it.
        let mut dirty = doc_from_generator("guard_ring", params);
        dirty
            .cell_mut("top")
            .unwrap()
            .shapes
            .push(rect(LI1, 5000, 5000, 5100, 5010));
        assert_fail(
            "generator:id=guard_ring,layer=li1,region_width=2000,region_height=2000,ring_width=400,taps=true",
            &dirty,
        );
    }

    #[test]
    fn generator_checker_rejects_unknown_id_and_bad_params() {
        // An unknown generator id fails to build (a task-authoring error).
        assert!(build(&ParsedChecker::parse("generator:id=no_such")).is_err());
        // A missing id is a build error too.
        assert!(build(&ParsedChecker::parse("generator")).is_err());
        // A non-numeric value for an integer field is a build error.
        assert!(build(&ParsedChecker::parse("generator:id=via_farm,rows=lots")).is_err());
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
        // boolean_result needs a result layer; array_pitch needs an instance count.
        assert!(build(&ParsedChecker::parse("boolean_result:min_area=1000")).is_err());
        assert!(build(&ParsedChecker::parse("array_pitch:pitch=800")).is_err());
    }
}
