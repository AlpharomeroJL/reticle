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
