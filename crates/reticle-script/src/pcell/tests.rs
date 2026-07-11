//! Tests for the sandboxed PCell producer.
//!
//! The suite proves the security-critical properties: a valid param-driven script produces
//! the expected top cell with valid provenance; produce is deterministic; and every hostile
//! or runaway input (infinite loop, exponential memory growth, output-cap overflow, filesystem
//! / `import` access, malformed source, no top cell) is rejected as a clean [`ProduceError`]
//! in bounded time, never a hang or a panic.

use std::time::{Duration, Instant};

use reticle_gen::{FieldSchema, PCellDef, ParamSchema};
use reticle_geometry::LayerId;
use reticle_model::{ShapeKind, Technology};
use serde_json::json;

use super::{ProduceError, SandboxLimits, produce};

/// A script that builds a top cell "BOX" with two rectangles sized by the `w` parameter.
const BOXES_SCRIPT: &str = r#"
create_cell("BOX");
add_rect("BOX", 1, 0, 0, 0, w, w);
add_rect("BOX", 2, 0, 10, 10, w - 10, w - 10);
set_top_cells(["BOX"]);
"#;

/// A param-driven pixel-array generator: the `examples/param_cell.rhai` body with its leading
/// `let` parameter block removed, so every parameter is an injected scope variable.
const SENSOR_SCRIPT: &str = r#"
create_cell("PIXEL");
add_rect("PIXEL", 1, 0, 0, 0, pixel_w, pixel_h);
let via_lo_x = (pixel_w - via) / 2;
let via_lo_y = (pixel_h - via) / 2;
add_rect("PIXEL", 2, 0, via_lo_x, via_lo_y, via_lo_x + via, via_lo_y + via);
create_cell("SENSOR");
add_array("SENSOR", "PIXEL", 0, 0, columns, rows, pitch_x, pitch_y);
set_top_cells(["SENSOR"]);
"#;

fn int_field(name: &str, default: i64) -> FieldSchema {
    FieldSchema::int(name, name, default, 0, 1_000_000, "dbu")
}

fn schema(id: &str, fields: Vec<FieldSchema>) -> ParamSchema {
    ParamSchema {
        generator_id: id.to_owned(),
        title: id.to_owned(),
        description: "test".to_owned(),
        fields,
    }
}

/// A `PCellDef` with the given id, schema fields, and script.
fn def(id: &str, fields: Vec<FieldSchema>, script: &str) -> PCellDef {
    PCellDef {
        id: id.to_owned(),
        title: id.to_owned(),
        description: "test".to_owned(),
        schema: schema(id, fields),
        script: script.to_owned(),
        engine_version: "8.2.0".to_owned(),
    }
}

fn boxes_def() -> PCellDef {
    def("user.boxes", vec![int_field("w", 400)], BOXES_SCRIPT)
}

fn sensor_def() -> PCellDef {
    def(
        "user.sensor",
        vec![
            int_field("pixel_w", 800),
            int_field("pixel_h", 800),
            int_field("via", 200),
            int_field("columns", 8),
            int_field("rows", 6),
            int_field("pitch_x", 1000),
            int_field("pitch_y", 1000),
        ],
        SENSOR_SCRIPT,
    )
}

/// A no-parameter def wrapping an arbitrary script, for the rejection tests.
fn bare(script: &str) -> PCellDef {
    def("user.bare", vec![], script)
}

#[test]
fn produces_expected_top_cell_and_valid_meta() {
    let d = boxes_def();
    let params = json!({ "w": 400 });
    let (cell, meta) = produce(
        &d,
        &params,
        &Technology::default(),
        SandboxLimits::default(),
    )
    .expect("a valid script produces a cell");

    // The produced top cell is the one the script declared, with exactly its geometry.
    assert_eq!(cell.name, "BOX");
    assert_eq!(cell.shapes.len(), 2);
    assert_eq!(cell.instances.len(), 0);
    assert_eq!(cell.arrays.len(), 0);
    assert_eq!(cell.shapes[0].layer, LayerId::new(1, 0));
    match &cell.shapes[0].kind {
        ShapeKind::Rect(r) => {
            assert_eq!((r.min.x, r.min.y, r.max.x, r.max.y), (0, 0, 400, 400));
        }
        other => panic!("expected a rect, got {other:?}"),
    }

    // The stamped provenance is well-formed and matches the def identity.
    assert!(meta.has_valid_hash(), "param_hash must be a valid digest");
    assert_eq!(meta.generator_id, "user.boxes");
    assert_eq!(meta.script_ref.as_deref(), Some("user.boxes"));
    assert_eq!(meta.engine_version, "8.2.0");
    assert_eq!(meta.param_hash, d.param_hash(&params));
}

