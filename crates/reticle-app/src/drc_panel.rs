//! The DRC (design-rule check) panel state and its window-free logic.
//!
//! The panel runs the [`reticle_drc`] engine over the *flattened* top cell and keeps
//! the resulting [`Violation`] list so the side panel can list them and the canvas
//! can draw a marker at each one. All the interesting parts, resolving the rule set,
//! flattening, running the check, and formatting a violation for the list, are plain
//! functions here so they are unit-tested without an egui context; the app module
//! only owns the thin painting and click wiring.
//!
//! The rule resolution mirrors the CLI's `default_rules`/`resolve_rules`: the
//! document's own technology rules are used when present, otherwise a small
//! synthesized width-rule set is checked so a demo document still reports something.

use reticle_geometry::{LayerId, Rect};
use reticle_model::{Cell, Document, Rule, RuleKind, RuleSet, Violation};

use reticle_drc::DrcEngine;

/// Minimum feature width, in DBU, for the synthesized fallback width rule.
///
/// Deliberately large so any thin synthetic feature in the demo is flagged, matching
/// the CLI's fallback threshold.
const DEFAULT_MIN_WIDTH: i64 = 100;

/// The DRC panel's stored result: the violations found by the last run.
///
/// The list is empty until [`DrcResults::run`] is called and after
/// [`DrcResults::clear`]. A `selected` index tracks which violation the user last
/// clicked so the canvas can emphasize it.
#[derive(Clone, Debug, Default)]
pub struct DrcResults {
    /// The violations from the most recent run, in engine order.
    violations: Vec<Violation>,
    /// Whether a check has been run at least once since the last clear (so the panel
    /// can distinguish "no run yet" from "ran, found nothing").
    has_run: bool,
    /// The index of the violation the user last clicked, if any.
    selected: Option<usize>,
}

impl DrcResults {
    /// Creates an empty result set (nothing checked yet).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs DRC over the flattened `top` cell of `doc`, replacing the stored list.
    ///
    /// The hierarchy under `top` is flattened first (see [`flatten_top_cell`]) so a
    /// pure-instance/array top cell is checked as the geometry it actually
    /// represents; the rule set is resolved with [`resolve_rules`]. Returns the
    /// number of violations found.
    pub fn run(&mut self, doc: &Document, top: &str) -> usize {
        let flat = flatten_top_cell(doc, top);
        let rules = resolve_rules(doc);
        let engine = DrcEngine::new(rules);
        self.violations = engine.check_cell(&flat, top);
        self.has_run = true;
        self.selected = None;
        self.violations.len()
    }

    /// Replaces the stored list with an externally computed violation set.
    ///
    /// This is how a live agent run or a transcript replay feeds the canvas
    /// overlay: each `run_drc` response it crosses carries a violation list, and
    /// installing it here updates the panel and the markers exactly as a local
    /// [`run`](Self::run) would. The selection is dropped because indices into
    /// the previous list are stale.
    pub fn set_violations(&mut self, violations: Vec<Violation>) {
        self.violations = violations;
        self.has_run = true;
        self.selected = None;
    }

    /// Clears the stored violations and the "has run" flag.
    pub fn clear(&mut self) {
        self.violations.clear();
        self.has_run = false;
        self.selected = None;
    }

    /// The stored violations.
    #[must_use]
    pub fn violations(&self) -> &[Violation] {
        &self.violations
    }

    /// Whether a check has run since the last clear.
    #[must_use]
    pub fn has_run(&self) -> bool {
        self.has_run
    }

    /// The number of stored violations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.violations.len()
    }

    /// Whether there are no stored violations.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.violations.is_empty()
    }

    /// The index of the currently highlighted violation, if any.
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Records `index` as the highlighted violation and returns its location.
    ///
    /// Returns `None` (leaving the selection unchanged) if `index` is out of range,
    /// so a stale click after a re-run cannot select a missing violation.
    pub fn select(&mut self, index: usize) -> Option<Rect> {
        let loc = self.violations.get(index)?.location;
        self.selected = Some(index);
        Some(loc)
    }
}

