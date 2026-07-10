//! DXF 2D-subset reader tests: hand-written fixtures parse into the expected
//! [`Document`], the `CIRCLE`/`ARC` polygonization is bounded and accurate,
//! honest gaps (bulge, `HATCH` edge boundaries) degrade gracefully, and
//! malformed or over-cap inputs are rejected or capped cleanly, without
//! panicking.
//!
//! Every test function name is prefixed `dxf_` so `cargo nextest run -E
//! 'test(dxf)'` selects this whole file (nextest's `test()` predicate matches
//! the bare test function name, not the binary name).

use proptest::prelude::*;
use reticle_geometry::{LayerId, Point};
use reticle_io::WarningKind;
use reticle_io::dxf::{
    Dxf, MAX_ARC_SEGMENTS, MAX_INPUT_BYTES, MAX_SHAPE_VERTICES, MIN_ARC_SEGMENTS,
};
use reticle_model::{Importer, ShapeKind};
use std::fmt::Write as _;

const OUTLINE: LayerId = LayerId::new(0, 0);
const WIRES: LayerId = LayerId::new(1, 0);
const PADS: LayerId = LayerId::new(2, 0);
const MARKS: LayerId = LayerId::new(3, 0);

/// Distance (in DBU) from a point to a center, as `f64`.
fn dist(p: Point, cx: f64, cy: f64) -> f64 {
    let dx = f64::from(p.x) - cx;
    let dy = f64::from(p.y) - cy;
    dx.hypot(dy)
}

// ---------------------------------------------------------------------------
// Fixture parsing: expected shapes and the layer mapping (success bar #1).
// ---------------------------------------------------------------------------

#[test]
fn dxf_parses_basic_fixture_into_expected_shapes_and_layers() {
    let bytes = std::fs::read("tests/corpus/basic.dxf").expect("basic.dxf fixture present");
    let import = Dxf
        .import_with_warnings(&bytes)
        .expect("a well-formed DXF fixture must import cleanly");
    assert!(
        import.warnings.is_empty(),
        "a clean fixture should carry no warnings, got {:?}",
        import.warnings
    );
    let doc = import.document;

    assert_eq!(doc.cell_count(), 1, "every shape lives flat in TOP");
    assert_eq!(doc.top_cells(), &["TOP".to_string()]);
    let top = doc.cell("TOP").expect("shapes form a TOP cell");
    assert_eq!(top.shapes.len(), 4, "LWPOLYLINE, LINE, CIRCLE, ARC");

    match &top.shapes[0].kind {
        ShapeKind::Polygon(p) => {
            assert_eq!(
                p.vertices(),
                &[
                    Point::new(0, 0),
                    Point::new(100, 0),
                    Point::new(100, 50),
                    Point::new(0, 50),
                ]
            );
            assert_eq!(top.shapes[0].layer, OUTLINE);
        }
        other => panic!("expected the closed LWPOLYLINE as a Polygon, got {other:?}"),
    }

    match &top.shapes[1].kind {
        ShapeKind::Path(p) => {
            assert_eq!(p.points(), &[Point::new(0, 0), Point::new(100, 100)]);
            assert_eq!(p.width(), 0);
            assert_eq!(top.shapes[1].layer, WIRES);
        }
        other => panic!("expected the LINE as a Path, got {other:?}"),
    }

    match &top.shapes[2].kind {
        ShapeKind::Polygon(p) => {
            assert_eq!(top.shapes[2].layer, PADS);
            assert!((MIN_ARC_SEGMENTS..=MAX_ARC_SEGMENTS).contains(&p.len()));
            for &v in p.vertices() {
                let d = dist(v, 50.0, 50.0);
                assert!(
                    (d - 25.0).abs() < 2.0,
                    "circle vertex {v:?} off-radius: {d}"
                );
            }
        }
        other => panic!("expected the CIRCLE as a Polygon, got {other:?}"),
    }

    match &top.shapes[3].kind {
        ShapeKind::Path(p) => {
            assert_eq!(top.shapes[3].layer, MARKS);
            let pts = p.points();
            assert!((2..=MAX_ARC_SEGMENTS + 1).contains(&pts.len()));
            assert_eq!(*pts.first().unwrap(), Point::new(230, 200), "start angle 0");
            assert_eq!(*pts.last().unwrap(), Point::new(200, 230), "end angle 90");
            for &v in pts {
                let d = dist(v, 200.0, 200.0);
                assert!((d - 30.0).abs() < 2.0, "arc vertex {v:?} off-radius: {d}");
            }
        }
        other => panic!("expected the ARC as a Path, got {other:?}"),
    }

    // Layer mapping returned as data, in encounter order.
    let names: Vec<&str> = doc
        .technology()
        .layers
        .iter()
        .map(|l| l.name.as_str())
        .collect();
    assert_eq!(names, ["OUTLINE", "WIRES", "PADS", "MARKS"]);
    assert_eq!(doc.technology().layers[0].id, OUTLINE);
    assert_eq!(doc.technology().layers[3].id, MARKS);
}

