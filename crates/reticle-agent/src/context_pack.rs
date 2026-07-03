//! Minimal, region-scoped context packs for a scoped agent session.
//!
//! A scoped session is opened on a REGION of the document plus, optionally, a single
//! design-rule [`Violation`] to repair (this is what Wave 2's "ask the agent to fix this
//! violation" DRC button hands the harness). Instead of conditioning the model on the
//! whole-document summary ([`document_summary`](crate::run::document_summary)), the
//! harness assembles a compact **context pack**: only the geometry that overlaps the
//! region, the violated rule stated structurally, and just the technology rules that
//! bear on the layers in play.
//!
//! # Why a pack, not the whole document
//!
//! The whole-document summary grows with the design: every cell, every shape count, and
//! every cell's bounding box. For a repair that is local to one small region, almost all
//! of that is irrelevant context the model still pays for in tokens. A pack keeps the
//! prompt proportional to the *region*, not the design, so a fix on a 500-DBU corner of a
//! million-shape layout is described in a few hundred characters. See
//! [`token_estimate`] and the crate's `context_pack` tests for the measured saving on a
//! synthetic multi-shape document (the benchmark chapter cites the number).
//!
//! # Contents of a pack
//!
//! [`ContextPack::assemble`] emits, in order:
//!
//! 1. a one-line region header (the query rectangle);
//! 2. the shapes whose bounding box overlaps the region, each as `layer + bbox`, capped
//!    at [`ContextPack::shape_cap`] with an "N more omitted" note when the cap bites;
//! 3. the violated rule, if any, as `kind`, `layer(s)`, `measured`, and `required`;
//! 4. the relevant technology rules: those whose primary or secondary layer is a layer
//!    that appears in the in-region shapes or in the violated rule.
//!
//! The overlap test is the same inclusive "touch or overlap" test the command surface's
//! `query_shapes` region filter uses (a shared edge or corner counts), so a pack and a
//! `query_shapes` on the same rectangle agree on which shapes are in scope.
//!
//! # Wiring
//!
//! The pack plugs into the same `context_hook` seam
//! [`document_summary`](crate::run::document_summary) uses: build the pack string from
//! the session's document and hand it to the model with `set_document_context`, for
//! example
//! `|m, session| m.set_document_context(pack.assemble(session.document()))`. The
//! assembler is a pure function of the document and the region/violation, so it needs no
//! network, no key, and no mutation.

use reticle_geometry::{LayerId, Rect, Shape as _};
use reticle_model::{Document, Rule, RuleKind, Violation};

/// A request to assemble a region-scoped context pack.
///
/// Holds the region to scope to, an optional violated [`Violation`] to state for repair,
/// and a cap on how many overlapping shapes to list (so a dense region still yields a
/// bounded prompt). Build one with [`ContextPack::new`], optionally attach a violation
/// with [`ContextPack::with_violation`] and adjust the cap with
/// [`ContextPack::with_shape_cap`], then call [`ContextPack::assemble`] with the document.
#[derive(Clone, Debug)]
pub struct ContextPack {
    /// The region the session is scoped to; only shapes overlapping this rectangle and
    /// the rules bearing on their layers enter the pack.
    region: Rect,
    /// The specific violation to repair, when the session was opened from a DRC hit.
    /// `None` for a plain region scope with no rule stated.
    violation: Option<Violation>,
    /// The most shapes to list before summarizing the remainder as a count. A small cap
    /// keeps a dense region's pack bounded; the default is [`DEFAULT_SHAPE_CAP`].
    shape_cap: usize,
}

/// The default number of overlapping shapes a pack lists before it summarizes the rest
/// as a count. Chosen so a typical local region is described in full while a pathological
/// dense region cannot blow the prompt up.
pub const DEFAULT_SHAPE_CAP: usize = 32;

impl ContextPack {
    /// A pack scoped to `region`, with no violation and the [`DEFAULT_SHAPE_CAP`].
    #[must_use]
    pub fn new(region: Rect) -> Self {
        Self {
            region,
            violation: None,
            shape_cap: DEFAULT_SHAPE_CAP,
        }
    }

    /// Attaches the violation the scoped session should repair.
    ///
    /// Its kind, layers, measured, and required values are stated in the pack, and its
    /// layers are added to the set that selects the relevant technology rules.
    #[must_use]
    pub fn with_violation(mut self, violation: Violation) -> Self {
        self.violation = Some(violation);
        self
    }

