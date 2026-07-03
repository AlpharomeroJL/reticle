//! Imports real `SkyWater` `sky130_fd_sc_hd` standard cells and proves the GDSII
//! importer handles production layouts, not just fixtures our own exporter wrote.
//!
//! The committed corpus at `tests/corpus/sky130/` holds the three smallest
//! representative cells (a filler, a well tap, and an inverter); see `NOTICE.md`
//! there for source, commit, and Apache-2.0 attribution. The full five-cell set
//! (adding `nand2_1` and `dfxtp_1`) is fetched by `scripts/fetch-sky130-cells.ps1`
//! and covered by the ignored external test at the bottom.

use reticle_geometry::{LayerId, Point};
use reticle_io::Gds;
use reticle_model::{Cell, Document, Exporter, Importer, ShapeKind};

/// Path to a committed corpus cell.
fn corpus_path(cell: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus/sky130")
        .join(format!("sky130_fd_sc_hd__{cell}.gds"))
}

/// Imports a committed corpus cell.
fn import_corpus_cell(cell: &str) -> Document {
    let path = corpus_path(cell);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("corpus cell {} should exist: {e}", path.display()));
    Gds.import(&bytes)
        .unwrap_or_else(|e| panic!("corpus cell {cell} should import: {e}"))
}

/// Shape counts by kind: (rectangles, polygons, paths).
fn kind_counts(cell: &Cell) -> (usize, usize, usize) {
    let count = |pred: fn(&ShapeKind) -> bool| cell.shapes.iter().filter(|s| pred(&s.kind)).count();
    (
        count(|k| matches!(k, ShapeKind::Rect(_))),
        count(|k| matches!(k, ShapeKind::Polygon(_))),
        count(|k| matches!(k, ShapeKind::Path(_))),
    )
}

/// The label texts of a cell, sorted, for multiset comparison.
fn sorted_label_texts(cell: &Cell) -> Vec<&str> {
    let mut texts: Vec<&str> = cell.labels.iter().map(|l| l.text.as_str()).collect();
    texts.sort_unstable();
    texts
}

/// Every committed cell imports as a single top cell with the structure the
/// upstream library ships: the expected mix of rectangles, polygons, and paths,
/// flat geometry (standard cells have no instances), and 1 nm database units.
#[test]
fn imports_committed_sky130_cells() {
    // (short name, shapes as (rect, poly, path), label count) per cell,
    // matching the upstream drive-1 layouts at the commit cited in NOTICE.md.
    let expected = [
        ("fill_1", (11, 0, 4), 5),
        ("tap_1", (20, 0, 4), 5),
        ("inv_1", (42, 2, 2), 8),
    ];
    for (name, kinds, labels) in expected {
        let doc = import_corpus_cell(name);
        let full = format!("sky130_fd_sc_hd__{name}");
        assert_eq!(doc.cell_count(), 1, "{name}: one cell per stream");
        assert_eq!(
            doc.top_cells(),
            std::slice::from_ref(&full),
            "{name}: top cell"
        );
        assert_eq!(
            doc.technology().dbu_per_micron,
            1000,
            "{name}: sky130 uses 1 dbu = 1 nm"
        );

        let cell = doc.cell(&full).expect("top cell present");
        assert_eq!(kind_counts(cell), kinds, "{name}: shape mix");
        assert_eq!(cell.labels.len(), labels, "{name}: label count");
        assert!(cell.instances.is_empty(), "{name}: leaf cell, no instances");
        assert!(cell.arrays.is_empty(), "{name}: leaf cell, no arrays");
    }
}

/// The inverter draws on the expected `SkyWater` layer/datatype pairs of the
/// digital stack, including the contact layers.
#[test]
fn inverter_uses_skywater_layer_map() {
    let doc = import_corpus_cell("inv_1");
    let cell = doc.cell("sky130_fd_sc_hd__inv_1").expect("top cell");
    let layers: std::collections::BTreeSet<LayerId> = cell.shapes.iter().map(|s| s.layer).collect();
    for (layer, datatype, what) in [
        (64, 20, "nwell drawing"),
        (65, 20, "diff drawing"),
        (66, 20, "poly drawing"),
        (66, 44, "licon1"),
        (67, 20, "li1 drawing"),
        (67, 44, "mcon"),
        (68, 20, "met1 drawing"),
    ] {
        assert!(
            layers.contains(&LayerId::new(layer, datatype)),
            "inv_1 should draw on {what} ({layer}/{datatype}); got {layers:?}"
        );
    }
}

