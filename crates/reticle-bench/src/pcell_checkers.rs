//! A checker exercising the Phase 2 user-PCell parameter API
//! ([`reticle_gen::PCellDef`]): schema-driven defaulting via
//! [`PCellDef::effective_params`], canonical identity via
//! [`PCellDef::effective_param_hash`], and range validation via
//! [`PCellDef::validate_params`].
//!
//! # Why the geometry is not produced by the sandboxed script
//!
//! A `PCellDef`'s geometry is normally produced by
//! `reticle_script::pcell::produce`, which interprets the rhai script in
//! [`PCellDef::script`] inside a sandbox. `reticle-bench` depends on `reticle-gen`
//! (the PCell *data* types: schema, defaulting, hashing, validation) but
//! deliberately not on `reticle-script` (the sandbox), so the checker here cannot
//! run a script to build its own reference geometry.
//!
//! [`PcellBoxPad`] instead builds a fixed [`PCellDef`] (`bench.box_pad`, a
//! concentric-square pad: an outer square of side `width`, inset by `margin` on
//! every side for a second, inner square) and computes the geometry that script
//! *would* draw with a small Rust closure, while still routing every parameter
//! through the real `PCellDef` methods: a task can omit `margin` to exercise the
//! schema's own default, or state one to exercise an explicit override, exactly the
//! distinction `effective_params` exists to resolve. A task graded by this checker is
//! solved the same way every other bench task is: the model draws the rectangles by
//! hand (`add_rect`); nothing here claims a script ran, and the doc comments on
//! [`PcellBoxPad::from_params`] and `box_pad_def` say so plainly.

use reticle_agent_api::Transcript;
use reticle_gen::{FieldSchema, PCellDef, ParamSchema};
use reticle_geometry::{LayerId, Point, Rect, Shape as _};
use reticle_model::Document;
use serde_json::Value;

use crate::checker::{CheckFailure, CheckResult, Checker};
use crate::params::{ParamError, ParsedChecker};

/// The cell a checker inspects: the first declared top cell, or any cell if none is
/// marked top. Mirrors [`crate::checkers`]'s and [`crate::geom_checkers`]'s own
/// `target_cell`.
fn target_cell(doc: &Document) -> Option<String> {
    doc.top_cells()
        .first()
        .cloned()
        .or_else(|| doc.cells().next().map(|c| c.name.clone()))
}

/// Shorthand for a single-reason failure.
fn fail(reason: impl Into<String>) -> CheckResult {
    CheckResult::Fail(vec![CheckFailure::new(reason)])
}

/// Builds the [`PcellBoxPad`] checker from a parsed checker string, or `Ok(None)` if
/// `parsed` does not name it, so the caller can fall through to the rest of the
/// registry.
///
/// # Errors
///
/// Returns a [`ParamError`] when `pcell_box` is missing a required parameter, a
/// parameter does not parse, or the resolved parameters fail the PCell's own
/// [`PCellDef::validate_params`].
pub fn build(parsed: &ParsedChecker) -> Result<Option<Box<dyn Checker>>, ParamError> {
    match parsed.name() {
        "pcell_box" => Ok(Some(Box::new(PcellBoxPad::from_params(parsed)?))),
        _ => Ok(None),
    }
}

/// A documentation-only rhai-shaped script mirroring the geometry [`box_pad_def`]'s
/// PCell would draw, generalized from `reticle_script::pcell::tests::BOXES_SCRIPT`
/// with a named `margin` parameter in place of a hard-coded `10`. Carried as the
/// [`PCellDef::script`] field's content for provenance only; see the module doc for
/// why it is never interpreted here.
const BOX_PAD_SCRIPT: &str = r#"
create_cell("PAD");
add_rect("PAD", outer_layer, outer_datatype, 0, 0, width, width);
add_rect("PAD", inner_layer, inner_datatype, margin, margin, width - margin, width - margin);
set_top_cells(["PAD"]);
"#;

/// Builds the fixed `bench.box_pad` PCell definition: an outer square of side
/// `width` and a concentric inner square inset by `margin` on every side.
fn box_pad_def() -> PCellDef {
    PCellDef {
        id: "bench.box_pad".to_owned(),
        title: "Box pad".to_owned(),
        description: "A concentric-square pad: an outer square of side `width`, \
            with an inner square inset by `margin` on every side."
            .to_owned(),
        schema: ParamSchema {
            generator_id: "bench.box_pad".to_owned(),
            title: "Box pad".to_owned(),
            description: "Concentric-square pad.".to_owned(),
            fields: vec![
                FieldSchema::int("width", "Outer side length.", 500, 100, 5_000, "dbu"),
                FieldSchema::int(
                    "margin",
                    "Inset of the inner square from the outer edge.",
                    20,
                    5,
                    500,
                    "dbu",
                ),
            ],
        },
        script: BOX_PAD_SCRIPT.to_owned(),
        engine_version: "0.1.0".to_owned(),
    }
}