    /// Overrides how many overlapping shapes are listed before the remainder is
    /// summarized as a count. Zero lists none (only the count).
    #[must_use]
    pub fn with_shape_cap(mut self, shape_cap: usize) -> Self {
        self.shape_cap = shape_cap;
        self
    }

    /// The region this pack is scoped to.
    #[must_use]
    pub fn region(&self) -> Rect {
        self.region
    }

    /// The shape cap this pack lists up to before summarizing the remainder.
    #[must_use]
    pub fn shape_cap(&self) -> usize {
        self.shape_cap
    }

    /// Assembles the compact context string for `doc`.
    ///
    /// Lists the shapes overlapping [`region`](Self::region) (capped), the violated rule
    /// if one was attached, and the technology rules whose layers appear in the in-region
    /// shapes or the violation. The result is a plain, model-facing string; it is a pure
    /// function of `doc` and this request, so it can be recomputed cheaply per iteration.
    #[must_use]
    pub fn assemble(&self, doc: &Document) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();

        // 1. Region header.
        let _ = writeln!(out, "Scoped region: {}", fmt_rect(self.region),);

        // 2. Overlapping shapes (capped), collecting the layers in play for rule
        //    selection.
        let mut layers_in_play: Vec<LayerId> = Vec::new();
        let mut overlapping: Vec<(LayerId, Rect, &'static str)> = Vec::new();
        for cell in doc.cells() {
            for shape in &cell.shapes {
                let bbox = shape.bounding_box();
                if !touches(&self.region, &bbox) {
                    continue;
                }
                if !layers_in_play.contains(&shape.layer) {
                    layers_in_play.push(shape.layer);
                }
                overlapping.push((shape.layer, bbox, shape_kind_label(shape)));
            }
        }

        let total = overlapping.len();
        if total == 0 {
            out.push_str("Shapes in region: none\n");
        } else {
            let shown = total.min(self.shape_cap);
            let _ = writeln!(out, "Shapes in region ({total} total, showing {shown}):");
            for (layer, bbox, kind) in overlapping.iter().take(self.shape_cap) {
                let _ = writeln!(out, "{}", fmt_shape_line(kind, *layer, *bbox));
            }
            if total > shown {
                let _ = writeln!(out, "- ({} more omitted)", total - shown);
            }
        }

        // 3. The violated rule, stated structurally.
        if let Some(v) = &self.violation {
            // The violation's layers also select relevant tech rules.
            if !layers_in_play.contains(&v.layer) {
                layers_in_play.push(v.layer);
            }
            if let Some(other) = v.other_layer
                && !layers_in_play.contains(&other)
            {
                layers_in_play.push(other);
            }
            let _ = writeln!(out, "Violated rule: {}", fmt_violation(v));
        }

        // 4. Relevant technology rules: those touching a layer in play.
        let tech = doc.technology();
        let relevant: Vec<&Rule> = tech
            .rules
            .iter()
            .filter(|r| rule_touches_layers(r, &layers_in_play))
            .collect();
        if relevant.is_empty() {
            out.push_str("Relevant technology rules: none\n");
        } else {
            let _ = writeln!(out, "Relevant technology rules ({}):", relevant.len());
            for r in relevant {
                let _ = writeln!(out, "- {}", fmt_rule(r));
            }
        }

        out
    }
}

/// The whole-document context at the same per-shape fidelity a pack provides: every
/// shape in every cell as `kind + layer + bbox`, plus the full technology rule list.
///
/// This is the honest baseline a scoped [`ContextPack`] is measured against. To *reason*
/// about a local geometric fix a model needs the coordinates of the shapes near the
/// edit, not just a count; the compact
/// [`document_summary`](crate::run::document_summary) gives per-cell aggregate counts and
/// omits every coordinate, so it is not a like-for-like comparison. This function lists
/// the geometry itself for the whole design, which is what the pack replaces with a
/// region-scoped slice. On a large design the pack is a small fraction of this; see
/// [`token_estimate`] and the module tests for the measured ratio.
#[must_use]
pub fn whole_document_context(doc: &Document) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let mut total = 0_usize;
    for cell in doc.cells() {
        if cell.shapes.is_empty() {
            continue;
        }
        let _ = writeln!(out, "cell {} ({} shapes):", cell.name, cell.shapes.len());
        for shape in &cell.shapes {
            let _ = writeln!(
                out,
                "{}",
                fmt_shape_line(shape_kind_label(shape), shape.layer, shape.bounding_box()),
            );
            total += 1;
        }
    }
    if total == 0 {
        out.push_str("(empty document, no shapes yet)\n");
    }
    let tech = doc.technology();
    if tech.rules.is_empty() {
        out.push_str("Technology rules: none\n");
    } else {
        let _ = writeln!(out, "Technology rules ({}):", tech.rules.len());
        for r in &tech.rules {
            let _ = writeln!(out, "- {}", fmt_rule(r));
        }
    }
    out
}

