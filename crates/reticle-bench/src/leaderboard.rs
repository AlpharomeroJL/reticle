//! A deterministic static leaderboard rendered from committed [`ResultRecord`]s.
//!
//! This module reads the committed result records (it never runs the suite),
//! aggregates them per provenance triple (`backend`, `model`, `quantization`) with a
//! per-tier breakdown, and renders a Markdown book page. The render is a pure function
//! of the record set: the same records produce the same bytes, with no timestamps and a
//! stable total ordering, which is what the golden byte-stability test in
//! `tests/leaderboard_deterministic.rs` pins.
//!
//! The record format is the API: a contributor runs the suite, drops the resulting
//! `*.result.json` files under `benchmarks/results/`, and the same
//! [`ResultRecord`] fields this module reads are what a new row is
//! built from. [`validate_records`] rejects a malformed submission with a clear message,
//! so a bad record never reaches the page.
//!
//! ## Tier provenance
//!
//! A [`ResultRecord`] does not carry its tier, so this module derives it from the task
//! id, whose stable convention is a `t<N>_` prefix (`t1_place_met1_rect` is tier 1).
//! This keeps the leaderboard self-contained: it depends only on the committed records,
//! not on the task TOMLs (which are frozen and owned by the runner). A task id that does
//! not follow the convention is a malformed record, rejected by [`validate_records`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::ResultRecord;

/// The five documented difficulty tiers a leaderboard row is broken down across.
const TIERS: [u8; 5] = [1, 2, 3, 4, 5];

/// Derives a task's tier from its id, which by convention begins with `t<N>_`.
///
/// Returns `None` when the id does not follow the convention, which
/// [`validate_records`] treats as a malformed record.
#[must_use]
pub fn tier_of(task_id: &str) -> Option<u8> {
    let rest = task_id.strip_prefix('t')?;
    let end = rest.find('_')?;
    rest[..end].parse().ok()
}

/// The system class a backend represents, so a bare model driven through Reticle's own
/// loop is never conflated with an agent system that brings its own loop and scaffold.
///
/// The classification is deliberately explicit and additive: a new agent-system or
/// multi-agent backend is named here rather than defaulting to "bare model".
#[must_use]
pub fn system_kind(backend: &str) -> &'static str {
    match backend {
        // Claude Code brings its own planning and tool-calling loop; Reticle only drives
        // it and grades the result (ADR 0052), so it is an agent system, not a bare model.
        "claude-code" => "agent system",
        _ => "bare model",
    }
}

/// The provenance triple a leaderboard row aggregates over.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct GroupKey {
    backend: String,
    model: String,
    quantization: Option<String>,
}

/// Running pass/total counts for a tier (or a whole row).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct Counts {
    total: u32,
    passed: u32,
}

impl Counts {
    fn add(&mut self, success: bool) {
        self.total += 1;
        if success {
            self.passed += 1;
        }
    }

    /// The pass rate scaled to integer permille (parts per thousand), used as a
    /// float-free sort key so row ordering is a stable total order.
    fn rate_permille(self) -> u32 {
        if self.total == 0 {
            0
        } else {
            (u64::from(self.passed) * 1000 / u64::from(self.total)) as u32
        }
    }

    /// The pass rate as a whole-number percent for display, rounded to nearest (ties up)
    /// with integer math so the value is identical on every platform.
    fn percent(self) -> u32 {
        if self.total == 0 {
            0
        } else {
            let total = u64::from(self.total);
            ((u64::from(self.passed) * 100 + total / 2) / total) as u32
        }
    }
}

/// One aggregated leaderboard row: a provenance triple with its overall and per-tier
/// counts and the distinct suite versions it spans.
#[derive(Clone, Debug)]
struct Row {
    key: GroupKey,
    overall: Counts,
    per_tier: BTreeMap<u8, Counts>,
    suite_versions: BTreeSet<String>,
}

impl Row {
    fn new(key: GroupKey) -> Self {
        Self {
            key,
            overall: Counts::default(),
            per_tier: BTreeMap::new(),
            suite_versions: BTreeSet::new(),
        }
    }

    fn fold(&mut self, record: &ResultRecord, tier: u8) {
        self.overall.add(record.success);
        self.per_tier.entry(tier).or_default().add(record.success);
        if !record.suite_version.is_empty() {
            self.suite_versions.insert(record.suite_version.clone());
        }
    }

