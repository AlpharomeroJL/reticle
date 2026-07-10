//! `reticle diff <before> <after>`: shape-level differences between two layouts.
//!
//! The command loads both files with the same GDSII/OASIS importers `import` and
//! `export` use (see [`load_document`]), picks each file's top cell the same way
//! `drc`, `route`, and `extract` already do (see [`pick_top_cell`]), and hands the
//! flattened geometry (see [`flatten_top_cell`]) to the pure [`reticle_diff::diff`]
//! engine. The output is the added/removed/changed shape counts plus one line per
//! affected layer, in the CLI's usual `key: value` style.
//!
//! # Exit-code contract
//!
//! [`run`] exits `0` when the two layouts are geometrically identical and
//! non-zero when they differ in any way, so a CI step (see
//! `examples/diff-action`) can fail a pull request exactly when a layout change
//! was not intended.

use std::path::Path;
use std::process::ExitCode;

use reticle_cli::{Result, flatten_top_cell, load_document, pick_top_cell};
use reticle_diff::LayoutDiff;
use reticle_geometry::LayerId;

/// What comparing two layout files produced: the top cell compared on each side
/// plus the raw geometric diff between their flattened geometry.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DiffSummary {
    /// The top cell name compared in `before`.
    pub before_top: String,
    /// The top cell name compared in `after`.
    pub after_top: String,
    /// The geometric diff between the two flattened top cells.
    pub diff: LayoutDiff,
}

impl DiffSummary {
    /// `true` when the two layouts are geometrically identical: nothing added,
    /// removed, or changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.diff.is_empty()
    }
}

/// One layer's added/removed shape counts, produced by [`per_layer_counts`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LayerCounts {
    /// The layer/datatype pair.
    pub layer: LayerId,
    /// Shapes added on this layer.
    pub added: usize,
    /// Shapes removed on this layer.
    pub removed: usize,
}

/// Loads `before` and `after`, picks each file's top cell, and diffs their
/// flattened geometry with [`reticle_diff::diff`].
///
/// Flattening first (see [`flatten_top_cell`]) means the comparison sees real,
/// placed geometry regardless of how the two files structured their cell
/// hierarchy, and picking the top cell with [`pick_top_cell`] (rather than reading
/// each document's declared top cells directly) means a file that leaves its top
/// cells undeclared still compares the same way `drc`/`route`/`extract` treat it.
///
/// # Errors
///
/// Returns [`reticle_cli::CliError::Io`] / [`reticle_cli::CliError::Model`] if
/// either file cannot be read or parsed, and [`reticle_cli::CliError::NoTopCell`]
/// if either document has no cells at all.
pub fn compute(before: &Path, after: &Path) -> Result<DiffSummary> {
    let before_doc = load_document(before)?;
    let after_doc = load_document(after)?;

    let before_top = pick_top_cell(&before_doc)?;
    let after_top = pick_top_cell(&after_doc)?;

    let before_flat = flatten_top_cell(&before_doc, &before_top);
    let after_flat = flatten_top_cell(&after_doc, &after_top);

    let diff = reticle_diff::diff(&before_flat, &after_flat);

    Ok(DiffSummary {
        before_top,
        after_top,
        diff,
    })
}

/// Groups a [`LayoutDiff`]'s added/removed shapes by layer, sorted by layer then
/// datatype. `changed` is always empty in `reticle-diff` v1 (see that crate's
/// docs), so it never contributes a row here.
#[must_use]
pub fn per_layer_counts(diff: &LayoutDiff) -> Vec<LayerCounts> {
    let mut by_layer: std::collections::BTreeMap<LayerId, (usize, usize)> =
        std::collections::BTreeMap::new();
    for shape in &diff.added {
        by_layer.entry(shape.layer).or_default().0 += 1;
    }
    for shape in &diff.removed {
        by_layer.entry(shape.layer).or_default().1 += 1;
    }
    by_layer
        .into_iter()
        .map(|(layer, (added, removed))| LayerCounts {
            layer,
            added,
            removed,
        })
        .collect()
}

/// Prints a [`DiffSummary`] in the CLI's `key: value` style: the two files and
/// the top cell compared in each, the added/removed/changed counts, then one line
/// per affected layer (omitted when nothing differs).
fn print_summary(before: &Path, after: &Path, summary: &DiffSummary) {
    println!("before: {} (top: {})", before.display(), summary.before_top);
    println!("after:  {} (top: {})", after.display(), summary.after_top);
    println!("added:   {}", summary.diff.added_count());
    println!("removed: {}", summary.diff.removed_count());
    println!("changed: {}", summary.diff.changed_count());

    let layers = per_layer_counts(&summary.diff);
    if !layers.is_empty() {
        println!("by layer:");
        for l in &layers {
            println!(
                "  {}/{}: +{} -{}",
                l.layer.layer, l.layer.datatype, l.added, l.removed
            );
        }
    }
}