/// A rough token estimate for `text`: characters divided by four.
///
/// The four-characters-per-token heuristic is the usual back-of-envelope for English and
/// JSON-ish text; it is not the model's real tokenizer, but it is stable and monotone, so
/// it is a fair way to compare two prompts. The tests use it to show a pack is materially
/// smaller than the whole-document context on the same document.
#[must_use]
pub fn token_estimate(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// The inclusive "overlap or touch" test the command surface's `query_shapes` region
/// filter uses: two rectangles that share only an edge or a corner still count.
///
/// Reimplemented here (rather than shared from `reticle-agent-api`, where it is private)
/// so a pack and a `query_shapes` on the same rectangle agree on scope. It is a
/// four-comparison test, so duplicating it is cheaper than widening another crate's
/// surface.
fn touches(a: &Rect, b: &Rect) -> bool {
    a.min.x <= b.max.x && b.min.x <= a.max.x && a.min.y <= b.max.y && b.min.y <= a.max.y
}

/// Whether `rule`'s primary or secondary layer is one of `layers`.
fn rule_touches_layers(rule: &Rule, layers: &[LayerId]) -> bool {
    layers.contains(&rule.layer)
        || rule
            .other_layer
            .is_some_and(|other| layers.contains(&other))
}

/// Formats a rectangle as `[minx,miny .. maxx,maxy]` in database units.
fn fmt_rect(r: Rect) -> String {
    format!("[{},{} .. {},{}]", r.min.x, r.min.y, r.max.x, r.max.y)
}

/// Formats a layer as `layer/datatype`.
fn fmt_layer(l: LayerId) -> String {
    format!("{}/{}", l.layer, l.datatype)
}

/// Formats one shape as a bullet line `- kind layer/datatype [bbox]`, the shared form
/// used by both the region listing and the whole-document listing so the two are
/// measured at identical fidelity.
fn fmt_shape_line(kind: &str, layer: LayerId, bbox: Rect) -> String {
    format!("- {kind} {} {}", fmt_layer(layer), fmt_rect(bbox))
}

/// A short label for a shape's geometry kind.
fn shape_kind_label(shape: &reticle_model::DrawShape) -> &'static str {
    use reticle_model::ShapeKind;
    match shape.kind {
        ShapeKind::Rect(_) => "rect",
        ShapeKind::Polygon(_) => "poly",
        ShapeKind::Path(_) => "path",
    }
}

/// Formats a rule kind as a short lowercase token.
///
/// [`RuleKind`] is `#[non_exhaustive]`, so an unrecognized future kind falls back to a
/// generic label rather than failing to compile.
fn fmt_kind(kind: RuleKind) -> &'static str {
    match kind {
        RuleKind::Width => "width",
        RuleKind::Spacing => "spacing",
        RuleKind::Enclosure => "enclosure",
        RuleKind::Extension => "extension",
        RuleKind::Notch => "notch",
        RuleKind::Area => "area",
        RuleKind::Density => "density",
        RuleKind::Angle => "angle",
        _ => "rule",
    }
}

/// Formats a technology rule as `name (kind, layer[, other], require >= value)`.
fn fmt_rule(rule: &Rule) -> String {
    let layers = match rule.other_layer {
        Some(other) => format!("{} vs {}", fmt_layer(rule.layer), fmt_layer(other)),
        None => fmt_layer(rule.layer),
    };
    format!(
        "{} ({}, {}, require {})",
        rule.name,
        fmt_kind(rule.kind),
        layers,
        rule.value,
    )
}

/// Formats a violation as `name (kind, layer[, other], measured M, required R)`.
fn fmt_violation(v: &Violation) -> String {
    let layers = match v.other_layer {
        Some(other) => format!("{} vs {}", fmt_layer(v.layer), fmt_layer(other)),
        None => fmt_layer(v.layer),
    };
    format!(
        "{} ({}, {}, measured {}, required {})",
        v.rule,
        fmt_kind(v.kind),
        layers,
        v.measured,
        v.required,
    )
}

