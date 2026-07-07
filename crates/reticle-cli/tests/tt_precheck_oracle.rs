//! The precheck oracle, proven both ways against committed fixtures.
//!
//! These tests read the committed fixture reports directories under
//! `tests/fixtures/tt-precheck/{pass,fail}/` with [`parse_reports_dir`] and assert that
//! the parser turns a known-good precheck run into `passed = true` with no failures, and
//! a seeded-violation run into a failing [`PrecheckReport`] with a parsed, actionable
//! failure (rule, location, message) plus the agent-loop feedback lines that carry it.
//!
//! The fixtures are **synthesized** from Tiny Tapeout's real precheck output format (see
//! `tests/fixtures/tt-precheck/NOTICE.md` for provenance and why): the live precheck
//! needs the multi-GB `hpretl/iic-osic-tools` image and the SKY130 PDK on Linux, run via
//! `just tt-precheck <gds>`, which is the operator's step. Parsing that format is proven
//! here in the ordinary gate with no Docker and no PDK.

use std::path::PathBuf;

use reticle_cli::tt_precheck::{PrecheckReport, parse_reports_dir};

/// The absolute path to a fixture reports directory (`pass` or `fail`).
fn fixture_dir(which: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("tt-precheck");
    p.push(which);
    p
}

/// A known-good precheck run parses as a pass: overall `passed`, no failures, and no
/// feedback for the loop to act on.
#[test]
fn known_good_fixture_passes_clean() {
    let report = parse_reports_dir(&fixture_dir("pass")).expect("read pass fixture");
    assert!(report.passed, "the all-green fixture must parse as a pass");
    assert!(
        report.failures.is_empty(),
        "a passing run has no failures, got {:?}",
        report.failures
    );
    assert!(
        report.feedback_lines().is_empty(),
        "a pass gives the agent loop nothing to correct"
    );
}

/// A seeded-violation precheck run parses as a fail with an actionable, structured
/// report: the Magic DRC failure is enriched with per-rectangle locations from
/// `magic_drc.txt`, and the structural boundary failure carries its message.
#[test]
fn seeded_violation_fixture_fails_with_a_parsed_actionable_report() {
    let report = parse_reports_dir(&fixture_dir("fail")).expect("read fail fixture");
    assert!(
        !report.passed,
        "a seeded violation must not parse as a pass"
    );
    assert!(
        !report.failures.is_empty(),
        "a failing run must yield failures"
    );

    // The Magic DRC row was enriched: its three rectangles became three located
    // failures, each with a real bounding box the editor can zoom to.
    let magic_located: Vec<_> = report
        .failures
        .iter()
        .filter(|f| f.rule.contains("met1") && f.location.is_some())
        .collect();
    assert_eq!(
        magic_located.len(),
        3,
        "three seeded Magic rectangles, three located failures, got {:?}",
        report.failures
    );
    // 12.500 um at 1000 DBU/um -> 12500 DBU.
    let first = magic_located[0].location.expect("a located Magic failure");
    assert_eq!(first.min.x, 12_500);
    assert_eq!(first.min.y, 30_000);

    // The structural boundary failure carries its precheck message and no location.
    let boundary = report
        .failures
        .iter()
        .find(|f| f.rule == "Boundary Check")
        .expect("the boundary-check failure is present");
    assert_eq!(boundary.message, "Shapes outside project area");
    assert!(boundary.location.is_none());

    // The whole report reaches the agent loop as actionable feedback lines: one per
    // failure, each naming its rule and message, ready to fold into the model context.
    let feedback = report.feedback_lines();
    assert_eq!(feedback.len(), report.failures.len());
    assert!(
        feedback
            .iter()
            .any(|l| l.contains("Shapes outside project area")),
        "the boundary failure is in the feedback"
    );
    assert!(
        feedback.iter().any(|l| l.contains("met1.2")),
        "a Magic rule name is in the feedback"
    );
}

/// The agent-loop seam: `feedback_lines()` yields exactly the `Vec<String>` the
/// propose-verify-correct loop folds into its model context (`Context::feedback`), so a
/// precheck failure drives correction the same way a DRC violation does. This asserts the
/// shape and content of that seam without pulling in the agent crate.
#[test]
fn feedback_lines_are_the_loop_correction_channel() {
    let fail = parse_reports_dir(&fixture_dir("fail")).expect("read fail fixture");
    let lines: Vec<String> = fail.feedback_lines();
    // Every failure contributes one non-empty, rule-prefixed line.
    assert_eq!(lines.len(), fail.failures.len());
    for (line, failure) in lines.iter().zip(&fail.failures) {
        assert!(!line.is_empty());
        assert!(
            line.starts_with(&failure.rule),
            "a feedback line leads with its rule so the model knows which check to satisfy"
        );
    }

    // A passing report yields no feedback: the loop reads "empty feedback" as "clean".
    let pass = parse_reports_dir(&fixture_dir("pass")).expect("read pass fixture");
    assert!(pass.feedback_lines().is_empty());
}

/// A missing reports directory is a clear error, not a silent pass. This guards against
/// the failure mode where the precheck never ran (no reports written) being mistaken for
/// a clean run.
#[test]
fn missing_reports_dir_is_an_error_not_a_pass() {
    let mut nowhere = fixture_dir("pass");
    nowhere.push("does-not-exist");
    let err = parse_reports_dir(&nowhere).expect_err("a missing reports dir must error");
    assert!(err.to_string().contains("results.md"));
}

/// The report constructors and the default agree with the parsed fixtures: a defaulted
/// report is not a pass (nothing has been checked), and the pass constructor matches the
/// parsed pass fixture's verdict.
#[test]
fn default_report_is_not_a_pass() {
    assert!(!PrecheckReport::default().passed);
    let parsed = parse_reports_dir(&fixture_dir("pass")).expect("read pass fixture");
    assert_eq!(parsed.passed, PrecheckReport::passed().passed);
}
