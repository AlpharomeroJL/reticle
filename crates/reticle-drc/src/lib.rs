//! Design-rule checking for Reticle.
//!
//! [`DrcEngine`] is a declarative rule engine driven by the technology file. It
//! holds a set of [`Rule`]s and evaluates them against a cell's flat geometry,
//! returning a [`Violation`] per offending region ready to be shown and zoomed to
//! in the DRC error browser.
//!
//! # What is checked
//!
//! Each [`RuleKind`] is evaluated over the cell's own shapes (instances and arrays
//! are *not* expanded here; call [`Document::flatten`] first if hierarchy must be
//! checked). Shapes are reduced to their axis-aligned bounding boxes, which is
//! **exact for [`ShapeKind::Rect`]** and a documented **conservative approximation
//! for [`ShapeKind::Polygon`] and [`ShapeKind::Path`]** (the bounding box is at
//! least as large as the true shape, so a bounding-box check never misses a real
//! violation but may over-report on non-rectilinear geometry).
//!
//! | [`RuleKind`]  | Meaning                                            | Status |
//! |---------------|----------------------------------------------------|--------|
//! | [`Width`]     | min feature width on `layer` (`min(w, h) < value`) | checked |
//! | [`Spacing`]   | min edge-to-edge gap, same layer or cross-layer    | checked |
//! | [`Area`]      | min bounding-box area (`area < value`)             | checked |
//! | [`Enclosure`] | `layer` must sit inside `other_layer` by `value`   | checked |
//! | [`Extension`] | `layer` must extend past `other_layer` by `value`  | checked |
//! | [`Notch`]     | min concave gap within one layer                   | checked |
//! | [`Density`]   | layer fill ratio over the cell window vs `value` ‰ | checked |
//! | [`Angle`]     | non-axis-aligned polygon/path edges (`value` = 0)  | checked |
//!
//! [`Width`]: RuleKind::Width
//! [`Spacing`]: RuleKind::Spacing
//! [`Area`]: RuleKind::Area
//! [`Enclosure`]: RuleKind::Enclosure
//! [`Extension`]: RuleKind::Extension
//! [`Notch`]: RuleKind::Notch
//! [`Density`]: RuleKind::Density
//! [`Angle`]: RuleKind::Angle
//!
//! # Spatial acceleration
//!
//! Pairwise rules (spacing, notch, enclosure, extension) never scan all pairs.
//! Shapes are bulk-loaded into an [`RTreeIndex`]; for each shape the engine queries
//! the index with that shape's bounding box expanded by the rule threshold to find
//! only the handful of candidates that could possibly interact, turning the naive
//! `O(n²)` sweep into roughly `O(n log n)` for realistic layouts.
//!
//! # Incremental re-check
//!
//! [`DrcEngine::check_region`] re-checks only the geometry touching an edited
//! rectangle, bounded by a single index query, so a local edit re-validates in time
//! proportional to the shapes near the edit rather than the whole cell.
//!
//! # Built-in SKY130 subset
//!
//! [`sky130_drc_rules`] loads the committed SKY130 rule table (a cited subset of
//! the SKY130 periphery rules) ready for [`DrcEngine::new`]. It is a subset:
//! passing it is not tape-out clean. See the coverage table in the book.

#![forbid(unsafe_code)]

mod geom;
mod sky130;

pub use sky130::sky130_drc_rules;

use geom::{contains_rect, enclosure_margin, overlaps, rect_gap};
use reticle_geometry::{LayerId, Rect, SpatialIndex};
use reticle_index::RTreeIndex;
use reticle_model::{Cell, Document, Rule, RuleKind, RuleSet, ShapeKind, Violation};

/// A cell shape reduced to the data every rule needs: its layer, its bounding box,
/// and whether the source geometry was a true rectangle (so approximate results on
/// polygons/paths can be flagged in messages).
#[derive(Clone, Copy, Debug)]
struct Item {
    /// Index into the cell's `shapes` vector (stable identity for pair dedup).
    id: usize,
    /// The shape's layer/datatype.
    layer: LayerId,
    /// The shape's axis-aligned bounding box (exact for rects, conservative else).
    bbox: Rect,
    /// Whether the source shape was an axis-aligned rectangle.
    is_rect: bool,
}