#[cfg(test)]
mod tests {
    use super::{ContextPack, token_estimate, whole_document_context};
    use crate::run::document_summary;
    use reticle_agent_api::args::{LayerArg, PointArg, RectArg};
    use reticle_agent_api::{AgentCommand, Session};
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{
        Cell, Document, DrawShape, Rule, RuleKind, ShapeKind, Technology, Violation,
    };

    /// The met1 layer used across the fixtures.
    const MET1: LayerId = LayerId {
        layer: 68,
        datatype: 20,
    };
    /// A second layer (met2) for two-layer and off-region fixtures.
    const MET2: LayerId = LayerId {
        layer: 69,
        datatype: 20,
    };

    /// Adds a rect on `layer` to cell `cell` from `(x0,y0)` to `(x1,y1)`.
    fn add_rect(
        session: &mut Session,
        cell: &str,
        layer: LayerId,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
    ) {
        session
            .apply(AgentCommand::AddRect {
                cell: cell.into(),
                layer: LayerArg {
                    layer: layer.layer,
                    datatype: layer.datatype,
                },
                rect: RectArg {
                    min: PointArg { x: x0, y: y0 },
                    max: PointArg { x: x1, y: y1 },
                },
            })
            .expect("add_rect");
    }

    /// A session with cell `top` holding one met1 rect near the origin and one met2 rect
    /// far away, so a small region around the origin captures only the first.
    fn two_shape_session() -> Session {
        let mut session = Session::new();
        session
            .apply(AgentCommand::CreateCell { name: "top".into() })
            .unwrap();
        add_rect(&mut session, "top", MET1, 0, 0, 500, 500);
        add_rect(&mut session, "top", MET2, 10_000, 10_000, 10_500, 10_500);
        session
    }

    /// A region covering the origin shape but not the far one.
    fn origin_region() -> Rect {
        Rect::new(Point::new(-100, -100), Point::new(600, 600))
    }

    /// Installs a technology on `session` (via the real `SetTechnology` path) carrying a
    /// met1 width rule and a met2 spacing rule. The parser names rules `<kind>_<layer>_
    /// <datatype>`, so these become `width_68_20` and `spacing_69_20`.
    fn set_two_rule_technology(session: &mut Session) {
        let source = "\
technology test
dbu_per_micron 1000
layer 68 20 met1 3A6FD490
layer 69 20 met2 C23A9E90
rule width 68 20 140
rule spacing 69 20 140
";
        session
            .apply(AgentCommand::SetTechnology {
                source: source.into(),
            })
            .expect("set technology");
    }

    #[test]
    fn pack_lists_only_shapes_overlapping_the_region() {
        let session = two_shape_session();
        let pack = ContextPack::new(origin_region());
        let text = pack.assemble(session.document());
        // The met1 shape at the origin is in; the far met2 shape is out.
        assert!(
            text.contains("68/20"),
            "in-region met1 shape listed: {text}"
        );
        assert!(
            !text.contains("69/20"),
            "far met2 shape must not appear: {text}"
        );
        assert!(
            text.contains("1 total"),
            "exactly one shape in region: {text}"
        );
    }

    #[test]
    fn empty_region_reports_no_shapes_and_no_rules() {
        let session = two_shape_session();
        // A region far from every shape.
        let pack = ContextPack::new(Rect::new(
            Point::new(-5000, -5000),
            Point::new(-4000, -4000),
        ));
        let text = pack.assemble(session.document());
        assert!(text.contains("Shapes in region: none"), "{text}");
        // With no shapes and no violation there are no layers in play, so no tech rules.
        assert!(text.contains("Relevant technology rules: none"), "{text}");
    }

    #[test]
    fn pack_states_the_violated_rule_when_present() {
        let session = two_shape_session();
        let violation = Violation::new(
            &Rule {
                name: "m1.width".into(),
                kind: RuleKind::Width,
                layer: MET1,
                other_layer: None,
                value: 140,
            },
            90,
            origin_region(),
            "met1 width 90 < 140".into(),
        );
        let pack = ContextPack::new(origin_region()).with_violation(violation);
        let text = pack.assemble(session.document());
        assert!(text.contains("Violated rule: m1.width"), "{text}");
        assert!(text.contains("width"), "kind stated: {text}");
        assert!(text.contains("measured 90"), "measured stated: {text}");
        assert!(text.contains("required 140"), "required stated: {text}");
    }

