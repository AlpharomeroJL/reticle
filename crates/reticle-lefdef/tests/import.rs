//! Integration tests for LEF/DEF import.
//!
//! Two-way coverage: the committed synthetic design imports and lowers to an
//! asserted [`Document`], and a battery of seeded-bad inputs (truncated, bad
//! numbers, unknown keywords, oversized, non-UTF-8) never panics or hangs and
//! either errors cleanly or imports with warnings.

use std::path::PathBuf;

use reticle_geometry::Orientation;
use reticle_lefdef::{LefDefError, NetSegment, import_lef_def, import_run_dir};
use reticle_model::PinDirection;

const LEF: &[u8] = include_bytes!("fixtures/tinycore.lef");
const DEF: &[u8] = include_bytes!("fixtures/tinycore.def");

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn imports_synthetic_design_to_asserted_document() {
    let design = import_lef_def(LEF, DEF).expect("import should succeed");

    // Design identity and top cell.
    assert_eq!(design.design_name, "tinycore");
    assert_eq!(design.top_cell(), "tinycore");
    assert_eq!(design.document.top_cells(), ["tinycore"]);

    // Cells: two macros plus the top design cell.
    assert_eq!(design.document.cell_count(), 3);
    assert!(design.document.cell("INV").is_some());
    assert!(design.document.cell("BUF").is_some());

    // Technology: resolution and the three LEF layers.
    let tech = design.document.technology();
    assert_eq!(tech.dbu_per_micron, 1000);
    let layer_names: Vec<&str> = tech.layers.iter().map(|l| l.name.as_str()).collect();
    assert!(layer_names.contains(&"li1"));
    assert!(layer_names.contains(&"mcon"));
    assert!(layer_names.contains(&"met1"));

    // INV macro: three pins with mapped directions, and drawn geometry.
    let inv = design.document.cell("INV").unwrap();
    assert_eq!(inv.pins.len(), 3);
    let dir = |name: &str| {
        inv.pins
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.direction)
    };
    assert_eq!(dir("A"), Some(PinDirection::Input));
    assert_eq!(dir("Y"), Some(PinDirection::Output));
    assert_eq!(dir("VPWR"), Some(PinDirection::Inout));
    // Three pin rects + one obstruction = four drawn shapes.
    assert_eq!(inv.shapes.len(), 4);
    // 0.20 micron at 1000 DBU/micron lands on 200 DBU.
    let a_pin = inv.pins.iter().find(|p| p.name == "A").unwrap();
    assert_eq!(a_pin.region.min.x, 200);

    // Placement: three placed instances on the top cell.
    let top = design.document.cell("tinycore").unwrap();
    assert_eq!(top.instances.len(), 3);
    let u1 = top.instances.iter().find(|i| i.cell == "INV").unwrap();
    assert_eq!(u1.transform.translation.x, 1000);
    // u3 is placed FN, which maps to MirrorX180.
    let fn_inst = top
        .instances
        .iter()
        .find(|i| i.transform.translation.x == 5000)
        .unwrap();
    assert_eq!(fn_inst.transform.orientation, Orientation::MirrorX180);

    // Die area.
    let die = design.die_area.expect("die area present");
    assert_eq!(die.min.x, 0);
    assert_eq!(die.max.x, 20000);

    // Rows: two, with the FS row mapped to MirrorX.
    assert_eq!(design.rows.len(), 2);
    assert_eq!(design.rows[0].name, "ROW_0");
    assert_eq!(design.rows[0].site, "unithd");
    assert_eq!(design.rows[0].count_x, 40);
    assert_eq!(design.rows[0].step_x, 460);
    assert_eq!(design.rows[0].orientation, Orientation::R0);
    assert_eq!(design.rows[1].orientation, Orientation::MirrorX);

    // Sites: one, converted to DBU.
    assert_eq!(design.sites.len(), 1);
    assert_eq!(design.sites[0].name, "unithd");
    assert_eq!(design.sites[0].width, 460);
    assert_eq!(design.sites[0].height, 2720);

    // Nets: two, with the routed geometry lowered.
    assert_eq!(design.nets.len(), 2);
    let n_in = design.nets.iter().find(|n| n.name == "n_in").unwrap();
    assert_eq!(n_in.use_kind.as_deref(), Some("SIGNAL"));
    // n_in routes on two layers and drops one via.
    let wires = n_in
        .segments
        .iter()
        .filter(|s| matches!(s, NetSegment::Wire { .. }))
        .count();
    let vias = n_in
        .segments
        .iter()
        .filter(|s| matches!(s, NetSegment::Via { .. }))
        .count();
    assert_eq!(wires, 2);
    assert_eq!(vias, 1);
    // The `*` repeated coordinate resolved to the previous x (1200).
    if let Some(NetSegment::Via { at, via }) = n_in
        .segments
        .iter()
        .find(|s| matches!(s, NetSegment::Via { .. }))
    {
        assert_eq!(at.x, 1200);
        assert_eq!(at.y, 1100);
        assert_eq!(via, "mcon");
    }

    // Pins: two external, mapped and placed.
    assert_eq!(design.pins.len(), 2);
    let in0 = design.pins.iter().find(|p| p.name == "in0").unwrap();
    assert_eq!(in0.direction, PinDirection::Input);
    assert_eq!(in0.net, "n_in");
    let region = in0.region.expect("in0 has a placed region");
    // Placed at (0, 5000) with a (-70,-70)-(70,70) box => centered on (0, 5000).
    assert_eq!(region.min.x, -70);
    assert_eq!(region.max.y, 5070);

    // Overlays are empty until lane 5B fills them from reports.
    assert!(design.overlays.utilization.is_none());
    assert!(design.overlays.congestion.is_empty());

    // A clean fixture imports without warnings.
    assert!(
        design.warnings.is_empty(),
        "unexpected warnings: {:?}",
        design.warnings
    );

    // The lowered document flattens without panicking and yields geometry.
    let flat = design.document.flatten("tinycore");
    assert!(!flat.is_empty());
}