    /// A row is PARTIAL when it has no result in one or more of the five tiers, so it
    /// did not span the full difficulty range and its denominator is not comparable to a
    /// full-tier row. This is derived from the records alone (no task-manifest coupling).
    fn is_partial(&self) -> bool {
        TIERS.iter().any(|t| !self.per_tier.contains_key(t))
    }

    /// The stable sort key: highest pass rate first, then most tasks, then the
    /// provenance triple ascending. Every component is a total order, so the row order
    /// is deterministic with no float comparison.
    fn sort_key(
        &self,
    ) -> (
        std::cmp::Reverse<u32>,
        std::cmp::Reverse<u32>,
        String,
        String,
        String,
    ) {
        (
            std::cmp::Reverse(self.overall.rate_permille()),
            std::cmp::Reverse(self.overall.total),
            self.key.backend.clone(),
            self.key.model.clone(),
            self.key.quantization.clone().unwrap_or_default(),
        )
    }
}

/// Aggregates `records` into sorted leaderboard rows.
fn aggregate(records: &[ResultRecord]) -> Vec<Row> {
    let mut rows: BTreeMap<GroupKey, Row> = BTreeMap::new();
    for record in records {
        // A record whose id has no tier prefix is skipped here rather than guessed at;
        // `validate_records` is the gate that rejects such a record before it is
        // submitted, so a rendered page never contains one.
        let Some(tier) = tier_of(&record.task_id) else {
            continue;
        };
        let key = GroupKey {
            backend: record.backend.clone(),
            model: record.model.clone(),
            quantization: record.quantization.clone(),
        };
        rows.entry(key.clone())
            .or_insert_with(|| Row::new(key))
            .fold(record, tier);
    }
    let mut rows: Vec<Row> = rows.into_values().collect();
    rows.sort_by_key(Row::sort_key);
    rows
}

/// Renders the full leaderboard book page from `records`.
///
/// The output is a pure function of the record set: stable ordering, no timestamps, and
/// whole-number percentages, so the same records always produce the same bytes. This is
/// the function the golden byte-stability test pins.
#[must_use]
pub fn render_leaderboard(records: &[ResultRecord]) -> String {
    let rows = aggregate(records);
    let mut out = String::new();

    out.push_str("# Leaderboard\n\n");
    out.push_str(
        "This page is generated deterministically from the committed benchmark result \
         records under `benchmarks/results/`. It does not run the suite; it aggregates \
         the `*.result.json` records the runs already wrote. Regenerate it with \
         `cargo run -p reticle-bench -- leaderboard`. The record format is the API: to \
         add a row, run the suite and open a pull request with your records (see \
         [Submitting a run](submitting.md)).\n\n",
    );
    let _ = write!(
        out,
        "It aggregates **{}** committed result record(s) into **{}** row(s), one per \
         `backend` / `model` / `quantization` triple. The numbers are exactly what the \
         committed records say and grow as more runs are committed.\n\n",
        records.len(),
        rows.len(),
    );

    out.push_str("## How to read a row\n\n");
    out.push_str(
        "- **Kind** labels a row as a *bare model* (a model driven through Reticle's own \
         propose-verify-correct loop), an *agent system* (a system that brings its own \
         loop and scaffold, such as Claude Code), or a *multi-agent* system. A bare-model \
         row and an agent-system row measure different things and are **not** comparable \
         head to head (see the [methodology](benchmark.md)).\n",
    );
    out.push_str(
        "- **Quantization** is carried where the backend reports one (for example \
         `Q4_K_M` on a local GGUF model), so a small quantized local model is never \
         conflated with a full-precision or frontier one.\n",
    );
    out.push_str(
        "- **PARTIAL** marks a row that has no result in one or more tiers, so it did not \
         span the full difficulty range and its denominator is not comparable to a \
         full-tier row.\n",
    );
    out.push_str(
        "- Each **Tier** cell is `passed/total`, and **Overall** is `passed/total \
         (rate)` over every committed record for that row.\n\n",
    );

    if rows.is_empty() {
        out.push_str(
            "## Rankings\n\nNo committed result records yet. The table appears once a run \
             commits its records under `benchmarks/results/`.\n",
        );
        return out;
    }

    out.push_str("## Rankings\n\n");
    out.push_str(
        "| Kind | Model | Backend | Quantization | Suite | Tier 1 | Tier 2 | Tier 3 | Tier 4 | Tier 5 | Overall | |\n",
    );
    out.push_str(
        "| ---- | ----- | ------- | ------------ | ----- | -----: | -----: | -----: | -----: | -----: | ------: | -- |\n",
    );
    for row in &rows {
        out.push_str(&render_row(row));
    }

    out.push('\n');
    out.push_str(
        "The labeling rules above are the honest account preserved from the \
         [benchmark methodology](benchmark.md): a machinery baseline, a local model, and \
         an agent system are always distinguishable, and a partial run is never published \
         as a full-suite score.\n",
    );
    out
}

