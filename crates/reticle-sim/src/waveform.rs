//! The F4 waveform-record contract: what a bounded simulation produces and the
//! waveform UI consumes.
//!
//! # Why integer-scaled
//!
//! Sample values are stored as `i64` in fixed nano-units (nanovolts, nanoamperes) and
//! time as `i64` femtoseconds, not `f64`. The project is integer-exact everywhere
//! (geometry in DBU); a contract fixture must be byte-stable and deterministic, and a
//! `f64` JSON round-trip is neither (formatting drifts across libraries and platforms).
//! The UI divides by `1e9` for display; the records themselves never carry a float, so a
//! recorded set hashes and diffs exactly.
//!
//! # Shape
//!
//! A [`WaveformSet`] carries the [`AnalysisKind`], a shared `time_fs` axis (femtoseconds;
//! empty for an operating point), a list of [`Probe`]s (each a node's series aligned to
//! the axis), and [`Bounds`] for axis scaling. A transient's per-probe `samples_nano` has
//! exactly one entry per `time_fs` point; an operating point has exactly one sample per
//! probe and an empty axis. [`WaveformSet::is_well_formed`] checks that invariant.

use serde::{Deserialize, Serialize};

/// The analysis a waveform set came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisKind {
    /// A time-domain transient sweep; the `time_fs` axis is populated and every probe
    /// has one sample per point.
    Transient,
    /// A single DC operating point; `time_fs` is empty and every probe has one sample.
    OperatingPoint,
}

/// The physical quantity a probe measures. Values are integer-scaled (see [`Probe`]), so
/// the quantity is metadata for the axis label, not a conversion the records depend on.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quantity {
    /// A node voltage; samples are in nanovolts.
    Voltage,
    /// A branch current; samples are in nanoamperes.
    Current,
    /// A node charge; samples are in nanocoulombs.
    Charge,
}

/// One recorded node: a stable id, the extracted netlist node it follows, its quantity,
/// and the per-sample series aligned to the set's `time_fs` axis.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Probe {
    /// Stable id used to key the series (unique within a set).
    pub id: String,
    /// The extracted netlist node this probe follows.
    pub node: String,
    /// What the samples measure.
    pub quantity: Quantity,
    /// The series in nano-units of `quantity` (nanovolts, nanoamperes, nanocoulombs),
    /// aligned to the set's `time_fs` axis. Integer-scaled so the record is exact and
    /// byte-stable; the UI divides by `1e9` for display.
    pub samples_nano: Vec<i64>,
}

/// Inclusive axis bounds over a set, for scaling: time in femtoseconds, value in the
/// probes' nano-units.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Bounds {
    /// Earliest time in the set (femtoseconds); `0` for an operating point.
    pub t_min_fs: i64,
    /// Latest time in the set (femtoseconds); `0` for an operating point.
    pub t_max_fs: i64,
    /// Minimum sample value across every probe (nano-units).
    pub y_min_nano: i64,
    /// Maximum sample value across every probe (nano-units).
    pub y_max_nano: i64,
}

/// A full waveform set: the analysis, the shared time axis, the probes, and the bounds.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct WaveformSet {
    /// Which analysis produced the set.
    pub analysis: AnalysisKind,
    /// The shared time axis in femtoseconds; empty for an operating point, otherwise
    /// strictly the axis every transient probe's `samples_nano` aligns to.
    pub time_fs: Vec<i64>,
    /// The recorded probes, in a stable order.
    pub probes: Vec<Probe>,
    /// Axis bounds for scaling.
    pub bounds: Bounds,
}

impl WaveformSet {
    /// Whether the set satisfies the contract invariant: a transient's axis is non-empty
    /// and every probe has exactly one sample per time point; an operating point has an
    /// empty axis and exactly one sample per probe. Empty of probes is not well formed.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        if self.probes.is_empty() {
            return false;
        }
        match self.analysis {
            AnalysisKind::Transient => {
                !self.time_fs.is_empty()
                    && self
                        .probes
                        .iter()
                        .all(|p| p.samples_nano.len() == self.time_fs.len())
            }
            AnalysisKind::OperatingPoint => {
                self.time_fs.is_empty() && self.probes.iter().all(|p| p.samples_nano.len() == 1)
            }
        }
    }
}
