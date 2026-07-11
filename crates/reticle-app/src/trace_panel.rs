//! The Inspector's Trace section state: F3 net-trace query records and the
//! shorts/opens navigator (ADR 0103).
//!
//! The trace-api lane (Phase 2) adds read-only spatial queries over an extracted
//! netlist: net-at-point, net-extent, and a shorts/opens connectivity report (see
//! [`reticle_extract::query`]). This module holds the Trace section's state and
//! its pure, unit-tested logic, mirroring [`crate::drc_panel`]: no `egui` here at
//! all, so the record types, the combined shorts/opens navigator, and the display
//! formatting are tested without a window. The thin egui glue that reads this
//! state lives in `App::trace_section` (`crate::app`), styled entirely from
//! [`crate::theme::components`]/[`crate::theme::tokens`], as [`crate::review_panel`]
//! is for the review workflow.
//!
//! # Live queries, fixture fallback
//!
//! [`query_at_point`], [`query_extent`], and [`query_report`] are the real thing
//! (the `f2f3-wiring` lane, Gate 3): they extract connectivity over the OPEN
//! document (`crate::app`'s marked hook supplies the document, top cell, and
//! selection) and call [`reticle_extract::query`]'s live functions directly,
//! `net_at_point`, `net_extent`, and `shorts_opens` over an [`IntentSpec`]
//! derived from the target cell's own pins (`intent_spec_from_pins`, private).
//! [`fixture_at_point`], [`fixture_extent`], and [`fixture_report`] parse the
//! committed F3 contract fixture (`tests/fixtures/contracts/f3_trace.json` in
//! `reticle-extract`) and remain the fallback `crate::app` uses when there is no
//! document/selection to query for real: an honest stand-in for the empty case,
//! not a permanent substitute.

use reticle_extract::query::{NetAtPoint, NetExtent, OpenRecord, ShortRecord, ShortsOpensReport};
use reticle_extract::{
    Extractor, ForbiddenPair, IntentNet, IntentSpec, Netlist, Terminal, check_intent, net_at_point,
    net_extent, shorts_opens,
};
use reticle_geometry::{Point, Shape as _};
use reticle_model::{Cell, Document, DrawShape};

/// The committed F3 contract fixture this phase renders against (see the module
/// docs). Shared with the trace-api lane's own producer test
/// (`reticle-extract/tests/f3_query.rs`); do not fork a second copy of this data.
const FIXTURE_JSON: &str =
    include_str!("../../reticle-extract/tests/fixtures/contracts/f3_trace.json");

/// One row of the combined shorts/opens navigable list: a short between two
/// nets, or an open (split net).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TraceRow {
    /// A short between two nets, from a `ShortsOpensReport`'s shorts list.
    Short(ShortRecord),
    /// An open (split net), from a `ShortsOpensReport`'s opens list.
    Open(OpenRecord),
}

/// The Trace Inspector section's state: the last-loaded net-at-point and
/// net-extent readouts, the last-loaded shorts/opens report, and the selected
/// row in its combined navigable list.
///
/// Every `load_*` method installs a fresh record, a real query result
/// ([`query_at_point`]/[`query_extent`]/[`query_report`]) when `crate::app`'s hook
/// can query the open document for real, else the fixture fallback
/// ([`fixture_at_point`]/[`fixture_extent`]/[`fixture_report`]);
/// [`TracePanelState::load_report`] additionally resets the navigator, matching
/// how a fresh DRC run replaces the selection in [`crate::drc_panel::DrcResults`].
#[derive(Clone, Debug, Default)]
pub struct TracePanelState {
    /// The last net-at-point result, if a `trace.at_point` query has run.
    at_point: Option<NetAtPoint>,
    /// The last net-extent result, if a `trace.net_extent` query has run.
    extent: Option<NetExtent>,
    /// The last shorts/opens report, if a `trace.shorts_opens` check has run.
    report: Option<ShortsOpensReport>,
    /// The selected row index into the combined shorts-then-opens list.
    selected: Option<usize>,
}

