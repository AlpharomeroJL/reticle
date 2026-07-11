//! The Inspector's Waveform section state: F4 `WaveformSet` records rendered as
//! a per-probe transient polyline or an operating-point readout (ADR 0110).
//!
//! Mirrors [`crate::trace_panel`]: no `egui` here at all, so the plotted-point
//! geometry, the probe list, and the display formatting are unit-tested without a
//! window. The thin egui glue that reads this state and paints the plot lives in
//! `App::waveform_section` (`crate::app`), styled entirely from
//! [`crate::theme::components`]/[`crate::theme::tokens`].
//!
//! # Fixture-first
//!
//! The bounded solver is a separate lane's Gate-3 deliverable, and which route it
//! ships (a vendored ngspice-WASM build, a pinned emscripten toolchain, or a
//! pure-Rust modified-nodal-analysis solver) is still an open decision the
//! `oracle-feasibility` lane makes. Until that solver lands, [`fixture_transient`]
//! parses the committed F4 contract fixture
//! (`tests/fixtures/contracts/f4_rc_transient.json` in `reticle-sim`) as the set
//! `waveform.run_oracle` loads; `App::waveform_section` shows an honest banner
//! naming this while it is true. [`fixture_transient`] is exactly the call the
//! live lane replaces; nothing else in this module or in `App::waveform_section`
//! needs to change.
//!
//! # Operating-point coverage
//!
//! The committed fixture is a transient sweep, so the operating-point rendering
//! path ([`operating_point_value`] and every `AnalysisKind::OperatingPoint` arm
//! below) is exercised by a synthetic single-sample set built in this module's
//! tests, mirroring the shape `reticle-sim`'s own
//! `f4_operating_point_shape_is_distinct` test constructs. A committed OP fixture
//! is left for a later phase (see `docs/decisions/0110-waveform-viewer.md`).

use reticle_sim::{AnalysisKind, Probe, Quantity, WaveformSet};

/// The committed F4 contract fixture this phase renders against (see the module
/// docs). Shared with `reticle-sim`'s own producer test (`tests/f4_waveform.rs`);
/// do not fork a second copy of this data.
const FIXTURE_JSON: &str =
    include_str!("../../reticle-sim/tests/fixtures/contracts/f4_rc_transient.json");

/// A point inside a plotted waveform trace, normalized to `0.0..=1.0` on both
/// axes: `x` is fractional time (the set's `t_min_fs`..`t_max_fs`), `y` is
/// fractional value (`y_min_nano`..`y_max_nano`), independent of the panel's
/// pixel size. The thin egui glue in `App::waveform_section` maps these into the
/// plot's screen rect; this module never touches a pixel.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct NormPoint {
    /// Fractional position along the time axis, `0.0` at `t_min_fs`.
    pub x: f32,
    /// Fractional position along the value axis, `0.0` at `y_min_nano`.
    pub y: f32,
}

/// Maps `value` into `0.0..=1.0` across `[min, max]`. A degenerate or inverted
/// range (no positive span: a flat probe, or the single instant an operating
/// point's bounds carry) returns `0.5` (centered) rather than dividing by zero, so
/// a set with no spread never produces `NaN`/`inf`.
fn normalize(value: i64, min: i64, max: i64) -> f32 {
    let span = max - min;
    if span <= 0 {
        return 0.5;
    }
    ((value - min) as f64 / span as f64) as f32
}

/// The selected probe's plotted polyline for an `AnalysisKind::Transient` set: one
/// [`NormPoint`] per `time_fs` sample, scaled by the set's `Bounds`. Empty for an
/// `AnalysisKind::OperatingPoint` set (see [`operating_point_value`] instead), an
/// out-of-range `probe_index`, or a set with no samples.
#[must_use]
pub fn transient_trace(set: &WaveformSet, probe_index: usize) -> Vec<NormPoint> {
    if set.analysis != AnalysisKind::Transient {
        return Vec::new();
    }
    let Some(probe) = set.probes.get(probe_index) else {
        return Vec::new();
    };
    let b = set.bounds;
    set.time_fs
        .iter()
        .zip(&probe.samples_nano)
        .map(|(&t, &v)| NormPoint {
            x: normalize(t, b.t_min_fs, b.t_max_fs),
            y: normalize(v, b.y_min_nano, b.y_max_nano),
        })
        .collect()
}

