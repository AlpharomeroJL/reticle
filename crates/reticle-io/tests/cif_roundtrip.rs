//! CIF classic-subset reader tests: hand-written fixtures parse into the
//! expected [`Document`], targeted well-formed inputs exercise scale, mirror,
//! rotation-snapping, dangling references, and name collisions, and malformed
//! or over-cap inputs are rejected or capped cleanly, without panicking.
//!
//! Every test function name is prefixed `cif_` so `cargo nextest run -E
//! 'test(cif)'` selects this whole file (nextest's `test()` predicate matches
//! the bare test function name, not the binary name).

use proptest::prelude::*;
use reticle_geometry::{LayerId, Orientation, Point, Rect};
use reticle_io::WarningKind;
use reticle_io::cif::{Cif, MAX_CELLS, MAX_INPUT_BYTES, MAX_SHAPE_VERTICES};
use reticle_model::{Importer, ModelError, ShapeKind};
use std::fmt::Write as _;

/// Layer 0/0: the first (and only) `L` name declared in `basic.cif`.
const M1: LayerId = LayerId::new(0, 0);

// ---------------------------------------------------------------------------
// Fixture parsing: expected cells, shapes, and layers (success bar #1).
// ---------------------------------------------------------------------------

#[test]
fn cif_parses_basic_fixture_into_expected_cells_and_shapes() {
    let bytes = std::fs::read("tests/corpus/basic.cif").expect("basic.cif fixture present");
    let import = Cif
        .import_with_warnings(&bytes)
        .expect("a well-formed CIF fixture must import cleanly");
    assert!(
        import.warnings.is_empty(),
        "a clean fixture should carry no warnings, got {:?}",
        import.warnings
    );
    let doc = import.document;

    // Two cells: the 9-named "leaf" symbol and the synthetic top-level "TOP".
    assert_eq!(doc.cell_count(), 2);
    assert_eq!(doc.top_cells(), &["TOP".to_string()]);

    let leaf = doc
        .cell("leaf")
        .expect("DS 1 was named `leaf` via a 9 statement");
    assert_eq!(leaf.shapes.len(), 2, "the box and the wire");

    match &leaf.shapes[0].kind {
        ShapeKind::Rect(r) => {
            assert_eq!(*r, Rect::new(Point::new(0, 0), Point::new(100, 200)));
            assert_eq!(leaf.shapes[0].layer, M1);
        }
        other => panic!("expected the B box as a Rect, got {other:?}"),
    }
    match &leaf.shapes[1].kind {
        ShapeKind::Path(p) => {
            assert_eq!(p.points(), &[Point::new(0, 0), Point::new(0, 300)]);
            assert_eq!(p.width(), 10);
            assert_eq!(leaf.shapes[1].layer, M1);
        }
        other => panic!("expected the W wire as a Path, got {other:?}"),
    }

    // The CIF layer name is preserved honestly in the technology layer table.
    assert_eq!(doc.technology().layers.len(), 1);
    assert_eq!(doc.technology().layers[0].id, M1);
    assert_eq!(doc.technology().layers[0].name, "M1");
    assert_eq!(doc.technology().dbu_per_micron, 100);

    // TOP places `leaf` twice: a plain translate, then a translate + M Y mirror.
    let top = doc.cell("TOP").expect("top-level content forms a TOP cell");
    assert!(top.shapes.is_empty());
    assert_eq!(top.instances.len(), 2);
    assert_eq!(top.instances[0].cell, "leaf");
    assert_eq!(
        top.instances[0].transform.translation,
        Point::new(1000, 2000)
    );
    assert_eq!(top.instances[0].transform.orientation, Orientation::R0);

    assert_eq!(top.instances[1].cell, "leaf");
    assert_eq!(
        top.instances[1].transform.translation,
        Point::new(3000, -4000)
    );
    // `M Y` mirrors about the X axis: negates y, leaves x untouched.
    assert_eq!(
        top.instances[1]
            .transform
            .orientation
            .apply(Point::new(3, 4)),
        Point::new(3, -4)
    );
}