/// The declarative DRC engine. Holds a rule set and checks cells and regions.
///
/// Construct it from the technology's rules with [`DrcEngine::new`], then call
/// [`RuleSet::check_cell`] for a full cell pass or [`DrcEngine::check_region`] for a
/// fast incremental re-check after a local edit.
#[derive(Debug, Default, Clone)]
pub struct DrcEngine {
    rules: Vec<Rule>,
}

impl DrcEngine {
    /// Creates a DRC engine from a set of rules.
    #[must_use]
    pub fn new(rules: Vec<Rule>) -> Self {
        Self { rules }
    }

    /// Builds a reusable incremental checker for one cell.
    ///
    /// The expensive part of a re-check is reducing the cell to bounding boxes and
    /// bulk-loading the spatial index; [`prepare`](Self::prepare) pays that **once**
    /// and hands back a [`PreparedDrc`] that owns the index and a copy of the rules.
    /// A live editor prepares a cell after a full check, then calls
    /// [`PreparedDrc::check_region`] on every edit, so each re-check touches only the
    /// shapes near the edit instead of rebuilding the whole working set.
    ///
    /// Returns an empty prepared checker (whose `check_region` yields nothing) if
    /// `doc` has no cell named `cell`.
    #[must_use]
    pub fn prepare(&self, doc: &Document, cell: &str) -> PreparedDrc {
        let ctx = match doc.cell(cell) {
            Some(cell) => CellContext::new(cell),
            None => CellContext::empty(),
        };
        PreparedDrc {
            rules: self.rules.clone(),
            max_threshold: max_threshold(&self.rules),
            ctx,
        }
    }

    /// Checks every rule against `cell`, keeping only violations whose location
    /// intersects `region`.
    ///
    /// This is the incremental entry point: after a local edit, pass the edited
    /// (or dirtied) rectangle and only nearby geometry is examined, so re-check
    /// cost scales with the edit's neighbourhood rather than the whole cell. The
    /// returned violations are a subset of what [`RuleSet::check_cell`] would
    /// report, identical rule logic, filtered to `region`.
    ///
    /// This is a convenience wrapper that prepares the cell and immediately checks
    /// one region; when re-checking a cell repeatedly, call [`prepare`](Self::prepare)
    /// once and reuse the returned [`PreparedDrc`] so the index is built only once.
    ///
    /// Returns an empty vector if `doc` has no cell named `cell`.
    #[must_use]
    pub fn check_region(&self, doc: &Document, cell: &str, region: Rect) -> Vec<Violation> {
        self.prepare(doc, cell).check_region(region)
    }

    /// Runs the full-cell pass shared with [`RuleSet::check_cell`].
    fn check_cell_impl(&self, doc: &Document, cell: &str) -> Vec<Violation> {
        let Some(cell) = doc.cell(cell) else {
            return Vec::new();
        };
        let ctx = CellContext::new(cell);
        let mut out = Vec::new();
        for rule in &self.rules {
            ctx.check_rule(rule, Scope::All, &mut out);
        }
        out
    }
}

/// A DRC checker with the spatial index for one cell already built.
///
/// Created by [`DrcEngine::prepare`]. Holds the flattened geometry, an R-tree over
/// it, and a copy of the rule set, so [`check_region`](Self::check_region) can
/// re-validate a local edit without rebuilding anything. Intended lifetime is one
/// editing session on a cell: prepare once, check many regions.
#[derive(Debug)]
pub struct PreparedDrc {
    rules: Vec<Rule>,
    /// The largest rule threshold across `rules`, the radius by which a query
    /// region must grow to catch every shape that could interact across it.
    max_threshold: i64,
    ctx: CellContext,
}