#[test]
fn import_run_dir_finds_and_picks_final_def() {
    let dir = fixtures_dir().join("run");
    let design = import_run_dir(&dir).expect("run import should succeed");
    // Two DEF stages live under results/; the later-sorting 6_final.def wins.
    assert_eq!(design.design_name, "tinycore");
    let top = design.document.cell("tinycore").unwrap();
    assert_eq!(top.instances.len(), 2);
    assert_eq!(design.nets.len(), 1);
}

#[test]
fn import_run_dir_missing_files_errors_cleanly() {
    let dir = fixtures_dir(); // has .lef/.def only under run/, not directly
    let empty = std::env::temp_dir().join("reticle_lefdef_empty_probe");
    let _ = std::fs::create_dir_all(&empty);
    match import_run_dir(&empty) {
        Err(LefDefError::MissingFile(_)) => {}
        other => panic!("expected MissingFile, got {other:?}"),
    }
    // A non-existent directory is a clean IO error, not a panic.
    let missing = dir.join("does-not-exist-xyz");
    assert!(matches!(import_run_dir(&missing), Err(LefDefError::Io(_))));
}

#[test]
fn oversized_input_is_refused() {
    // A slice claiming to be over the ceiling is refused before any parsing. We do
    // not actually allocate 256 MiB; a zero-filled Vec of the threshold+1 is enough
    // to trip the length check, and Vec of zeros is cheap.
    let big = vec![b' '; reticle_lefdef::MAX_INPUT_BYTES + 1];
    match import_lef_def(&big, DEF) {
        Err(LefDefError::TooLarge { which: "LEF", .. }) => {}
        other => panic!("expected TooLarge for LEF, got {other:?}"),
    }
    match import_lef_def(LEF, &big) {
        Err(LefDefError::TooLarge { which: "DEF", .. }) => {}
        other => panic!("expected TooLarge for DEF, got {other:?}"),
    }
}

#[test]
fn empty_inputs_do_not_panic() {
    let design = import_lef_def(b"", b"").expect("empty import is not an error");
    // No design name => a synthetic top cell named "top".
    assert_eq!(design.design_name, "top");
    assert_eq!(design.document.cell_count(), 1);
}

#[test]
fn truncated_lef_errors_without_panicking() {
    // A macro that ends mid-SIZE: the height number never arrives.
    let bad = b"MACRO INV\n  SIZE 1.0 BY\n";
    assert!(matches!(
        import_lef_def(bad, b""),
        Err(LefDefError::Lef { .. })
    ));
}

#[test]
fn bad_def_coordinate_errors_without_panicking() {
    let bad = b"DESIGN d ;\nDIEAREA ( 0 0 ) ( notanumber 5 ) ;\n";
    assert!(matches!(
        import_lef_def(b"", bad),
        Err(LefDefError::Def { .. })
    ));
}

#[test]
fn unknown_keywords_are_skipped_not_fatal() {
    let lef = b"WEIRDDIRECTIVE foo bar ;\nLAYER m1\n TYPE ROUTING ;\nEND m1\n";
    let def = b"DESIGN d ;\nUNITS DISTANCE MICRONS 1000 ;\nZANYSECTION blah ;\n";
    let design = import_lef_def(lef, def).expect("unknown keywords are skipped");
    assert_eq!(design.design_name, "d");
}

#[test]
fn skipped_specialnets_records_a_warning() {
    let def = b"DESIGN d ;\nUNITS DISTANCE MICRONS 1000 ;\nSPECIALNETS 1 ;\n - VDD ;\nEND SPECIALNETS\nEND DESIGN\n";
    let design = import_lef_def(b"", def).expect("import");
    assert!(
        design
            .warnings
            .iter()
            .any(|w| w.summary.contains("SPECIALNETS"))
    );
}

/// Fuzz-lite: truncating the good fixtures at many offsets, and flipping bytes,
/// must never panic or hang. Reaching the end of this test is the assertion.
#[test]
fn seeded_mutations_never_panic() {
    for cut in (0..LEF.len()).step_by(7) {
        let _ = import_lef_def(&LEF[..cut], DEF);
    }
    for cut in (0..DEF.len()).step_by(7) {
        let _ = import_lef_def(LEF, &DEF[..cut]);
    }
    // Deterministic byte flips across the DEF.
    let mut buf = DEF.to_vec();
    for i in (0..buf.len()).step_by(13) {
        let orig = buf[i];
        buf[i] = 0xFF; // invalid UTF-8 lead byte
        let _ = import_lef_def(LEF, &buf);
        buf[i] = b'(';
        let _ = import_lef_def(LEF, &buf);
        buf[i] = b';';
        let _ = import_lef_def(LEF, &buf);
        buf[i] = orig;
    }
}
