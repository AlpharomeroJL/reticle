//! GDSII label (TEXT element) round-trip tests.
//!
//! Labels are the net and port names GDSII carries as TEXT elements. Import
//! surfaces them as [`reticle_model::Label`]s on [`reticle_model::Cell::labels`];
//! export writes each label back as a TEXT element on its layer/texttype. These
//! tests cover both directions plus the full round-trip, alongside shapes, so
//! label support never regresses the geometry path.

use gds21::{
    GdsBoundary, GdsElement, GdsLibrary, GdsPoint, GdsStrans, GdsStruct, GdsTextElem, GdsUnits,
};
use reticle_geometry::{LayerId, Point, Rect};
use reticle_io::Gds;
use reticle_model::{
    Anchor, Cell, Document, DrawShape, Exporter, Importer, Label, ShapeKind, Technology,
};

/// met1 drawing (SKY130 convention).
const MET1: LayerId = LayerId::new(68, 20);
/// met1.label: the label-purpose datatype riding on met1.
const MET1_LABEL: LayerId = LayerId::new(68, 5);
/// li1.label: a second label-purpose layer.
const LI1_LABEL: LayerId = LayerId::new(67, 5);

/// Builds a GDSII byte stream directly with `gds21` (not our exporter), holding
/// one struct with a boundary and two TEXT elements. The second TEXT carries the
/// optional fields (strans, width, path type) a foreign tool may emit.
fn foreign_gds_with_text() -> Vec<u8> {
    let mut lib = GdsLibrary::new("labels_lib");
    lib.units = GdsUnits::new(1e-3, 1e-9); // 1000 DBU per micron

    let mut chip = GdsStruct::new("chip");
    chip.elems.push(GdsElement::GdsBoundary(GdsBoundary {
        layer: 68,
        datatype: 20,
        xy: GdsPoint::vec(&[(0, 0), (2000, 0), (2000, 400), (0, 400), (0, 0)]),
        ..GdsBoundary::default()
    }));
    chip.elems.push(GdsElement::GdsTextElem(GdsTextElem {
        string: "VDD".to_string(),
        layer: 68,
        texttype: 5,
        xy: GdsPoint::new(500, 1000),
        ..GdsTextElem::default()
    }));
    chip.elems.push(GdsElement::GdsTextElem(GdsTextElem {
        string: "clk_in".to_string(),
        layer: 67,
        texttype: 5,
        xy: GdsPoint::new(-30, 40),
        width: Some(50),
        path_type: Some(1),
        strans: Some(GdsStrans {
            reflected: true,
            abs_mag: false,
            abs_angle: false,
            mag: Some(2.0),
            angle: Some(90.0),
        }),
        ..GdsTextElem::default()
    }));
    lib.structs.push(chip);

    let mut bytes = Vec::new();
    lib.write(&mut bytes).expect("gds21 write should succeed");
    bytes
}

#[test]
fn import_reads_text_elements_as_labels() {
    let bytes = foreign_gds_with_text();
    let doc = Gds.import(&bytes).expect("import should succeed");

    let chip = doc.cell("chip").expect("chip cell present");

    // The boundary still imports as a rectangle; TEXT no longer disturbs shapes.
    assert_eq!(chip.shapes.len(), 1);
    match &chip.shapes[0].kind {
        ShapeKind::Rect(r) => {
            assert_eq!(*r, Rect::new(Point::new(0, 0), Point::new(2000, 400)));
            assert_eq!(chip.shapes[0].layer, MET1);
        }
        other => panic!("expected rectangle, got {other:?}"),
    }

    // Both TEXT elements arrive as labels, in element order, anchored Center.
    assert_eq!(chip.labels.len(), 2);
    assert_eq!(
        chip.labels[0],
        Label::new("VDD", Point::new(500, 1000), MET1_LABEL)
    );
    assert_eq!(
        chip.labels[1],
        Label::new("clk_in", Point::new(-30, 40), LI1_LABEL)
    );
    assert_eq!(chip.labels[1].anchor, Anchor::Center);

    // Label layers join the derived layer table alongside shape layers.
    let layers: Vec<LayerId> = doc.technology().layers.iter().map(|l| l.id).collect();
    assert!(layers.contains(&MET1));
    assert!(layers.contains(&MET1_LABEL));
    assert!(layers.contains(&LI1_LABEL));
}

/// Builds a document whose top cell carries a met1 rectangle plus power labels,
/// and a labeled child cell, so per-cell label attribution is exercised.
fn labeled_document() -> Document {
    let mut doc = Document::new();

    let mut top = Cell::new("top");
    top.shapes.push(DrawShape::new(
        MET1,
        ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(2000, 400))),
    ));
    top.labels
        .push(Label::new("VDD", Point::new(500, 1000), MET1_LABEL));
    top.labels
        .push(Label::new("GND", Point::new(-250, -75), LI1_LABEL));
    doc.insert_cell(top);

    let mut leaf = Cell::new("leaf");
    leaf.shapes.push(DrawShape::new(
        MET1,
        ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(90, 90))),
    ));
    leaf.labels
        .push(Label::new("A", Point::new(45, 45), MET1_LABEL));
    doc.insert_cell(leaf);

    doc.set_top_cells(vec!["top".to_string(), "leaf".to_string()]);
    doc.set_technology(Technology {
        name: "labels".to_string(),
        dbu_per_micron: 1000,
        ..Technology::default()
    });
    doc
}