    /// Builds a document (bypassing the technology-file parser) with one met1 shape at
    /// the origin and a technology carrying a met1 width rule and a met2 width rule, so a
    /// test controls the rule layers precisely.
    fn doc_with_met1_shape_and_two_rules() -> Document {
        let mut doc = Document::new();
        let mut top = Cell::new("top");
        top.shapes.push(DrawShape::new(
            MET1,
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(500, 500))),
        ));
        doc.insert_cell(top);
        doc.set_technology(Technology {
            name: "t".into(),
            dbu_per_micron: 1000,
            layers: Vec::new(),
            rules: vec![
                Rule {
                    name: "m1.width".into(),
                    kind: RuleKind::Width,
                    layer: MET1,
                    other_layer: None,
                    value: 140,
                },
                Rule {
                    name: "m2.width".into(),
                    kind: RuleKind::Width,
                    layer: MET2,
                    other_layer: None,
                    value: 140,
                },
            ],
            stack: Vec::new(),
        });
        doc
    }

    #[test]
    fn relevant_rules_are_filtered_to_layers_in_play() {
        // A document whose technology carries a met1 rule and an unrelated met2 rule.
        // A region that captures only the met1 shape must surface the met1 rule and not
        // the met2 rule.
        let doc = doc_with_met1_shape_and_two_rules();
        let pack = ContextPack::new(origin_region());
        let text = pack.assemble(&doc);
        assert!(text.contains("m1.width"), "met1 rule surfaced: {text}");
        assert!(
            !text.contains("m2.width"),
            "met2 rule must be filtered out (no met2 shape in region): {text}"
        );
    }

    #[test]
    fn shape_cap_bounds_the_listing_and_notes_the_remainder() {
        let mut session = Session::new();
        session
            .apply(AgentCommand::CreateCell { name: "top".into() })
            .unwrap();
        // Five overlapping shapes; cap at two.
        for i in 0..5 {
            let x = i * 10;
            add_rect(&mut session, "top", MET1, x, 0, x + 5, 5);
        }
        let pack = ContextPack::new(Rect::new(Point::new(-10, -10), Point::new(1000, 1000)))
            .with_shape_cap(2);
        let text = pack.assemble(session.document());
        assert!(text.contains("5 total, showing 2"), "{text}");
        assert!(text.contains("(3 more omitted)"), "{text}");
    }

    /// The measurement the benchmark chapter cites: on a synthetic multi-shape document,
    /// a region-scoped pack for a local repair is materially smaller (by the chars/4
    /// token estimate) than the whole-document context a geometry-aware fix would need.
    ///
    /// The baseline is [`whole_document_context`], not
    /// [`document_summary`](crate::run::document_summary): to reason about a local fix a
    /// model needs the coordinates of nearby shapes, and the summary omits every
    /// coordinate (it lists only per-cell counts), so it is not a like-for-like
    /// comparison. The whole-document context lists every shape at the same fidelity the
    /// pack uses; the pack lists only the one in-region shape plus the single relevant
    /// rule.
    ///
    /// Synthetic input: one `top` cell with 200 met1 shapes marching across x, a met1
    /// width rule and a met2 spacing rule in the technology, and a repair region that
    /// overlaps exactly one shape. Measured on this input (chars/4 token estimate), the
    /// whole-document context is 1878 tokens (7509 chars) and the scoped pack is 62
    /// tokens (245 chars): a 30x reduction, about 97% fewer tokens. The lossy per-cell
    /// summary is 19 tokens but omits every coordinate, so it cannot ground a fix and is
    /// not the baseline. The assertion below pins a conservative 10x floor so the number
    /// is defended without being brittle to small format tweaks; the exact counts are
    /// recorded here and in the benchmark chapter.
    #[test]
    fn scoped_pack_is_materially_smaller_than_whole_document_context() {
        let mut session = Session::new();
        session
            .apply(AgentCommand::CreateCell { name: "top".into() })
            .unwrap();
        // 200 shapes marching across x; only the one at the origin is in the region.
        for i in 0..200 {
            let x = i * 1000;
            add_rect(&mut session, "top", MET1, x, 0, x + 200, 200);
        }
        // A technology with a rule on each of two layers, so both listings carry rules.
        set_two_rule_technology(&mut session);

        // Honest baseline: the whole document at per-shape fidelity (what a model needs
        // to reason locally), not the lossy per-cell summary.
        let whole = whole_document_context(session.document());
        let whole_tokens = token_estimate(&whole);
        // For contrast, the lossy summary the unscoped hook currently sends is tiny only
        // because it drops all geometry; a fix cannot be reasoned from counts alone.
        let summary_tokens = token_estimate(&document_summary(&session));

        // A scoped pack for a repair local to the origin shape.
        let violation = Violation::new(
            &Rule {
                name: "m1.width".into(),
                kind: RuleKind::Width,
                layer: MET1,
                other_layer: None,
                value: 140,
            },
            120,
            Rect::new(Point::new(0, 0), Point::new(200, 200)),
            "met1 width 120 < 140".into(),
        );
        let pack = ContextPack::new(Rect::new(Point::new(-50, -50), Point::new(250, 250)))
            .with_violation(violation);
        let pack_text = pack.assemble(session.document());
        let pack_tokens = token_estimate(&pack_text);

        // The pack overlaps exactly one shape regardless of how many the design holds, so
        // it carries the met1 rule (relevant) but not the met2 rule (irrelevant here).
        assert!(
            pack_text.contains("1 total"),
            "one shape in region: {pack_text}"
        );
        assert!(
            pack_text.contains("width_68_20"),
            "relevant rule kept: {pack_text}"
        );
        assert!(
            !pack_text.contains("spacing_69_20"),
            "irrelevant rule dropped: {pack_text}"
        );
        // The honest whole-document baseline does carry both rules.
        assert!(
            whole.contains("spacing_69_20"),
            "whole-doc lists all rules: {whole}"
        );

        // The pack is a small fraction of the honest whole-document context (measured
        // ~30x on this input); pin a conservative 10x floor so the recorded number is
        // defended without being brittle to small format tweaks.
        assert!(
            pack_tokens * 10 < whole_tokens,
            "pack ({pack_tokens} tok) must be under a tenth of the whole-document context \
             ({whole_tokens} tok); summary-only baseline was {summary_tokens} tok"
        );
    }

    #[test]
    fn token_estimate_is_chars_over_four_rounded_up() {
        assert_eq!(token_estimate(""), 0);
        assert_eq!(token_estimate("abcd"), 1);
        assert_eq!(token_estimate("abcde"), 2);
    }

    /// The pack drives the real loop through the `context_hook` seam: a scoped run hands
    /// the model `pack.assemble(session.document())` before each proposal, exactly the
    /// form the CLI and the DRC-fix button use. This proves the wiring end to end (the
    /// hook is invoked, the pack reflects the growing document) without a network.
    #[test]
    fn pack_drives_the_loop_as_the_context_hook() {
        use crate::run::{LoopOptions, Provenance, run_agent_task};
        use reticle_bench::model::MockModel;
        use reticle_bench::{BenchTask, CheckerRegistry, Tier};

        let create = AgentCommand::CreateCell { name: "top".into() };
        let rect = AgentCommand::AddRect {
            cell: "top".into(),
            layer: LayerArg {
                layer: MET1.layer,
                datatype: MET1.datatype,
            },
            rect: RectArg {
                min: PointArg { x: 0, y: 0 },
                max: PointArg { x: 500, y: 500 },
            },
        };
        let mut model = MockModel::new().with_script("t1_drc", vec![vec![create, rect]]);
        let task = BenchTask {
            id: "t1_drc".into(),
            tier: Tier(1),
            prompt: "Draw a clean met1 rectangle.".into(),
            technology: "sky130.tech".into(),
            checker: "drc_clean".into(),
            intent: None,
        };
        let out_dir =
            std::env::temp_dir().join(format!("reticle-ctxpack-hook-{}", std::process::id()));
        let region = Rect::new(Point::new(-100, -100), Point::new(600, 600));
        let mut hook_calls = 0_u32;
        let outcome = run_agent_task(
            &task,
            &mut model,
            &CheckerRegistry::default(),
            "",
            "0.1.0",
            LoopOptions::default(),
            &out_dir,
            0,
            &Provenance::new("mock"),
            |_model, session| {
                hook_calls += 1;
                // Drive the scoped-pack path the DRC-fix button uses. A document-aware
                // model would forward this to `set_document_context`; `MockModel` ignores
                // it, so we only assert the pack tracks the growing document. The wiring
                // itself (the hook being called per iteration with the live session) is
                // what this exercises.
                let text = ContextPack::new(region).assemble(session.document());
                // First call: the document is still empty, so the region holds no shapes.
                if hook_calls == 1 {
                    assert!(text.contains("Shapes in region: none"), "{text}");
                }
            },
        )
        .expect("run");
        assert!(hook_calls >= 1, "the hook was invoked at least once");
        assert!(outcome.record.success, "the clean rect passes drc_clean");
        let _ = std::fs::remove_dir_all(&out_dir);
    }
}