#[test]
fn dxf_parses_classic_polyline_and_hatch_fixture() {
    let bytes =
        std::fs::read("tests/corpus/polyline_hatch.dxf").expect("polyline_hatch.dxf present");
    let import = Dxf
        .import_with_warnings(&bytes)
        .expect("a well-formed POLYLINE/HATCH fixture must import cleanly");
    assert!(
        import.warnings.is_empty(),
        "a clean fixture should carry no warnings, got {:?}",
        import.warnings
    );
    let doc = import.document;
    let top = doc.cell("TOP").expect("shapes form a TOP cell");
    assert_eq!(top.shapes.len(), 2, "the POLYLINE frame and the HATCH loop");

    match &top.shapes[0].kind {
        ShapeKind::Polygon(p) => {
            assert_eq!(
                p.vertices(),
                &[
                    Point::new(0, 0),
                    Point::new(40, 0),
                    Point::new(40, 40),
                    Point::new(0, 40),
                ]
            );
        }
        other => panic!("expected the closed POLYLINE as a Polygon, got {other:?}"),
    }
    match &top.shapes[1].kind {
        ShapeKind::Polygon(p) => {
            assert_eq!(
                p.vertices(),
                &[
                    Point::new(5, 5),
                    Point::new(35, 5),
                    Point::new(35, 35),
                    Point::new(5, 35),
                ]
            );
        }
        other => panic!("expected the HATCH polyline boundary as a Polygon, got {other:?}"),
    }

    let names: Vec<&str> = doc
        .technology()
        .layers
        .iter()
        .map(|l| l.name.as_str())
        .collect();
    assert_eq!(names, ["FRAME", "FILL"]);
}

// ---------------------------------------------------------------------------
// CIRCLE/ARC polygonization: bounded and accurate (success bar #2).
// ---------------------------------------------------------------------------

#[test]
fn dxf_circle_polygonizes_within_tolerance_and_bounded_vertices() {
    let src = b"0\nSECTION\n2\nENTITIES\n0\nCIRCLE\n8\nA\n10\n0.0\n20\n0.0\n40\n1000.0\n0\nENDSEC\n0\nEOF\n";
    let doc = Dxf.import(src).expect("a CIRCLE must import cleanly");
    let top = doc.cell("TOP").expect("the CIRCLE forms a TOP cell");
    let ShapeKind::Polygon(p) = &top.shapes[0].kind else {
        panic!("expected a Polygon");
    };
    assert!(
        p.len() >= MIN_ARC_SEGMENTS,
        "circle must not degenerate below the floor"
    );
    assert!(
        p.len() <= MAX_ARC_SEGMENTS,
        "circle must respect the hard ceiling"
    );
    for &v in p.vertices() {
        let d = dist(v, 0.0, 0.0);
        assert!(
            (d - 1000.0).abs() < 2.0,
            "vertex {v:?} strayed from the radius: {d}"
        );
    }
}

#[test]
fn dxf_huge_radius_circle_clamps_to_segment_ceiling() {
    let src =
        b"0\nSECTION\n2\nENTITIES\n0\nCIRCLE\n8\nA\n10\n0.0\n20\n0.0\n40\n2000000000.0\n0\nENDSEC\n0\nEOF\n";
    let doc = Dxf
        .import(src)
        .expect("an enormous radius is still bounded geometry, not a panic");
    let top = doc.cell("TOP").expect("the CIRCLE forms a TOP cell");
    let ShapeKind::Polygon(p) = &top.shapes[0].kind else {
        panic!("expected a Polygon");
    };
    assert_eq!(p.len(), MAX_ARC_SEGMENTS);
}

// ---------------------------------------------------------------------------
// Honest gaps degrade gracefully: bulge is ignored, HATCH edge boundaries skip.
// ---------------------------------------------------------------------------

#[test]
fn dxf_lwpolyline_bulge_is_ignored_not_fatal() {
    let src = b"0\nSECTION\n2\nENTITIES\n0\nLWPOLYLINE\n8\nA\n70\n1\n\
                10\n0.0\n20\n0.0\n42\n0.5\n10\n10.0\n20\n0.0\n10\n10.0\n20\n10.0\n\
                0\nENDSEC\n0\nEOF\n";
    let doc = Dxf
        .import(src)
        .expect("a bulge value must not be fatal; it is ignored, drawn as a straight edge");
    let top = doc.cell("TOP").expect("the LWPOLYLINE forms a TOP cell");
    match &top.shapes[0].kind {
        ShapeKind::Polygon(p) => {
            assert_eq!(
                p.vertices(),
                &[Point::new(0, 0), Point::new(10, 0), Point::new(10, 10)]
            );
        }
        other => panic!("expected a Polygon, got {other:?}"),
    }
}

