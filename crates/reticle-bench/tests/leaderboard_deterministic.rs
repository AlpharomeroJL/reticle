//! Golden byte-stability test for the static leaderboard.
//!
//! The leaderboard is generated from committed result records and must be a pure
//! function of that record set: the same records always produce the same bytes, with a
//! stable ordering and no timestamps. This test pins that over a fixed fixture record
//! set (under `tests/fixtures/leaderboard/`) so a change that breaks determinism, or that
//! silently alters the rendered page, fails here rather than producing a book page that
//! churns on every regeneration.
//!
//! The fixture set is deliberately independent of the live `benchmarks/results/` tree
//! (which the bench workers write to concurrently), so this test's outcome never depends
//! on how many real records happen to be committed.

use std::path::{Path, PathBuf};

use reticle_bench::{ValidateError, load_records, render_leaderboard, validate_records};

/// The fixtures directory shipped alongside this test.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/leaderboard")
}

/// Reads the committed golden page, normalizing line endings so a Windows checkout that
/// rewrote the file to CRLF still compares equal to the render (which always emits LF).
fn golden() -> String {
    let text = std::fs::read_to_string(fixtures().join("golden.md")).expect("read golden.md");
    text.replace("\r\n", "\n")
}

#[test]
fn render_matches_the_committed_golden_page() {
    let records = load_records(&fixtures().join("records")).expect("load fixture records");
    let page = render_leaderboard(&records);
    assert_eq!(
        page,
        golden(),
        "the rendered leaderboard drifted from tests/fixtures/leaderboard/golden.md; if this \
         change is intended, regenerate it with \
         `cargo run -p reticle-bench -- leaderboard \
         --results crates/reticle-bench/tests/fixtures/leaderboard/records \
         --out crates/reticle-bench/tests/fixtures/leaderboard/golden.md`"
    );
}

#[test]
fn render_is_byte_stable_and_order_independent() {
    let mut records = load_records(&fixtures().join("records")).expect("load fixture records");
    let first = render_leaderboard(&records);
    let second = render_leaderboard(&records);
    assert_eq!(first, second, "the same records must render the same bytes");

    // Reversing the input must not change a byte: aggregation and ordering are total.
    records.reverse();
    let reversed = render_leaderboard(&records);
    assert_eq!(first, reversed, "the render must not depend on input order");

    // A rotation is another order the render must be invariant to.
    records.rotate_left(1);
    let rotated = render_leaderboard(&records);
    assert_eq!(first, rotated, "the render must not depend on input order");
}

#[test]
fn render_carries_no_timestamp() {
    let records = load_records(&fixtures().join("records")).expect("load fixture records");
    let page = render_leaderboard(&records);
    // A year-like token would betray a clock leaking into the output. The only digits in
    // the page are record counts, tier counts, and percentages, none of which look like a
    // year here.
    assert!(
        !page.contains("2025") && !page.contains("2026"),
        "the leaderboard must not embed a timestamp"
    );
}

#[test]
fn a_valid_record_set_builds() {
    let records =
        validate_records(&fixtures().join("records")).expect("the fixture record set is valid");
    assert_eq!(records.len(), 11, "every fixture record is accepted");
}

#[test]
fn a_record_with_no_tier_prefix_is_rejected_with_a_clear_message() {
    let err = validate_records(&fixtures().join("malformed/bad_tier.result.json"))
        .expect_err("a task id without a tier prefix is malformed");
    match err {
        ValidateError::Schema { reason, index, .. } => {
            assert_eq!(index, 0);
            assert!(
                reason.contains("tier prefix"),
                "the message should name the tier-prefix rule: {reason}"
            );
        }
        other => panic!("expected a schema rejection, got {other}"),
    }
}

#[test]
fn a_file_that_is_not_a_record_array_is_rejected_with_a_clear_message() {
    let err = validate_records(&fixtures().join("malformed/not_array.result.json"))
        .expect_err("a JSON object is not a record array");
    match err {
        ValidateError::Parse { message, .. } => {
            assert!(
                !message.is_empty(),
                "the parse error should carry the deserializer's message"
            );
        }
        other => panic!("expected a parse rejection, got {other}"),
    }
}