/// One probe's value in an `AnalysisKind::OperatingPoint` set, as a [`NormPoint`]
/// scaled by the set's `Bounds` on the value axis. There being no time span, every
/// probe's `x` is spread evenly across the plot by its index (`probe_index` of
/// `probes.len()`) instead of collapsing onto one column. `None` for an
/// `AnalysisKind::Transient` set, an out-of-range `probe_index`, or a probe with
/// no recorded sample.
#[must_use]
pub fn operating_point_value(set: &WaveformSet, probe_index: usize) -> Option<NormPoint> {
    if set.analysis != AnalysisKind::OperatingPoint {
        return None;
    }
    let probe = set.probes.get(probe_index)?;
    let sample = *probe.samples_nano.first()?;
    let b = set.bounds;
    let count = set.probes.len().max(1) as f32;
    Some(NormPoint {
        x: (probe_index as f32 + 0.5) / count,
        y: normalize(sample, b.y_min_nano, b.y_max_nano),
    })
}

/// The unit suffix for a display value of `quantity` (voltage -> `"V"`, and so on).
#[must_use]
pub fn quantity_unit(quantity: Quantity) -> &'static str {
    match quantity {
        Quantity::Voltage => "V",
        Quantity::Current => "A",
        Quantity::Charge => "C",
    }
}

/// The full axis-label word for `quantity` (`"Voltage"`, `"Current"`, `"Charge"`).
#[must_use]
pub fn quantity_label(quantity: Quantity) -> &'static str {
    match quantity {
        Quantity::Voltage => "Voltage",
        Quantity::Current => "Current",
        Quantity::Charge => "Charge",
    }
}

/// The value-axis label for `quantity` (`"Voltage (V)"`).
#[must_use]
pub fn quantity_axis_label(quantity: Quantity) -> String {
    format!("{} ({})", quantity_label(quantity), quantity_unit(quantity))
}

/// Formats a nano-scaled sample as a display value in its base unit: divides by
/// `1e9` (the F4 contract's display convention; see `reticle_sim::waveform`) and
/// appends the quantity's unit.
#[must_use]
pub fn format_sample(sample_nano: i64, quantity: Quantity) -> String {
    let value = sample_nano as f64 / 1.0e9;
    format!("{value:.6} {}", quantity_unit(quantity))
}

/// Formats a femtosecond time as nanoseconds (divides by `1e6`; `1 ns == 1e6 fs`).
#[must_use]
pub fn format_time_ns(t_fs: i64) -> String {
    format!("{:.3} ns", t_fs as f64 / 1.0e6)
}

/// The probe list's row label: its id, the netlist node it follows, and its
/// quantity.
#[must_use]
pub fn probe_row(probe: &Probe) -> String {
    format!(
        "{}  ({})  {}",
        probe.id,
        probe.node,
        quantity_label(probe.quantity)
    )
}

/// One operating-point readout line: the probe's id, node, and formatted value
/// (see [`format_sample`]). `0` when the probe carries no sample (defensive; a
/// well-formed operating point always has exactly one).
#[must_use]
pub fn operating_point_row(probe: &Probe) -> String {
    let value = probe.samples_nano.first().copied().unwrap_or(0);
    format!(
        "{}  ({})  {}",
        probe.id,
        probe.node,
        format_sample(value, probe.quantity)
    )
}

/// Dumps `set` to CSV: a header row (`time_fs`, then one `id_n<unit>` column per
/// probe, e.g. `out_nV` for a voltage probe named `out`), and one data row per
/// time sample. An operating point (empty `time_fs`) writes a single row at
/// `time_fs` `0` from each probe's lone sample. Every value is the raw integer
/// nano/femto-unit sample, never the display-divided float, so the export
/// round-trips the recorded values exactly (the F4 contract's own
/// byte-stability rationale; see `docs/decisions/0104-f4-waveform-record-contract.md`).
/// A probe shorter than the time axis (a shape
/// `WaveformSet::is_well_formed` never allows) leaves that cell blank rather than
/// panicking.
#[must_use]
pub fn to_csv(set: &WaveformSet) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("time_fs");
    for probe in &set.probes {
        let _ = write!(out, ",{}_n{}", probe.id, quantity_unit(probe.quantity));
    }
    out.push('\n');

    let rows = if set.time_fs.is_empty() {
        // An operating point has no time axis: one row from each probe's lone
        // sample, or no rows at all for an empty (unloaded-shape) set.
        usize::from(!set.probes.is_empty())
    } else {
        set.time_fs.len()
    };
    for row in 0..rows {
        let t = set.time_fs.get(row).copied().unwrap_or(0);
        let _ = write!(out, "{t}");
        for probe in &set.probes {
            out.push(',');
            if let Some(v) = probe.samples_nano.get(row) {
                let _ = write!(out, "{v}");
            }
        }
        out.push('\n');
    }
    out
}