#[test]
fn dxf_hatch_edge_boundary_is_skipped_with_warning() {
    let src = b"0\nSECTION\n2\nENTITIES\n0\nHATCH\n8\nA\n91\n1\n92\n1\n93\n2\n72\n1\n\
                10\n0.0\n20\n0.0\n11\n10.0\n21\n10.0\n0\nENDSEC\n0\nEOF\n";
    let import = Dxf
        .import_with_warnings(src)
        .expect("an edge-type HATCH boundary is recoverable, not fatal");
    assert!(
        import
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::UnsupportedFeature),
        "skipping an edge-type boundary must warn: {:?}",
        import.warnings
    );
    assert!(
        import.document.cell("TOP").is_none(),
        "no polyline boundary loop was found, so nothing was drawn"
    );
}

// ---------------------------------------------------------------------------
// Malformed input: structured errors, never a panic (success bar #3).
// ---------------------------------------------------------------------------

#[test]
fn dxf_truncated_missing_value_line_errors() {
    let src = b"0\nSECTION\n2\nENTITIES\n0\nLINE\n8\n";
    Dxf.import(src)
        .expect_err("a trailing group code with no value line is truncated input");
}

#[test]
fn dxf_bad_pair_non_numeric_group_code_errors() {
    let src = b"0\nSECTION\n2\nENTITIES\nx\nLINE\n0\nENDSEC\n0\nEOF\n";
    Dxf.import(src)
        .expect_err("a non-numeric group code is malformed input");
}

#[test]
fn dxf_bad_number_token_errors() {
    let src = b"0\nSECTION\n2\nENTITIES\n0\nLINE\n8\nA\n10\nabc\n20\n0.0\n11\n1.0\n21\n1.0\n\
                0\nENDSEC\n0\nEOF\n";
    Dxf.import(src)
        .expect_err("a non-numeric coordinate is malformed input");
}

#[test]
fn dxf_unterminated_polyline_errors() {
    let src = b"0\nSECTION\n2\nENTITIES\n0\nPOLYLINE\n8\nA\n70\n1\n\
                0\nVERTEX\n8\nA\n10\n0.0\n20\n0.0\n0\nENDSEC\n0\nEOF\n";
    Dxf.import(src)
        .expect_err("a POLYLINE never closed by a matching SEQEND is malformed input");
}

#[test]
fn dxf_unterminated_polyline_at_eof_errors() {
    // No ENDSEC/EOF at all after the VERTEX: the file simply stops.
    let src = b"0\nSECTION\n2\nENTITIES\n0\nPOLYLINE\n8\nA\n70\n1\n\
                0\nVERTEX\n8\nA\n10\n0.0\n20\n0.0\n";
    Dxf.import(src)
        .expect_err("a POLYLINE left open at end of input is malformed input");
}

#[test]
fn dxf_oversized_input_is_refused_before_parsing() {
    let bytes = vec![b'0'; MAX_INPUT_BYTES + 1];
    let err = Dxf
        .import_with_warnings(&bytes)
        .expect_err("an oversized input must be refused");
    assert!(!err.to_string().is_empty(), "the refusal carries a message");
}

#[test]
fn dxf_invalid_utf8_errors() {
    let bytes: &[u8] = &[0xFF, 0xFE, b'0', b'\n'];
    Dxf.import(bytes)
        .expect_err("non-UTF-8 bytes are malformed input");
}

#[test]
fn dxf_over_cap_polygon_vertices_is_dropped_with_warning() {
    let mut src = String::from("0\nSECTION\n2\nENTITIES\n0\nLWPOLYLINE\n8\nA\n70\n1\n");
    for i in 0..=MAX_SHAPE_VERTICES {
        let _ = writeln!(src, "10\n{}.0\n20\n{}.0", i % 1000, (i * 7) % 1000);
    }
    src.push_str("0\nENDSEC\n0\nEOF\n");

    let import = Dxf
        .import_with_warnings(src.as_bytes())
        .expect("an over-cap polyline is recoverable, not fatal");
    assert!(
        import
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::LimitExceeded),
        "an over-cap polyline must warn: {:?}",
        import.warnings
    );
    assert!(
        import.document.cell("TOP").is_none(),
        "the oversized polyline was dropped, not truncated-and-kept"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Arbitrary bytes never panic the importer, whatever they contain.
    #[test]
    fn dxf_import_never_panics_on_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let result = Dxf.import(&bytes);
        prop_assert!(result.is_ok() || result.is_err());
    }

    /// Arbitrary *pair-shaped* text (alternating small-int/short-token lines,
    /// the actual shape of DXF) never panics either, exercising the entity
    /// dispatch and POLYLINE/HATCH state machines much more often than raw
    /// bytes would.
    #[test]
    fn dxf_import_never_panics_on_pairish_text(
        pairs in proptest::collection::vec((0i32..120, "[A-Za-z0-9_.-]{0,8}"), 0..80)
    ) {
        let mut src = String::new();
        for (code, value) in pairs {
            let _ = writeln!(src, "{code}\n{value}");
        }
        let result = Dxf.import(src.as_bytes());
        prop_assert!(result.is_ok() || result.is_err());
    }
}