/// Flattens the hierarchy under `top` of `doc` into a single-cell document.
///
/// The result has exactly one cell named `top` whose shapes are the fully expanded
/// geometry of `top`, with the document's technology carried over so tech-derived
/// rules still resolve. This mirrors the CLI's `flatten_top_cell` so the panel
/// checks the whole design rather than only the top cell's own geometry.
#[must_use]
pub fn flatten_top_cell(doc: &Document, top: &str) -> Document {
    let mut cell = Cell::new(top);
    cell.shapes = doc.flatten(top);
    let mut flat = Document::new();
    flat.set_technology(doc.technology().clone());
    flat.insert_cell(cell);
    flat.set_top_cells(vec![top.to_owned()]);
    flat
}

/// Resolves the DRC rule set to run against `doc`.
///
/// Uses the document technology's own rules when it has any; otherwise synthesizes a
/// [`default_rules`] width-rule set so a rule-less demo document still reports
/// violations. This mirrors the CLI's `resolve_rules` for the no-technology-file
/// case.
#[must_use]
pub fn resolve_rules(doc: &Document) -> Vec<Rule> {
    let own = &doc.technology().rules;
    if own.is_empty() {
        default_rules(doc)
    } else {
        own.clone()
    }
}

/// Synthesizes a minimal width-rule set covering every layer that carries geometry.
///
/// One [`RuleKind::Width`] rule per layer (and always the common metal layer `1/0`
/// if the document is empty), so a document with any sub-threshold feature reports at
/// least one violation. Mirrors the CLI's `default_rules`.
#[must_use]
pub fn default_rules(doc: &Document) -> Vec<Rule> {
    let mut layers: Vec<LayerId> = Vec::new();
    for cell in doc.cells() {
        for shape in &cell.shapes {
            if !layers.contains(&shape.layer) {
                layers.push(shape.layer);
            }
        }
    }
    if layers.is_empty() {
        layers.push(LayerId::new(1, 0));
    }
    layers.sort_unstable();

    layers
        .into_iter()
        .map(|layer| Rule {
            name: format!("default_min_width_{}_{}", layer.layer, layer.datatype),
            kind: RuleKind::Width,
            layer,
            other_layer: None,
            value: DEFAULT_MIN_WIDTH,
        })
        .collect()
}

/// Formats a violation's location as a compact `(x0, y0)-(x1, y1)` DBU string.
#[must_use]
pub fn format_location(loc: &Rect) -> String {
    format!(
        "({}, {})-({}, {})",
        loc.min.x, loc.min.y, loc.max.x, loc.max.y
    )
}

/// Formats one violation into a single list line: rule, location, and message.
#[must_use]
pub fn format_violation(v: &Violation) -> String {
    format!(
        "{}  {}  {}",
        v.rule,
        format_location(&v.location),
        v.message
    )
}

/// The stable keyword for a [`RuleKind`], matching the wire form the agent API
/// emits and parses.
#[must_use]
pub fn rule_kind_tag(kind: RuleKind) -> &'static str {
    match kind {
        RuleKind::Width => "width",
        RuleKind::Spacing => "spacing",
        RuleKind::Enclosure => "enclosure",
        RuleKind::Extension => "extension",
        RuleKind::Notch => "notch",
        RuleKind::Area => "area",
        RuleKind::Density => "density",
        RuleKind::Angle => "angle",
        // `RuleKind` is non-exhaustive; tag any future kind neutrally.
        _ => "rule",
    }
}