impl TracePanelState {
    /// An empty Trace section: nothing queried yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether nothing has been queried yet (the section renders its empty
    /// state).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.at_point.is_none() && self.extent.is_none() && self.report.is_none()
    }

    /// The last net-at-point result, if queried.
    #[must_use]
    pub fn at_point(&self) -> Option<&NetAtPoint> {
        self.at_point.as_ref()
    }

    /// The last net-extent result, if queried.
    #[must_use]
    pub fn extent(&self) -> Option<&NetExtent> {
        self.extent.as_ref()
    }

    /// The last shorts/opens report, if run.
    #[must_use]
    pub fn report(&self) -> Option<&ShortsOpensReport> {
        self.report.as_ref()
    }

    /// The selected row index into the combined shorts-then-opens list, if any.
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Installs a fresh net-at-point result.
    pub fn load_at_point(&mut self, record: NetAtPoint) {
        self.at_point = Some(record);
    }

    /// Installs a fresh net-extent result.
    pub fn load_extent(&mut self, record: NetExtent) {
        self.extent = Some(record);
    }

    /// Installs a fresh shorts/opens report, selecting its first row (or
    /// clearing the selection when the report is clean).
    pub fn load_report(&mut self, record: ShortsOpensReport) {
        self.selected = if record.is_empty() { None } else { Some(0) };
        self.report = Some(record);
    }

    /// The number of navigable rows in the current report (0 if none is loaded
    /// or the report is clean).
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.report.as_ref().map_or(0, ShortsOpensReport::len)
    }

    /// The row at `index` in the combined shorts-then-opens list (shorts first,
    /// then opens), or `None` if the index is out of range or no report is
    /// loaded.
    #[must_use]
    pub fn row(&self, index: usize) -> Option<TraceRow> {
        let report = self.report.as_ref()?;
        match report.shorts.get(index) {
            Some(short) => Some(TraceRow::Short(short.clone())),
            None => report
                .opens
                .get(index - report.shorts.len())
                .cloned()
                .map(TraceRow::Open),
        }
    }

    /// Selects row `index` directly (a list click). Returns `false` (leaving the
    /// selection unchanged) if `index` is out of range or no report is loaded.
    pub fn select(&mut self, index: usize) -> bool {
        if index < self.row_count() {
            self.selected = Some(index);
            true
        } else {
            false
        }
    }

    /// Advances the selection to the next row, wrapping to the first after the
    /// last. A no-op when the report is empty or unloaded.
    pub fn select_next(&mut self) -> bool {
        self.step_from(1)
    }

    /// Moves the selection to the previous row, wrapping to the last before the
    /// first. A no-op when the report is empty or unloaded.
    pub fn select_prev(&mut self) -> bool {
        let count = self.row_count();
        if count == 0 {
            return false;
        }
        self.step_from(count - 1)
    }

    /// Steps the selection forward by `delta` rows modulo the row count,
    /// treating no current selection as position 0. `delta = count - 1` steps
    /// backward by one (mod arithmetic).
    fn step_from(&mut self, delta: usize) -> bool {
        let count = self.row_count();
        if count == 0 {
            return false;
        }
        let pos = self.selected.unwrap_or(0);
        self.selected = Some((pos + delta) % count);
        true
    }
}

/// Parses the embedded F3 fixture into a JSON value.
fn fixture_root() -> serde_json::Value {
    serde_json::from_str(FIXTURE_JSON).expect("committed F3 fixture is valid JSON")
}

/// Parses the embedded F3 fixture's `net_at_point` record: the fallback for
/// [`query_at_point`] when there is no document/selection to query for real
/// (see the module docs).
#[must_use]
pub fn fixture_at_point() -> NetAtPoint {
    serde_json::from_value(fixture_root()["net_at_point"].clone())
        .expect("F3 fixture net_at_point matches `NetAtPoint`")
}

/// Parses the embedded F3 fixture's `net_extent` record (see [`fixture_at_point`]).
#[must_use]
pub fn fixture_extent() -> NetExtent {
    serde_json::from_value(fixture_root()["net_extent"].clone())
        .expect("F3 fixture net_extent matches `NetExtent`")
}

/// Parses the embedded F3 fixture's shorts/opens `report` record (see
/// [`fixture_at_point`]).
#[must_use]
pub fn fixture_report() -> ShortsOpensReport {
    serde_json::from_value(fixture_root()["report"].clone())
        .expect("F3 fixture report matches `ShortsOpensReport`")
}

