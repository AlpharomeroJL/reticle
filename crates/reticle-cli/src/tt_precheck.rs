//! Parse Tiny Tapeout's own precheck output into a structured, agent-consumable report.
//!
//! Tiny Tapeout's precheck (the `precheck` module of `TinyTapeout/tt-support-tools`) is
//! the authoritative gate a GDS-mode submission must clear. It runs Magic DRC and a set
//! of `KLayout` checks over the GDS plus structural checks (pins against the template,
//! the tile boundary, the layer whitelist, forbidden layers, and the top-cell name),
//! and it is Linux-native. Reticle runs it through a pinned Docker container wrapped as
//! `just tt-precheck <gds>` (see `scripts/tt-precheck.ps1`), which captures the
//! precheck's report files under a reports directory.
//!
//! This module is the Rust side: it turns those report files into a
//! [`PrecheckReport`] so the propose-verify-correct loop can consume a precheck failure
//! the same way it consumes a DRC [`Violation`](reticle_model::Violation). The parser is
//! deliberately dependency-light (standard library only) and does not need Docker, the
//! image, or the PDK to compile or to be unit-tested: it parses the precheck's committed
//! output format.
//!
//! # What it parses
//!
//! The precheck writes, into its reports directory:
//!
//! - `results.md`, a Markdown table `| Check | Result |` with one row per check, a green
//!   check for a pass and `❌ Fail: <message>` for a failure (the message is the
//!   `PrecheckFailure` exception string, for example
//!   `Top macro name mismatch: expected tt_um_x, got top`); and
//! - `magic_drc.txt`, the Magic DRC report: a cell-name header, then per error-type
//!   blocks separated by dashed rules, each block naming the rule and listing offending
//!   rectangles as four floating-point micron coordinates
//!   (`llx lly urx ury`), ending in an `[INFO]: COUNT: <n>` line.
//!
//! [`parse_results_md`] turns the table into one [`PrecheckFailure`] per failed row.
//! [`parse_magic_drc`] turns each Magic error block into one [`PrecheckFailure`] per
//! offending rectangle, carrying the rule name and the rectangle's location. Both feed
//! [`PrecheckReport`], whose [`feedback_lines`](PrecheckReport::feedback_lines) yields
//! the exact `Vec<String>` the agent loop folds into its model context, so precheck
//! failures reach the model on the next proposal just like DRC feedback does.

use std::path::Path;

use reticle_geometry::{Point, Rect};

/// A single structured precheck failure, modeled like a DRC
/// [`Violation`](reticle_model::Violation): the rule that failed, an optional layer, an
/// optional location, and a human-readable message the agent loop can act on.
///
/// Not every precheck failure is geometric. A structural failure (a wrong top-cell name,
/// a missing pin, a forbidden layer) has no rectangle, so [`location`](Self::location) is
/// `None`; a Magic DRC rectangle violation carries its bounding box. The `rule` is the
/// check or rule name (`Magic DRC`, `Pin Check`, `met1.1`, ...), and `message` is the
/// precheck's own wording, preserved verbatim so nothing is lost in translation.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PrecheckFailure {
    /// The check or DRC rule that failed (for example `Magic DRC`, `Boundary Check`,
    /// `Top Cell Name`, or a Magic rule name like `met1.2`).
    pub rule: String,
    /// The GDS layer the failure is on, `layer/datatype` as reported, when the precheck
    /// names one. Structural checks (top-cell name, pins) leave this `None`.
    pub layer: Option<String>,
    /// The bounding box of the offending geometry in database units, when the failure is
    /// geometric (a Magic DRC rectangle). `None` for a structural failure with no
    /// coordinates.
    pub location: Option<Rect>,
    /// The precheck's own message for this failure, preserved verbatim.
    pub message: String,
}

