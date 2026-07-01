//! Integration tests for the Reticle scripting API.
//!
//! Each test drives [`ScriptEngine`] with an inline script and asserts against the
//! resulting [`Document`](reticle_model::Document): that geometry was created with
//! the expected counts and bounding boxes, that DRC reports the expected number of
//! violations, that GDSII export produces bytes, and that a malformed script
//! returns an error rather than panicking.

use reticle_geometry::{Point, Rect};
use reticle_model::ShapeKind;
use reticle_script::{ScriptEngine, ScriptError};

/// Creating a cell and shapes reflects into the document with correct counts and
/// a correct bounding box.
#[test]
fn creates_geometry_and_reflects_document() {
    let mut engine = ScriptEngine::new();
    engine
        .eval(
            r#"
            create_cell("TOP");
            add_rect("TOP", 1, 0, 0, 0, 100, 200);
            add_rect("TOP", 1, 0, 50, 50, 300, 400);
            add_polygon("TOP", 2, 0, [0, 0, 500, 0, 500, 500]);
            add_path("TOP", 3, 0, 20, [0, 0, 100, 0, 100, 100]);
            "#,
        )
        .expect("script should evaluate");

    let doc = engine.document();
    assert_eq!(doc.cell_count(), 1, "one cell created");

    let top = doc.cell("TOP").expect("TOP cell exists");
    assert_eq!(top.shapes.len(), 4, "four shapes added");

    // First shape is the 100x200 rect.
    match &top.shapes[0].kind {
        ShapeKind::Rect(r) => {
            assert_eq!(*r, Rect::new(Point::new(0, 0), Point::new(100, 200)));
        }
        other => panic!("expected a rect, got {other:?}"),
    }

    // The cell bbox spans all shapes. The rects and polygon reach (500, 500); the
    // width-20 path's centerline box (0,0)-(100,100) is expanded by its half-width
    // (10) on every side, so the lower-left corner is (-10, -10).
    let bbox = doc.cell_bbox("TOP").expect("bbox exists");
    assert_eq!(bbox, Rect::new(Point::new(-10, -10), Point::new(500, 500)));
}

/// Bounding-box and count query functions are callable from a script and return
/// values the script can act on; the created document matches.
#[test]
fn query_functions_report_document_state() {
    let mut engine = ScriptEngine::new();
    engine
        .eval(
            r#"
            create_cell("A");
            add_rect("A", 1, 0, 10, 20, 110, 220);

            // Query from within the script and stash results into a marker cell's
            // geometry so the host can read them back deterministically.
            let n = shape_count("A");           // 1
            let bb = cell_bbox("A");             // [10, 20, 110, 220]

            create_cell("REPORT");
            add_rect("REPORT", 9, 0, n, bb[0], bb[2], bb[3]);
            "#,
        )
        .expect("script should evaluate");

    let doc = engine.document();
    assert_eq!(doc.cell_count(), 2);
    assert!(doc.cell("A").is_some());

    let report = doc.cell("REPORT").expect("REPORT exists");
    match &report.shapes[0].kind {
        // Encoded [n=1, min_x=10, max_x=110, max_y=220] -> Rect((1,10),(110,220)).
        ShapeKind::Rect(r) => {
            assert_eq!(*r, Rect::new(Point::new(1, 10), Point::new(110, 220)));
        }
        other => panic!("expected a rect, got {other:?}"),
    }
}

/// A script that adds an instance and an array records them on the parent cell,
/// and `flatten_count` matches the expanded geometry.
#[test]
fn instances_and_arrays_expand() {
    let mut engine = ScriptEngine::new();
    engine
        .eval(
            r#"
            create_cell("LEAF");
            add_rect("LEAF", 1, 0, 0, 0, 10, 10);   // 1 shape

            create_cell("TOP");
            add_instance("TOP", "LEAF", 100, 100);   // +1 leaf shape when flat
            add_array("TOP", "LEAF", 0, 0, 4, 3, 20, 20); // +12 leaf shapes when flat

            create_cell("MARK");
            add_rect("MARK", 5, 0, 0, 0, flatten_count("TOP"), 1);
            "#,
        )
        .expect("script should evaluate");

    let doc = engine.document();
    let top = doc.cell("TOP").expect("TOP exists");
    assert_eq!(top.instances.len(), 1);
    assert_eq!(top.arrays.len(), 1);

    // 1 (instance) + 12 (4x3 array) = 13 flattened leaf shapes.
    assert_eq!(doc.flatten("TOP").len(), 13);

    // And the script observed the same count (encoded as the rect's max.x).
    let mark = doc.cell("MARK").expect("MARK exists");
    match &mark.shapes[0].kind {
        ShapeKind::Rect(r) => assert_eq!(r.max.x, 13),
        other => panic!("expected a rect, got {other:?}"),
    }
}

/// `flatten_into` materializes a hierarchy into a new flat cell.
#[test]
fn flatten_into_materializes_a_cell() {
    let mut engine = ScriptEngine::new();
    engine
        .eval(
            r#"
            create_cell("LEAF");
            add_rect("LEAF", 1, 0, 0, 0, 10, 10);
            create_cell("TOP");
            add_array("TOP", "LEAF", 0, 0, 2, 2, 20, 20);  // 4 leaf shapes flat
            let written = flatten_into("TOP", "FLAT");
            create_cell("N");
            add_rect("N", 0, 0, 0, 0, written, 1);
            "#,
        )
        .expect("script should evaluate");

    let doc = engine.document();
    let flat = doc.cell("FLAT").expect("FLAT exists");
    assert_eq!(flat.shapes.len(), 4, "2x2 array flattened to 4 shapes");
    assert!(flat.instances.is_empty() && flat.arrays.is_empty());
}