/// Renders one Markdown table row.
fn render_row(row: &Row) -> String {
    let quant = row.key.quantization.as_deref().unwrap_or("-");
    let suite = if row.suite_versions.is_empty() {
        "-".to_string()
    } else {
        row.suite_versions
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut cells = String::new();
    for tier in TIERS {
        match row.per_tier.get(&tier) {
            Some(c) => {
                let _ = write!(cells, " {}/{} |", c.passed, c.total);
            }
            None => cells.push_str(" - |"),
        }
    }
    let partial = if row.is_partial() { "PARTIAL" } else { "" };
    format!(
        "| {} | `{}` | {} | {} | {} |{} **{}/{} ({}%)** | {} |\n",
        system_kind(&row.key.backend),
        row.key.model,
        display_backend(&row.key.backend),
        quant,
        suite,
        cells,
        row.overall.passed,
        row.overall.total,
        row.overall.percent(),
        partial,
    )
}

/// A backend label for display, falling back to a dash for a record written before the
/// `backend` field existed (which deserializes as the empty string).
fn display_backend(backend: &str) -> &str {
    if backend.is_empty() { "-" } else { backend }
}

/// A malformed submitted record, reported with enough context to fix it.
#[derive(Debug)]
pub enum ValidateError {
    /// A results path could not be read or walked.
    Io {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// A file's JSON could not be parsed into a `Vec<ResultRecord>`.
    Parse {
        /// The offending file.
        path: PathBuf,
        /// The deserializer's message.
        message: String,
    },
    /// A record parsed but violates a schema rule the leaderboard relies on.
    Schema {
        /// The file the record came from.
        path: PathBuf,
        /// The record's index within the file's array.
        index: usize,
        /// What is wrong, in one clear sentence.
        reason: String,
    },
    /// No result records were found under the given path.
    Empty {
        /// The path that held no records.
        path: PathBuf,
    },
}

impl std::fmt::Display for ValidateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidateError::Io { path, source } => {
                write!(f, "reading {}: {source}", path.display())
            }
            ValidateError::Parse { path, message } => {
                write!(
                    f,
                    "{}: not a JSON array of result records: {message}",
                    path.display()
                )
            }
            ValidateError::Schema {
                path,
                index,
                reason,
            } => write!(f, "{}: record #{index}: {reason}", path.display()),
            ValidateError::Empty { path } => {
                write!(f, "{}: no result records found", path.display())
            }
        }
    }
}

impl std::error::Error for ValidateError {}

/// Validates one record against the schema rules the leaderboard relies on, returning a
/// clear reason on the first violation.
fn validate_one(record: &ResultRecord) -> Result<(), String> {
    if record.task_id.trim().is_empty() {
        return Err("`task_id` is empty".to_string());
    }
    if tier_of(&record.task_id).is_none() {
        return Err(format!(
            "`task_id` \"{}\" has no tier prefix (expected a `t<N>_` id, e.g. `t1_place_met1_rect`)",
            record.task_id
        ));
    }
    if record.model.trim().is_empty() {
        return Err("`model` is empty".to_string());
    }
    if record.suite_version.trim().is_empty() {
        return Err("`suite_version` is empty".to_string());
    }
    Ok(())
}

