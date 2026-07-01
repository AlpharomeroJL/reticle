//! Integration tests for the `reticle-cli` pipeline library.
//!
//! Each test builds a small [`Document`] in process, exports it to a temporary
//! GDSII file, then drives the public pipeline functions on the reloaded document:
//! import summary, DRC (with a rule that flags a thin shape), routing, extraction,
//! export round-trips, and, guarded on a GPU being present, an offscreen render.
//! Temporary files live under [`std::env::temp_dir`] and are removed on the way
//! out.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use reticle_cli::{
    Format, RenderOutcome, default_rules, framing_camera, load_document, pick_top_cell,
    resolve_rules, run_drc, run_export, run_extract, run_render, run_route, summarize,
    synth_route_request,
};
use reticle_geometry::{LayerId, Point, Rect};
use reticle_io::Gds;
use reticle_model::{Cell, Document, DrawShape, Exporter, RuleKind, ShapeKind, Technology};
use reticle_render::WgpuContext;

/// The metal layer every test shape lives on.
const METAL: LayerId = LayerId::new(1, 0);

/// A process-unique counter so parallel tests never collide on a temp filename.
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A temp path with a unique name and the given extension. Removed by [`TempFile`].
fn temp_path(stem: &str, ext: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let mut path = std::env::temp_dir();
    path.push(format!("reticle_cli_{stem}_{pid}_{n}.{ext}"));
    path
}

/// An RAII guard that deletes a temp file when dropped, even on test failure.
struct TempFile(PathBuf);

impl TempFile {
    /// Wraps a path, taking ownership of its cleanup.
    fn new(path: PathBuf) -> Self {
        Self(path)
    }

    /// The wrapped path.
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A rectangle draw-shape on the metal layer.
fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
    DrawShape::new(
        METAL,
        ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
    )
}

/// Builds a small document whose top cell `top` carries only rectangles (so it
/// round-trips through both GDSII and the OASIS subset).
///
/// The geometry is chosen so that:
/// * a deliberately thin `10 x 200` rectangle trips a min-width DRC rule,
/// * three well-separated rectangles form three disjoint nets for extraction,
/// * those separations give the router real terminals to connect.
fn sample_document() -> Document {
    let mut cell = Cell::new("top");
    // A thin bar: 10 DBU wide, well under the 100 DBU default width rule.
    cell.shapes.push(rect(0, 0, 10, 200));
    // Two fat, separated squares: distinct nets, spaced apart from the bar.
    cell.shapes.push(rect(1_000, 0, 1_300, 300));
    cell.shapes.push(rect(5_000, 0, 5_300, 300));

    let mut doc = Document::new();
    doc.insert_cell(cell);
    doc.set_top_cells(vec!["top".to_string()]);

    // A technology with a positive resolution so GDSII export writes sane units.
    let tech = Technology {
        name: "test_tech".to_string(),
        dbu_per_micron: 1_000,
        layers: Vec::new(),
        rules: Vec::new(),
    };
    doc.set_technology(tech);
    doc
}

/// Writes `doc` to a temporary GDSII file and returns the guard.
fn export_to_temp_gds(doc: &Document) -> TempFile {
    let bytes = Gds.export(doc).expect("export sample document to GDSII");
    let file = TempFile::new(temp_path("doc", "gds"));
    std::fs::write(file.path(), &bytes).expect("write temp GDSII file");
    file
}

#[test]
fn import_summary_counts_cells_shapes_and_layers() {
    let doc = sample_document();
    let gds = export_to_temp_gds(&doc);

    // The extension is `.gds`, so the loader must pick the GDSII importer.
    assert_eq!(Format::from_path(gds.path()), Format::Gds);

    let reloaded = load_document(gds.path()).expect("reload the exported GDSII");
    let summary = summarize(&reloaded);

    assert_eq!(summary.cell_count, 1);
    assert_eq!(summary.top_cells, vec!["top".to_string()]);
    assert_eq!(
        summary.shape_count, 3,
        "three rectangles survive the round-trip"
    );
    assert_eq!(summary.instance_count, 0);
    assert_eq!(summary.array_count, 0);
    assert_eq!(summary.layers, vec![METAL], "all shapes are on metal 1/0");
}

