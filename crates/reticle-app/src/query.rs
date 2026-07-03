//! A small filter language for the selection query bar.
//!
//! The query bar accepts a space-separated list of predicates combined with a
//! logical and, so `layer:METAL1 width<400 area>1000` selects every shape on the
//! `METAL1` layer that is narrower than 400 DBU and larger than 1000 DBU². Parsing
//! is split from evaluation so both halves are unit-testable without an egui
//! context or a live document:
//!
//! * [`Query::parse`] turns the raw text into a list of [`Predicate`]s, or a
//!   [`ParseError`] naming the offending token. It resolves nothing about the
//!   document; a `layer:` predicate keeps the *name* the user typed.
//! * [`Query::matches`] evaluates the parsed predicates against a single
//!   [`DrawShape`], given a [`LayerLookup`] that resolves a layer name to its
//!   [`LayerId`]. [`Query::select`] runs it over a whole slice and collects the
//!   matching indices, which is exactly what the panel unions into the selection.
//!
//! The grammar is deliberately tiny:
//!
//! ```text
//! query      := predicate (WS predicate)*
//! predicate  := layer-pred | cell-pred | metric-pred
//! layer-pred := "layer:" NAME
//! cell-pred  := "cell:" NAME
//! metric-pred:= metric OP INT
//! metric     := "area" | "width" | "height"
//! OP         := "<" | "<=" | ">" | ">=" | "=" | "=="
//! ```
//!
//! `NAME` matching is case-insensitive. Metric comparisons are on the shape
//! bounding box in DBU (`area` in DBU², `width`/`height` in DBU), using the same
//! `i64` arithmetic as [`reticle_geometry::Rect`], so no precision is lost.

use reticle_geometry::{LayerId, Shape};
use reticle_model::DrawShape;
use std::collections::HashMap;
use std::fmt;

/// A comparison operator in a metric predicate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Comparator {
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `=` or `==`
    Eq,
}

impl Comparator {
    /// Applies the comparator to two values (`lhs OP rhs`).
    #[must_use]
    fn apply(self, lhs: i64, rhs: i64) -> bool {
        match self {
            Comparator::Lt => lhs < rhs,
            Comparator::Le => lhs <= rhs,
            Comparator::Gt => lhs > rhs,
            Comparator::Ge => lhs >= rhs,
            Comparator::Eq => lhs == rhs,
        }
    }
}

impl fmt::Display for Comparator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Comparator::Lt => "<",
            Comparator::Le => "<=",
            Comparator::Gt => ">",
            Comparator::Ge => ">=",
            Comparator::Eq => "=",
        };
        f.write_str(s)
    }
}

/// Which bounding-box metric a metric predicate compares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Metric {
    /// Bounding-box area in DBU².
    Area,
    /// Bounding-box width in DBU.
    Width,
    /// Bounding-box height in DBU.
    Height,
}

impl Metric {
    /// Reads this metric off a shape's bounding box, in DBU (or DBU² for area).
    #[must_use]
    fn value_of(self, shape: &DrawShape) -> i64 {
        let bbox = shape.bounding_box();
        match self {
            Metric::Area => bbox.area(),
            Metric::Width => bbox.width(),
            Metric::Height => bbox.height(),
        }
    }

    /// The keyword that names this metric in a query.
    #[must_use]
    fn keyword(self) -> &'static str {
        match self {
            Metric::Area => "area",
            Metric::Width => "width",
            Metric::Height => "height",
        }
    }
}

/// One parsed predicate. A [`Query`] is an implicit AND of these.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Predicate {
    /// Match shapes on the named layer (name resolved at evaluation time,
    /// case-insensitive).
    Layer(String),
    /// Match when the query's cell scope equals this name (case-insensitive).
    ///
    /// The scene the panel selects over is the flattened top cell, so individual
    /// shapes carry no cell provenance. This predicate therefore filters on the
    /// *scope* the evaluator is given (the current top cell): `cell:TOP` keeps
    /// every shape when `TOP` is being viewed and drops all of them otherwise.
    /// It exists so a query can assert which cell it targets and read naturally
    /// alongside the other predicates.
    Cell(String),
    /// Compare a bounding-box metric against a constant.
    Metric {
        /// Which metric (area/width/height).
        metric: Metric,
        /// The comparison operator.
        cmp: Comparator,
        /// The right-hand constant in DBU (or DBU² for area).
        value: i64,
    },
}