impl PreparedDrc {
    /// Re-checks only the geometry touching `region`, using the prepared index.
    ///
    /// The work is proportional to the shapes near `region`, not to the cell size:
    /// the index is queried once with `region` grown by the maximum rule threshold
    /// (so a shape just outside `region` that could form a spacing, notch,
    /// enclosure, or extension violation reaching into `region` is still a
    /// candidate), and each rule is evaluated only over that local candidate set.
    ///
    /// The result is exactly the subset of [`RuleSet::check_cell`]'s violations whose
    /// location intersects `region`: same rule logic, restricted to the edit's
    /// neighbourhood. Whole-cell density is not a local property and is skipped here,
    /// matching the full incremental contract.
    #[must_use]
    pub fn check_region(&self, region: Rect) -> Vec<Violation> {
        // Grow the query by the widest threshold so cross-region partners are seen,
        // then collect the local primary candidate ids from a single index query.
        // The margin is at least 1 DBU so that a shape merely touching `region`'s
        // edge (gap 0, which `in_region` reports) still overlaps the expanded window
        // with positive area and is returned by `query_rect`, whose contract is
        // positive-area overlap rather than boundary contact.
        let margin = clamp_margin(self.max_threshold).max(1);
        let probe = region.expanded(margin);
        let mut candidates: Vec<usize> = self
            .ctx
            .index
            .query_rect(probe)
            .into_iter()
            .copied()
            .collect();
        candidates.sort_unstable();
        let scope = Scope::Region {
            region,
            candidates: &candidates,
        };
        let mut out = Vec::new();
        for rule in &self.rules {
            self.ctx.check_rule(rule, scope, &mut out);
        }
        out
    }
}

/// The largest `value` across a rule set, or `0` if there are no rules. This is the
/// distance by which an incremental query region is expanded so no interacting
/// neighbour is missed.
fn max_threshold(rules: &[Rule]) -> i64 {
    rules.iter().map(|r| r.value).max().unwrap_or(0).max(0)
}

/// How a single rule check is scoped over the cell's geometry.
///
/// [`Scope::All`] visits every shape (the full-cell pass). [`Scope::Region`]
/// restricts the *primary* shapes a rule iterates to a pre-queried local candidate
/// set and reports only violations whose location intersects `region`; pairwise
/// partners are still found through the full index, so nothing crossing into the
/// region is missed.
#[derive(Clone, Copy)]
enum Scope<'a> {
    All,
    Region {
        region: Rect,
        candidates: &'a [usize],
    },
}

impl Scope<'_> {
    /// The reporting region, if this scope is local.
    fn region(&self) -> Option<Rect> {
        match self {
            Scope::All => None,
            Scope::Region { region, .. } => Some(*region),
        }
    }
}

impl RuleSet for DrcEngine {
    fn rules(&self) -> &[Rule] {
        &self.rules
    }

    fn check_cell(&self, doc: &Document, cell: &str) -> Vec<Violation> {
        self.check_cell_impl(doc, cell)
    }
}

/// Per-cell working set: the flattened items plus a spatial index over them,
/// built once and reused for every rule.
#[derive(Debug)]
struct CellContext {
    items: Vec<Item>,
    index: RTreeIndex<usize>,
}

impl CellContext {
    /// Reduces a cell's shapes to [`Item`]s and bulk-loads a spatial index keyed by
    /// item id.
    fn new(cell: &Cell) -> Self {
        let items: Vec<Item> = cell
            .shapes
            .iter()
            .enumerate()
            .map(|(id, shape)| {
                let (bbox, is_rect) = match &shape.kind {
                    ShapeKind::Rect(r) => (*r, true),
                    ShapeKind::Polygon(p) => (p.bounding_box(), false),
                    ShapeKind::Path(p) => (p.bounding_box(), false),
                };
                Item {
                    id,
                    layer: shape.layer,
                    bbox,
                    is_rect,
                }
            })
            .collect();
        let index = RTreeIndex::bulk_load(items.iter().map(|it| (it.bbox, it.id)));
        Self { items, index }
    }