#[test]
fn pick_top_cell_prefers_declared_top() {
    let doc = sample_document();
    assert_eq!(pick_top_cell(&doc).expect("a top cell"), "top");

    // With no declared tops, it falls back to any cell (deterministically the
    // lexicographically smallest), so single-cell files still work.
    let mut doc2 = Document::new();
    doc2.insert_cell(Cell::new("only"));
    assert_eq!(pick_top_cell(&doc2).expect("fallback top"), "only");
}

#[test]
fn drc_flags_thin_shape_with_default_rules() {
    let doc = sample_document();
    let gds = export_to_temp_gds(&doc);
    let reloaded = load_document(gds.path()).expect("reload GDSII");
    let top = pick_top_cell(&reloaded).expect("top cell");

    // No technology file and the document has no rules of its own, so the
    // synthesized default width rule (100 DBU) must apply.
    let rules = resolve_rules(&reloaded, None).expect("resolve default rules");
    assert!(!rules.is_empty(), "a default rule set is synthesized");
    assert!(rules.iter().all(|r| r.kind == RuleKind::Width));

    let violations = run_drc(&reloaded, &top, rules).expect("run DRC");
    assert!(
        !violations.is_empty(),
        "the 10-DBU-wide bar must violate the 100-DBU width rule"
    );
    // Every reported violation names a rule and carries a message.
    for v in &violations {
        assert!(!v.rule.is_empty());
        assert!(!v.message.is_empty());
    }
}

#[test]
fn drc_uses_supplied_technology_file() {
    let doc = sample_document();
    let gds = export_to_temp_gds(&doc);
    let reloaded = load_document(gds.path()).expect("reload GDSII");
    let top = pick_top_cell(&reloaded).expect("top cell");

    // A technology file with a strict width rule on metal 1/0.
    let tech = TempFile::new(temp_path("tech", "tech"));
    std::fs::write(
        tech.path(),
        "technology strict\n\
         dbu_per_micron 1000\n\
         layer 1 0 metal1 4488FFFF\n\
         rule width 1 0 100\n",
    )
    .expect("write technology file");

    let rules = resolve_rules(&reloaded, Some(tech.path())).expect("parse tech rules");
    assert_eq!(rules.len(), 1, "exactly the one width rule from the file");
    assert_eq!(rules[0].kind, RuleKind::Width);

    let violations = run_drc(&reloaded, &top, rules).expect("run DRC with tech file");
    assert!(
        !violations.is_empty(),
        "thin bar violates the file's width rule"
    );
}

#[test]
fn drc_reports_missing_cell() {
    let doc = sample_document();
    let rules = default_rules(&doc);
    let err = run_drc(&doc, "does_not_exist", rules).expect_err("missing cell is an error");
    assert!(err.to_string().contains("does_not_exist"));
}

#[test]
fn extract_reports_three_nets() {
    let doc = sample_document();
    let gds = export_to_temp_gds(&doc);
    let reloaded = load_document(gds.path()).expect("reload GDSII");
    let top = pick_top_cell(&reloaded).expect("top cell");

    let (net_count, sizes) = run_extract(&reloaded, &top).expect("extract connectivity");
    assert_eq!(net_count, 3, "three disjoint rectangles are three nets");
    assert_eq!(sizes, vec![1, 1, 1], "each net has exactly one shape");
    assert_eq!(sizes.iter().sum::<usize>(), 3);
}

#[test]
fn route_connects_synthesized_nets() {
    let mut doc = sample_document();
    let top = pick_top_cell(&doc).expect("top cell");

    let request = synth_route_request(&doc, &top);
    assert_eq!(request.cell, "top");
    assert!(
        !request.nets.is_empty(),
        "three shapes synthesize at least one net"
    );

    let shapes_before = doc.cell(&top).expect("cell").shapes.len();
    let report = run_route(&mut doc, &request);

    assert_eq!(
        report.routed + report.failed,
        request.nets.len(),
        "every net is accounted for as routed or failed"
    );
    assert!(
        report.routed >= 1,
        "at least one net should route on open metal"
    );
    assert!(
        report.total_length_dbu > 0,
        "a routed net has positive length"
    );

    let shapes_after = doc.cell(&top).expect("cell").shapes.len();
    assert!(
        shapes_after > shapes_before,
        "routing appends wire geometry to the cell"
    );
}