/// Passes iff the target cell contains the two nested rectangles a `bench.box_pad`
/// PCell instance draws for the task's parameters: an outer square of side `width` at
/// the origin on one layer, and a concentric inner square inset by `margin` on a
/// second layer.
///
/// Parameters (`pcell_box:outer=68/20,inner=67/20,width=400`): `outer`/`inner`
/// (required layers), `width` (optional, overrides the schema default of `500`),
/// `margin` (optional, overrides the schema default of `20`). Any parameter the
/// checker string omits resolves through [`PCellDef::effective_params`] exactly as a
/// real produce would, so a task can exercise the schema's own default (by omitting
/// `margin`) or an explicit override (by stating it).
#[derive(Clone, Debug)]
pub struct PcellBoxPad {
    /// Layer the outer square is drawn on.
    outer: LayerId,
    /// Layer the inner square is drawn on.
    inner: LayerId,
    /// Resolved outer side length, in DBU.
    width: i32,
    /// Resolved inset of the inner square, in DBU.
    margin: i32,
    /// The effective-parameter hash ([`PCellDef::effective_param_hash`]) for this
    /// resolved instance, surfaced in failure messages for reproducibility (the
    /// value a produce would stamp into its `ProduceMeta`, per ADR 0102 F2).
    effective_hash: String,
}

impl PcellBoxPad {
    /// Builds a box-pad checker from parsed parameters.
    ///
    /// Resolves `width`/`margin` through the real `bench.box_pad` [`PCellDef`]:
    /// [`PCellDef::effective_params`] fills in whichever of `width`/`margin` the
    /// checker string omits from the schema defaults, [`PCellDef::validate_params`]
    /// rejects a resolved value outside the schema's declared range, and
    /// [`PCellDef::effective_param_hash`] computes the canonical identity hash this
    /// resolved instance would carry.
    ///
    /// # Errors
    ///
    /// [`ParamError`] if `outer`/`inner` are missing/malformed, `width`/`margin` (when
    /// given) do not parse as integers, or the resolved parameters fail the PCell's
    /// own range validation.
    pub fn from_params(p: &ParsedChecker) -> Result<Self, ParamError> {
        let outer = p.layer("outer")?;
        let inner = p.layer("inner")?;
        let def = box_pad_def();

        let mut partial = serde_json::Map::new();
        for key in ["width", "margin"] {
            if let Some(raw) = p.get(key) {
                let v: i64 = raw.parse().map_err(|_| ParamError::Invalid {
                    key: key.to_owned(),
                    value: raw.to_owned(),
                    expected: "i64",
                })?;
                partial.insert(key.to_owned(), Value::from(v));
            }
        }
        let partial = Value::Object(partial);

        let resolved = def.effective_params(&partial);
        def.validate_params(&resolved)
            .map_err(|e| ParamError::Invalid {
                key: "width/margin".to_owned(),
                value: format!("{resolved} ({e})"),
                expected: "within the bench.box_pad schema range",
            })?;
        let effective_hash = def.effective_param_hash(&partial);

        let as_i32 =
            |field: &str| -> Result<i32, ParamError> {
                let v = resolved.get(field).and_then(Value::as_i64).ok_or_else(|| {
                    ParamError::Invalid {
                        key: field.to_owned(),
                        value: resolved.to_string(),
                        expected: "an integer field in the resolved params",
                    }
                })?;
                i32::try_from(v).map_err(|_| ParamError::Invalid {
                    key: field.to_owned(),
                    value: v.to_string(),
                    expected: "i32",
                })
            };
        let width = as_i32("width")?;
        let margin = as_i32("margin")?;

        Ok(Self {
            outer,
            inner,
            width,
            margin,
            effective_hash,
        })
    }

    /// A short prefix of [`Self::effective_hash`] for compact failure messages.
    fn short_hash(&self) -> &str {
        let end = self.effective_hash.len().min(12);
        &self.effective_hash[..end]
    }
}

