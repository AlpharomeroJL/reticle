//! Net-trace query checkers, built on the F3 trace-query API
//! ([`reticle_extract::query`]) that Phase 2's trace-api lane added.
//!
//! `crate::checkers::IntentCheck` and the connectivity checkers in
//! [`crate::geom_checkers`] all re-derive connectivity themselves (via
//! [`reticle_extract::build_components`] or [`reticle_extract::check_intent`]). The
//! checkers here instead exercise the higher-level, UI-facing query functions a
//! click-to-trace tool actually calls: [`reticle_extract::net_at_point`] (what net is
//! under this point) and [`reticle_extract::net_extent`] (that net's bounding box and
//! member count). A checker validates a final, static document rather than a live
//! cache, so it always passes `0` as the `revision` argument the query API carries for
//! cache invalidation; the value is never asserted on.
//!
//! Three checkers ship here:
//!
//! - [`NetTraceConnected`]: two probe points must resolve to the *same* net (a
//!   click-trace confirms two locations are electrically joined).
//! - [`NetTraceExtent`]: the net under a probe point must have a [`net_extent`]
//!   bounding box at least as wide (and, optionally, as tall) as required.
//! - [`NetTraceIsolated`]: two probe points must resolve to *different* nets (a
//!   click-trace confirms two locations are kept apart, the opposite property).
//!
//! Each is unit-tested in both directions.

use reticle_agent_api::Transcript;
use reticle_extract::{Extractor, net_at_point, net_extent, sky130_connection_rules};
use reticle_geometry::Point;
use reticle_model::Document;

use crate::checker::{CheckFailure, CheckResult, Checker};
use crate::params::{ParamError, ParsedChecker};

/// The cell a checker inspects: the first declared top cell, or any cell if none is
/// marked top. Mirrors [`crate::checkers`]'s and [`crate::geom_checkers`]'s own
/// `target_cell`, so every document-level checker in the crate agrees on which cell
/// to read.
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