#[test]
fn export_round_trips_through_gds() {
    let doc = sample_document();
    let gds = export_to_temp_gds(&doc);
    let reloaded = load_document(gds.path()).expect("reload GDSII");

    // Re-export the reloaded document to a second GDSII file via the pipeline
    // function and confirm it reloads to the same shape count.
    let out = TempFile::new(temp_path("roundtrip", "gds"));
    run_export(&reloaded, out.path(), Format::Gds).expect("export via pipeline");
    assert!(out.path().exists(), "export writes the output file");

    let again = load_document(out.path()).expect("reload the re-exported GDSII");
    assert_eq!(summarize(&again).shape_count, 3);
    assert_eq!(summarize(&again).layers, vec![METAL]);
}

#[test]
fn export_converts_gds_to_oasis() {
    let doc = sample_document();
    let gds = export_to_temp_gds(&doc);
    let reloaded = load_document(gds.path()).expect("reload GDSII");

    // The sample is rectangles only, so the OASIS subset can represent it.
    let out = TempFile::new(temp_path("converted", "oas"));
    run_export(&reloaded, out.path(), Format::Oasis).expect("convert to OASIS");

    // The `.oas` extension routes the loader to the OASIS importer.
    assert_eq!(Format::from_path(out.path()), Format::Oasis);
    let oasis_doc = load_document(out.path()).expect("reload the OASIS file");
    assert_eq!(summarize(&oasis_doc).shape_count, 3);
}

#[test]
fn format_parsing_accepts_known_names_and_rejects_others() {
    assert_eq!(Format::parse("gds").unwrap(), Format::Gds);
    assert_eq!(Format::parse("GDSII").unwrap(), Format::Gds);
    assert_eq!(Format::parse("oasis").unwrap(), Format::Oasis);
    assert_eq!(Format::parse("oas").unwrap(), Format::Oasis);
    assert!(Format::parse("dxf").is_err());
}

#[test]
fn framing_camera_centers_on_the_design() {
    // A 300 x 300 box centered at (150, 150) should give a camera centered there
    // with a positive scale.
    let bbox = Rect::new(Point::new(0, 0), Point::new(300, 300));
    let camera = framing_camera(bbox, 256, 256);
    assert_eq!(camera.center, Point::new(150, 150));
    assert!(camera.pixels_per_dbu > 0.0);
}

#[test]
fn render_writes_png_when_gpu_is_available() {
    // Render needs a GPU; skip gracefully when none is present (e.g. headless CI).
    let Some(_ctx) = WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter; skipping render_writes_png_when_gpu_is_available");
        return;
    };

    let doc = sample_document();
    let top = pick_top_cell(&doc).expect("top cell");
    let png = TempFile::new(temp_path("render", "png"));

    let outcome = run_render(&doc, &top, png.path(), 64, 48).expect("render to PNG");
    match outcome {
        RenderOutcome::Rendered {
            path,
            width,
            height,
        } => {
            assert_eq!(width, 64);
            assert_eq!(height, 48);
            assert!(path.exists(), "the PNG file is written");
            // Decode it back to confirm it is a valid 64x48 PNG.
            let image = image::open(&path).expect("decode the written PNG");
            assert_eq!(image.width(), 64);
            assert_eq!(image.height(), 48);
        }
        RenderOutcome::NoGpu => {
            panic!("GPU was available a moment ago; render should have produced an image");
        }
    }
}

#[test]
fn render_reports_missing_cell() {
    let doc = sample_document();
    let png = TempFile::new(temp_path("render_missing", "png"));
    // This must fail on the missing cell regardless of GPU availability, since the
    // cell is validated before any GPU work.
    let err = run_render(&doc, "nope", png.path(), 16, 16).expect_err("missing cell errors");
    assert!(err.to_string().contains("nope"));
}