/// Assembles the scoped-fix context string handed to the agent for one violation.
///
/// This is the payload behind the DRC panel's "Ask agent to fix" affordance: it
/// pins the agent to the violation's region (its bounding box, in DBU) and the
/// rule it broke (name, kind, layers, and measured-versus-required values), so a
/// scoped run has an objective target and a bounded area to work in rather than
/// the whole design. The MINIMAL context pack and the real scoped harness are
/// Wave 3 Lane 3B; this string is the seam that harness consumes, and it is also
/// what seeds the agent panel's prompt today so the affordance is honest.
///
/// The layer is rendered `layer/datatype`; a two-layer rule (spacing, enclosure,
/// extension) also names its other layer. The region is the four DBU corners of
/// the violation's [`location`](Violation::location) plus its width and height.
#[must_use]
pub fn fix_violation_prompt(v: &Violation) -> String {
    use std::fmt::Write as _;
    let loc = &v.location;
    let mut layers = format!("{}/{}", v.layer.layer, v.layer.datatype);
    if let Some(other) = v.other_layer {
        let _ = write!(layers, " vs {}/{}", other.layer, other.datatype);
    }
    format!(
        "Fix this DRC violation at region ({}, {})-({}, {}) \
         [w={} h={} DBU] per rule \"{}\" (kind {}, layer {}, measured {} vs required {}). \
         Keep the fix inside that region and re-run DRC to confirm it clears. \
         Context: {}",
        loc.min.x,
        loc.min.y,
        loc.max.x,
        loc.max.y,
        loc.width(),
        loc.height(),
        v.rule,
        rule_kind_tag(v.kind),
        layers,
        v.measured,
        v.required,
        v.message,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use reticle_geometry::Point;
    use reticle_model::{DrawShape, ShapeKind};

    /// A document whose top cell has a single thin metal-1 rectangle (below the
    /// default 100-DBU width) so the fallback rule flags exactly one violation.
    fn thin_feature_doc() -> Document {
        let mut cell = Cell::new("TOP");
        cell.shapes.push(DrawShape::new(
            LayerId::new(4, 0),
            // 10 DBU wide, well under the 100-DBU minimum width.
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(10, 2000))),
        ));
        let mut doc = Document::new();
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["TOP".to_owned()]);
        doc
    }

    #[test]
    fn default_rules_cover_every_geometry_layer() {
        let doc = demo::demo_document();
        let rules = default_rules(&doc);
        // The demo draws on NWELL, ACTIVE, POLY, METAL1, METAL2 (five layers with
        // own geometry across its cells).
        let layers: std::collections::HashSet<_> = rules.iter().map(|r| r.layer).collect();
        assert!(layers.contains(&LayerId::new(4, 0)), "metal1 rule missing");
        assert!(rules.iter().all(|r| r.kind == RuleKind::Width));
        assert!(!rules.is_empty());
    }

    #[test]
    fn empty_document_still_gets_a_metal_rule() {
        let rules = default_rules(&Document::new());
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].layer, LayerId::new(1, 0));
    }

    #[test]
    fn run_finds_thin_feature_violation() {
        let mut results = DrcResults::new();
        let n = results.run(&thin_feature_doc(), "TOP");
        assert!(n >= 1, "a 10-DBU wide feature should violate min width");
        assert!(results.has_run());
        assert!(!results.is_empty());
        // Every reported violation names the width rule and has a real location.
        for v in results.violations() {
            assert!(v.rule.contains("width"), "unexpected rule {}", v.rule);
            assert!(!v.location.is_empty() || v.location.width() >= 0);
        }
    }

    #[test]
    fn clear_empties_results() {
        let mut results = DrcResults::new();
        results.run(&thin_feature_doc(), "TOP");
        assert!(results.has_run());
        results.clear();
        assert!(results.is_empty());
        assert!(!results.has_run());
        assert!(results.selected().is_none());
    }

    #[test]
    fn set_violations_installs_an_external_list_and_drops_selection() {
        let mut results = DrcResults::new();
        results.run(&thin_feature_doc(), "TOP");
        results.select(0).expect("index 0 exists");
        let external = vec![Violation {
            rule: "agent_min_width".to_owned(),
            kind: RuleKind::Width,
            layer: LayerId::new(4, 0),
            other_layer: None,
            measured: 60,
            required: 100,
            location: Rect::new(Point::new(0, 0), Point::new(60, 2000)),
            message: "from a transcript run_drc response".to_owned(),
        }];
        results.set_violations(external);
        assert!(results.has_run());
        assert_eq!(results.len(), 1);
        assert_eq!(results.violations()[0].rule, "agent_min_width");
        assert!(results.selected().is_none(), "stale selection dropped");
        // An empty external list still counts as "ran, found nothing".
        results.set_violations(Vec::new());
        assert!(results.has_run());
        assert!(results.is_empty());
    }

    #[test]
    fn select_records_index_and_returns_location() {
        let mut results = DrcResults::new();
        results.run(&thin_feature_doc(), "TOP");
        let want = results.violations()[0].location;
        let got = results.select(0).expect("index 0 exists");
        assert_eq!(got, want);
        assert_eq!(results.selected(), Some(0));
        // Out-of-range selection is ignored.
        assert!(results.select(9999).is_none());
        assert_eq!(results.selected(), Some(0));
    }

    #[test]
    fn flatten_expands_hierarchy_for_checking() {
        let doc = demo::demo_document();
        let flat = flatten_top_cell(&doc, demo::TOP_CELL);
        assert_eq!(flat.cell_count(), 1);
        let cell = flat.cell(demo::TOP_CELL).expect("flat top cell");
        assert_eq!(cell.shapes.len(), doc.flatten(demo::TOP_CELL).len());
        assert!(cell.instances.is_empty() && cell.arrays.is_empty());
    }

    #[test]
    fn fix_violation_prompt_carries_region_and_rule() {
        let v = Violation {
            rule: "min_width_met1".to_owned(),
            kind: RuleKind::Width,
            layer: LayerId::new(4, 0),
            other_layer: None,
            measured: 60,
            required: 100,
            location: Rect::new(Point::new(23_000, 0), Point::new(23_060, 2000)),
            message: "feature 60 < min width 100".to_owned(),
        };
        let prompt = fix_violation_prompt(&v);
        // The region bbox corners and its extent are present.
        assert!(prompt.contains("(23000, 0)-(23060, 2000)"));
        assert!(prompt.contains("w=60 h=2000 DBU"));
        // The rule name, kind, layer, and measured-vs-required are present.
        assert!(prompt.contains("min_width_met1"));
        assert!(prompt.contains("kind width"));
        assert!(prompt.contains("layer 4/0"));
        assert!(prompt.contains("measured 60 vs required 100"));
        // The original message is carried as context.
        assert!(prompt.contains("feature 60 < min width 100"));
        // No other layer named for a one-layer rule.
        assert!(!prompt.contains(" vs 0/0"));
    }

    #[test]
    fn fix_violation_prompt_names_the_other_layer_for_two_layer_rules() {
        let v = Violation {
            rule: "met1_met2_spacing".to_owned(),
            kind: RuleKind::Spacing,
            layer: LayerId::new(4, 0),
            other_layer: Some(LayerId::new(5, 0)),
            measured: 80,
            required: 140,
            location: Rect::new(Point::new(0, 0), Point::new(80, 200)),
            message: "gap 80 < min spacing 140".to_owned(),
        };
        let prompt = fix_violation_prompt(&v);
        assert!(prompt.contains("kind spacing"));
        assert!(prompt.contains("layer 4/0 vs 5/0"));
    }

    #[test]
    fn rule_kind_tag_matches_the_wire_keywords() {
        // The tags must round-trip through the agent API's kind parsing, so a
        // scoped fix prompt names kinds the same way run_drc reports them.
        assert_eq!(rule_kind_tag(RuleKind::Width), "width");
        assert_eq!(rule_kind_tag(RuleKind::Spacing), "spacing");
        assert_eq!(rule_kind_tag(RuleKind::Enclosure), "enclosure");
        assert_eq!(rule_kind_tag(RuleKind::Area), "area");
        assert_eq!(rule_kind_tag(RuleKind::Angle), "angle");
    }

    #[test]
    fn format_violation_includes_rule_and_coords() {
        let v = Violation {
            rule: "min_width".to_owned(),
            kind: reticle_model::RuleKind::Width,
            layer: reticle_geometry::LayerId::new(1, 0),
            other_layer: None,
            measured: 5,
            required: 10,
            location: Rect::new(Point::new(1, 2), Point::new(3, 4)),
            message: "too thin".to_owned(),
        };
        let line = format_violation(&v);
        assert!(line.contains("min_width"));
        assert!(line.contains("(1, 2)-(3, 4)"));
        assert!(line.contains("too thin"));
    }
}