/// The center of `shape`'s bounding box, in DBU.
///
/// [`net_at_point`]'s coverage test
/// (`reticle_extract::connectivity::shape_covers_point`, private to that crate
/// but exercised here through [`net_at_point`] itself) only checks bounding-box
/// containment, so this point is always "covered" by `shape` itself, whatever
/// its kind (rect, polygon, or path). Used as the query point for a selected
/// shape: the live Trace section has no separate coordinate-entry field (ADR
/// 0103), so a selection's own footprint is the real point source `crate::app`'s
/// `f2f3-wiring` hook feeds [`query_at_point`].
#[must_use]
pub fn shape_probe_point(shape: &DrawShape) -> Point {
    let b = shape.bounding_box();
    Point::new(
        b.min.x + (b.max.x - b.min.x) / 2,
        b.min.y + (b.max.y - b.min.y) / 2,
    )
}

/// Extracts connectivity over the flattened `top` cell of `doc`: same-layer-only
/// (no via/contact rules), matching `crate::netlight`'s net-highlight extraction
/// so a live trace query agrees with clicking the same shape on canvas.
fn extract_live(doc: &Document, top: &str) -> (Vec<DrawShape>, Netlist) {
    let shapes = doc.flatten(top);
    let netlist = Extractor::new().extract_shapes(&shapes);
    (shapes, netlist)
}

/// Runs a REAL net-at-point query over `doc`'s flattened `top` cell (the live
/// counterpart to [`fixture_at_point`]; see the module docs). `revision` is the
/// caller's document-generation token, carried through into the result
/// unchanged.
#[must_use]
pub fn query_at_point(doc: &Document, top: &str, point: Point, revision: u64) -> NetAtPoint {
    let (shapes, netlist) = extract_live(doc, top);
    net_at_point(&shapes, &netlist, point, revision)
}

/// Runs a REAL net-extent query for the net named `net_name` over `doc`'s
/// flattened `top` cell (the live counterpart to [`fixture_extent`]), or `None`
/// if no net of that name exists.
#[must_use]
pub fn query_extent(doc: &Document, top: &str, net_name: &str, revision: u64) -> Option<NetExtent> {
    let (shapes, netlist) = extract_live(doc, top);
    net_extent(&shapes, &netlist, net_name, revision)
}

/// The cap on distinct pin names [`intent_spec_from_pins`] turns into intent
/// nets; see its doc comment.
const MAX_INTENT_NETS: usize = 256;

/// Derives a connectivity [`IntentSpec`] from `cell`'s own first-class
/// `Pin` (`reticle_model::Pin`) list: the closest thing the document has to an
/// authored connectivity intent, since no separate intent-authoring flow exists
/// yet (`crate::pcell_panel` notes the equivalent gap on the PCell-authoring
/// side).
///
/// Pins sharing a name become one [`IntentNet`] (every physical tap of the same
/// net, e.g. two `VDD` pins, must join); every pair of distinctly named pins
/// becomes a [`ForbiddenPair`] (differently named nets must stay apart). A cell
/// with fewer than two distinct pin names yields an empty spec (nothing is
/// checkable with one or zero named nets): a real, honest "nothing to check",
/// not a fixture stand-in.
///
/// Capped at [`MAX_INTENT_NETS`] distinct pin names so a document with an
/// unreasonable number of differently named pins cannot force the `O(n^2)`
/// forbidden-pair construction to blow up; a pin whose name is past the cap is
/// dropped from the derived spec (an additional tap of an already-included net
/// name is never dropped, only additional net *names* past the cap).
fn intent_spec_from_pins(cell: &Cell) -> IntentSpec {
    let mut nets: Vec<IntentNet> = Vec::new();
    for pin in &cell.pins {
        if let Some(net) = nets.iter_mut().find(|n| n.name == pin.name) {
            net.terminals.push(Terminal {
                name: pin.name.clone(),
                layer: pin.layer,
                region: pin.region,
            });
        } else if nets.len() < MAX_INTENT_NETS {
            nets.push(IntentNet {
                name: pin.name.clone(),
                terminals: vec![Terminal {
                    name: pin.name.clone(),
                    layer: pin.layer,
                    region: pin.region,
                }],
            });
        }
    }

    let mut forbidden = Vec::new();
    for i in 0..nets.len() {
        for j in (i + 1)..nets.len() {
            forbidden.push(ForbiddenPair {
                net_a: nets[i].name.clone(),
                net_b: nets[j].name.clone(),
            });
        }
    }
    IntentSpec { nets, forbidden }
}