#[test]
fn cif_parses_toplevel_only_fixture_with_rotated_box_and_polygon() {
    let bytes = std::fs::read("tests/corpus/rotated.cif").expect("rotated.cif fixture present");
    let doc = Cif
        .import(&bytes)
        .expect("a well-formed top-level-only CIF must import cleanly");

    assert_eq!(
        doc.cell_count(),
        1,
        "no DS/DF at all: only the synthetic TOP cell"
    );
    assert_eq!(doc.top_cells(), &["TOP".to_string()]);

    let top = doc.cell("TOP").expect("top-level content forms a TOP cell");
    assert_eq!(top.shapes.len(), 2);

    match &top.shapes[0].kind {
        ShapeKind::Polygon(p) => {
            assert_eq!(
                p.vertices(),
                &[
                    Point::new(0, 0),
                    Point::new(100, 0),
                    Point::new(100, 100),
                    Point::new(0, 100),
                ]
            );
        }
        other => panic!("expected the P statement as a Polygon, got {other:?}"),
    }
    match &top.shapes[1].kind {
        ShapeKind::Rect(r) => {
            // w=40 h=20 centered at (200,200), direction (0,1): a 90-degree turn
            // swaps the extents (20 wide, 40 tall).
            assert_eq!(*r, Rect::new(Point::new(190, 180), Point::new(210, 220)));
        }
        other => panic!("expected the B box as a Rect, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Well-formed behavioral edge cases: scale, mirror, rotation snap, dangling
// references, and name collisions.
// ---------------------------------------------------------------------------

#[test]
fn cif_ds_scale_factor_multiplies_coordinates() {
    let src = b"DS 1 2 1;L M1;B 10 10 5 5;DF;";
    let doc = Cif.import(src).expect("scaled symbol should import");
    let cell = doc
        .cell("cif_1")
        .expect("unnamed symbol 1 keeps its default name");
    match &cell.shapes[0].kind {
        ShapeKind::Rect(r) => {
            // Every coordinate is doubled by the a/b = 2/1 scale before storage.
            assert_eq!(*r, Rect::new(Point::new(0, 0), Point::new(20, 20)));
        }
        other => panic!("expected Rect, got {other:?}"),
    }
}

#[test]
fn cif_mirror_x_negates_x_coordinate_only() {
    let src = b"DS 1 1 1;L M1;B 10 10 5 5;DF;C 1 M X;E;";
    let doc = Cif.import(src).expect("M X call should import");
    let top = doc.cell("TOP").expect("the C call forms a TOP cell");
    let orientation = top.instances[0].transform.orientation;
    assert_eq!(orientation.apply(Point::new(3, 4)), Point::new(-3, 4));
}

#[test]
fn cif_mirror_y_negates_y_coordinate_only() {
    let src = b"DS 1 1 1;L M1;B 10 10 5 5;DF;C 1 M Y;E;";
    let doc = Cif.import(src).expect("M Y call should import");
    let top = doc.cell("TOP").expect("the C call forms a TOP cell");
    let orientation = top.instances[0].transform.orientation;
    assert_eq!(orientation.apply(Point::new(3, 4)), Point::new(3, -4));
}

#[test]
fn cif_call_rotation_snaps_to_nearest_manhattan_with_warning() {
    let src = b"DS 1 1 1;L M1;B 10 10 5 5;DF;C 1 R 1 1;E;";
    let import = Cif
        .import_with_warnings(src)
        .expect("a snapped rotation is recoverable");
    assert!(
        import
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::ValueClamped),
        "a non-Manhattan R direction must warn: {:?}",
        import.warnings
    );
    let top = import
        .document
        .cell("TOP")
        .expect("the C call forms a TOP cell");
    let orientation = top.instances[0].transform.orientation;
    // 45 degrees snaps to the nearest quadrant, a 90-degree turn.
    assert_eq!(orientation.apply(Point::new(1, 0)), Point::new(0, 1));
}

#[test]
fn cif_dangling_call_reference_is_tolerated() {
    let src = b"C 999 T 0 0;E;";
    let doc = Cif
        .import(src)
        .expect("a call to an undefined symbol is not fatal");
    let top = doc.cell("TOP").expect("the C call forms a TOP cell");
    assert_eq!(top.instances[0].cell, "cif_999");
    assert!(
        doc.cell("cif_999").is_none(),
        "the referenced symbol was never defined and stays undefined"
    );
}

#[test]
fn cif_duplicate_symbol_name_is_disambiguated() {
    let src = b"DS 1 1 1;9 dup;DF;DS 2 1 1;9 dup;DF;";
    let import = Cif
        .import_with_warnings(src)
        .expect("a duplicate 9 name is recoverable");
    assert!(
        import
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::ValueClamped),
        "a duplicate name must warn: {:?}",
        import.warnings
    );
    let doc = import.document;
    assert_eq!(
        doc.cell_count(),
        2,
        "both symbols survive under distinct names"
    );
    assert!(doc.cell("dup").is_some());
    assert!(
        doc.cells()
            .any(|c| c.name != "dup" && c.name.starts_with("dup")),
        "the second `dup` was disambiguated with a suffix"
    );
}

#[test]
fn cif_top_level_name_sets_technology_name() {
    let doc = Cif
        .import(b"9 mydesign;E;")
        .expect("a top-level 9 statement is valid");
    assert_eq!(doc.technology().name, "mydesign");
}

#[test]
fn cif_unknown_command_is_skipped_with_warning() {
    let src = b"DS 1 1 1;L M1;B 1 1 0 0;DF;42 foo bar;E;";
    let import = Cif
        .import_with_warnings(src)
        .expect("an unrecognized command is recoverable");
    assert!(
        import
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::UnsupportedFeature),
        "an unknown command must warn: {:?}",
        import.warnings
    );
    // The good content around the unknown statement still imported.
    let cell = import
        .document
        .cell("cif_1")
        .expect("symbol 1 still imported");
    assert_eq!(cell.shapes.len(), 1);
}