/// Parses the embedded F4 fixture into the `WaveformSet` `waveform.run_oracle`
/// loads (see the module docs' Fixture-first section).
#[must_use]
pub fn fixture_transient() -> WaveformSet {
    serde_json::from_str(FIXTURE_JSON).expect("committed F4 fixture matches `WaveformSet`")
}

/// The Waveform Inspector section's state: the last-loaded `WaveformSet` and the
/// probe list's selection.
///
/// Every [`load`](Self::load) installs a fresh set (from [`fixture_transient`]
/// until the live solver lands), matching how
/// [`crate::trace_panel::TracePanelState`] stands in for the trace-api lane's live
/// queries.
#[derive(Clone, Debug, Default)]
pub struct WaveformPanelState {
    /// The last-loaded waveform set, if `waveform.run_oracle` has run.
    set: Option<WaveformSet>,
    /// The probe list's selected row index (drives which probe's transient trace
    /// is plotted; an operating point shows every probe regardless).
    selected: usize,
}

impl WaveformPanelState {
    /// An empty Waveform section: nothing loaded yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether no set has been loaded yet (the section renders its empty state).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.set.is_none()
    }

    /// The last-loaded set, if any.
    #[must_use]
    pub fn set(&self) -> Option<&WaveformSet> {
        self.set.as_ref()
    }

    /// The last-loaded set's analysis kind, if any.
    #[must_use]
    pub fn analysis(&self) -> Option<AnalysisKind> {
        self.set.as_ref().map(|s| s.analysis)
    }

    /// Whether the loaded set is a transient sweep (plots a polyline). `false`
    /// for an operating point (see [`Self::operating_point_points`]) and when
    /// nothing is loaded.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        self.analysis() == Some(AnalysisKind::Transient)
    }

    /// Installs a fresh set, selecting its first probe (`0`, whether or not it has
    /// one).
    pub fn load(&mut self, set: WaveformSet) {
        self.selected = 0;
        self.set = Some(set);
    }

    /// The number of probes in the current set (`0` if none is loaded).
    #[must_use]
    pub fn probe_count(&self) -> usize {
        self.set.as_ref().map_or(0, |s| s.probes.len())
    }

    /// The probe list's selected row index.
    #[must_use]
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Selects probe `index` directly (a list click). Returns `false` (leaving the
    /// selection unchanged) if `index` is out of range or no set is loaded.
    pub fn select(&mut self, index: usize) -> bool {
        if index < self.probe_count() {
            self.selected = index;
            true
        } else {
            false
        }
    }

    /// The selected probe, if any is loaded.
    #[must_use]
    pub fn selected_probe(&self) -> Option<&Probe> {
        self.set.as_ref().and_then(|s| s.probes.get(self.selected))
    }

    /// The probe list's row labels, in the set's stable order (see [`probe_row`]).
    #[must_use]
    pub fn probe_rows(&self) -> Vec<String> {
        self.set
            .as_ref()
            .map(|s| s.probes.iter().map(probe_row).collect())
            .unwrap_or_default()
    }

    /// The selected probe's plotted polyline (see [`transient_trace`]); empty
    /// unless the loaded set is a transient.
    #[must_use]
    pub fn selected_trace(&self) -> Vec<NormPoint> {
        self.set
            .as_ref()
            .map(|s| transient_trace(s, self.selected))
            .unwrap_or_default()
    }

    /// Every probe's plotted point for an operating point (see
    /// [`operating_point_value`]), in probe order; empty unless the loaded set is
    /// an operating point.
    #[must_use]
    pub fn operating_point_points(&self) -> Vec<NormPoint> {
        let Some(set) = self.set.as_ref() else {
            return Vec::new();
        };
        (0..set.probes.len())
            .filter_map(|i| operating_point_value(set, i))
            .collect()
    }

    /// Every probe's operating-point readout line (see [`operating_point_row`]);
    /// empty unless the loaded set is an operating point.
    #[must_use]
    pub fn operating_point_rows(&self) -> Vec<String> {
        match &self.set {
            Some(s) if s.analysis == AnalysisKind::OperatingPoint => {
                s.probes.iter().map(operating_point_row).collect()
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_sim::Bounds;

    /// A synthetic operating-point set (the committed F4 fixture is transient-only;
    /// ledgered in `RESULT.md` and `docs/decisions/0110-waveform-viewer.md`),
    /// mirroring the shape `reticle-sim`'s own
    /// `f4_operating_point_shape_is_distinct` test builds.
    fn synthetic_operating_point() -> WaveformSet {
        WaveformSet {
            analysis: AnalysisKind::OperatingPoint,
            time_fs: vec![],
            probes: vec![
                Probe {
                    id: "vdd".to_owned(),
                    node: "n_vdd".to_owned(),
                    quantity: Quantity::Voltage,
                    samples_nano: vec![1_800_000_000],
                },
                Probe {
                    id: "ibias".to_owned(),
                    node: "n_bias".to_owned(),
                    quantity: Quantity::Current,
                    samples_nano: vec![250_000_000],
                },
            ],
            bounds: Bounds {
                t_min_fs: 0,
                t_max_fs: 0,
                y_min_nano: 250_000_000,
                y_max_nano: 1_800_000_000,
            },
        }
    }

    #[test]
    fn fixture_transient_parses_the_committed_f4_fixture_and_is_well_formed() {
        let set = fixture_transient();
        assert!(set.is_well_formed());
        assert_eq!(set.analysis, AnalysisKind::Transient);
        assert_eq!(set.probes.len(), 1);
        assert_eq!(set.probes[0].id, "out");
    }

    #[test]
    fn synthetic_operating_point_is_well_formed() {
        assert!(synthetic_operating_point().is_well_formed());
    }

    #[test]
    fn new_state_is_empty() {
        let state = WaveformPanelState::new();
        assert!(state.is_empty());
        assert_eq!(state.probe_count(), 0);
        assert_eq!(state.selected(), 0);
        assert!(state.selected_probe().is_none());
        assert!(state.analysis().is_none());
        assert!(!state.is_transient(), "nothing loaded is not a transient");
    }

    #[test]
    fn is_transient_distinguishes_the_two_analysis_kinds() {
        let mut state = WaveformPanelState::new();
        state.load(fixture_transient());
        assert!(state.is_transient());
        state.load(synthetic_operating_point());
        assert!(!state.is_transient());
    }

    #[test]
    fn loading_the_fixture_clears_is_empty_and_selects_the_first_probe() {
        let mut state = WaveformPanelState::new();
        state.load(fixture_transient());
        assert!(!state.is_empty());
        assert_eq!(state.probe_count(), 1);
        assert_eq!(state.selected(), 0);
        assert_eq!(state.analysis(), Some(AnalysisKind::Transient));
        assert_eq!(state.selected_probe().unwrap().id, "out");
        assert_eq!(state.selected_trace().len(), 5);
        assert!(state.operating_point_points().is_empty());
    }

    #[test]
    fn transient_trace_yields_one_point_per_time_sample_at_the_bounds_extremes() {
        let set = fixture_transient();
        let points = transient_trace(&set, 0);
        assert_eq!(points.len(), set.time_fs.len());
        // First sample sits at the bounds' minimum time and value (t=0, V=0).
        assert!((points[0].x - 0.0).abs() < 1e-6);
        assert!((points[0].y - 0.0).abs() < 1e-6);
        // Last sample sits at the bounds' maximum on both axes (the RC transient's
        // highest recorded time and value coincide, per the fixture).
        let last = *points.last().unwrap();
        assert!((last.x - 1.0).abs() < 1e-6);
        assert!((last.y - 1.0).abs() < 1e-6);
        // Points never run backward in time.
        for pair in points.windows(2) {
            assert!(pair[1].x > pair[0].x);
        }
    }

    #[test]
    fn transient_trace_is_empty_for_an_operating_point_set() {
        let op = synthetic_operating_point();
        assert!(transient_trace(&op, 0).is_empty());
    }

    #[test]
    fn transient_trace_is_empty_for_an_out_of_range_probe() {
        let set = fixture_transient();
        assert!(transient_trace(&set, 99).is_empty());
    }

    #[test]
    fn operating_point_value_spreads_probes_and_scales_within_the_shared_bounds() {
        let op = synthetic_operating_point();
        let vdd = operating_point_value(&op, 0).expect("probe 0");
        assert!((vdd.x - 0.25).abs() < 1e-6, "probe 0 of 2 sits at x=0.25");
        assert!((vdd.y - 1.0).abs() < 1e-6, "vdd sits at the bounds max");
        let ibias = operating_point_value(&op, 1).expect("probe 1");
        assert!((ibias.x - 0.75).abs() < 1e-6, "probe 1 of 2 sits at x=0.75");
        assert!((ibias.y - 0.0).abs() < 1e-6, "ibias sits at the bounds min");
    }

    #[test]
    fn operating_point_value_is_none_for_a_transient_set() {
        let set = fixture_transient();
        assert!(operating_point_value(&set, 0).is_none());
    }

    #[test]
    fn normalize_centers_a_degenerate_or_inverted_range_instead_of_dividing_by_zero() {
        assert!((normalize(5, 5, 5) - 0.5).abs() < f32::EPSILON);
        assert!((normalize(5, 10, 5) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn state_select_rejects_out_of_range_and_keeps_the_prior_selection() {
        let mut state = WaveformPanelState::new();
        state.load(synthetic_operating_point());
        assert!(state.select(1));
        assert_eq!(state.selected(), 1);
        assert!(!state.select(99));
        assert_eq!(state.selected(), 1, "prior selection is unchanged");
    }

    #[test]
    fn a_fresh_load_resets_a_prior_selection() {
        let mut state = WaveformPanelState::new();
        state.load(synthetic_operating_point());
        state.select(1);
        assert_eq!(state.selected(), 1);
        state.load(fixture_transient());
        assert_eq!(state.selected(), 0, "a fresh load starts at row 0");
    }

    #[test]
    fn probe_rows_label_id_node_and_quantity() {
        let mut state = WaveformPanelState::new();
        state.load(synthetic_operating_point());
        let rows = state.probe_rows();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].contains("vdd"));
        assert!(rows[0].contains("n_vdd"));
        assert!(rows[0].contains("Voltage"));
    }

    #[test]
    fn operating_point_rows_report_the_divided_value_and_unit() {
        let mut state = WaveformPanelState::new();
        state.load(synthetic_operating_point());
        let rows = state.operating_point_rows();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].contains("1.800000 V"));
        assert!(rows[1].contains("0.250000 A"));
        assert_eq!(state.operating_point_points().len(), 2);
    }

    #[test]
    fn operating_point_rows_are_empty_for_a_transient_set() {
        let mut state = WaveformPanelState::new();
        state.load(fixture_transient());
        assert!(state.operating_point_rows().is_empty());
    }

    #[test]
    fn format_sample_divides_by_1e9_and_appends_the_unit() {
        let text = format_sample(1_800_000_000, Quantity::Voltage);
        assert!(text.starts_with("1.8"));
        assert!(text.ends_with('V'));
    }

    #[test]
    fn format_time_ns_divides_by_1e6() {
        assert_eq!(format_time_ns(3_000_000), "3.000 ns");
    }

    #[test]
    fn quantity_axis_label_names_the_quantity_and_its_unit() {
        assert_eq!(quantity_axis_label(Quantity::Current), "Current (A)");
    }

    #[test]
    fn to_csv_round_trips_the_integer_sample_values() {
        let set = fixture_transient();
        let csv = to_csv(&set);
        let mut lines = csv.lines();
        let header = lines.next().unwrap();
        assert_eq!(header, "time_fs,out_nV");
        let rows: Vec<&str> = lines.collect();
        assert_eq!(rows.len(), set.time_fs.len());
        for (i, row) in rows.iter().enumerate() {
            let mut cols = row.split(',');
            let t: i64 = cols.next().unwrap().parse().unwrap();
            let v: i64 = cols.next().unwrap().parse().unwrap();
            assert_eq!(t, set.time_fs[i]);
            assert_eq!(v, set.probes[0].samples_nano[i]);
            assert!(cols.next().is_none(), "exactly one data column");
        }
    }

    #[test]
    fn to_csv_writes_a_single_row_for_an_operating_point_set() {
        let op = synthetic_operating_point();
        let csv = to_csv(&op);
        let mut lines = csv.lines();
        assert_eq!(lines.next().unwrap(), "time_fs,vdd_nV,ibias_nA");
        let row = lines.next().unwrap();
        assert_eq!(row, "0,1800000000,250000000");
        assert!(lines.next().is_none(), "exactly one OP row");
    }
}