/// Parses `x/y` (DBU) into a point.
fn parse_point(raw: &str) -> Option<(i32, i32)> {
    let (x, y) = raw.split_once('/')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// Reads `key` as a required `x/y` point parameter.
fn point_param(p: &ParsedChecker, key: &str) -> Result<Point, ParamError> {
    let raw = p.get(key).ok_or_else(|| ParamError::Missing {
        key: key.to_owned(),
    })?;
    let (x, y) = parse_point(raw).ok_or_else(|| ParamError::Invalid {
        key: key.to_owned(),
        value: raw.to_owned(),
        expected: "x/y",
    })?;
    Ok(Point::new(x, y))
}

/// Builds one of the net-trace checkers from a parsed checker string, or `Ok(None)`
/// if `parsed` does not name one, so the caller can fall through to the rest of the
/// registry.
///
/// The recognized names are `net_trace_connected`, `net_trace_extent`, and
/// `net_trace_isolated`.
///
/// # Errors
///
/// Returns a [`ParamError`] when a recognized checker is missing a required
/// parameter or a parameter does not parse.
pub fn build(parsed: &ParsedChecker) -> Result<Option<Box<dyn Checker>>, ParamError> {
    let checker: Box<dyn Checker> = match parsed.name() {
        "net_trace_connected" => Box::new(NetTraceConnected::from_params(parsed)?),
        "net_trace_extent" => Box::new(NetTraceExtent::from_params(parsed)?),
        "net_trace_isolated" => Box::new(NetTraceIsolated::from_params(parsed)?),
        _ => return Ok(None),
    };
    Ok(Some(checker))
}

/// Extracts the target cell's connectivity with the shared SKY130 connection rules
/// (the same rules [`crate::checkers::IntentCheck`] and the connectivity checkers in
/// [`crate::geom_checkers`] use), returning the flattened shapes alongside the
/// [`reticle_extract::Netlist`] whose member indices index into them.
fn extract(
    doc: &Document,
    cell: &str,
) -> (Vec<reticle_model::DrawShape>, reticle_extract::Netlist) {
    let shapes = doc.flatten(cell);
    let netlist = Extractor::new()
        .with_rules(sky130_connection_rules())
        .extract_shapes(&shapes);
    (shapes, netlist)
}

// --------------------------------------------------------------------------------
// net_trace_connected
// --------------------------------------------------------------------------------

/// Passes iff probe points `a` and `b` resolve, through [`net_at_point`], to the same
/// net, and that net's member-shape count (via [`net_extent`]) is at least
/// `min_shapes`.
///
/// Parameters (`net_trace_connected:a=10/10,b=990/10,min_shapes=2`): `a`/`b`
/// (required probe points, `x/y` in DBU), `min_shapes` (optional, default `1`).
///
/// This is a click-to-trace check: resolve the net under one point, then ask for its
/// extent, exactly the sequence a trace UI runs. A point that covers no shape, or two
/// points that resolve to different nets, fails with the query's own verdict rather
/// than a geometric guess about what should be connected.
#[derive(Clone, Copy, Debug)]
pub struct NetTraceConnected {
    /// The first probe point.
    a: Point,
    /// The second probe point, expected on the same net as `a`.
    b: Point,
    /// Minimum member-shape count the shared net must carry.
    min_shapes: usize,
}

impl NetTraceConnected {
    /// Builds a connected-trace checker from parsed parameters.
    ///
    /// # Errors
    ///
    /// [`ParamError`] if `a`/`b` are missing/malformed, or `min_shapes` does not
    /// parse.
    pub fn from_params(p: &ParsedChecker) -> Result<Self, ParamError> {
        let a = point_param(p, "a")?;
        let b = point_param(p, "b")?;
        let min_shapes = match p.get("min_shapes") {
            None => 1usize,
            Some(raw) => raw.parse::<usize>().map_err(|_| ParamError::Invalid {
                key: "min_shapes".to_owned(),
                value: raw.to_owned(),
                expected: "usize",
            })?,
        };
        Ok(Self { a, b, min_shapes })
    }
}

impl Checker for NetTraceConnected {
    fn check(&self, doc: &Document, _transcript: &Transcript) -> CheckResult {
        let Some(cell) = target_cell(doc) else {
            return fail("document has no cell to check");
        };
        let (shapes, netlist) = extract(doc, &cell);

        let Some(net_a) = net_at_point(&shapes, &netlist, self.a, 0).net else {
            return fail(format!(
                "probe point ({}, {}) covers no net",
                self.a.x, self.a.y
            ));
        };
        let Some(net_b) = net_at_point(&shapes, &netlist, self.b, 0).net else {
            return fail(format!(
                "probe point ({}, {}) covers no net",
                self.b.x, self.b.y
            ));
        };
        if net_a.name != net_b.name {
            return fail(format!(
                "point ({}, {}) is on net `{}` but point ({}, {}) is on net `{}`; they are not connected",
                self.a.x, self.a.y, net_a.name, self.b.x, self.b.y, net_b.name
            ));
        }
        let Some(extent) = net_extent(&shapes, &netlist, &net_a.name, 0) else {
            return fail(format!("net `{}` has no extent", net_a.name));
        };
        if extent.shape_count < self.min_shapes {
            return fail(format!(
                "net `{}` spans {} shape(s), expected at least {}",
                net_a.name, extent.shape_count, self.min_shapes
            ));
        }
        CheckResult::Pass
    }
}

// --------------------------------------------------------------------------------
// net_trace_extent
// --------------------------------------------------------------------------------

/// Passes iff the net covering `probe` has a [`net_extent`] bounding box at least
/// `min_width` DBU wide (and, if given, `min_height` DBU tall).
///
/// Parameters (`net_trace_extent:probe=50/100,min_width=1500,min_height=200`):
/// `probe` (required point), `min_width` (required, DBU), `min_height` (optional,
/// DBU).
///
/// Unlike [`crate::geom_checkers::LayerArea`], which sums every shape's own area on a
/// layer regardless of whether the shapes touch, this measures the extent of the
/// single net the probe actually lands on: two overlapping segments merge into one
/// wide net (this passes), but two segments that merely happen to span the same total
/// width without touching do not, since `net_at_point`/`net_extent` only ever see the
/// piece the probe is on.
#[derive(Clone, Copy, Debug)]
pub struct NetTraceExtent {
    /// The probe point identifying the net to measure.
    probe: Point,
    /// Minimum required bounding-box width, in DBU.
    min_width: i64,
    /// Minimum required bounding-box height, in DBU, if constrained.
    min_height: Option<i64>,
}

impl NetTraceExtent {
    /// Builds a net-extent checker from parsed parameters.
    ///
    /// # Errors
    ///
    /// [`ParamError`] if `probe`/`min_width` are missing/malformed, or `min_height`
    /// does not parse.
    pub fn from_params(p: &ParsedChecker) -> Result<Self, ParamError> {
        let probe = point_param(p, "probe")?;
        let min_width = p.i64("min_width")?;
        let min_height = if p.has("min_height") {
            Some(p.i64("min_height")?)
        } else {
            None
        };
        Ok(Self {
            probe,
            min_width,
            min_height,
        })
    }
}

impl Checker for NetTraceExtent {
    fn check(&self, doc: &Document, _transcript: &Transcript) -> CheckResult {
        let Some(cell) = target_cell(doc) else {
            return fail("document has no cell to check");
        };
        let (shapes, netlist) = extract(doc, &cell);

        let Some(net) = net_at_point(&shapes, &netlist, self.probe, 0).net else {
            return fail(format!(
                "probe point ({}, {}) covers no net",
                self.probe.x, self.probe.y
            ));
        };
        let Some(extent) = net_extent(&shapes, &netlist, &net.name, 0) else {
            return fail(format!("net `{}` has no extent", net.name));
        };
        let width = extent.bbox.max_x - extent.bbox.min_x;
        if width < self.min_width {
            return fail(format!(
                "net `{}` spans {width} dbu in x, expected at least {}",
                net.name, self.min_width
            ));
        }
        if let Some(min_height) = self.min_height {
            let height = extent.bbox.max_y - extent.bbox.min_y;
            if height < min_height {
                return fail(format!(
                    "net `{}` spans {height} dbu in y, expected at least {min_height}",
                    net.name
                ));
            }
        }
        CheckResult::Pass
    }
}

// --------------------------------------------------------------------------------
// net_trace_isolated
// --------------------------------------------------------------------------------

/// Passes iff probe points `a` and `b` each cover a net (via [`net_at_point`]), and
/// the two nets are different, i.e. the geometry under `a` is not shorted to the
/// geometry under `b`.
///
/// Parameters (`net_trace_isolated:a=50/50,b=950/50`): `a`/`b` (required probe
/// points, `x/y` in DBU).
///
/// The complement of [`NetTraceConnected`]: proves two locations stay apart rather
/// than proving they join.
#[derive(Clone, Copy, Debug)]
pub struct NetTraceIsolated {
    /// The first probe point.
    a: Point,
    /// The second probe point, expected on a different net from `a`.
    b: Point,
}

impl NetTraceIsolated {
    /// Builds an isolated-trace checker from parsed parameters.
    ///
    /// # Errors
    ///
    /// [`ParamError`] if `a`/`b` are missing/malformed.
    pub fn from_params(p: &ParsedChecker) -> Result<Self, ParamError> {
        Ok(Self {
            a: point_param(p, "a")?,
            b: point_param(p, "b")?,
        })
    }
}

impl Checker for NetTraceIsolated {
    fn check(&self, doc: &Document, _transcript: &Transcript) -> CheckResult {
        let Some(cell) = target_cell(doc) else {
            return fail("document has no cell to check");
        };
        let (shapes, netlist) = extract(doc, &cell);

        let Some(net_a) = net_at_point(&shapes, &netlist, self.a, 0).net else {
            return fail(format!(
                "probe point ({}, {}) covers no net",
                self.a.x, self.a.y
            ));
        };
        let Some(net_b) = net_at_point(&shapes, &netlist, self.b, 0).net else {
            return fail(format!(
                "probe point ({}, {}) covers no net",
                self.b.x, self.b.y
            ));
        };
        if net_a.name == net_b.name {
            return fail(format!(
                "point ({}, {}) and point ({}, {}) are on the same net `{}`; they must stay separate",
                self.a.x, self.a.y, self.b.x, self.b.y, net_a.name
            ));
        }
        CheckResult::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::{NetTraceConnected, NetTraceExtent, NetTraceIsolated, build};
    use crate::checker::CheckResult;
    use crate::params::ParsedChecker;
    use reticle_agent_api::Transcript;
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{Cell, Document, DrawShape, ShapeKind};

    const MET1: LayerId = LayerId::new(68, 20);

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
            .expect("spec names a net-trace checker");
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

    // ---- net_trace_connected -----------------------------------------------------

    #[test]
    fn net_trace_connected_two_way() {
        // Good: two overlapping met1 rects (one net spanning both probes).
        let joined = doc_with(vec![
            rect(MET1, 0, 0, 600, 200),
            rect(MET1, 500, 0, 1000, 200),
        ]);
        assert_pass(
            "net_trace_connected:a=50/100,b=950/100,min_shapes=2",
            &joined,
        );
        assert_pass("net_trace_connected:a=50/100,b=950/100", &joined);

        // Bad: same two rects, but disjoint now (a gap between them), so the probes
        // land on two different nets.
        let split = doc_with(vec![
            rect(MET1, 0, 0, 400, 200),
            rect(MET1, 600, 0, 1000, 200),
        ]);
        assert_fail("net_trace_connected:a=50/100,b=950/100", &split);

        // Bad: a probe point covers no shape at all.
        let one_rect = doc_with(vec![rect(MET1, 0, 0, 400, 200)]);
        assert_fail("net_trace_connected:a=50/100,b=950/100", &one_rect);

        // Bad: connected, but the net has fewer members than min_shapes demands.
        assert_fail(
            "net_trace_connected:a=50/100,b=550/100,min_shapes=5",
            &doc_with(vec![rect(MET1, 0, 0, 600, 200)]),
        );
    }

    // ---- net_trace_extent ---------------------------------------------------------

    #[test]
    fn net_trace_extent_two_way() {
        // Good: two overlapping met1 rects merge into one net spanning 0..1600.
        let wide = doc_with(vec![
            rect(MET1, 0, 0, 800, 200),
            rect(MET1, 700, 0, 1600, 200),
        ]);
        assert_pass("net_trace_extent:probe=50/100,min_width=1500", &wide);
        assert_pass(
            "net_trace_extent:probe=50/100,min_width=1500,min_height=200",
            &wide,
        );

        // Bad: the height bound is not met (the strap is only 200 dbu tall).
        assert_fail(
            "net_trace_extent:probe=50/100,min_width=1500,min_height=500",
            &wide,
        );

        // Bad: the two segments do not touch, so the probe's net is only the first
        // segment (width 800), short of the 1500 bound, even though the two shapes
        // together would span far enough if they were connected.
        let split = doc_with(vec![
            rect(MET1, 0, 0, 800, 200),
            rect(MET1, 1200, 0, 2000, 200),
        ]);
        assert_fail("net_trace_extent:probe=50/100,min_width=1500", &split);

        // Bad: the probe covers no shape.
        assert_fail(
            "net_trace_extent:probe=5000/5000,min_width=1",
            &doc_with(vec![rect(MET1, 0, 0, 100, 100)]),
        );
    }

    // ---- net_trace_isolated --------------------------------------------------------

    #[test]
    fn net_trace_isolated_two_way() {
        // Good: two disjoint met1 pads, far enough apart to be distinct nets.
        let apart = doc_with(vec![
            rect(MET1, 0, 0, 100, 100),
            rect(MET1, 900, 0, 1000, 100),
        ]);
        assert_pass("net_trace_isolated:a=50/50,b=950/50", &apart);

        // Bad: one big pad covers both probes, so they are (wrongly) the same net.
        let merged = doc_with(vec![rect(MET1, 0, 0, 1000, 100)]);
        assert_fail("net_trace_isolated:a=50/50,b=950/50", &merged);

        // Bad: a probe covers no shape at all.
        assert_fail(
            "net_trace_isolated:a=50/50,b=950/50",
            &doc_with(vec![rect(MET1, 0, 0, 100, 100)]),
        );
    }

    // ---- dispatch / param errors ---------------------------------------------------

    #[test]
    fn unknown_name_is_not_a_net_trace_checker() {
        assert!(
            build(&ParsedChecker::parse("drc_clean"))
                .expect("no param error")
                .is_none()
        );
    }

    #[test]
    fn missing_required_param_is_an_error() {
        assert!(build(&ParsedChecker::parse("net_trace_connected:a=1/1")).is_err());
        assert!(build(&ParsedChecker::parse("net_trace_extent:probe=1/1")).is_err());
        assert!(build(&ParsedChecker::parse("net_trace_isolated:a=1/1")).is_err());
    }

    #[test]
    fn malformed_point_is_invalid() {
        assert!(build(&ParsedChecker::parse("net_trace_connected:a=nope,b=1/1")).is_err());
        assert!(NetTraceConnected::from_params(&ParsedChecker::parse("x:a=1/1,b=1")).is_err());
        assert!(
            NetTraceExtent::from_params(&ParsedChecker::parse("x:probe=1,min_width=1")).is_err()
        );
        assert!(NetTraceIsolated::from_params(&ParsedChecker::parse("x:a=1/1,b=oops")).is_err());
    }
}