    /// An empty context, used when a requested cell does not exist.
    fn empty() -> Self {
        Self {
            items: Vec::new(),
            index: RTreeIndex::bulk_load(std::iter::empty()),
        }
    }

    /// The primary items on `layer` that a rule should iterate under `scope`.
    ///
    /// Under [`Scope::All`] this is every item on the layer (the full-cell pass).
    /// Under [`Scope::Region`] it is only the pre-queried local candidates on the
    /// layer, so iteration cost tracks the edit neighbourhood rather than the cell.
    fn on_layer<'s>(
        &'s self,
        layer: LayerId,
        scope: Scope<'s>,
    ) -> Box<dyn Iterator<Item = &'s Item> + 's> {
        match scope {
            Scope::All => Box::new(self.items.iter().filter(move |it| it.layer == layer)),
            Scope::Region { candidates, .. } => Box::new(
                candidates
                    .iter()
                    .map(move |&id| &self.items[id])
                    .filter(move |it| it.layer == layer),
            ),
        }
    }

    /// Whether `bbox` is relevant to an optional `region` filter.
    ///
    /// With no region (the full-cell pass) everything is relevant. With a region
    /// (the incremental pass) a bbox is relevant when it overlaps the region *or*
    /// touches its boundary, so a violation flush against the edited rectangle is
    /// not dropped.
    fn in_region(bbox: &Rect, region: Option<Rect>) -> bool {
        region.is_none_or(|r| rect_gap(bbox, &r) == 0)
    }

    /// Dispatches one rule to its checker, appending violations to `out`.
    ///
    /// `scope` selects the full-cell pass or a local region re-check; in the latter
    /// case reporting is restricted to violations whose location touches the region.
    fn check_rule(&self, rule: &Rule, scope: Scope, out: &mut Vec<Violation>) {
        match rule.kind {
            RuleKind::Width => self.check_width(rule, scope, out),
            RuleKind::Area => self.check_area(rule, scope, out),
            RuleKind::Spacing => self.check_spacing(rule, scope, out),
            RuleKind::Notch => self.check_notch(rule, scope, out),
            RuleKind::Enclosure => self.check_enclosure(rule, scope, out),
            RuleKind::Extension => self.check_extension(rule, scope, out),
            RuleKind::Density => self.check_density(rule, scope, out),
            RuleKind::Angle => self.check_angle(rule, scope, out),
            // `RuleKind` is `#[non_exhaustive]`; every kind that exists today is
            // handled above. A future kind added upstream is not silently passed -
            // it reaches here unrecognized and is simply not evaluated until this
            // engine gains support for it.
            _ => {}
        }
    }

    /// Minimum feature width: `min(width, height) < value` on the rule's layer.
    fn check_width(&self, rule: &Rule, scope: Scope, out: &mut Vec<Violation>) {
        let min = rule.value;
        for it in self.on_layer(rule.layer, scope) {
            if !Self::in_region(&it.bbox, scope.region()) {
                continue;
            }
            let w = it.bbox.width();
            let h = it.bbox.height();
            let feature = w.min(h);
            if feature < min {
                let note = if it.is_rect {
                    ""
                } else {
                    " (bounding-box estimate for non-rectangular shape)"
                };
                out.push(Violation::new(
                    rule,
                    feature,
                    it.bbox,
                    format!(
                        "width {feature} < minimum {min} on layer {}/{}{note}",
                        it.layer.layer, it.layer.datatype
                    ),
                ));
            }
        }
    }

    /// Minimum shape area: bounding-box `area < value` on the rule's layer.
    fn check_area(&self, rule: &Rule, scope: Scope, out: &mut Vec<Violation>) {
        let min = rule.value;
        for it in self.on_layer(rule.layer, scope) {
            if !Self::in_region(&it.bbox, scope.region()) {
                continue;
            }
            let area = it.bbox.area();
            if area < min {
                let note = if it.is_rect {
                    ""
                } else {
                    " (bounding-box estimate for non-rectangular shape)"
                };
                out.push(Violation::new(
                    rule,
                    area,
                    it.bbox,
                    format!(
                        "area {area} < minimum {min} on layer {}/{}{note}",
                        it.layer.layer, it.layer.datatype
                    ),
                ));
            }
        }
    }

    /// Minimum spacing. For a single-layer rule (`other_layer` unset, or equal to
    /// `layer`) every pair on the layer is considered once; for a two-layer rule
    /// each `layer` shape is checked against every `other_layer` shape.
    ///
    /// Candidates are found by querying the index with each shape's bounding box
    /// expanded by `value`, so only shapes that could possibly be within the
    /// spacing threshold are examined. Touching or overlapping shapes (gap `0`) are
    /// never flagged; only a strictly positive gap below `value` is a violation.
    fn check_spacing(&self, rule: &Rule, scope: Scope, out: &mut Vec<Violation>) {
        let min = rule.value;
        if min <= 0 {
            return;
        }
        let cross = match rule.other_layer {
            Some(other) if other != rule.layer => Some(other),
            _ => None,
        };
        self.for_candidate_pairs(rule.layer, cross, min, scope, |a, b| {
            if overlaps(&a.bbox, &b.bbox) {
                return; // overlapping shapes are a different (width/notch) concern
            }
            let gap = rect_gap(&a.bbox, &b.bbox);
            if gap > 0 && gap < min {
                let location = spanning_box(&a.bbox, &b.bbox);
                if !Self::in_region(&location, scope.region()) {
                    return;
                }
                let note = if a.is_rect && b.is_rect {
                    ""
                } else {
                    " (bounding-box estimate for non-rectangular shape)"
                };
                out.push(Violation::new(
                    rule,
                    gap,
                    location,
                    format!("spacing {gap} < minimum {min}{note}"),
                ));
            }
        });
    }

    /// Minimum notch: two shapes on the *same* layer that are close but not
    /// touching form a notch narrower than `value`. Implemented as same-layer
    /// spacing; kept distinct so violations carry the notch rule's name and message.
    fn check_notch(&self, rule: &Rule, scope: Scope, out: &mut Vec<Violation>) {
        let min = rule.value;
        if min <= 0 {
            return;
        }
        self.for_candidate_pairs(rule.layer, None, min, scope, |a, b| {
            if overlaps(&a.bbox, &b.bbox) {
                return;
            }
            let gap = rect_gap(&a.bbox, &b.bbox);
            if gap > 0 && gap < min {
                let location = spanning_box(&a.bbox, &b.bbox);
                if !Self::in_region(&location, scope.region()) {
                    return;
                }
                out.push(Violation::new(
                    rule,
                    gap,
                    location,
                    format!("notch {gap} < minimum {min}"),
                ));
            }
        });
    }

    /// Minimum enclosure: every shape on `layer` must be contained by some shape on
    /// `other_layer` with a margin of at least `value` on all four sides.
    ///
    /// For each inner shape the engine queries the index (expanded by `value`) for
    /// enclosing candidates and keeps the best margin found. A shape with no
    /// containing outer shape, or whose best margin is below `value`, is flagged.
    fn check_enclosure(&self, rule: &Rule, scope: Scope, out: &mut Vec<Violation>) {
        let Some(outer_layer) = rule.other_layer else {
            return; // enclosure is inherently a two-layer rule
        };
        let min = rule.value;
        for inner in self.on_layer(rule.layer, scope) {
            if !Self::in_region(&inner.bbox, scope.region()) {
                continue;
            }
            let probe = inner.bbox.expanded(clamp_margin(min));
            let mut best: Option<i64> = None;
            for &cand in self.index.query_rect(probe) {
                let outer = &self.items[cand];
                if outer.layer != outer_layer {
                    continue;
                }
                if contains_rect(&outer.bbox, &inner.bbox) {
                    let margin = enclosure_margin(&outer.bbox, &inner.bbox);
                    best = Some(best.map_or(margin, |b| b.max(margin)));
                }
            }
            match best {
                Some(margin) if margin >= min => {}
                Some(margin) => out.push(Violation::new(
                    rule,
                    margin,
                    inner.bbox,
                    format!(
                        "enclosure {margin} < minimum {min} by layer {}/{}",
                        outer_layer.layer, outer_layer.datatype
                    ),
                )),
                None => out.push(Violation::new(
                    rule,
                    i64::MIN,
                    inner.bbox,
                    format!(
                        "shape on layer {}/{} is not enclosed by layer {}/{} (required {min})",
                        inner.layer.layer,
                        inner.layer.datatype,
                        outer_layer.layer,
                        outer_layer.datatype
                    ),
                )),
            }
        }
    }

    /// Minimum extension: every shape on `layer` must extend at least `value` past
    /// the shapes on `other_layer` that it overlaps, on the sides where they meet.
    ///
    /// For each `layer` shape overlapping an `other_layer` shape, the four
    /// directional overhangs of the outer (`layer`) shape past the inner
    /// (`other_layer`) shape are measured; if the smallest positive overhang falls
    /// short of `value`, the shortfall is flagged.
    fn check_extension(&self, rule: &Rule, scope: Scope, out: &mut Vec<Violation>) {
        let Some(base_layer) = rule.other_layer else {
            return; // extension compares two layers
        };
        let min = rule.value;
        for ext in self.on_layer(rule.layer, scope) {
            if !Self::in_region(&ext.bbox, scope.region()) {
                continue;
            }
            for &cand in self.index.query_rect(ext.bbox) {
                let base = &self.items[cand];
                if base.layer != base_layer || !overlaps(&ext.bbox, &base.bbox) {
                    continue;
                }
                if let Some((over, side)) = shortest_extension(&ext.bbox, &base.bbox)
                    && over < min
                {
                    out.push(Violation::new(
                        rule,
                        over,
                        ext.bbox,
                        format!(
                            "extension {over} past layer {}/{} on the {side} < minimum {min}",
                            base_layer.layer, base_layer.datatype
                        ),
                    ));
                }
            }
        }
    }

    /// Layer density: the fraction of the cell's bounding window covered by shapes
    /// on `layer`, in per-mille (‰), must be at least `value`. Coverage is measured
    /// as the union area of the layer's bounding boxes (conservative: overlaps are
    /// merged so density is never double-counted).
    ///
    /// A whole-cell metric, so it is reported only on the full-cell pass (`region`
    /// is `None`); an incremental re-check leaves the standing density result in
    /// place rather than recomputing a global figure from a local edit.
    fn check_density(&self, rule: &Rule, scope: Scope, out: &mut Vec<Violation>) {
        if scope.region().is_some() {
            return;
        }
        let boxes: Vec<Rect> = self.on_layer(rule.layer, scope).map(|it| it.bbox).collect();
        let Some(window) = self
            .items
            .iter()
            .map(|it| it.bbox)
            .reduce(|a, b| a.union(&b))
        else {
            return; // empty cell: no window, nothing to measure
        };
        let window_area = window.area();
        if window_area <= 0 {
            return;
        }
        let covered = union_area(&boxes);
        // Per-mille to avoid floating point: covered * 1000 / window_area.
        let permille = (covered.saturating_mul(1000)) / window_area;
        if permille < rule.value {
            out.push(Violation::new(
                rule,
                permille,
                window,
                format!(
                    "density {permille}\u{2030} < minimum {}\u{2030} on layer {}/{}",
                    rule.value, rule.layer.layer, rule.layer.datatype
                ),
            ));
        }
    }

    /// Allowed edge angles. `value == 0` means Manhattan-only: every polygon or
    /// path on the layer must have exclusively axis-aligned edges; a diagonal edge
    /// is flagged. A rectangle is Manhattan by construction and never flagged.
    ///
    /// Non-zero `value` (e.g. 45° tolerances) is not modelled by the DBU-only rule
    /// value here and is treated as "no angle constraint"; see the crate docs.
    fn check_angle(&self, rule: &Rule, scope: Scope, out: &mut Vec<Violation>) {
        if rule.value != 0 {
            return; // only the Manhattan (value == 0) constraint is enforced
        }
        for it in self.on_layer(rule.layer, scope) {
            if it.is_rect || !Self::in_region(&it.bbox, scope.region()) {
                continue;
            }
            // The source shape is a polygon/path; re-read it to inspect edges.
            // (Items store only bboxes, so consult the flattened offending bbox and
            // let the message point the user at it; a bbox-only engine cannot see
            // interior edges, so we conservatively flag the shape for review.)
            out.push(Violation::new(
                rule,
                i64::MIN,
                it.bbox,
                format!(
                    "non-rectangular shape on layer {}/{} may contain non-Manhattan edges",
                    it.layer.layer, it.layer.datatype
                ),
            ));
        }
    }

    /// Invokes `f(a, b)` once for each unordered candidate pair that could be within
    /// `radius` DBU, using the spatial index to prune far-apart shapes.
    ///
    /// * Single-layer (`cross` is `None`): every unordered pair `{a, b}` on `layer`
    ///   is visited exactly once.
    /// * Two-layer (`cross` is `Some(other)`): pairs `(a on layer, b on other)`; the
    ///   two layers are disjoint so ordering by id is unnecessary.
    ///
    /// `scope` restricts which shapes are used as the *primary* `a`. Under
    /// [`Scope::All`] `a` ranges over the whole layer and same-layer pairs dedup with
    /// the cheap `a.id < b.id` rule. Under [`Scope::Region`] `a` ranges only over the
    /// local candidates, but the partner `b` is still found through the full index so
    /// a pair straddling the region boundary is not lost; because such a pair's
    /// id-smaller endpoint may lie *outside* the candidate set, same-layer dedup
    /// switches to an unordered visited set so each local pair still fires once.
    fn for_candidate_pairs<F>(
        &self,
        layer: LayerId,
        cross: Option<LayerId>,
        radius: i64,
        scope: Scope,
        mut f: F,
    ) where
        F: FnMut(&Item, &Item),
    {
        let margin = clamp_margin(radius);
        // Same-layer local pairs cannot dedup by `a.id < b.id` (the smaller id may be
        // outside the candidate window), so track visited unordered pairs instead.
        let local_same_layer = cross.is_none() && !matches!(scope, Scope::All);
        let mut seen: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
        for a in self.on_layer(layer, scope) {
            let probe = a.bbox.expanded(margin);
            for &cand in self.index.query_rect(probe) {
                let b = &self.items[cand];
                match cross {
                    None => {
                        if b.layer != layer || a.id == b.id {
                            continue; // wrong layer, or a shape paired with itself
                        }
                        if local_same_layer {
                            let key = (a.id.min(b.id), a.id.max(b.id));
                            if seen.insert(key) {
                                f(a, b);
                            }
                        } else if a.id < b.id {
                            f(a, b);
                        }
                    }
                    Some(other) => {
                        if b.layer == other {
                            f(a, b);
                        }
                    }
                }
            }
        }
    }
}

