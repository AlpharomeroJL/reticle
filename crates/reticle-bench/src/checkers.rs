//! Built-in checkers and the registry that dispatches a task's checker name.
//!
//! A [`BenchTask`] names a checker by string; the [`CheckerRegistry`] maps that name
//! to a [`Checker`] implementation the runner invokes on the final document. Three
//! checkers ship here:
//!
//! - [`RectPresent`] passes iff a rectangle exists on a target layer (the simplest
//!   "did the model draw the thing" check).
//! - [`DrcClean`] runs the built-in SKY130 rule subset and passes iff there are no
//!   violations.
//! - [`IntentCheck`] runs [`reticle_extract::check_intent`] against the task's intent
//!   spec and passes iff it is satisfied (no opens, no shorts).
//!
//! Each is unit-tested in both directions: it accepts a known-good document and
//! rejects a known-bad one.

use std::collections::HashMap;

use reticle_agent_api::Transcript;
use reticle_extract::IntentSpec;
use reticle_model::{Document, RuleSet, ShapeKind};

use crate::{BenchTask, CheckFailure, CheckResult, Checker};

/// The cell a document-level checker inspects: the first declared top cell, or any
/// cell if none is marked top, so a single-cell document still checks. Mirrors the
/// agent session's own top-cell choice.
fn target_cell(doc: &Document) -> Option<String> {
    doc.top_cells()
        .first()
        .cloned()
        .or_else(|| doc.cells().next().map(|c| c.name.clone()))
}

/// Passes iff at least one rectangle exists on the configured layer/datatype.
///
/// The layer is the check's parameter, so one registry entry (`rect_present`) can be
/// bound to whatever layer a tier-1 placement task targets (met1 by default).
#[derive(Clone, Copy, Debug)]
pub struct RectPresent {
    /// GDSII layer number the rectangle must be on.
    pub layer: u16,
    /// GDSII datatype the rectangle must be on.
    pub datatype: u16,
}

impl RectPresent {
    /// A checker for a rectangle on `layer`/`datatype`.
    #[must_use]
    pub fn new(layer: u16, datatype: u16) -> Self {
        Self { layer, datatype }
    }
}

impl Checker for RectPresent {
    fn check(&self, doc: &Document, _transcript: &Transcript) -> CheckResult {
        let present = doc.cells().any(|cell| {
            cell.shapes.iter().any(|s| {
                s.layer.layer == self.layer
                    && s.layer.datatype == self.datatype
                    && matches!(s.kind, ShapeKind::Rect(_))
            })
        });
        if present {
            CheckResult::Pass
        } else {
            CheckResult::Fail(vec![CheckFailure::new(format!(
                "no rectangle on layer {}/{}",
                self.layer, self.datatype
            ))])
        }
    }
}

/// Passes iff the document's target cell is clean under the built-in SKY130 rule
/// subset (`reticle_drc::sky130_drc_rules`).
#[derive(Clone, Copy, Debug, Default)]
pub struct DrcClean;

impl Checker for DrcClean {
    fn check(&self, doc: &Document, _transcript: &Transcript) -> CheckResult {
        let Some(cell) = target_cell(doc) else {
            return CheckResult::Fail(vec![CheckFailure::new("document has no cell to check")]);
        };
        let engine = reticle_drc::DrcEngine::new(reticle_drc::sky130_drc_rules());
        let violations = engine.check_cell(doc, &cell);
        if violations.is_empty() {
            CheckResult::Pass
        } else {
            CheckResult::Fail(
                violations
                    .iter()
                    .map(|v| CheckFailure::new(format!("{}: {}", v.rule, v.message)))
                    .collect(),
            )
        }
    }
}

/// Passes iff the task's connectivity intent is satisfied on the target cell (no
/// opens and no shorts), via [`reticle_extract::check_intent`].
///
/// The spec is parsed once at construction from the task's serialized intent string,
/// so a malformed spec is caught when the registry is built rather than mid-run.
#[derive(Clone, Debug)]
pub struct IntentCheck {
    /// The connectivity intent to enforce.
    spec: IntentSpec,
}

