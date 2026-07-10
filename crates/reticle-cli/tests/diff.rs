//! Integration tests for `reticle diff`: spawn the built binary and check both its
//! printed report and its process exit code, since the exit-code contract (zero
//! for identical layouts, non-zero for any difference) is what
//! `examples/diff-action` relies on to fail a PR check.
//!
//! This file also carries the (ignored by default) test that writes the committed
//! worked-example fixtures under `examples/diff-action/example/`, so the example
//! geometry and these tests share one definition.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use reticle_geometry::{LayerId, Point, Rect};
use reticle_io::Gds;
use reticle_model::{Cell, Document, DrawShape, Exporter, ShapeKind, Technology};

/// The layer the worked example and the ad hoc fixtures draw on: SKY130 met1
/// (layer 68, datatype 20), the same convention `examples/collab` uses.
const MET1: LayerId = LayerId::new(68, 20);

/// A process-unique counter so parallel tests never collide on a temp filename.
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A temp `.gds` path with a unique name.
fn temp_gds_path(stem: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let mut path = std::env::temp_dir();
    path.push(format!("reticle_cli_diff_it_{stem}_{pid}_{n}.gds"));
    path
}

/// An RAII guard that deletes a temp file when dropped, even on test failure.
struct TempFile(PathBuf);

impl TempFile {
    fn new(path: PathBuf) -> Self {
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
    Rect::new(Point::new(x0, y0), Point::new(x1, y1))
}

/// A single-cell document named `top`, carrying `rects` on [`MET1`].
fn doc_with_rects(rects: &[Rect]) -> Document {
    let mut cell = Cell::new("top");
    for r in rects {
        cell.shapes.push(DrawShape::new(MET1, ShapeKind::Rect(*r)));
    }
    let mut doc = Document::new();
    doc.insert_cell(cell);
    doc.set_top_cells(vec!["top".to_string()]);
    doc.set_technology(Technology {
        name: "diff_example".to_string(),
        dbu_per_micron: 1_000,
        layers: Vec::new(),
        rules: Vec::new(),
        stack: Vec::new(),
    });
    doc
}

/// The worked example's baseline: one 2 x 2 um rectangle.
fn example_before() -> Document {
    doc_with_rects(&[rect(0, 0, 2_000, 2_000)])
}

/// The worked example's update: the same rectangle plus a second, disjoint one on
/// the same layer, so the diff is exactly one added shape and nothing removed.
fn example_after() -> Document {
    doc_with_rects(&[rect(0, 0, 2_000, 2_000), rect(3_000, 0, 5_000, 2_000)])
}

fn write_gds(doc: &Document, path: &Path) {
    let bytes = Gds.export(doc).expect("export sample document to GDSII");
    std::fs::write(path, &bytes).expect("write GDSII file");
}

/// Runs the built `reticle` binary's `diff` subcommand and returns
/// `(exit_success, stdout)`.
fn run_diff(before: &Path, after: &Path) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .arg("diff")
        .arg(before)
        .arg(after)
        .output()
        .expect("run `reticle diff`");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

#[test]
fn identical_layouts_exit_zero() {
    let doc = example_before();
    let a = TempFile::new(temp_gds_path("identical_a"));
    let b = TempFile::new(temp_gds_path("identical_b"));
    write_gds(&doc, a.path());
    write_gds(&doc, b.path());

    let (success, stdout) = run_diff(a.path(), b.path());
    assert!(success, "identical layouts must exit zero:\n{stdout}");
    assert!(stdout.contains("added:   0"), "{stdout}");
    assert!(stdout.contains("removed: 0"), "{stdout}");
    assert!(stdout.contains("changed: 0"), "{stdout}");
    assert!(
        !stdout.contains("by layer:"),
        "an empty diff prints no per-layer section:\n{stdout}"
    );
}

#[test]
fn differing_layouts_exit_nonzero_and_report_counts() {
    let a = TempFile::new(temp_gds_path("differ_a"));
    let b = TempFile::new(temp_gds_path("differ_b"));
    write_gds(&example_before(), a.path());
    write_gds(&example_after(), b.path());

    let (success, stdout) = run_diff(a.path(), b.path());
    assert!(!success, "differing layouts must exit non-zero:\n{stdout}");
    assert!(stdout.contains("added:   1"), "{stdout}");
    assert!(stdout.contains("removed: 0"), "{stdout}");
    assert!(stdout.contains("changed: 0"), "{stdout}");
    assert!(
        stdout.contains(&format!("{}/{}", MET1.layer, MET1.datatype)),
        "the per-layer summary should name the affected layer:\n{stdout}"
    );
}

#[test]
fn missing_file_is_a_clean_error_not_a_panic() {
    let missing = PathBuf::from("this_file_does_not_exist_reticle_diff_fixture.gds");
    let output = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .arg("diff")
        .arg(&missing)
        .arg(&missing)
        .output()
        .expect("run `reticle diff` on a missing file");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error:"), "{stderr}");
}

/// Regenerates the committed worked-example fixtures under
/// `examples/diff-action/example/`. Not run by default (`cargo nextest run` skips
/// `#[ignore]`d tests); run it deliberately after changing the example geometry:
///
/// ```text
/// cargo test -p reticle-cli --test diff -- --ignored write_example_fixtures
/// ```
#[test]
#[ignore = "writes committed fixtures under examples/diff-action/example/; run deliberately"]
fn write_example_fixtures() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/diff-action/example");
    std::fs::create_dir_all(&dir).expect("create examples/diff-action/example/");
    write_gds(&example_before(), &dir.join("before.gds"));
    write_gds(&example_after(), &dir.join("after.gds"));
}