/// A parsed query: a conjunction of predicates.
///
/// An empty query (no predicates) matches nothing, so an empty query bar never
/// silently selects the whole scene.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Query {
    predicates: Vec<Predicate>,
}

/// Why a query string could not be parsed.
///
/// Every variant names the specific token at fault so the panel can show a
/// one-line reason next to the query bar instead of a generic failure.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ParseError {
    /// A `layer:`/`cell:` token had nothing after the colon (e.g. `layer:`).
    EmptyValue {
        /// The key whose value was missing (`"layer"` or `"cell"`).
        key: String,
    },
    /// A metric predicate had no comparator (e.g. `area1000`, `width`).
    MissingComparator {
        /// The offending token as typed.
        token: String,
    },
    /// A metric predicate's right-hand side was not an integer (e.g. `area>big`).
    NotANumber {
        /// The metric keyword.
        metric: String,
        /// The text that failed to parse as an integer.
        value: String,
    },
    /// The token matched no predicate form at all (e.g. `color:red`, `foo`).
    UnknownToken {
        /// The offending token as typed.
        token: String,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::EmptyValue { key } => {
                write!(f, "'{key}:' needs a value, e.g. {key}:METAL1")
            }
            ParseError::MissingComparator { token } => {
                write!(
                    f,
                    "'{token}' needs a comparator, e.g. width<400 or area>=1000"
                )
            }
            ParseError::NotANumber { metric, value } => {
                write!(f, "'{metric}' expects a number, got '{value}'")
            }
            ParseError::UnknownToken { token } => {
                write!(f, "unknown term '{token}'")
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// Resolves a layer name to its [`LayerId`] for evaluating a `layer:` predicate.
///
/// The app builds one of these from its layer table each time it runs a query.
/// Matching is case-insensitive; names are compared after lowercasing.
#[derive(Clone, Debug, Default)]
pub struct LayerLookup {
    by_name: HashMap<String, LayerId>,
}

impl LayerLookup {
    /// Builds a lookup from `(name, id)` pairs. Later duplicates overwrite earlier
    /// ones, matching how the layer table treats a repeated name.
    pub fn new<I, S>(entries: I) -> Self
    where
        I: IntoIterator<Item = (S, LayerId)>,
        S: AsRef<str>,
    {
        let by_name = entries
            .into_iter()
            .map(|(name, id)| (name.as_ref().to_ascii_lowercase(), id))
            .collect();
        Self { by_name }
    }

    /// Resolves `name` (case-insensitively) to a layer id, if the table has it.
    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<LayerId> {
        self.by_name.get(&name.to_ascii_lowercase()).copied()
    }
}

impl Query {
    /// Parses a query string into a conjunction of predicates.
    ///
    /// Tokens are split on ASCII whitespace and each is parsed independently.
    /// Returns the first [`ParseError`] encountered, so a malformed query never
    /// yields a partial (and misleading) selection.
    ///
    /// # Errors
    ///
    /// Returns a [`ParseError`] describing the first token that does not match any
    /// predicate form.
    pub fn parse(input: &str) -> Result<Self, ParseError> {
        let mut predicates = Vec::new();
        for token in input.split_whitespace() {
            predicates.push(parse_token(token)?);
        }
        Ok(Self { predicates })
    }

    /// The parsed predicates, in the order they appeared.
    #[must_use]
    pub fn predicates(&self) -> &[Predicate] {
        &self.predicates
    }

    /// Whether the query has no predicates (parsed from empty/whitespace input).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.predicates.is_empty()
    }

    /// Evaluates the query against one shape.
    ///
    /// `scope` is the name of the cell whose flattened shapes are being queried
    /// (the current top cell), used only by [`Predicate::Cell`]. A `layer:`
    /// predicate that names a layer absent from `layers` never matches. An empty
    /// query matches nothing.
    #[must_use]
    pub fn matches(&self, shape: &DrawShape, layers: &LayerLookup, scope: &str) -> bool {
        if self.predicates.is_empty() {
            return false;
        }
        self.predicates
            .iter()
            .all(|p| predicate_matches(p, shape, layers, scope))
    }

    /// Runs the query over `shapes` and collects the indices that match, in order.
    ///
    /// This is what the panel unions into the live selection.
    #[must_use]
    pub fn select(&self, shapes: &[DrawShape], layers: &LayerLookup, scope: &str) -> Vec<usize> {
        if self.predicates.is_empty() {
            return Vec::new();
        }
        shapes
            .iter()
            .enumerate()
            .filter(|(_, s)| self.matches(s, layers, scope))
            .map(|(i, _)| i)
            .collect()
    }
}

/// Evaluates a single predicate against a shape.
fn predicate_matches(
    predicate: &Predicate,
    shape: &DrawShape,
    layers: &LayerLookup,
    scope: &str,
) -> bool {
    match predicate {
        Predicate::Layer(name) => layers.resolve(name) == Some(shape.layer()),
        Predicate::Cell(name) => name.eq_ignore_ascii_case(scope),
        Predicate::Metric { metric, cmp, value } => cmp.apply(metric.value_of(shape), *value),
    }
}

/// Parses one whitespace-free token into a predicate.
fn parse_token(token: &str) -> Result<Predicate, ParseError> {
    if let Some(value) = token.strip_prefix("layer:") {
        return non_empty_value(value, "layer").map(|v| Predicate::Layer(v.to_owned()));
    }
    if let Some(value) = token.strip_prefix("cell:") {
        return non_empty_value(value, "cell").map(|v| Predicate::Cell(v.to_owned()));
    }
    for metric in [Metric::Area, Metric::Width, Metric::Height] {
        if let Some(rest) = token.strip_prefix(metric.keyword()) {
            return parse_metric(metric, rest, token);
        }
    }
    Err(ParseError::UnknownToken {
        token: token.to_owned(),
    })
}

/// Rejects an empty `key:` value.
fn non_empty_value<'a>(value: &'a str, key: &str) -> Result<&'a str, ParseError> {
    if value.is_empty() {
        Err(ParseError::EmptyValue {
            key: key.to_owned(),
        })
    } else {
        Ok(value)
    }
}