/// Handles `reticle diff`: compare two layout files and print their shape-level
/// differences.
///
/// # Errors
///
/// Propagates [`compute`]'s errors: an unreadable/unparseable file, or a document
/// with no cells to compare.
///
/// Returns [`ExitCode::SUCCESS`] when the two layouts are geometrically identical
/// and [`ExitCode::FAILURE`] when they differ (see the [module docs](self) for the
/// exit-code contract this gives `examples/diff-action`).
pub fn run(before: &Path, after: &Path) -> Result<ExitCode> {
    let summary = compute(before, after)?;
    print_summary(before, after, &summary);
    if summary.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use reticle_cli::CliError;
    use reticle_geometry::{Point, Rect};
    use reticle_io::Gds;
    use reticle_model::{Cell, Document, DrawShape, Exporter, ShapeKind, Technology};

    /// SKY130 met1 (layer 68, datatype 20), the same convention
    /// `examples/collab` uses.
    const MET1: LayerId = LayerId::new(68, 20);

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_gds_path(stem: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let mut path = std::env::temp_dir();
        path.push(format!("reticle_cli_diffmod_{stem}_{pid}_{n}.gds"));
        path
    }

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> Rect {
        Rect::new(Point::new(x0, y0), Point::new(x1, y1))
    }

    fn doc_with_rects(rects: &[Rect]) -> Document {
        let mut cell = Cell::new("top");
        for r in rects {
            cell.shapes.push(DrawShape::new(MET1, ShapeKind::Rect(*r)));
        }
        let mut doc = Document::new();
        doc.insert_cell(cell);
        doc.set_top_cells(vec!["top".to_string()]);
        doc.set_technology(Technology {
            name: "test_tech".to_string(),
            dbu_per_micron: 1_000,
            layers: Vec::new(),
            rules: Vec::new(),
            stack: Vec::new(),
        });
        doc
    }

    /// Writes `doc` to a fresh temp GDSII file and returns its path.
    fn export_temp(doc: &Document, stem: &str) -> PathBuf {
        let bytes = Gds.export(doc).expect("export sample document to GDSII");
        let path = temp_gds_path(stem);
        std::fs::write(&path, &bytes).expect("write temp GDSII file");
        path
    }

    #[test]
    fn compute_reports_empty_diff_for_identical_files() {
        let doc = doc_with_rects(&[rect(0, 0, 2_000, 2_000)]);
        let a = export_temp(&doc, "same_a");
        let b = export_temp(&doc, "same_b");

        let summary = compute(&a, &b).expect("diff identical files");
        assert!(summary.is_empty());
        assert_eq!(summary.before_top, "top");
        assert_eq!(summary.after_top, "top");
        assert!(per_layer_counts(&summary.diff).is_empty());

        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }

    #[test]
    fn compute_reports_one_added_shape() {
        let before = doc_with_rects(&[rect(0, 0, 2_000, 2_000)]);
        let after = doc_with_rects(&[rect(0, 0, 2_000, 2_000), rect(3_000, 0, 5_000, 2_000)]);
        let a = export_temp(&before, "add_before");
        let b = export_temp(&after, "add_after");

        let summary = compute(&a, &b).expect("diff differing files");
        assert!(!summary.is_empty());
        assert_eq!(summary.diff.added_count(), 1);
        assert_eq!(summary.diff.removed_count(), 0);
        assert_eq!(summary.diff.changed_count(), 0);

        let layers = per_layer_counts(&summary.diff);
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].layer, MET1);
        assert_eq!(layers[0].added, 1);
        assert_eq!(layers[0].removed, 0);

        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }

    #[test]
    fn compute_reports_removed_shape() {
        let before = doc_with_rects(&[rect(0, 0, 2_000, 2_000), rect(3_000, 0, 5_000, 2_000)]);
        let after = doc_with_rects(&[rect(0, 0, 2_000, 2_000)]);
        let a = export_temp(&before, "rm_before");
        let b = export_temp(&after, "rm_after");

        let summary = compute(&a, &b).expect("diff differing files");
        assert_eq!(summary.diff.added_count(), 0);
        assert_eq!(summary.diff.removed_count(), 1);

        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }

    #[test]
    fn compute_reports_missing_file() {
        let missing = PathBuf::from("this_file_does_not_exist_diffmod.gds");
        let err = compute(&missing, &missing).expect_err("missing file is an error");
        assert!(matches!(err, CliError::Io { .. }));
    }
}