/// Runs a REAL shorts/opens check over `doc`'s `top` cell (the live counterpart
/// to [`fixture_report`]; see the module docs): derives an [`IntentSpec`] from
/// the cell's own pins (`intent_spec_from_pins`, private), checks it with
/// [`check_intent`] (the SKY130 via/contact stack), and maps the result with
/// [`shorts_opens`]. An unknown `top` (or one with no pins) yields an empty,
/// real report rather than a fixture stand-in.
#[must_use]
pub fn query_report(doc: &Document, top: &str, revision: u64) -> ShortsOpensReport {
    let spec = doc
        .cell(top)
        .map_or_else(IntentSpec::default, intent_spec_from_pins);
    let report = check_intent(doc, top, &spec);
    shorts_opens(&report, revision)
}

/// Formats a net-at-point result: the net name and shape count, or a miss
/// message when the queried point hit no net.
#[must_use]
pub fn format_at_point(record: &NetAtPoint) -> String {
    match &record.net {
        Some(net) => {
            let n = net.shape_indices.len();
            format!("{}  ({n} shape{})", net.name, if n == 1 { "" } else { "s" })
        }
        None => "No net at this point".to_owned(),
    }
}

/// Formats a net-extent result: the net name, bounding box (DBU), and shape
/// count.
#[must_use]
pub fn format_extent(record: &NetExtent) -> String {
    let n = record.shape_count;
    format!(
        "{}  ({}, {})-({}, {})  {n} shape{}",
        record.net,
        record.bbox.min_x,
        record.bbox.min_y,
        record.bbox.max_x,
        record.bbox.max_y,
        if n == 1 { "" } else { "s" }
    )
}