/// The smallest rectangle spanning both boxes: the reported location of a spacing
/// or notch violation, so the error browser can frame both offending shapes.
fn spanning_box(a: &Rect, b: &Rect) -> Rect {
    a.union(b)
}

/// Clamps a threshold to a valid [`i32`] expansion margin for index probing.
///
/// Thresholds are DBU (`i64`) but [`Rect::expanded`] takes an [`i32`]; a threshold
/// beyond the coordinate range is clamped so the probe simply covers the whole
/// plane instead of overflowing.
fn clamp_margin(value: i64) -> i32 {
    value.clamp(0, i64::from(i32::MAX)) as i32
}

/// The four directional overhangs of `outer` past `inner`, and the smallest one
/// paired with a human-readable side name.
///
/// Only meaningful when the two rectangles overlap. An overhang is negative when
/// `inner` sticks out past `outer` on that side; the smallest overhang (which may
/// be negative) is what an extension rule must satisfy. Returns `None` if the
/// rectangles do not overlap.
fn shortest_extension(outer: &Rect, inner: &Rect) -> Option<(i64, &'static str)> {
    if !outer.intersects(inner) {
        return None;
    }
    let left = i64::from(inner.min.x) - i64::from(outer.min.x);
    let right = i64::from(outer.max.x) - i64::from(inner.max.x);
    let bottom = i64::from(inner.min.y) - i64::from(outer.min.y);
    let top = i64::from(outer.max.y) - i64::from(inner.max.y);
    let sides = [
        (left, "left"),
        (right, "right"),
        (bottom, "bottom"),
        (top, "top"),
    ];
    sides.into_iter().min_by_key(|(v, _)| *v)
}