impl IntentCheck {
    /// Builds an intent checker from a serialized [`IntentSpec`] (the task's `intent`
    /// field, as JSON).
    ///
    /// # Errors
    ///
    /// Returns the parse error message if `intent` is not a valid [`IntentSpec`].
    pub fn from_json(intent: &str) -> Result<Self, String> {
        let spec: IntentSpec =
            serde_json::from_str(intent).map_err(|e| format!("invalid intent spec: {e}"))?;
        Ok(Self { spec })
    }

    /// Builds an intent checker directly from a parsed spec.
    #[must_use]
    pub fn new(spec: IntentSpec) -> Self {
        Self { spec }
    }
}

impl Checker for IntentCheck {
    fn check(&self, doc: &Document, _transcript: &Transcript) -> CheckResult {
        let Some(cell) = target_cell(doc) else {
            return CheckResult::Fail(vec![CheckFailure::new("document has no cell to check")]);
        };
        let report = reticle_extract::check_intent(doc, &cell, &self.spec);
        if report.is_satisfied() {
            return CheckResult::Pass;
        }
        let mut failures = Vec::new();
        for open in &report.opens {
            failures.push(CheckFailure::new(format!(
                "open on net {}: {}",
                open.net, open.detail
            )));
        }
        for short in &report.shorts {
            failures.push(CheckFailure::new(format!(
                "short between {} and {}",
                short.net_a, short.net_b
            )));
        }
        CheckResult::Fail(failures)
    }
}

/// A registry mapping a task's checker name to a [`Checker`] implementation.
///
/// The runner looks a checker up by [`BenchTask::checker`] and applies it to the
/// final document. [`default`](CheckerRegistry::default) installs the standard
/// checkers under stable names; [`for_task`](CheckerRegistry::for_task) additionally
/// derives a per-task intent checker from the task's `intent` field so intent tasks
/// need no separate registration.
pub struct CheckerRegistry {
    checkers: HashMap<String, Box<dyn Checker>>,
}

impl std::fmt::Debug for CheckerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut names: Vec<_> = self.checkers.keys().collect();
        names.sort();
        f.debug_struct("CheckerRegistry")
            .field("checkers", &names)
            .finish()
    }
}

impl Default for CheckerRegistry {
    fn default() -> Self {
        let mut checkers: HashMap<String, Box<dyn Checker>> = HashMap::new();
        // "rect_present" defaults to met1 (68/20), the tier-1 placement layer.
        checkers.insert("rect_present".into(), Box::new(RectPresent::new(68, 20)));
        checkers.insert("drc_clean".into(), Box::new(DrcClean));
        Self { checkers }
    }
}

impl CheckerRegistry {
    /// An empty registry with no checkers installed.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            checkers: HashMap::new(),
        }
    }

    /// Installs `checker` under `name`, replacing any existing entry, and returns
    /// `self` for chaining.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, checker: Box<dyn Checker>) -> Self {
        self.checkers.insert(name.into(), checker);
        self
    }

    /// The checker registered under `name`, if any.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&dyn Checker> {
        self.checkers.get(name).map(AsRef::as_ref)
    }

    /// A registry specialized for `task`: the default checkers plus, when the task
    /// carries an `intent`, an [`IntentCheck`] bound to the task's own checker name.
    ///
    /// This lets a task set `checker = "intent"` (or any name) and supply an `intent`
    /// spec; the spec is compiled into a checker under that name.
    ///
    /// # Errors
    ///
    /// Returns an error if the task names an intent-style checker but its `intent`
    /// field does not parse as an [`IntentSpec`].
    pub fn for_task(task: &BenchTask) -> Result<Self, String> {
        let mut registry = Self::default();
        if let Some(intent) = &task.intent {
            let checker = IntentCheck::from_json(intent)?;
            registry = registry.with(task.checker.clone(), Box::new(checker));
        }
        Ok(registry)
    }
}

#[cfg(test)]
mod tests {
    use super::{CheckerRegistry, DrcClean, IntentCheck, RectPresent};
    use crate::{CheckResult, Checker};
    use reticle_agent_api::Transcript;
    use reticle_extract::{IntentNet, IntentSpec, Terminal};
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{Cell, Document, DrawShape, ShapeKind};

    /// Layer id for met1 in the SKY130 tech (68/20).
    fn met1() -> LayerId {
        LayerId::new(68, 20)
    }