impl Checker for PcellBoxPad {
    fn check(&self, doc: &Document, _transcript: &Transcript) -> CheckResult {
        let Some(cell) = target_cell(doc) else {
            return fail("document has no cell to check");
        };
        let shapes = doc.flatten(&cell);

        let outer_want = Rect::new(Point::new(0, 0), Point::new(self.width, self.width));
        let inner_min = self.margin;
        let inner_max = self.width - self.margin;
        let inner_want = Rect::new(
            Point::new(inner_min, inner_min),
            Point::new(inner_max, inner_max),
        );

        let has_outer = shapes
            .iter()
            .any(|s| s.layer == self.outer && s.bounding_box() == outer_want);
        if !has_outer {
            return fail(format!(
                "no {0}x{0} rectangle at the origin on outer layer {1}/{2} \
                 (the bench.box_pad PCell's outer square for width={0}) [param_hash {3}]",
                self.width,
                self.outer.layer,
                self.outer.datatype,
                self.short_hash()
            ));
        }
        let has_inner = shapes
            .iter()
            .any(|s| s.layer == self.inner && s.bounding_box() == inner_want);
        if !has_inner {
            return fail(format!(
                "no ({inner_min},{inner_min})-({inner_max},{inner_max}) rectangle on inner layer \
                 {}/{} (the bench.box_pad PCell's inner square for width={}, margin={}) [param_hash {}]",
                self.inner.layer,
                self.inner.datatype,
                self.width,
                self.margin,
                self.short_hash()
            ));
        }
        CheckResult::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::{PcellBoxPad, box_pad_def, build};
    use crate::checker::CheckResult;
    use crate::params::ParsedChecker;
    use reticle_agent_api::Transcript;
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{Cell, Document, DrawShape, ShapeKind};

    const OUTER: LayerId = LayerId::new(68, 20);
    const INNER: LayerId = LayerId::new(67, 20);

    fn rect(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    }

    fn doc_with(shapes: Vec<DrawShape>) -> Document {
        let mut cell = Cell::new("top");
        cell.shapes = shapes;
        let mut doc = Document::new();
        doc.insert_cell(cell);
        doc
    }

    fn run(spec: &str, doc: &Document) -> CheckResult {
        let parsed = ParsedChecker::parse(spec);
        let checker = build(&parsed)
            .expect("checker params parse")
            .expect("spec names the pcell_box checker");
        checker.check(doc, &Transcript::default())
    }

    fn assert_pass(spec: &str, doc: &Document) {
        assert!(
            run(spec, doc).is_pass(),
            "expected `{spec}` to PASS but it failed: {:?}",
            run(spec, doc)
        );
    }

    fn assert_fail(spec: &str, doc: &Document) {
        assert!(
            matches!(run(spec, doc), CheckResult::Fail(f) if !f.is_empty()),
            "expected `{spec}` to FAIL but it passed"
        );
    }

    #[test]
    fn schema_has_width_and_margin_with_documented_defaults() {
        let def = box_pad_def();
        assert_eq!(def.schema.fields.len(), 2);
        assert_eq!(def.schema.field("width").unwrap().default, 500);
        assert_eq!(def.schema.field("margin").unwrap().default, 20);
    }

    #[test]
    fn pcell_box_two_way_default_margin() {
        // Good: width=400 stated, margin left to the schema default of 20.
        let good = doc_with(vec![
            rect(OUTER, 0, 0, 400, 400),
            rect(INNER, 20, 20, 380, 380),
        ]);
        assert_pass("pcell_box:outer=68/20,inner=67/20,width=400", &good);

        // Bad: the inner square uses the wrong margin (50, not the default 20).
        let wrong_margin = doc_with(vec![
            rect(OUTER, 0, 0, 400, 400),
            rect(INNER, 50, 50, 350, 350),
        ]);
        assert_fail("pcell_box:outer=68/20,inner=67/20,width=400", &wrong_margin);

        // Bad: the outer square is missing entirely.
        let no_outer = doc_with(vec![rect(INNER, 20, 20, 380, 380)]);
        assert_fail("pcell_box:outer=68/20,inner=67/20,width=400", &no_outer);

        // Bad: empty document.
        assert_fail(
            "pcell_box:outer=68/20,inner=67/20,width=400",
            &doc_with(vec![]),
        );
    }

    #[test]
    fn pcell_box_two_way_explicit_margin_override() {
        // Good: width=600, margin=50 both explicit, on a different layer pair.
        let good = doc_with(vec![
            rect(LayerId::new(69, 20), 0, 0, 600, 600),
            rect(LayerId::new(68, 20), 50, 50, 550, 550),
        ]);
        assert_pass(
            "pcell_box:outer=69/20,inner=68/20,width=600,margin=50",
            &good,
        );

        // Bad: drawn with the *default* margin (20) instead of the stated override (50).
        let default_margin_used = doc_with(vec![
            rect(LayerId::new(69, 20), 0, 0, 600, 600),
            rect(LayerId::new(68, 20), 20, 20, 580, 580),
        ]);
        assert_fail(
            "pcell_box:outer=69/20,inner=68/20,width=600,margin=50",
            &default_margin_used,
        );
    }

    #[test]
    fn out_of_range_width_is_a_build_error() {
        // The schema caps width at 5000; 50000 must fail to build, not silently clamp.
        assert!(
            PcellBoxPad::from_params(&ParsedChecker::parse(
                "pcell_box:outer=68/20,inner=67/20,width=50000"
            ))
            .is_err()
        );
    }

    #[test]
    fn missing_layer_is_a_build_error() {
        assert!(build(&ParsedChecker::parse("pcell_box:inner=67/20,width=400")).is_err());
        assert!(build(&ParsedChecker::parse("pcell_box:outer=68/20,width=400")).is_err());
    }

    #[test]
    fn unknown_name_is_not_the_pcell_checker() {
        assert!(
            build(&ParsedChecker::parse("drc_clean"))
                .expect("no param error")
                .is_none()
        );
    }
}