impl PrecheckFailure {
    /// A structural (non-geometric) failure: a rule name and a message, no layer or
    /// location.
    #[must_use]
    pub fn structural(rule: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            rule: rule.into(),
            layer: None,
            location: None,
            message: message.into(),
        }
    }

    /// A one-line rendering for a log or for model feedback: the rule, the message, and
    /// the location if there is one.
    #[must_use]
    pub fn summary(&self) -> String {
        match self.location {
            Some(r) => format!(
                "{}: {} at ({},{})..({},{})",
                self.rule, self.message, r.min.x, r.min.y, r.max.x, r.max.y
            ),
            None => format!("{}: {}", self.rule, self.message),
        }
    }
}

/// The parsed result of a Tiny Tapeout precheck run over one GDS.
///
/// A run either passed (every check green) or produced one or more [`PrecheckFailure`]s.
/// `passed` is the precheck's own verdict (its exit code / the absence of a failed row),
/// not merely `failures.is_empty()`, so a parse that finds no failures in a run the
/// precheck itself failed does not silently read as a pass.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PrecheckReport {
    /// Whether the precheck passed overall.
    pub passed: bool,
    /// Every parsed failure, in report order.
    pub failures: Vec<PrecheckFailure>,
}

impl PrecheckReport {
    /// A passing report with no failures.
    #[must_use]
    pub fn passed() -> Self {
        Self {
            passed: true,
            failures: Vec::new(),
        }
    }

    /// A failing report carrying the given failures.
    #[must_use]
    pub fn failed(failures: Vec<PrecheckFailure>) -> Self {
        Self {
            passed: false,
            failures,
        }
    }

    /// The failure summaries as the `Vec<String>` the agent loop folds into its model
    /// context.
    ///
    /// The propose-verify-correct loop drives correction off a `Vec<String>` of feedback
    /// lines (`reticle_bench::model::Context::feedback`), the same channel the DRC
    /// verifier uses. Handing the loop `report.feedback_lines()` therefore makes a
    /// precheck failure reach the model on the next proposal exactly as a DRC violation
    /// does, which is the point of the oracle: a tile can be generated, prechecked, and
    /// corrected in one loop. A passing report yields an empty vector (no feedback to
    /// give), so a caller can treat "empty feedback" as "nothing to fix".
    #[must_use]
    pub fn feedback_lines(&self) -> Vec<String> {
        self.failures.iter().map(PrecheckFailure::summary).collect()
    }
}

/// Parse the precheck's `results.md` Markdown summary into a [`PrecheckReport`].
///
/// The table is `| Check | Result |` with one row per check. A passing row's result cell
/// is a green check; a failing row's is `❌ Fail: <message>`. This reads each data row,
/// records a [`PrecheckFailure::structural`] for every failed row (the check name as the
/// rule, the text after `Fail:` as the message), and sets `passed` to true only when no
/// row failed and at least one real check row was seen (an empty or header-only table is
/// not a pass).
///
/// Structural precheck failures (top-cell name, pins, boundary, layers) surface only in
/// this table; the geometric detail of a Magic DRC failure is in `magic_drc.txt`, parsed
/// by [`parse_magic_drc`] and merged by [`parse_reports_dir`].
#[must_use]
pub fn parse_results_md(markdown: &str) -> PrecheckReport {
    let mut failures = Vec::new();
    let mut saw_check_row = false;

    for line in markdown.lines() {
        let trimmed = line.trim();
        // Table rows start and end with a pipe and have a middle pipe: `| a | b |`.
        if !trimmed.starts_with('|') || !trimmed.ends_with('|') {
            continue;
        }
        let cells: Vec<&str> = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        if cells.len() != 2 {
            continue;
        }
        let (name, result) = (cells[0], cells[1]);
        // Skip the header row and the `|---|---|` separator.
        if name.eq_ignore_ascii_case("check") || is_separator_cell(name) {
            continue;
        }
        saw_check_row = true;
        if let Some(message) = failure_message(result) {
            failures.push(PrecheckFailure::structural(name, message));
        }
    }

    let passed = saw_check_row && failures.is_empty();
    PrecheckReport { passed, failures }
}