/// Pin and net labels survive import with their text, position, and the
/// `SkyWater` label purposes (datatype 5 for pin labels, 59 for the pwell net).
#[test]
fn preserves_pin_labels() {
    let doc = import_corpus_cell("inv_1");
    let cell = doc.cell("sky130_fd_sc_hd__inv_1").expect("top cell");

    // The inverter's full label multiset: pins A and Y (Y twice, once per li1
    // finger), the four supply nets, and the cell-name text on 83/44.
    assert_eq!(
        sorted_label_texts(cell),
        ["A", "VGND", "VNB", "VPB", "VPWR", "Y", "Y", "inv_1"]
    );

    let label = |text: &str| {
        cell.labels
            .iter()
            .find(|l| l.text == text)
            .unwrap_or_else(|| panic!("label {text} missing"))
    };
    // Input pin A: on the li1 label purpose (67/5) at the upstream coordinates.
    let a = label("A");
    assert_eq!(a.layer, LayerId::new(67, 5));
    assert_eq!(a.position, Point::new(445, 1190));
    // Supply labels sit on met1 (68/5); the pwell net uses purpose 59.
    assert_eq!(label("VPWR").layer, LayerId::new(68, 5));
    assert_eq!(label("VGND").layer, LayerId::new(68, 5));
    assert_eq!(label("VNB").layer, LayerId::new(64, 59));

    // The tap cell's body pins live on li1 instead of met1.
    let doc = import_corpus_cell("tap_1");
    let cell = doc.cell("sky130_fd_sc_hd__tap_1").expect("top cell");
    assert_eq!(
        sorted_label_texts(cell),
        ["VGND", "VNB", "VPB", "VPWR", "tap_1"]
    );
    let vnb = cell
        .labels
        .iter()
        .find(|l| l.text == "VNB")
        .expect("VNB label");
    assert_eq!(vnb.layer, LayerId::new(67, 5));
}

/// Import -> export -> import is stable on real cells: geometry and labels are
/// preserved element-for-element, and a second export produces byte-identical
/// output (the exporter is deterministic, no timestamps or map ordering leaks).
#[test]
fn round_trip_is_stable() {
    for name in ["fill_1", "tap_1", "inv_1"] {
        let doc1 = import_corpus_cell(name);
        let bytes2 = Gds.export(&doc1).expect("export imported document");
        let doc2 = Gds.import(&bytes2).expect("re-import our own export");
        let bytes3 = Gds.export(&doc2).expect("second export");

        let full = format!("sky130_fd_sc_hd__{name}");
        let c1 = doc1.cell(&full).expect("cell after first import");
        let c2 = doc2.cell(&full).expect("cell after re-import");
        assert_eq!(c1.shapes, c2.shapes, "{name}: shapes survive round trip");
        assert_eq!(c1.labels, c2.labels, "{name}: labels survive round trip");
        assert_eq!(doc1.top_cells(), doc2.top_cells(), "{name}: top cells");
        assert_eq!(
            doc1.technology().dbu_per_micron,
            doc2.technology().dbu_per_micron,
            "{name}: units survive round trip"
        );
        assert_eq!(bytes2, bytes3, "{name}: export is deterministic");
    }
}

/// Round-trips the full fetched set, including the larger `nand2_1` (polygon
/// diffusion) and `dfxtp_1` (144 shapes, 31 polygons). Ignored by default
/// because the files are not committed; run `scripts/fetch-sky130-cells.ps1`
/// first, then `cargo test -p reticle-io --test sky130_cells -- --ignored`.
#[test]
#[ignore = "needs scratch/cells from scripts/fetch-sky130-cells.ps1"]
fn round_trips_full_fetched_set() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch/cells");
    for name in ["inv", "nand2", "dfxtp", "fill", "tap"] {
        let path = dir.join(format!("sky130_fd_sc_hd__{name}_1.gds"));
        let bytes =
            std::fs::read(&path).unwrap_or_else(|e| panic!("fetch {} first: {e}", path.display()));
        let doc1 = Gds.import(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let bytes2 = Gds.export(&doc1).expect("export");
        let doc2 = Gds.import(&bytes2).expect("re-import");
        let bytes3 = Gds.export(&doc2).expect("second export");

        let full = format!("sky130_fd_sc_hd__{name}_1");
        let c1 = doc1.cell(&full).expect("cell after first import");
        let c2 = doc2.cell(&full).expect("cell after re-import");
        assert_eq!(c1.shapes, c2.shapes, "{name}: shapes survive round trip");
        assert_eq!(c1.labels, c2.labels, "{name}: labels survive round trip");
        assert_eq!(bytes2, bytes3, "{name}: export is deterministic");
    }
}