/// Reads and validates every `*.result.json` under `path` (a file or a directory),
/// returning the parsed records on success.
///
/// This is the submission gate: a valid record set is accepted and returned; a malformed
/// one is rejected with a [`ValidateError`] whose message names the file, the record
/// index, and the reason. It reuses the same [`ResultRecord`] shape the runner writes, so
/// the record format is the single source of truth.
///
/// # Errors
///
/// Returns a [`ValidateError`] if a path cannot be read, a file is not a JSON array of
/// records, a record violates a schema rule, or no records are found.
pub fn validate_records(path: &Path) -> Result<Vec<ResultRecord>, ValidateError> {
    let files = collect_result_files(path)?;
    let mut all = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).map_err(|source| ValidateError::Io {
            path: file.clone(),
            source,
        })?;
        let records: Vec<ResultRecord> =
            serde_json::from_str(&text).map_err(|e| ValidateError::Parse {
                path: file.clone(),
                message: e.to_string(),
            })?;
        for (index, record) in records.iter().enumerate() {
            validate_one(record).map_err(|reason| ValidateError::Schema {
                path: file.clone(),
                index,
                reason,
            })?;
        }
        all.extend(records);
    }
    if all.is_empty() {
        return Err(ValidateError::Empty {
            path: path.to_path_buf(),
        });
    }
    Ok(all)
}

/// Loads every result record under `dir` for rendering, without the semantic validation
/// [`validate_records`] adds. A file that is not a JSON array of records is an error; a
/// record with an unrecognized tier prefix is skipped during aggregation rather than
/// failing the whole render.
///
/// # Errors
///
/// Returns a [`ValidateError`] if a path cannot be read or a file is not a JSON array of
/// records.
pub fn load_records(dir: &Path) -> Result<Vec<ResultRecord>, ValidateError> {
    let files = collect_result_files(dir)?;
    let mut all = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).map_err(|source| ValidateError::Io {
            path: file.clone(),
            source,
        })?;
        let records: Vec<ResultRecord> =
            serde_json::from_str(&text).map_err(|e| ValidateError::Parse {
                path: file.clone(),
                message: e.to_string(),
            })?;
        all.extend(records);
    }
    Ok(all)
}

/// Collects the `*.result.json` files under `path` in a stable, sorted order. If `path`
/// is a single file it is returned as-is (so a contributor can validate one record file).
fn collect_result_files(path: &Path) -> Result<Vec<PathBuf>, ValidateError> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut files = Vec::new();
    walk(path, &mut files)?;
    // Sort so reads happen in a deterministic order regardless of directory iteration.
    files.sort();
    Ok(files)
}