/// True for a Markdown table separator cell like `---` or `:---:` (any run of dashes and
/// colons).
fn is_separator_cell(cell: &str) -> bool {
    !cell.is_empty() && cell.chars().all(|c| c == '-' || c == ':')
}

/// If `result_cell` denotes a failure (`❌ Fail: <message>` or a plain `Fail: <message>`
/// / `FAIL <message>`), return the message after the `Fail` marker; otherwise `None`.
///
/// The precheck writes `❌ Fail: <str(exception)>`. This tolerates the emoji being
/// absent (some terminals or captures strip it) and a missing colon, so a faithfully
/// captured or lightly reencoded table still parses.
fn failure_message(result_cell: &str) -> Option<String> {
    let lower = result_cell.to_ascii_lowercase();
    if !lower.contains("fail") {
        return None;
    }
    // Take everything after the first "fail" token, then strip a leading ':' and space.
    let idx = lower.find("fail")?;
    let after = &result_cell[idx + "fail".len()..];
    let message = after.trim_start_matches([':', ' ']).trim();
    Some(if message.is_empty() {
        "check failed".to_owned()
    } else {
        message.to_owned()
    })
}

/// Parse Magic's DRC report (`magic_drc.txt`) into one [`PrecheckFailure`] per offending
/// rectangle.
///
/// The report format (from `precheck/magic_drc.tcl`, which collects `drc listall why`)
/// is a cell-name header, then, for each violated rule, the rule's description line
/// followed by one or more rectangles, each `llx lly urx ury` in microns, blocks
/// separated by dashed rules (`----`), and a trailing `[INFO]: COUNT: <n>` summary line.
///
/// Coordinates are microns; they are converted to database units with `dbu_per_micron`
/// (SKY130 is 1000 DBU per micron) and rounded to the nearest integer, so a failure's
/// [`location`](PrecheckFailure::location) is a real [`Rect`] the editor can zoom to.
/// The `rule` is the rule description line verbatim; `message` restates it with the
/// measured rectangle. A report with a `COUNT: 0` (or no rectangles) yields no failures.
#[must_use]
pub fn parse_magic_drc(report: &str, dbu_per_micron: i64) -> Vec<PrecheckFailure> {
    let mut failures = Vec::new();
    let mut current_rule: Option<String> = None;

    for line in report.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || is_dashed_rule(trimmed) {
            continue;
        }
        // The summary line ends the useful content; nothing after COUNT is a violation.
        if trimmed.starts_with("[INFO]") || trimmed.to_ascii_uppercase().contains("COUNT:") {
            continue;
        }
        if let Some(rect) = parse_micron_rect(trimmed, dbu_per_micron) {
            // A coordinate line: attribute it to the rule named just above it.
            let rule = current_rule
                .clone()
                .unwrap_or_else(|| "Magic DRC".to_owned());
            let message = format!(
                "{rule}: violation rectangle {} {} {} {} um",
                fields(trimmed)[0],
                fields(trimmed)[1],
                fields(trimmed)[2],
                fields(trimmed)[3],
            );
            failures.push(PrecheckFailure {
                rule,
                layer: None,
                location: Some(rect),
                message,
            });
        } else {
            // A non-coordinate, non-summary line names the current rule.
            current_rule = Some(trimmed.to_owned());
        }
    }

    failures
}

/// The whitespace-separated fields of a line.
fn fields(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}

/// True for a line that is only dashes (a section separator), for example `----------`.
fn is_dashed_rule(line: &str) -> bool {
    line.len() >= 3 && line.chars().all(|c| c == '-')
}