#[test]
fn param_injection_by_schema_field_name_drives_geometry() {
    // Explicit params different from the schema defaults prove the values are injected by
    // field name and actually drive the produced geometry (this mirrors param_cell.rhai).
    let d = sensor_def();
    let params = json!({
        "pixel_w": 500, "pixel_h": 500, "via": 100,
        "columns": 3, "rows": 2, "pitch_x": 600, "pitch_y": 600
    });
    let (cell, _meta) = produce(
        &d,
        &params,
        &Technology::default(),
        SandboxLimits::default(),
    )
    .expect("the sensor script produces a top cell");

    assert_eq!(cell.name, "SENSOR");
    assert_eq!(cell.arrays.len(), 1);
    let array = &cell.arrays[0];
    assert_eq!(array.cell, "PIXEL");
    assert_eq!(array.columns, 3, "columns param must drive the array");
    assert_eq!(array.rows, 2, "rows param must drive the array");
    assert_eq!(array.column_pitch, 600);
    assert_eq!(array.row_pitch, 600);
}

#[test]
fn absent_params_fall_back_to_schema_defaults() {
    // With no params supplied, each field's schema default is injected, so the script still
    // produces its geometry (columns=8, rows=6 from the defaults).
    let d = sensor_def();
    let (cell, _meta) = produce(
        &d,
        &json!({}),
        &Technology::default(),
        SandboxLimits::default(),
    )
    .expect("defaults fill in for absent params");
    assert_eq!(cell.arrays[0].columns, 8);
    assert_eq!(cell.arrays[0].rows, 6);
}

#[test]
fn produce_is_deterministic_in_geometry_and_hash() {
    let d = boxes_def();
    let params = json!({ "w": 400 });
    let tech = Technology::default();

    let (cell_a, meta_a) = produce(&d, &params, &tech, SandboxLimits::default()).expect("run 1");
    let (cell_b, meta_b) = produce(&d, &params, &tech, SandboxLimits::default()).expect("run 2");

    // Identical def + params yield byte-identical geometry and identical provenance.
    assert_eq!(cell_a, cell_b, "geometry must be deterministic");
    assert_eq!(meta_a, meta_b, "provenance must be deterministic");
    assert_eq!(meta_a.param_hash, meta_b.param_hash);
}

#[test]
fn different_params_change_geometry_and_hash() {
    let d = boxes_def();
    let tech = Technology::default();
    let (cell_400, meta_400) =
        produce(&d, &json!({ "w": 400 }), &tech, SandboxLimits::default()).expect("w=400");
    let (cell_500, meta_500) =
        produce(&d, &json!({ "w": 500 }), &tech, SandboxLimits::default()).expect("w=500");

    assert_ne!(
        cell_400, cell_500,
        "a different param must change the geometry"
    );
    assert_ne!(
        meta_400.param_hash, meta_500.param_hash,
        "a different param must change the identity hash"
    );
}