/// Recursively collects `*.result.json` files under `dir`.
fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), ValidateError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // A missing directory yields no files rather than an error, so rendering an empty
        // results tree produces an empty leaderboard instead of a crash.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(ValidateError::Io {
                path: dir.to_path_buf(),
                source,
            });
        }
    };
    for entry in entries {
        let entry = entry.map_err(|source| ValidateError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out)?;
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".result.json"))
        {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(
        task_id: &str,
        model: &str,
        backend: &str,
        quant: Option<&str>,
        success: bool,
    ) -> ResultRecord {
        ResultRecord {
            task_id: task_id.into(),
            model: model.into(),
            suite_version: "0.4.0".into(),
            success,
            iterations: 1,
            first_proposal_violations: 0,
            final_violations: 0,
            wall_ms: 10,
            backend: backend.into(),
            quantization: quant.map(Into::into),
        }
    }

    #[test]
    fn tier_of_reads_the_prefix() {
        assert_eq!(tier_of("t1_place_met1_rect"), Some(1));
        assert_eq!(tier_of("t5_tap_cell_core"), Some(5));
        assert_eq!(tier_of("t12_wide"), Some(12));
        assert_eq!(tier_of("cand_foo"), None);
        assert_eq!(tier_of("place_rect"), None);
        assert_eq!(tier_of("t_foo"), None);
    }

    #[test]
    fn system_kind_separates_agent_systems_from_bare_models() {
        assert_eq!(system_kind("claude-code"), "agent system");
        assert_eq!(system_kind("ollama"), "bare model");
        assert_eq!(system_kind("mock"), "bare model");
        assert_eq!(system_kind(""), "bare model");
    }

    #[test]
    fn aggregate_groups_by_provenance_triple_and_tier() {
        let records = vec![
            rec("t1_a", "gpt-oss:16k", "ollama", Some("MXFP4"), true),
            rec("t1_b", "gpt-oss:16k", "ollama", Some("MXFP4"), false),
            rec("t2_c", "gpt-oss:16k", "ollama", Some("MXFP4"), true),
            // A different quantization is a different row.
            rec("t1_a", "qwen:16k", "ollama", Some("Q4_K_M"), true),
        ];
        let rows = aggregate(&records);
        assert_eq!(rows.len(), 2);
        let gpt = rows.iter().find(|r| r.key.model == "gpt-oss:16k").unwrap();
        assert_eq!(gpt.overall.total, 3);
        assert_eq!(gpt.overall.passed, 2);
        assert_eq!(
            gpt.per_tier[&1],
            Counts {
                total: 2,
                passed: 1
            }
        );
        assert_eq!(
            gpt.per_tier[&2],
            Counts {
                total: 1,
                passed: 1
            }
        );
    }

    #[test]
    fn rows_sort_by_pass_rate_then_total_then_key() {
        let records = vec![
            // 50% row, 2 tasks.
            rec("t1_a", "low", "ollama", None, true),
            rec("t1_b", "low", "ollama", None, false),
            // 100% row, 1 task -> ranks above the 50% row.
            rec("t1_a", "high", "ollama", None, true),
        ];
        let rows = aggregate(&records);
        assert_eq!(rows[0].key.model, "high");
        assert_eq!(rows[1].key.model, "low");
    }

    #[test]
    fn partial_row_is_flagged_when_a_tier_is_missing() {
        // Tiers 1..3 present, 4 and 5 absent -> PARTIAL.
        let records = vec![
            rec("t1_a", "cc", "claude-code", None, true),
            rec("t2_b", "cc", "claude-code", None, true),
            rec("t3_c", "cc", "claude-code", None, true),
        ];
        let rows = aggregate(&records);
        assert!(rows[0].is_partial());

        // All five tiers present -> not partial.
        let full: Vec<_> = TIERS
            .iter()
            .map(|t| rec(&format!("t{t}_x"), "m", "ollama", None, true))
            .collect();
        let rows = aggregate(&full);
        assert!(!rows[0].is_partial());
    }

    #[test]
    fn render_is_byte_identical_across_calls_and_input_order() {
        let mut records = vec![
            rec("t1_a", "gpt-oss:16k", "ollama", Some("MXFP4"), true),
            rec("t2_b", "gpt-oss:16k", "ollama", Some("MXFP4"), false),
            rec("t1_a", "claude-sonnet-5", "claude-code", None, true),
        ];
        let first = render_leaderboard(&records);
        let second = render_leaderboard(&records);
        assert_eq!(first, second, "same records must render the same bytes");
        records.reverse();
        let reversed = render_leaderboard(&records);
        assert_eq!(first, reversed, "render must be independent of input order");
        // No timestamp leaked into the output.
        assert!(
            !first.contains("202"),
            "no year-like timestamp in the output"
        );
    }

    #[test]
    fn render_marks_the_partial_row_and_labels_the_kinds() {
        let records = vec![
            rec("t1_a", "claude-sonnet-5", "claude-code", None, true),
            rec("t1_b", "gpt-oss:16k", "ollama", Some("MXFP4"), true),
        ];
        let md = render_leaderboard(&records);
        assert!(md.contains("agent system"));
        assert!(md.contains("bare model"));
        assert!(md.contains("MXFP4"));
        assert!(md.contains("PARTIAL"), "a single-tier row is partial");
    }

    #[test]
    fn empty_record_set_renders_a_placeholder_not_a_crash() {
        let md = render_leaderboard(&[]);
        assert!(md.contains("# Leaderboard"));
        assert!(md.contains("No committed result records yet"));
    }

    #[test]
    fn validate_one_rejects_malformed_records() {
        let mut bad = rec("t1_ok", "m", "ollama", None, true);
        assert!(validate_one(&bad).is_ok());

        bad.task_id = String::new();
        assert!(validate_one(&bad).unwrap_err().contains("task_id"));

        bad.task_id = "no_tier_prefix".into();
        assert!(validate_one(&bad).unwrap_err().contains("tier prefix"));

        let mut bad = rec("t1_ok", "", "ollama", None, true);
        assert!(validate_one(&bad).unwrap_err().contains("model"));

        bad.model = "m".into();
        bad.suite_version = String::new();
        assert!(validate_one(&bad).unwrap_err().contains("suite_version"));
    }
}