/// Parse a Magic coordinate line (`llx lly urx ury`, four floats in microns) into a
/// [`Rect`] in database units, or `None` if the line is not exactly four numbers.
fn parse_micron_rect(line: &str, dbu_per_micron: i64) -> Option<Rect> {
    let parts = fields(line);
    if parts.len() != 4 {
        return None;
    }
    let mut coords = [0i32; 4];
    for (i, p) in parts.iter().enumerate() {
        let microns: f64 = p.parse().ok()?;
        // Round microns -> DBU at nearest integer; SKY130 uses 1000 DBU/micron.
        let dbu = (microns * dbu_per_micron as f64).round();
        // Keep within the i32 DBU grid; a tile is far inside this range.
        if !dbu.is_finite() || dbu.abs() > f64::from(i32::MAX) {
            return None;
        }
        coords[i] = dbu as i32;
    }
    Some(Rect::new(
        Point::new(coords[0], coords[1]),
        Point::new(coords[2], coords[3]),
    ))
}

/// The SKY130 database resolution: 1000 database units per micron. The precheck runs the
/// SKY130 PDK, so Magic's micron coordinates convert to DBU at this scale.
pub const SKY130_DBU_PER_MICRON: i64 = 1000;

/// Merge a precheck reports directory into one [`PrecheckReport`].
///
/// Reads `results.md` (required: it is the precheck's own verdict) and, when the Magic
/// DRC row failed, enriches that row with the per-rectangle detail from `magic_drc.txt`
/// so a Magic failure carries real locations rather than only the summary string. The
/// overall `passed` verdict is `results.md`'s.
///
/// # Errors
///
/// Returns a [`PrecheckParseError`] if `results.md` is absent or unreadable. A missing
/// `magic_drc.txt` is not an error (the run may have passed DRC, or not reached it); its
/// detail is simply omitted.
pub fn parse_reports_dir(dir: &Path) -> Result<PrecheckReport, PrecheckParseError> {
    let results_path = dir.join("results.md");
    let markdown = std::fs::read_to_string(&results_path).map_err(|source| {
        PrecheckParseError::MissingResults {
            path: results_path.clone(),
            source,
        }
    })?;
    let mut report = parse_results_md(&markdown);

    // If Magic DRC failed and a detailed report is present, replace the summary-only
    // Magic failure with the per-rectangle failures.
    let magic_failed = report
        .failures
        .iter()
        .any(|f| f.rule.eq_ignore_ascii_case("Magic DRC"));
    let magic_path = dir.join("magic_drc.txt");
    if magic_failed && let Ok(magic_text) = std::fs::read_to_string(&magic_path) {
        let detailed = parse_magic_drc(&magic_text, SKY130_DBU_PER_MICRON);
        if !detailed.is_empty() {
            report
                .failures
                .retain(|f| !f.rule.eq_ignore_ascii_case("Magic DRC"));
            report.failures.extend(detailed);
        }
    }

    Ok(report)
}

/// An error reading a precheck reports directory.
#[derive(Debug)]
pub enum PrecheckParseError {
    /// `results.md` was absent or unreadable.
    MissingResults {
        /// The path that was attempted.
        path: std::path::PathBuf,
        /// The underlying IO error.
        source: std::io::Error,
    },
}

impl std::fmt::Display for PrecheckParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingResults { path, source } => write!(
                f,
                "precheck results.md not found at {}: {source} \
                 (did the precheck run and write its reports directory?)",
                path.display()
            ),
        }
    }
}

