//! Reads a committed on-disk GDSII corpus fixture, proving the importer works on
//! a real `.gds` blob (not only bytes produced in the same process). The fixture
//! at `tests/corpus/basic.gds` is a tiny valid GDSII stream.
//!
//! To regenerate the fixture after an intentional format change, run:
//! `cargo test -p reticle-io --test gds_corpus -- --ignored regenerate_corpus`.

use reticle_geometry::{LayerId, Point, Rect};
use reticle_io::Gds;
use reticle_model::{Cell, Document, DrawShape, Exporter, Importer, ShapeKind};

/// Path to the committed corpus fixture.
fn corpus_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/basic.gds")
}

#[test]
fn imports_committed_corpus_fixture() {
    let bytes = std::fs::read(corpus_path()).expect("corpus fixture should exist");
    let doc = Gds.import(&bytes).expect("corpus fixture should import");

    // The fixture holds a single top cell `CORPUS` with one rectangle on L10/D0.
    let cell = doc.cell("CORPUS").expect("CORPUS cell present");
    assert_eq!(cell.shapes.len(), 1);
    match &cell.shapes[0].kind {
        ShapeKind::Rect(r) => {
            assert_eq!(*r, Rect::new(Point::new(0, 0), Point::new(1000, 1000)));
            assert_eq!(cell.shapes[0].layer, LayerId::new(10, 0));
        }
        other => panic!("expected rectangle, got {other:?}"),
    }
    assert_eq!(doc.top_cells(), &["CORPUS".to_string()]);
}

/// Regenerates the corpus fixture from the exporter. Ignored by default so it
/// never runs (or rewrites the file) during normal test runs.
#[test]
#[ignore = "run explicitly to regenerate tests/corpus/basic.gds"]
fn regenerate_corpus() {
    let mut doc = Document::new();
    let mut cell = Cell::new("CORPUS");
    cell.shapes.push(DrawShape::new(
        LayerId::new(10, 0),
        ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(1000, 1000))),
    ));
    doc.insert_cell(cell);
    doc.set_top_cells(vec!["CORPUS".to_string()]);

    let bytes = Gds.export(&doc).expect("export");
    let path = corpus_path();
    std::fs::create_dir_all(path.parent().unwrap()).expect("create corpus dir");
    std::fs::write(&path, &bytes).expect("write corpus fixture");
}