/// Formats one navigable row: a short between two nets (with a touch location
/// in DBU) or an open's piece count.
#[must_use]
pub fn format_row(row: &TraceRow) -> String {
    match row {
        TraceRow::Short(s) => format!(
            "Short: {} - {}  at ({}, {})-({}, {})",
            s.net_a, s.net_b, s.at.min_x, s.at.min_y, s.at.max_x, s.at.max_y
        ),
        TraceRow::Open(o) => format!("Open: {}  {} pieces", o.net, o.pieces),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(min_x: i64, min_y: i64, max_x: i64, max_y: i64) -> reticle_extract::query::RectRecord {
        reticle_extract::query::RectRecord {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    /// A report with `shorts` shorts and `opens` opens, for wraparound tests that
    /// need more rows than the committed fixture carries.
    fn synthetic_report(shorts: usize, opens: usize) -> ShortsOpensReport {
        ShortsOpensReport {
            revision: 1,
            shorts: (0..shorts)
                .map(|i| ShortRecord {
                    net_a: format!("A{i}"),
                    net_b: format!("B{i}"),
                    at: rect(0, 0, 1, 1),
                })
                .collect(),
            opens: (0..opens)
                .map(|i| OpenRecord {
                    net: format!("N{i}"),
                    pieces: 2,
                })
                .collect(),
        }
    }

    #[test]
    fn fixture_functions_parse_the_committed_f3_fixture() {
        // Guards against silent drift between the fixture and these parsers; the
        // expected values mirror `reticle-extract/tests/f3_query.rs`.
        let at_point = fixture_at_point();
        assert_eq!(at_point.revision, 7);
        let net = at_point.net.expect("the fixture point is on a net");
        assert_eq!(net.name, "VDD");
        assert_eq!(net.shape_indices, vec![0, 3, 5]);

        let extent = fixture_extent();
        assert_eq!(extent.revision, 7);
        assert_eq!(extent.net, "VDD");
        assert_eq!(extent.shape_count, 3);
        assert_eq!(extent.bbox.max_x, 12_000);

        let report = fixture_report();
        assert_eq!(report.revision, 7);
        assert_eq!(report.len(), 2);
        assert!(!report.is_clean());
        assert_eq!(report.shorts[0].net_a, "VDD");
        assert_eq!(report.opens[0].net, "CLK");
    }

    #[test]
    fn new_state_is_empty() {
        let state = TracePanelState::new();
        assert!(state.is_empty());
        assert!(state.at_point().is_none());
        assert!(state.extent().is_none());
        assert!(state.report().is_none());
        assert_eq!(state.row_count(), 0);
        assert_eq!(state.selected(), None);
    }

    #[test]
    fn loading_any_record_clears_is_empty() {
        let mut state = TracePanelState::new();
        state.load_at_point(fixture_at_point());
        assert!(!state.is_empty());
        assert!(state.extent().is_none(), "only at_point was loaded");
    }

    #[test]
    fn load_report_selects_first_row_when_nonempty() {
        let mut state = TracePanelState::new();
        state.load_report(fixture_report());
        assert_eq!(state.row_count(), 2);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn load_report_with_a_clean_report_selects_nothing() {
        let mut state = TracePanelState::new();
        state.load_report(ShortsOpensReport {
            revision: 1,
            ..ShortsOpensReport::default()
        });
        assert_eq!(state.row_count(), 0);
        assert_eq!(state.selected(), None);
        assert!(state.report().expect("report installed").is_clean());
    }

    #[test]
    fn a_fresh_load_report_resets_a_prior_selection() {
        let mut state = TracePanelState::new();
        state.load_report(synthetic_report(2, 2));
        state.select(3);
        assert_eq!(state.selected(), Some(3));
        state.load_report(synthetic_report(1, 1));
        assert_eq!(
            state.selected(),
            Some(0),
            "a fresh report starts at row 0, not the stale index"
        );
    }

    #[test]
    fn row_dispatches_shorts_then_opens_by_index() {
        let mut state = TracePanelState::new();
        state.load_report(synthetic_report(2, 3));
        assert!(matches!(state.row(0), Some(TraceRow::Short(_))));
        assert!(matches!(state.row(1), Some(TraceRow::Short(_))));
        assert!(matches!(state.row(2), Some(TraceRow::Open(_))));
        assert!(matches!(state.row(4), Some(TraceRow::Open(_))));
        assert_eq!(state.row(5), None, "out of range");
    }

    #[test]
    fn row_is_none_when_no_report_is_loaded() {
        let state = TracePanelState::new();
        assert_eq!(state.row(0), None);
    }

    #[test]
    fn select_next_and_prev_wrap_over_the_combined_rows() {
        let mut state = TracePanelState::new();
        state.load_report(synthetic_report(1, 2)); // 3 rows total, starts at 0.
        assert_eq!(state.selected(), Some(0));

        assert!(state.select_next());
        assert_eq!(state.selected(), Some(1));
        assert!(state.select_next());
        assert_eq!(state.selected(), Some(2));
        assert!(state.select_next());
        assert_eq!(state.selected(), Some(0), "next wraps past the last row");

        assert!(state.select_prev());
        assert_eq!(state.selected(), Some(2), "prev wraps before the first row");
        assert!(state.select_prev());
        assert_eq!(state.selected(), Some(1));
    }

    #[test]
    fn next_and_prev_are_a_no_op_without_a_loaded_report() {
        let mut state = TracePanelState::new();
        assert!(!state.select_next());
        assert!(!state.select_prev());
        assert_eq!(state.selected(), None);
    }

    #[test]
    fn select_rejects_an_out_of_range_index() {
        let mut state = TracePanelState::new();
        state.load_report(synthetic_report(1, 1));
        assert!(!state.select(99));
        assert_eq!(
            state.selected(),
            Some(0),
            "the prior selection is unchanged"
        );
        assert!(state.select(1));
        assert_eq!(state.selected(), Some(1));
    }

    #[test]
    fn format_at_point_reports_the_net_and_shape_count() {
        let text = format_at_point(&fixture_at_point());
        assert!(text.contains("VDD"));
        assert!(text.contains('3'));
    }

    #[test]
    fn format_at_point_reports_a_miss() {
        let miss = NetAtPoint {
            revision: 1,
            net: None,
        };
        assert_eq!(format_at_point(&miss), "No net at this point");
    }

    #[test]
    fn format_extent_includes_bbox_and_shape_count() {
        let text = format_extent(&fixture_extent());
        assert!(text.contains("VDD"));
        assert!(text.contains("12000"));
        assert!(text.contains('3'));
    }

    #[test]
    fn format_row_labels_shorts_and_opens_distinctly() {
        let short = TraceRow::Short(ShortRecord {
            net_a: "VDD".to_owned(),
            net_b: "GND".to_owned(),
            at: rect(1, 2, 3, 4),
        });
        let open = TraceRow::Open(OpenRecord {
            net: "CLK".to_owned(),
            pieces: 2,
        });
        let short_text = format_row(&short);
        assert!(short_text.starts_with("Short:"));
        assert!(short_text.contains("VDD"));
        assert!(short_text.contains("GND"));

        let open_text = format_row(&open);
        assert!(open_text.starts_with("Open:"));
        assert!(open_text.contains("CLK"));
        assert!(open_text.contains('2'));
    }

    use reticle_geometry::{LayerId, Rect};
    use reticle_model::{Pin, ShapeKind};

    /// A document with two differently named pins (`NET_A`, `NET_B`) joined by
    /// one real shape spanning both pin regions: a genuine short between two
    /// intended-separate nets, derived entirely from the document's own pins
    /// (no fixture, no `IntentSpec` authored by the test beyond the pins
    /// themselves).
    fn shorted_pins_doc() -> Document {
        let layer = LayerId::new(3, 0);
        let mut cell = Cell::new("TOP");
        cell.pins.push(Pin::new(
            "NET_A",
            Rect::new(Point::new(0, 0), Point::new(10, 10)),
            layer,
        ));
        cell.pins.push(Pin::new(
            "NET_B",
            Rect::new(Point::new(20, 20), Point::new(30, 30)),
            layer,
        ));
        cell.shapes.push(DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(30, 30))),
        ));
        let mut doc = Document::new();
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["TOP".to_owned()]);
        doc
    }

    /// The success-bar test: given a document with a known short, the trace
    /// panel's live query renders the REAL `ShortsOpensReport` from
    /// `reticle_extract::query`, not the fixture.
    #[test]
    fn query_report_finds_a_real_short_from_the_documents_own_pins() {
        let doc = shorted_pins_doc();
        let report = query_report(&doc, "TOP", 42);
        assert_eq!(report.revision, 42);
        assert_eq!(
            report.shorts.len(),
            1,
            "the two touching, differently named pins must short"
        );
        assert_eq!(report.shorts[0].net_a, "NET_A");
        assert_eq!(report.shorts[0].net_b, "NET_B");
        assert!(report.opens.is_empty());
        assert_ne!(
            report,
            fixture_report(),
            "must be the real computed report, not the canned fixture"
        );
    }

    /// Two pins that never touch: a real, honest clean result, not the fixture
    /// (whose report is never clean).
    #[test]
    fn query_report_over_a_clean_document_is_real_not_the_fixture() {
        let layer = LayerId::new(3, 0);
        let mut cell = Cell::new("TOP");
        cell.pins.push(Pin::new(
            "NET_A",
            Rect::new(Point::new(0, 0), Point::new(10, 10)),
            layer,
        ));
        cell.pins.push(Pin::new(
            "NET_B",
            Rect::new(Point::new(1000, 1000), Point::new(1010, 1010)),
            layer,
        ));
        cell.shapes.push(DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(10, 10))),
        ));
        cell.shapes.push(DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(1000, 1000), Point::new(1010, 1010))),
        ));
        let mut doc = Document::new();
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["TOP".to_owned()]);

        let report = query_report(&doc, "TOP", 5);
        assert_eq!(report.revision, 5);
        assert!(report.is_clean());
    }

    #[test]
    fn query_report_unknown_top_cell_is_a_real_empty_report() {
        let doc = Document::new();
        let report = query_report(&doc, "MISSING", 1);
        assert!(report.is_clean());
        assert_eq!(report.revision, 1);
    }

    /// `query_at_point`/`query_extent` over a real document, proving both call
    /// into `reticle_extract::query`'s live functions rather than the fixture.
    #[test]
    fn query_at_point_and_extent_are_real_not_the_fixture() {
        let layer = LayerId::new(5, 0);
        let mut cell = Cell::new("TOP");
        cell.shapes.push(DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(100, 100))),
        ));
        let mut doc = Document::new();
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["TOP".to_owned()]);

        let at = query_at_point(&doc, "TOP", Point::new(50, 50), 9);
        assert_eq!(at.revision, 9);
        let net = at
            .net
            .clone()
            .expect("the point is covered by the one shape");
        assert_eq!(net.shape_indices, vec![0]);

        let extent =
            query_extent(&doc, "TOP", &net.name, 9).expect("the resolved net has an extent");
        assert_eq!(extent.shape_count, 1);
        assert_eq!(extent.bbox.max_x, 100);
        assert_ne!(
            at,
            fixture_at_point(),
            "must be the real query, not the fixture"
        );
    }

    #[test]
    fn query_extent_unknown_net_is_none() {
        let doc = shorted_pins_doc();
        assert!(query_extent(&doc, "TOP", "NO_SUCH_NET", 1).is_none());
    }

    #[test]
    fn shape_probe_point_is_the_bbox_center() {
        let shape = DrawShape::new(
            LayerId::new(1, 0),
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(10, 20))),
        );
        assert_eq!(shape_probe_point(&shape), Point::new(5, 10));
    }
}