impl std::error::Error for PrecheckParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::MissingResults { source, .. } => Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passing_results_md_parses_as_passed_with_no_failures() {
        let md = "\
# Tiny Tapeout Precheck Results

| Check | Result |
|-----------|--------|
| Magic DRC | ✅ |
| KLayout FEOL | ✅ |
| KLayout BEOL | ✅ |
| Boundary Check | ✅ |
| Layer Check | ✅ |
| Pin Check | ✅ |
| Top Cell Name | ✅ |
";
        let report = parse_results_md(md);
        assert!(report.passed, "all-green table is a pass");
        assert!(report.failures.is_empty());
        assert!(report.feedback_lines().is_empty());
    }

    #[test]
    fn failing_results_md_parses_the_structural_failure() {
        let md = "\
# Tiny Tapeout Precheck Results

| Check | Result |
|-----------|--------|
| Magic DRC | ✅ |
| Boundary Check | ✅ |
| Top Cell Name | ❌ Fail: Top macro name mismatch: expected tt_um_reticle_tile, got top |
| Pin Check | ✅ |
";
        let report = parse_results_md(md);
        assert!(!report.passed, "a failed row means the run did not pass");
        assert_eq!(report.failures.len(), 1);
        let f = &report.failures[0];
        assert_eq!(f.rule, "Top Cell Name");
        assert_eq!(
            f.message,
            "Top macro name mismatch: expected tt_um_reticle_tile, got top"
        );
        assert!(f.location.is_none(), "a name mismatch has no rectangle");
        // The failure reaches the loop as one feedback line.
        assert_eq!(report.feedback_lines().len(), 1);
        assert!(report.feedback_lines()[0].contains("Top macro name mismatch"));
    }

    #[test]
    fn empty_or_header_only_table_is_not_a_pass() {
        let header_only = "| Check | Result |\n|---|---|\n";
        let report = parse_results_md(header_only);
        assert!(
            !report.passed,
            "a header-only table has no verdict, so it is not a pass"
        );
        assert!(parse_results_md("").failures.is_empty());
        assert!(!parse_results_md("").passed);
    }

    #[test]
    fn failure_message_tolerates_missing_emoji_and_colon() {
        assert_eq!(
            failure_message("❌ Fail: something wrong").as_deref(),
            Some("something wrong")
        );
        assert_eq!(
            failure_message("Fail something wrong").as_deref(),
            Some("something wrong")
        );
        assert_eq!(failure_message("✅").as_deref(), None);
        assert_eq!(failure_message("Fail:").as_deref(), Some("check failed"));
    }

    #[test]
    fn magic_drc_report_parses_each_rectangle_with_a_location() {
        // Format per precheck/magic_drc.tcl: cell header, dashed rules, a rule line,
        // rectangles as four micron floats, and a COUNT summary.
        let report = "\
tt_um_reticle_tile
----------------------------------------
Metal1 spacing < 0.14um (met1.2)
----------------------------------------
 10.000 20.000 10.100 20.140
 30.500 40.000 30.600 40.140
----------------------------------------
[INFO]: COUNT: 2 (divide by 3 or 4 for the real count)
";
        let failures = parse_magic_drc(report, SKY130_DBU_PER_MICRON);
        assert_eq!(failures.len(), 2, "two rectangles, two failures");
        let first = &failures[0];
        assert_eq!(first.rule, "Metal1 spacing < 0.14um (met1.2)");
        let loc = first.location.expect("a Magic rectangle has a location");
        // 10.000 um -> 10000 DBU, 0.100 um -> 100 DBU wide.
        assert_eq!(loc.min, Point::new(10_000, 20_000));
        assert_eq!(loc.max, Point::new(10_100, 20_140));
        assert!(first.summary().contains("met1.2"));
    }

    #[test]
    fn magic_drc_zero_count_yields_no_failures() {
        let report = "\
tt_um_reticle_tile
----------------------------------------
[INFO]: COUNT: 0
";
        assert!(parse_magic_drc(report, SKY130_DBU_PER_MICRON).is_empty());
    }

    #[test]
    fn micron_rect_rejects_non_coordinate_lines() {
        assert!(parse_micron_rect("Metal1 spacing < 0.14um", SKY130_DBU_PER_MICRON).is_none());
        assert!(parse_micron_rect("10.0 20.0 30.0", SKY130_DBU_PER_MICRON).is_none());
        assert!(parse_micron_rect("10.0 20.0 30.0 40.0 50.0", SKY130_DBU_PER_MICRON).is_none());
    }

    #[test]
    fn report_constructors_and_feedback_round_trip() {
        assert!(PrecheckReport::passed().passed);
        let failing = PrecheckReport::failed(vec![PrecheckFailure::structural(
            "Pin Check",
            "ua[0] missing",
        )]);
        assert!(!failing.passed);
        assert_eq!(failing.feedback_lines(), vec!["Pin Check: ua[0] missing"]);
    }
}