// ---------------------------------------------------------------------------
// Malformed input: structured errors, never a panic (success bar #2).
// ---------------------------------------------------------------------------

#[test]
fn cif_truncated_missing_final_semicolon_errors() {
    let err = Cif
        .import(b"DS 1 1 1;L M1;B 1 1 0 0")
        .expect_err("a statement missing its terminating ; is truncated input");
    assert!(matches!(err, ModelError::Unsupported(_)));
}

#[test]
fn cif_truncated_unterminated_symbol_definition_errors() {
    Cif.import(b"DS 1 1 1;L M1;B 1 1 0 0;")
        .expect_err("a DS never closed with a matching DF is truncated input");
}

#[test]
fn cif_unterminated_comment_errors() {
    Cif.import(b"DS 1 1 1;(a comment that never closes")
        .expect_err("an unclosed ( comment is malformed input");
}

#[test]
fn cif_invalid_utf8_errors() {
    let bytes: &[u8] = &[0xFF, 0xFE, b'E', b';'];
    Cif.import(bytes)
        .expect_err("non-UTF-8 bytes are malformed input");
}

#[test]
fn cif_bad_number_token_errors() {
    Cif.import(b"DS abc 1 1;DF;")
        .expect_err("a non-numeric DS symbol number is malformed input");
    Cif.import(b"DS 1 1 1;L M1;B x 1 0 0;DF;")
        .expect_err("a non-numeric box coordinate is malformed input");
}

#[test]
fn cif_redefining_a_symbol_number_errors() {
    Cif.import(b"DS 1 1 1;DF;DS 1 1 1;DF;")
        .expect_err("redefining the same DS symbol number is malformed input");
}

#[test]
fn cif_nested_ds_errors() {
    Cif.import(b"DS 1 1 1;DS 2 1 1;DF;DF;")
        .expect_err("CIF does not support a DS nested inside another DS");
}

#[test]
fn cif_df_without_matching_ds_errors() {
    Cif.import(b"DF;")
        .expect_err("a DF with no open DS is malformed input");
}

#[test]
fn cif_shape_before_any_layer_errors() {
    Cif.import(b"B 1 1 0 0;")
        .expect_err("a shape statement before any L layer statement is malformed input");
}

#[test]
fn cif_oversized_input_is_refused_before_parsing() {
    // One byte past the documented maximum; content is irrelevant since size is
    // checked before any parsing begins.
    let bytes = vec![b'E'; MAX_INPUT_BYTES + 1];
    let err = Cif
        .import_with_warnings(&bytes)
        .expect_err("an oversized input must be refused");
    assert!(!err.to_string().is_empty(), "the refusal carries a message");
}

#[test]
fn cif_over_cap_polygon_vertices_is_dropped_with_warning() {
    let mut src = String::from("DS 1 1 1;L M1;P ");
    for i in 0..=MAX_SHAPE_VERTICES {
        let _ = write!(src, "{} {} ", i % 1000, (i * 7) % 1000);
    }
    src.push_str(";DF;");

    let import = Cif
        .import_with_warnings(src.as_bytes())
        .expect("an over-cap polygon is recoverable, not fatal");
    assert!(
        import
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::LimitExceeded),
        "an over-cap polygon must warn: {:?}",
        import.warnings
    );
    let cell = import
        .document
        .cell("cif_1")
        .expect("symbol 1 still imported");
    assert!(
        cell.shapes.is_empty(),
        "the oversized polygon was dropped, not truncated-and-kept"
    );
}

#[test]
fn cif_over_cap_cell_count_is_dropped_with_warning() {
    let mut src = String::new();
    for i in 0..=MAX_CELLS {
        let _ = writeln!(src, "DS {i} 1 1;DF;");
    }

    let import = Cif
        .import_with_warnings(src.as_bytes())
        .expect("an over-cap cell count is recoverable, not fatal");
    assert!(
        import
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::LimitExceeded),
        "exceeding the cell cap must warn: {:?}",
        import.warnings
    );
    assert_eq!(
        import.document.cell_count(),
        MAX_CELLS,
        "exactly the cap's worth of symbols were materialized"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Arbitrary bytes never panic the importer, whatever they contain.
    #[test]
    fn cif_import_never_panics_on_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..256)) {
        let result = Cif.import(&bytes);
        prop_assert!(result.is_ok() || result.is_err());
    }
}