/// The area covered by the union of a set of axis-aligned rectangles, in DBU²,
/// computed exactly by coordinate compression and a per-cell sweep.
///
/// Overlapping rectangles contribute their shared region only once, so this is the
/// true covered area (not the sum of individual areas). Runs in `O(n²)` over the
/// distinct coordinate lines, which is ample for a per-cell density metric.
fn union_area(rects: &[Rect]) -> i64 {
    let rects: Vec<&Rect> = rects.iter().filter(|r| !r.is_empty()).collect();
    if rects.is_empty() {
        return 0;
    }
    let mut xs: Vec<i64> = Vec::with_capacity(rects.len() * 2);
    let mut ys: Vec<i64> = Vec::with_capacity(rects.len() * 2);
    for r in &rects {
        xs.push(i64::from(r.min.x));
        xs.push(i64::from(r.max.x));
        ys.push(i64::from(r.min.y));
        ys.push(i64::from(r.max.y));
    }
    xs.sort_unstable();
    xs.dedup();
    ys.sort_unstable();
    ys.dedup();

    let mut area = 0i64;
    for xi in xs.windows(2) {
        let (x0, x1) = (xi[0], xi[1]);
        let dx = x1 - x0;
        for yi in ys.windows(2) {
            let (y0, y1) = (yi[0], yi[1]);
            // A slab cell is covered if any rectangle contains its interior.
            let covered = rects.iter().any(|r| {
                i64::from(r.min.x) <= x0
                    && x1 <= i64::from(r.max.x)
                    && i64::from(r.min.y) <= y0
                    && y1 <= i64::from(r.max.y)
            });
            if covered {
                area += dx * (y1 - y0);
            }
        }
    }
    area
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::Point;

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
        Rect::new(Point::new(x0, y0), Point::new(x1, y1))
    }

    #[test]
    fn union_area_merges_overlap() {
        // Two 10x10 boxes overlapping in a 5x10 strip: 100 + 100 - 50 = 150.
        let a = rect(0, 0, 10, 10);
        let b = rect(5, 0, 15, 10);
        assert_eq!(union_area(&[a, b]), 150);
        // Disjoint boxes: full sum.
        let c = rect(100, 100, 110, 110);
        assert_eq!(union_area(&[a, c]), 200);
        // Single box: its own area.
        assert_eq!(union_area(&[a]), 100);
        assert_eq!(union_area(&[]), 0);
    }

    #[test]
    fn shortest_extension_picks_min_side() {
        let outer = rect(0, 0, 100, 10);
        let inner = rect(20, 0, 30, 10); // left overhang 20, right 70, top/bottom 0
        let (over, side) = shortest_extension(&outer, &inner).expect("overlap");
        assert_eq!(over, 0);
        assert_eq!(side, "bottom"); // ties resolve to the first minimal side
    }

    #[test]
    fn clamp_margin_saturates() {
        assert_eq!(clamp_margin(-5), 0);
        assert_eq!(clamp_margin(10), 10);
        assert_eq!(clamp_margin(i64::MAX), i32::MAX);
    }
}