#[test]
fn infinite_loop_is_rejected_by_the_operation_cap_in_bounded_time() {
    let d = bare("let i = 0; loop { i += 1; }");
    let limits = SandboxLimits {
        max_operations: 100_000,
        ..SandboxLimits::default()
    };

    let start = Instant::now();
    let err = produce(&d, &json!({}), &Technology::default(), limits)
        .expect_err("an infinite loop must be rejected");
    let elapsed = start.elapsed();

    match &err {
        ProduceError::LimitExceeded(msg) => {
            assert!(msg.contains("operation"), "message names the bound: {msg}");
        }
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
    // The whole point: the op cap makes this terminate promptly, not hang.
    assert!(
        elapsed < Duration::from_secs(5),
        "must be bounded, took {elapsed:?}"
    );
}

#[test]
fn exponential_string_growth_is_rejected_by_the_memory_cap() {
    // `s += s` doubles memory each operation: ~40 iterations reach a terabyte, far under any
    // operation count. The op cap alone cannot catch this; the string-size cap must.
    let d = bare(r#"let s = "x"; loop { s += s; }"#);
    let start = Instant::now();
    let err = produce(
        &d,
        &json!({}),
        &Technology::default(),
        SandboxLimits::default(),
    )
    .expect_err("exponential growth must be rejected");
    let elapsed = start.elapsed();

    match &err {
        ProduceError::LimitExceeded(msg) => {
            assert!(msg.contains("size limit"), "message names the bound: {msg}");
        }
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
    assert!(
        elapsed < Duration::from_secs(5),
        "must be bounded, took {elapsed:?}"
    );
}

#[test]
fn too_many_shapes_is_rejected_by_the_output_cap() {
    // A high operation cap so the shape cap, not the op cap, is what trips.
    let d = bare(
        r#"
        create_cell("C");
        for i in 0..40 { add_rect("C", 1, 0, 0, 0, 10, 10); }
        set_top_cells(["C"]);
        "#,
    );
    let limits = SandboxLimits {
        max_operations: 50_000_000,
        max_shapes: 5,
        max_cells: 4_096,
    };
    let err = produce(&d, &json!({}), &Technology::default(), limits)
        .expect_err("over-cap shape output must be rejected");
    match &err {
        ProduceError::LimitExceeded(msg) => {
            assert!(msg.contains("shape"), "names the bound: {msg}");
        }
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
}

#[test]
fn too_many_cells_is_rejected_by_the_output_cap() {
    let d = bare(
        r#"
        for i in 0..10 { create_cell("C" + i); }
        set_top_cells(["C0"]);
        "#,
    );
    let limits = SandboxLimits {
        max_operations: 50_000_000,
        max_shapes: 2_000_000,
        max_cells: 3,
    };
    let err = produce(&d, &json!({}), &Technology::default(), limits)
        .expect_err("over-cap cell output must be rejected");
    match &err {
        ProduceError::LimitExceeded(msg) => assert!(msg.contains("cell"), "names the bound: {msg}"),
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
}

#[test]
fn no_top_cell_is_reported_cleanly() {
    // The script builds a cell but never declares a top cell.
    let d = bare(r#"create_cell("ORPHAN"); add_rect("ORPHAN", 1, 0, 0, 0, 10, 10);"#);
    let err = produce(
        &d,
        &json!({}),
        &Technology::default(),
        SandboxLimits::default(),
    )
    .expect_err("a script with no top cell must error");
    assert_eq!(err, ProduceError::NoTopCell);
}

#[test]
fn declared_top_cell_that_does_not_exist_is_no_top_cell() {
    let d = bare(r#"create_cell("REAL"); set_top_cells(["MISSING"]);"#);
    let err = produce(
        &d,
        &json!({}),
        &Technology::default(),
        SandboxLimits::default(),
    )
    .expect_err("a dangling top-cell name must error");
    assert_eq!(err, ProduceError::NoTopCell);
}

#[test]
fn filesystem_and_import_access_is_blocked_with_no_host_escape() {
    let tech = Technology::default();

    // `import` cannot read a `.rhai` file from disk: the dummy resolver makes it a clean
    // script error, not a filesystem read.
    let imp = bare(r#"import "evil" as e; e::run();"#);
    let err = produce(&imp, &json!({}), &tech, SandboxLimits::default())
        .expect_err("import must be blocked");
    assert!(
        matches!(err, ProduceError::Script(_)),
        "blocked import is a Script error, got {err:?}"
    );

    // No host filesystem function is exposed, so a call to one is an ordinary unknown-function
    // script error (there is nothing to escape through).
    let openf = bare(r#"open("/etc/passwd");"#);
    let err = produce(&openf, &json!({}), &tech, SandboxLimits::default())
        .expect_err("there is no fs function to call");
    assert!(
        matches!(err, ProduceError::Script(_)),
        "unknown fs function is a Script error, got {err:?}"
    );
}

#[test]
fn malformed_script_is_a_script_error_not_a_panic() {
    let d = bare("this is not @# valid rhai");
    let err = produce(
        &d,
        &json!({}),
        &Technology::default(),
        SandboxLimits::default(),
    )
    .expect_err("a malformed script must error");
    assert!(
        matches!(err, ProduceError::Script(_)),
        "malformed source is a Script error, got {err:?}"
    );
}

#[test]
fn hostile_deeply_nested_params_are_rejected_without_overflow() {
    // A parameter value nested past the depth cap is rejected up front as InvalidParams, so
    // nothing downstream recurses over it (neither scope injection nor param-hash
    // canonicalization) and the stack cannot overflow (which would abort the tab). Depth 64 is
    // over the cap of 32 yet shallow enough for the test harness itself to build and drop.
    let mut nested = json!(0);
    for _ in 0..64 {
        nested = json!([nested]);
    }
    let d = def("user.deep", vec![int_field("w", 1)], BOXES_SCRIPT);
    let params = json!({ "w": nested });
    let err = produce(
        &d,
        &params,
        &Technology::default(),
        SandboxLimits::default(),
    )
    .expect_err("over-deep params must be rejected");
    match &err {
        ProduceError::InvalidParams(msg) => {
            assert!(msg.contains("depth"), "message names the bound: {msg}");
        }
        other => panic!("expected InvalidParams, got {other:?}"),
    }
}

#[test]
fn adversarial_inputs_never_panic() {
    // A permanent, WSL-free companion to the `pcell_produce` fuzz target: a batch of hostile
    // (script, params) pairs run through `produce` under tight limits. Reaching the end of the
    // loop is the assertion, a panic on any input would abort the test (and, in wasm, the tab).
    let tight = SandboxLimits {
        max_operations: 100_000,
        max_shapes: 20_000,
        max_cells: 500,
    };
    let cases: &[(&str, serde_json::Value)] = &[
        ("loop {}", json!({})),
        ("let i = 0; loop { i += 1; }", json!({})),
        (r#"let s = "x"; loop { s += s; }"#, json!({})),
        ("let a = [1]; loop { a += a; }", json!({})),
        ("fn r(n) { r(n + 1) } r(0);", json!({})),
        (r#"import "x" as y; y::f();"#, json!({})),
        (r#"open("/etc/passwd");"#, json!({})),
        (r#"eval("create_cell(\"E\");");"#, json!({})),
        ("@#$%^&*()", json!({})),
        ("create_cell(", json!({})),
        ("create_cell(\"C\"); create_cell(\"C\");", json!({})),
        ("", json!({})),
        (r#"set_top_cells(["GHOST"]);"#, json!({})),
        (
            r#"create_cell("C"); add_rect("C",1,0,0,0,w,w); set_top_cells(["C"]);"#,
            json!({ "w": "not a number" }),
        ),
        (
            r#"create_cell("C"); add_rect("C",1,0,0,0,w,w); set_top_cells(["C"]);"#,
            json!({ "w": [1, 2, 3] }),
        ),
        (
            r#"create_cell("C"); add_rect("C",1,0,0,0,w,w); set_top_cells(["C"]);"#,
            json!({ "w": { "nested": { "deep": 1 } } }),
        ),
        (
            r#"create_cell("C"); add_rect("C",1,0,0,0,w,w); set_top_cells(["C"]);"#,
            json!({ "w": 9_999_999_999_i64 }),
        ),
        (
            r#"create_cell("C"); add_rect("C",-1,0,0,0,10,10); set_top_cells(["C"]);"#,
            json!({}),
        ),
        (
            r#"let i=0; while i<100000 { create_cell("c"+i); i+=1; }"#,
            json!({}),
        ),
        (
            r#"create_cell("C"); let i=0; while i<100000 { add_rect("C",1,0,0,0,1,1); i+=1; } set_top_cells(["C"]);"#,
            json!({}),
        ),
    ];
    let tech = Technology::default();
    for (script, params) in cases {
        // Explicitly discard the result: the only thing under test is the absence of a panic.
        let _ = produce(&bare(script), params, &tech, tight);
    }
}

#[test]
fn sandbox_limits_have_conservative_defaults() {
    let l = SandboxLimits::default();
    assert!(l.max_operations > 0 && l.max_shapes > 0 && l.max_cells > 0);
}
