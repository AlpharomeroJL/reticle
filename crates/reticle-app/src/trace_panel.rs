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
//! # Fixture-first
//!
//! The trace-api lane's live query functions are not wired in until Gate 2. Until
//! then, [`fixture_at_point`], [`fixture_extent`], and [`fixture_report`] parse the
//! committed F3 contract fixture (`tests/fixtures/contracts/f3_trace.json` in
//! `reticle-extract`) as a stand-in data source, so the panel is fully demoable
//! now. Those three functions are exactly the seam the trace-api lane replaces
//! with real query calls; nothing else in this module or in `App::trace_section`
//! needs to change.

use reticle_extract::query::{NetAtPoint, NetExtent, OpenRecord, ShortRecord, ShortsOpensReport};

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
/// Every `load_*` method installs a fresh record (typically from
/// [`fixture_at_point`]/[`fixture_extent`]/[`fixture_report`] until Gate 2, a real
/// query result after); [`TracePanelState::load_report`] additionally resets the
/// navigator, matching how a fresh DRC run replaces the selection in
/// [`crate::drc_panel::DrcResults`].
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

/// Parses the embedded F3 fixture's `net_at_point` record: a stand-in for the
/// trace-api lane's live net-at-point query until Gate 2 (see the module docs).
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
}
