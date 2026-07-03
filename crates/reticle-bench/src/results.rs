//! Writing result records and rendering a summary table.
//!
//! [`write_records`] serializes a run's [`ResultRecord`]s to a JSON file under a
//! results directory. [`summarize`] rolls the records up into a [`Summary`] (overall
//! and per-tier success rate, mean iterations, and first-versus-final violations) and
//! [`Summary::to_markdown`] renders it as a Markdown report.
//!
//! The summary needs each record's tier, which the record does not carry, so the
//! caller pairs every record with the [`Tier`] it ran at.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::{ResultRecord, Tier};

/// A failure writing results to disk.
#[derive(Debug)]
pub enum WriteError {
    /// The results directory could not be created.
    CreateDir {
        /// The directory that could not be created.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The records could not be serialized to JSON.
    Serialize {
        /// The serializer's message.
        message: String,
    },
    /// The output file could not be written.
    Write {
        /// The file that could not be written.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::CreateDir { path, source } => {
                write!(f, "creating {}: {source}", path.display())
            }
            WriteError::Serialize { message } => write!(f, "serializing results: {message}"),
            WriteError::Write { path, source } => {
                write!(f, "writing {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for WriteError {}

/// Writes `records` as a pretty-printed JSON array to `dir/<file_name>`, creating the
/// directory if needed, and returns the path written.
///
/// # Errors
///
/// Returns a [`WriteError`] if the directory cannot be created, the records cannot be
/// serialized, or the file cannot be written.
pub fn write_records(
    dir: &Path,
    file_name: &str,
    records: &[ResultRecord],
) -> Result<PathBuf, WriteError> {
    std::fs::create_dir_all(dir).map_err(|source| WriteError::CreateDir {
        path: dir.to_path_buf(),
        source,
    })?;
    let json = serde_json::to_string_pretty(records).map_err(|e| WriteError::Serialize {
        message: e.to_string(),
    })?;
    let path = dir.join(file_name);
    std::fs::write(&path, json).map_err(|source| WriteError::Write {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

/// Aggregated statistics for one tier (or the whole run).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct TierStats {
    /// How many tasks were counted.
    pub total: u32,
    /// How many passed their check.
    pub passed: u32,
    /// Sum of iterations across the counted tasks (for the mean).
    pub iterations_sum: u64,
    /// Sum of first-proposal violations across the counted tasks.
    pub first_violations_sum: u64,
    /// Sum of final violations across the counted tasks.
    pub final_violations_sum: u64,
}

impl TierStats {
    /// Folds one record into the running totals.
    fn add(&mut self, record: &ResultRecord) {
        self.total += 1;
        if record.success {
            self.passed += 1;
        }
        self.iterations_sum += u64::from(record.iterations);
        self.first_violations_sum += u64::from(record.first_proposal_violations);
        self.final_violations_sum += u64::from(record.final_violations);
    }

    /// The pass fraction in `[0, 1]`, or `0.0` when no tasks were counted.
    #[must_use]
    pub fn success_rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            f64::from(self.passed) / f64::from(self.total)
        }
    }

    /// The mean iteration count, or `0.0` when no tasks were counted.
    #[must_use]
    pub fn mean_iterations(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.iterations_sum as f64 / f64::from(self.total)
        }
    }
}

/// A rolled-up view of a run: overall stats plus a breakdown by tier.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Summary {
    /// Statistics over every counted task.
    pub overall: TierStats,
    /// Statistics per tier, ordered by tier number.
    pub per_tier: BTreeMap<u8, TierStats>,
}

/// Rolls `records` (each paired with the [`Tier`] it ran at) into a [`Summary`].
///
/// The pairing is required because a [`ResultRecord`] does not carry its tier; the
/// caller supplies it from the originating task.
#[must_use]
pub fn summarize(records: &[(Tier, ResultRecord)]) -> Summary {
    let mut summary = Summary::default();
    for (tier, record) in records {
        summary.overall.add(record);
        summary.per_tier.entry(tier.0).or_default().add(record);
    }
    summary
}

impl Summary {
    /// Renders the summary as a Markdown report: a per-tier table plus an overall row,
    /// with a leading title.
    ///
    /// `title` names the run (for example the suite version) in the heading.
    #[must_use]
    pub fn to_markdown(&self, title: &str) -> String {
        let mut out = String::new();
        // Writing into a String is infallible; the trait method is in scope only for
        // `write!`, so the Result is discarded deliberately.
        let _ = write!(out, "# Benchmark results: {title}\n\n");
        out.push_str(
            "| Tier | Tasks | Passed | Success rate | Mean iterations | First violations | Final violations |\n",
        );
        out.push_str(
            "| ---- | ----- | ------ | ------------ | --------------- | ---------------- | ---------------- |\n",
        );
        for (tier, stats) in &self.per_tier {
            out.push_str(&row(&format!("{tier}"), *stats));
        }
        out.push_str(&row("all", self.overall));
        out
    }
}

/// One Markdown table row for a labeled set of stats.
fn row(label: &str, stats: TierStats) -> String {
    format!(
        "| {label} | {} | {} | {:.0}% | {:.2} | {} | {} |\n",
        stats.total,
        stats.passed,
        stats.success_rate() * 100.0,
        stats.mean_iterations(),
        stats.first_violations_sum,
        stats.final_violations_sum,
    )
}

#[cfg(test)]
mod tests {
    use super::{summarize, write_records};
    use crate::{ResultRecord, Tier};

    /// A result record with the given id, tier-agnostic fields set for testing.
    fn record(id: &str, success: bool, iterations: u32, first: u32, final_v: u32) -> ResultRecord {
        ResultRecord {
            task_id: id.into(),
            model: "mock".into(),
            suite_version: "0.1.0".into(),
            success,
            iterations,
            first_proposal_violations: first,
            final_violations: final_v,
            wall_ms: 3,
            backend: "mock".into(),
            quantization: None,
        }
    }

    #[test]
    fn summarize_computes_overall_and_per_tier() {
        let records = vec![
            (Tier(1), record("a", true, 1, 0, 0)),
            (Tier(1), record("b", false, 4, 2, 2)),
            (Tier(2), record("c", true, 2, 3, 0)),
        ];
        let summary = summarize(&records);
        assert_eq!(summary.overall.total, 3);
        assert_eq!(summary.overall.passed, 2);
        assert!((summary.overall.success_rate() - 2.0 / 3.0).abs() < 1e-9);
        // Tier 1: one of two passed.
        let t1 = summary.per_tier.get(&1).expect("tier 1");
        assert_eq!(t1.total, 2);
        assert_eq!(t1.passed, 1);
        assert!((t1.mean_iterations() - 2.5).abs() < 1e-9);
        // Tier 2: the single task passed.
        let t2 = summary.per_tier.get(&2).expect("tier 2");
        assert_eq!(t2.passed, 1);
        assert_eq!(t2.first_violations_sum, 3);
    }

    #[test]
    fn markdown_contains_rows_and_overall() {
        let records = vec![(Tier(1), record("a", true, 1, 0, 0))];
        let md = summarize(&records).to_markdown("suite 0.1.0");
        assert!(md.contains("# Benchmark results: suite 0.1.0"));
        assert!(md.contains("| 1 |"));
        assert!(md.contains("| all |"));
        assert!(md.contains("100%"));
    }

    #[test]
    fn write_records_round_trips_json() {
        let dir =
            std::env::temp_dir().join(format!("reticle-bench-results-{}", std::process::id()));
        let records = vec![record("a", true, 1, 0, 0)];
        let path = write_records(&dir, "run.json", &records).expect("write");
        let text = std::fs::read_to_string(&path).expect("read back");
        let back: Vec<ResultRecord> = serde_json::from_str(&text).expect("parse");
        assert_eq!(back, records);
    }

    #[test]
    fn empty_stats_are_zero_not_nan() {
        let summary = summarize(&[]);
        // Exactly zero (the empty-total branch returns a literal 0.0), asserted with an
        // epsilon to satisfy the float-comparison lint and to reject a NaN result.
        assert!(summary.overall.success_rate().abs() < 1e-9);
        assert!(summary.overall.mean_iterations().abs() < 1e-9);
    }
}