/// Parses the `OP INT` tail of a metric predicate (`rest` is everything after the
/// metric keyword; `token` is the whole token, for error messages).
fn parse_metric(metric: Metric, rest: &str, token: &str) -> Result<Predicate, ParseError> {
    let (cmp, num) = split_comparator(rest).ok_or_else(|| ParseError::MissingComparator {
        token: token.to_owned(),
    })?;
    let value = num.parse::<i64>().map_err(|_| ParseError::NotANumber {
        metric: metric.keyword().to_owned(),
        value: num.to_owned(),
    })?;
    Ok(Predicate::Metric { metric, cmp, value })
}

/// Splits a comparator off the front of `rest`, returning the operator and the
/// remaining (numeric) text. The two-character operators are tried first so `<=`
/// is not mis-read as `<` followed by `=`.
fn split_comparator(rest: &str) -> Option<(Comparator, &str)> {
    for (sym, cmp) in [
        ("<=", Comparator::Le),
        (">=", Comparator::Ge),
        ("==", Comparator::Eq),
    ] {
        if let Some(num) = rest.strip_prefix(sym) {
            return Some((cmp, num));
        }
    }
    for (sym, cmp) in [
        ("<", Comparator::Lt),
        (">", Comparator::Gt),
        ("=", Comparator::Eq),
    ] {
        if let Some(num) = rest.strip_prefix(sym) {
            return Some((cmp, num));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::{Point, Rect};
    use reticle_model::ShapeKind;

    const M1: LayerId = LayerId::new(4, 0);
    const M2: LayerId = LayerId::new(5, 0);

    fn lookup() -> LayerLookup {
        LayerLookup::new([("METAL1", M1), ("METAL2", M2)])
    }

    fn rect_on(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    }

    #[test]
    fn parses_layer_predicate() {
        let q = Query::parse("layer:METAL1").unwrap();
        assert_eq!(q.predicates(), &[Predicate::Layer("METAL1".to_owned())]);
    }

    #[test]
    fn parses_all_comparators() {
        let cases = [
            ("width<400", Comparator::Lt, 400),
            ("width<=400", Comparator::Le, 400),
            ("width>400", Comparator::Gt, 400),
            ("width>=400", Comparator::Ge, 400),
            ("width=400", Comparator::Eq, 400),
            ("width==400", Comparator::Eq, 400),
        ];
        for (src, cmp, value) in cases {
            let q = Query::parse(src).unwrap();
            assert_eq!(
                q.predicates(),
                &[Predicate::Metric {
                    metric: Metric::Width,
                    cmp,
                    value,
                }],
                "parsing {src}"
            );
        }
    }

    #[test]
    fn parses_compound_query() {
        let q = Query::parse("layer:METAL1 width<400 area>1000").unwrap();
        assert_eq!(
            q.predicates(),
            &[
                Predicate::Layer("METAL1".to_owned()),
                Predicate::Metric {
                    metric: Metric::Width,
                    cmp: Comparator::Lt,
                    value: 400,
                },
                Predicate::Metric {
                    metric: Metric::Area,
                    cmp: Comparator::Gt,
                    value: 1000,
                },
            ]
        );
    }

    #[test]
    fn parses_area_and_height() {
        let q = Query::parse("area>=1000 height<50").unwrap();
        assert_eq!(
            q.predicates(),
            &[
                Predicate::Metric {
                    metric: Metric::Area,
                    cmp: Comparator::Ge,
                    value: 1000,
                },
                Predicate::Metric {
                    metric: Metric::Height,
                    cmp: Comparator::Lt,
                    value: 50,
                },
            ]
        );
    }

    #[test]
    fn extra_whitespace_is_ignored() {
        let q = Query::parse("   layer:METAL1    width<400  ").unwrap();
        assert_eq!(q.predicates().len(), 2);
    }

    #[test]
    fn empty_query_parses_to_no_predicates() {
        let q = Query::parse("   ").unwrap();
        assert!(q.is_empty());
    }

    #[test]
    fn negative_and_large_values_parse() {
        let q = Query::parse("width>-5 area<=9000000000").unwrap();
        assert_eq!(
            q.predicates(),
            &[
                Predicate::Metric {
                    metric: Metric::Width,
                    cmp: Comparator::Gt,
                    value: -5,
                },
                Predicate::Metric {
                    metric: Metric::Area,
                    cmp: Comparator::Le,
                    value: 9_000_000_000,
                },
            ]
        );
    }

    #[test]
    fn malformed_empty_layer_value_errors() {
        let err = Query::parse("layer:").unwrap_err();
        assert_eq!(
            err,
            ParseError::EmptyValue {
                key: "layer".to_owned()
            }
        );
    }

    #[test]
    fn malformed_empty_cell_value_errors() {
        let err = Query::parse("cell:").unwrap_err();
        assert_eq!(
            err,
            ParseError::EmptyValue {
                key: "cell".to_owned()
            }
        );
    }

    #[test]
    fn malformed_missing_comparator_errors() {
        let err = Query::parse("area1000").unwrap_err();
        assert_eq!(
            err,
            ParseError::MissingComparator {
                token: "area1000".to_owned()
            }
        );
    }

    #[test]
    fn bare_metric_keyword_errors() {
        let err = Query::parse("width").unwrap_err();
        assert_eq!(
            err,
            ParseError::MissingComparator {
                token: "width".to_owned()
            }
        );
    }

    #[test]
    fn malformed_non_numeric_value_errors() {
        let err = Query::parse("area>big").unwrap_err();
        assert_eq!(
            err,
            ParseError::NotANumber {
                metric: "area".to_owned(),
                value: "big".to_owned(),
            }
        );
    }

    #[test]
    fn empty_metric_value_errors_as_not_a_number() {
        let err = Query::parse("width<").unwrap_err();
        assert_eq!(
            err,
            ParseError::NotANumber {
                metric: "width".to_owned(),
                value: String::new(),
            }
        );
    }

    #[test]
    fn unknown_token_errors() {
        let err = Query::parse("color:red").unwrap_err();
        assert_eq!(
            err,
            ParseError::UnknownToken {
                token: "color:red".to_owned()
            }
        );
    }

    #[test]
    fn first_bad_token_wins() {
        // The layer token is valid; the second token is the first failure.
        let err = Query::parse("layer:METAL1 nonsense").unwrap_err();
        assert_eq!(
            err,
            ParseError::UnknownToken {
                token: "nonsense".to_owned()
            }
        );
    }

    #[test]
    fn error_messages_are_human_readable() {
        assert!(
            Query::parse("layer:")
                .unwrap_err()
                .to_string()
                .contains("needs a value")
        );
        assert!(
            Query::parse("area>big")
                .unwrap_err()
                .to_string()
                .contains("expects a number")
        );
        assert!(
            Query::parse("nope")
                .unwrap_err()
                .to_string()
                .contains("unknown term")
        );
    }

    #[test]
    fn matches_layer_predicate_case_insensitively() {
        let q = Query::parse("layer:metal1").unwrap();
        let on_m1 = rect_on(M1, 0, 0, 100, 100);
        let on_m2 = rect_on(M2, 0, 0, 100, 100);
        assert!(q.matches(&on_m1, &lookup(), "TOP"));
        assert!(!q.matches(&on_m2, &lookup(), "TOP"));
    }

    #[test]
    fn unknown_layer_name_matches_nothing() {
        let q = Query::parse("layer:METAL9").unwrap();
        let on_m1 = rect_on(M1, 0, 0, 100, 100);
        assert!(!q.matches(&on_m1, &lookup(), "TOP"));
    }

    #[test]
    fn matches_width_and_area_metrics() {
        // 300 wide, 300 tall => area 90_000.
        let s = rect_on(M1, 0, 0, 300, 300);
        assert!(
            Query::parse("width<400")
                .unwrap()
                .matches(&s, &lookup(), "T")
        );
        assert!(
            !Query::parse("width>400")
                .unwrap()
                .matches(&s, &lookup(), "T")
        );
        assert!(
            Query::parse("area>1000")
                .unwrap()
                .matches(&s, &lookup(), "T")
        );
        assert!(
            Query::parse("area<1000")
                .unwrap()
                .matches(&s, &lookup(), "T")
                .eq(&false)
        );
        assert!(
            Query::parse("height=300")
                .unwrap()
                .matches(&s, &lookup(), "T")
        );
    }

    #[test]
    fn compound_predicates_are_anded() {
        // METAL1, 300x300, area 90_000.
        let s = rect_on(M1, 0, 0, 300, 300);
        // All three hold.
        assert!(
            Query::parse("layer:METAL1 width<400 area>1000")
                .unwrap()
                .matches(&s, &lookup(), "TOP")
        );
        // Width clause fails => whole query fails.
        assert!(
            !Query::parse("layer:METAL1 width<200")
                .unwrap()
                .matches(&s, &lookup(), "TOP")
        );
        // Layer clause fails => whole query fails.
        assert!(
            !Query::parse("layer:METAL2 width<400")
                .unwrap()
                .matches(&s, &lookup(), "TOP")
        );
    }

    #[test]
    fn cell_predicate_scopes_by_top_cell_name() {
        let s = rect_on(M1, 0, 0, 100, 100);
        assert!(
            Query::parse("cell:TOP")
                .unwrap()
                .matches(&s, &lookup(), "TOP")
        );
        assert!(
            Query::parse("cell:top")
                .unwrap()
                .matches(&s, &lookup(), "TOP")
        );
        assert!(
            !Query::parse("cell:LEAF")
                .unwrap()
                .matches(&s, &lookup(), "TOP")
        );
    }

    #[test]
    fn empty_query_matches_nothing() {
        let q = Query::default();
        let s = rect_on(M1, 0, 0, 100, 100);
        assert!(!q.matches(&s, &lookup(), "TOP"));
        assert!(q.select(&[s], &lookup(), "TOP").is_empty());
    }

    #[test]
    fn select_collects_matching_indices_in_order() {
        let shapes = vec![
            rect_on(M1, 0, 0, 300, 300),   // 0: METAL1, small
            rect_on(M2, 0, 0, 300, 300),   // 1: METAL2
            rect_on(M1, 0, 0, 100, 100),   // 2: METAL1, smaller
            rect_on(M1, 0, 0, 5000, 5000), // 3: METAL1, big
        ];
        let hits =
            Query::parse("layer:METAL1 width<=300")
                .unwrap()
                .select(&shapes, &lookup(), "TOP");
        assert_eq!(hits, vec![0, 2]);
    }

    #[test]
    fn select_on_area_threshold() {
        let shapes = vec![
            rect_on(M1, 0, 0, 10, 10),   // area 100
            rect_on(M1, 0, 0, 100, 100), // area 10_000
            rect_on(M1, 0, 0, 40, 40),   // area 1_600
        ];
        let hits = Query::parse("area>1000")
            .unwrap()
            .select(&shapes, &lookup(), "TOP");
        assert_eq!(hits, vec![1, 2]);
    }
}