/// Loading a technology with rules and running DRC over deliberately bad geometry
/// yields a violation count the script can read back.
#[test]
fn drc_reports_violation_count() {
    let mut engine = ScriptEngine::new();
    engine
        .eval(
            r#"
            load_technology(
                "dbu_per_micron 1000\n" +
                "layer 1 0 metal1 4488FFFF\n" +
                "rule width   1 0 100\n" +
                "rule spacing 1 0 140\n"
            );

            create_cell("BAD");
            add_rect("BAD", 1, 0, 0, 0, 50, 1000);      // width 50 < 100
            add_rect("BAD", 1, 0, 110, 0, 400, 1000);   // gap 60 < 140

            let v = run_drc("BAD");
            create_cell("VCOUNT");
            add_rect("VCOUNT", 0, 0, 0, 0, v, 1);
            "#,
        )
        .expect("script should evaluate");

    let doc = engine.document();
    let vcount = doc.cell("VCOUNT").expect("VCOUNT exists");
    let count = match &vcount.shapes[0].kind {
        ShapeKind::Rect(r) => r.max.x,
        other => panic!("expected a rect, got {other:?}"),
    };
    // One width violation (the sliver) + one spacing violation (the 60 DBU gap).
    assert_eq!(count, 2, "expected exactly two violations, got {count}");
}

/// `add_*_rule` helpers accumulate rules without a technology file, and `run_drc`
/// uses them.
#[test]
fn programmatic_rules_drive_drc() {
    let mut engine = ScriptEngine::new();
    engine
        .eval(
            r#"
            add_width_rule("min-width", 1, 0, 100);
            create_cell("C");
            add_rect("C", 1, 0, 0, 0, 40, 40);   // 40 < 100 on both sides
            create_cell("R");
            add_rect("R", 0, 0, 0, 0, rule_count(), run_drc("C"));
            "#,
        )
        .expect("script should evaluate");

    let doc = engine.document();
    let r = doc.cell("R").expect("R exists");
    match &r.shapes[0].kind {
        // rule_count() == 1 -> max.x; run_drc("C") == 1 violation -> max.y.
        ShapeKind::Rect(rect) => {
            assert_eq!(rect.max.x, 1, "one rule registered");
            assert_eq!(rect.max.y, 1, "one width violation");
        }
        other => panic!("expected a rect, got {other:?}"),
    }
}

/// A script that exports GDSII yields a non-empty blob.
#[test]
fn exports_gdsii_bytes() {
    let mut engine = ScriptEngine::new();
    engine
        .eval(
            r#"
            create_cell("TOP");
            add_rect("TOP", 1, 0, 0, 0, 1000, 1000);
            set_top_cells(["TOP"]);
            let bytes = export_gds();

            // Record the byte length so the host can assert it is non-empty.
            create_cell("LEN");
            add_rect("LEN", 0, 0, 0, 0, bytes.len(), 1);
            "#,
        )
        .expect("script should evaluate");

    let doc = engine.document();
    let len_cell = doc.cell("LEN").expect("LEN exists");
    let len = match &len_cell.shapes[0].kind {
        ShapeKind::Rect(r) => r.max.x,
        other => panic!("expected a rect, got {other:?}"),
    };
    assert!(len > 0, "GDSII export produced {len} bytes; expected > 0");
}

/// A malformed script returns an error rather than panicking.
#[test]
fn malformed_script_returns_error() {
    let mut engine = ScriptEngine::new();
    let result = engine.eval("this is not valid rhai @@@ )(");
    assert!(
        matches!(result, Err(ScriptError::Eval(_))),
        "malformed script should return ScriptError::Eval, got {result:?}"
    );
    // The engine is still usable afterwards.
    engine
        .eval(r#"create_cell("OK");"#)
        .expect("engine recovers after a failed script");
    assert!(engine.document().cell("OK").is_some());
}

/// A runtime trap inside a registered function (unknown cell) surfaces as an error,
/// not a panic.
#[test]
fn runtime_trap_returns_error() {
    let mut engine = ScriptEngine::new();
    // Adding a shape to a cell that does not exist is a model error, surfaced as a
    // rhai runtime error.
    let result = engine.eval(r#"add_rect("MISSING", 1, 0, 0, 0, 10, 10);"#);
    assert!(
        matches!(result, Err(ScriptError::Eval(_))),
        "unknown-cell edit should error, got {result:?}"
    );
}

/// Repeated `eval` calls compose against the same document.
#[test]
fn evals_compose() {
    let mut engine = ScriptEngine::new();
    engine.eval(r#"create_cell("TOP");"#).expect("first eval");
    engine
        .eval(r#"add_rect("TOP", 1, 0, 0, 0, 10, 10);"#)
        .expect("second eval");
    engine
        .eval(r#"add_rect("TOP", 1, 0, 10, 10, 20, 20);"#)
        .expect("third eval");

    let doc = engine.document();
    assert_eq!(doc.cell("TOP").expect("TOP exists").shapes.len(), 2);
}