    /// A one-cell document holding `shapes` in a cell named `top`.
    fn doc_with(shapes: Vec<DrawShape>) -> Document {
        let mut cell = Cell::new("top");
        cell.shapes = shapes;
        let mut doc = Document::new();
        doc.insert_cell(cell);
        doc
    }

    /// A met1 rectangle from the origin to `(size, size)`.
    fn met1_rect(size: i32) -> DrawShape {
        DrawShape::new(
            met1(),
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(size, size))),
        )
    }

    #[test]
    fn rect_present_accepts_and_rejects() {
        let checker = RectPresent::new(68, 20);
        let tx = Transcript::default();
        // Good: a met1 rect exists.
        assert!(
            checker
                .check(&doc_with(vec![met1_rect(500)]), &tx)
                .is_pass()
        );
        // Bad: empty cell has no such rect.
        let bad = checker.check(&doc_with(vec![]), &tx);
        assert!(matches!(bad, CheckResult::Fail(f) if !f.is_empty()));
    }

    #[test]
    fn drc_clean_accepts_clean_and_rejects_narrow() {
        let checker = DrcClean;
        let tx = Transcript::default();
        // Good: a 500x500 met1 rect is well above min width (140) and min area (83000).
        assert!(
            checker
                .check(&doc_with(vec![met1_rect(500)]), &tx)
                .is_pass()
        );
        // Bad: a 100-wide met1 rect violates m1.1 (min width 140) and m1.6 (min area).
        let bad = checker.check(&doc_with(vec![met1_rect(100)]), &tx);
        assert!(matches!(bad, CheckResult::Fail(f) if !f.is_empty()));
    }

    #[test]
    fn intent_check_accepts_connected_and_rejects_open() {
        // Two overlapping met1 rects joined into one net; terminals sit on each.
        let joined = doc_with(vec![
            DrawShape::new(
                met1(),
                ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(300, 300))),
            ),
            DrawShape::new(
                met1(),
                ShapeKind::Rect(Rect::new(Point::new(200, 0), Point::new(500, 300))),
            ),
        ]);
        let spec = IntentSpec {
            nets: vec![IntentNet {
                name: "n".into(),
                terminals: vec![
                    Terminal {
                        name: "a".into(),
                        layer: met1(),
                        region: Rect::new(Point::new(0, 0), Point::new(10, 10)),
                    },
                    Terminal {
                        name: "b".into(),
                        layer: met1(),
                        region: Rect::new(Point::new(490, 290), Point::new(500, 300)),
                    },
                ],
            }],
            forbidden: vec![],
        };
        let checker = IntentCheck::new(spec);
        let tx = Transcript::default();
        assert!(checker.check(&joined, &tx).is_pass());

        // Bad: the two terminals now sit on disjoint rects, so the net is open.
        let split = doc_with(vec![
            DrawShape::new(
                met1(),
                ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(100, 100))),
            ),
            DrawShape::new(
                met1(),
                ShapeKind::Rect(Rect::new(Point::new(400, 200), Point::new(500, 300))),
            ),
        ]);
        let bad = checker.check(&split, &tx);
        assert!(matches!(bad, CheckResult::Fail(f) if !f.is_empty()));
    }

    #[test]
    fn default_registry_has_named_checkers() {
        let reg = CheckerRegistry::default();
        assert!(reg.get("rect_present").is_some());
        assert!(reg.get("drc_clean").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn for_task_compiles_intent_checker() {
        let intent = serde_json::to_string(&IntentSpec::default()).unwrap();
        let task = crate::BenchTask {
            id: "t".into(),
            tier: crate::Tier(2),
            prompt: "p".into(),
            technology: "sky130.tech".into(),
            checker: "intent".into(),
            intent: Some(intent),
        };
        let reg = CheckerRegistry::for_task(&task).expect("build");
        assert!(reg.get("intent").is_some());
    }

    #[test]
    fn for_task_rejects_malformed_intent() {
        let task = crate::BenchTask {
            id: "t".into(),
            tier: crate::Tier(2),
            prompt: "p".into(),
            technology: "sky130.tech".into(),
            checker: "intent".into(),
            intent: Some("{not valid".into()),
        };
        assert!(CheckerRegistry::for_task(&task).is_err());
    }
}