#[test]
fn export_writes_labels_as_text_elements() {
    let doc = labeled_document();
    let bytes = Gds.export(&doc).expect("export should succeed");

    // Parse the stream with gds21 directly: the labels must exist as TEXT
    // elements with the right string, layer, texttype, and insertion point.
    let lib = GdsLibrary::from_bytes(bytes).expect("gds21 should parse our export");
    let top = lib
        .structs
        .iter()
        .find(|s| s.name == "top")
        .expect("top struct present");
    let texts: Vec<&GdsTextElem> = top
        .elems
        .iter()
        .filter_map(|e| match e {
            GdsElement::GdsTextElem(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), 2);
    assert_eq!(texts[0].string, "VDD");
    assert_eq!(texts[0].layer, 68);
    assert_eq!(texts[0].texttype, 5);
    assert_eq!(texts[0].xy, GdsPoint::new(500, 1000));
    assert_eq!(texts[1].string, "GND");
    assert_eq!(texts[1].layer, 67);
    assert_eq!(texts[1].texttype, 5);
    assert_eq!(texts[1].xy, GdsPoint::new(-250, -75));

    // Shapes still export alongside the labels.
    assert!(
        top.elems
            .iter()
            .any(|e| matches!(e, GdsElement::GdsBoundary(_))),
        "the met1 rectangle must still be exported"
    );
}

#[test]
fn gds_roundtrip_preserves_labels_alongside_shapes() {
    let original = labeled_document();

    let bytes = Gds.export(&original).expect("export should succeed");
    let imported = Gds.import(&bytes).expect("import should succeed");

    // The top cell keeps its shape and both labels, field for field (text,
    // position, layer, and the Center anchor).
    let top = imported.cell("top").expect("top cell present");
    assert_eq!(top.shapes.len(), 1);
    assert_eq!(
        top.labels,
        vec![
            Label::new("VDD", Point::new(500, 1000), MET1_LABEL),
            Label::new("GND", Point::new(-250, -75), LI1_LABEL),
        ]
    );

    // The child cell keeps its own label: attribution stays per cell.
    let leaf = imported.cell("leaf").expect("leaf cell present");
    assert_eq!(
        leaf.labels,
        vec![Label::new("A", Point::new(45, 45), MET1_LABEL)]
    );
    assert_eq!(leaf.shapes.len(), 1);

    // Resolution and the label layers survive too.
    assert_eq!(imported.technology().dbu_per_micron, 1000);
    let layers: Vec<LayerId> = imported.technology().layers.iter().map(|l| l.id).collect();
    assert!(layers.contains(&MET1_LABEL));
    assert!(layers.contains(&LI1_LABEL));
}

#[test]
fn gds_roundtrip_with_labels_is_idempotent() {
    // Export -> import -> export reproduces identical bytes with labels present,
    // proving the label mapping is stable across cycles like the geometry one.
    let doc = labeled_document();
    let bytes1 = Gds.export(&doc).expect("first export");
    let reimported = Gds.import(&bytes1).expect("import");
    let bytes2 = Gds.export(&reimported).expect("second export");
    assert_eq!(
        bytes1, bytes2,
        "GDS export with labels must be stable across a round-trip"
    );
}

/// Path to the committed labeled corpus fixture.
fn labels_corpus_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/labels.gds")
}

#[test]
fn imports_committed_labeled_corpus_fixture() {
    // A committed on-disk fixture written by gds21 itself (not our exporter),
    // holding TEXT elements with foreign-tool options; see the regenerate test.
    let bytes = std::fs::read(labels_corpus_path()).expect("labels fixture should exist");
    let doc = Gds.import(&bytes).expect("labels fixture should import");

    let chip = doc.cell("chip").expect("chip cell present");
    assert_eq!(chip.shapes.len(), 1);
    assert_eq!(chip.labels.len(), 2);
    assert_eq!(
        chip.labels[0],
        Label::new("VDD", Point::new(500, 1000), MET1_LABEL)
    );
    assert_eq!(
        chip.labels[1],
        Label::new("clk_in", Point::new(-30, 40), LI1_LABEL)
    );
}

/// Regenerates the labeled corpus fixture from the gds21-built stream. Ignored
/// by default so it never rewrites the committed file during normal runs.
#[test]
#[ignore = "run explicitly to regenerate tests/corpus/labels.gds"]
fn regenerate_labels_corpus() {
    let bytes = foreign_gds_with_text();
    let path = labels_corpus_path();
    std::fs::create_dir_all(path.parent().unwrap()).expect("create corpus dir");
    std::fs::write(&path, &bytes).expect("write labels fixture");
}
