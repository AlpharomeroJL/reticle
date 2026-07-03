//! Focused, per-command-family tests for the agent command surface, plus the
//! replay round-trip contract.
//!
//! Each test drives a [`Session`] through the public [`Session::apply`] entry point
//! and asserts the response shape, the revision behaviour, and the stable-id
//! semantics. These complement the property test in `proptest_commands.rs`.

use reticle_agent_api::args::{
    AlignArg, AxisArg, BooleanOpArg, LayerArg, OrientationArg, PointArg, RectArg, TransformArg,
};
use reticle_agent_api::{
    AgentCommand, AgentResponse, CommandResult, ElementId, ErrorCode, Session, replay,
    transcript_of, verify_replay,
};

/// A layer/datatype argument.
fn layer(l: u16, d: u16) -> LayerArg {
    LayerArg {
        layer: l,
        datatype: d,
    }
}

/// A rectangle argument from two corners.
fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> RectArg {
    RectArg {
        min: PointArg { x: x0, y: y0 },
        max: PointArg { x: x1, y: y1 },
    }
}

/// Unwraps a successful `Ok` response and returns the affected ids.
fn affected(result: CommandResult) -> Vec<ElementId> {
    match result.expect("command succeeded") {
        AgentResponse::Ok { affected, .. } => affected,
        other => panic!("expected Ok response, got {other:?}"),
    }
}

/// Unwraps a successful `Data` response and returns its JSON value.
fn data(result: CommandResult) -> serde_json::Value {
    match result.expect("command succeeded") {
        AgentResponse::Data { value, .. } => value,
        other => panic!("expected Data response, got {other:?}"),
    }
}

/// Unwraps a successful `Blob` response and returns its bytes.
fn blob(result: CommandResult) -> Vec<u8> {
    match result.expect("command succeeded") {
        AgentResponse::Blob { bytes, .. } => bytes,
        other => panic!("expected Blob response, got {other:?}"),
    }
}

// ===== create / cell family =================================================

#[test]
fn create_cell_advances_revision_and_rejects_duplicates() {
    let mut s = Session::new();
    assert_eq!(s.revision(), 0);

    let r = s.apply(AgentCommand::CreateCell { name: "top".into() });
    assert!(affected(r).is_empty(), "creating a cell affects no element");
    assert_eq!(s.revision(), 1, "a mutation advances the revision");
    assert!(s.document().cell("top").is_some());

    // A duplicate name is an InvalidArgument, and the revision does not move.
    let err = s
        .apply(AgentCommand::CreateCell { name: "top".into() })
        .expect_err("duplicate cell rejected");
    assert_eq!(err.code, ErrorCode::InvalidArgument);
    assert_eq!(
        s.revision(),
        1,
        "a failed command leaves the revision alone"
    );
}

#[test]
fn delete_cell_removes_it_and_reports_missing() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    s.apply(AgentCommand::DeleteCell { name: "top".into() })
        .expect("delete top");
    assert!(s.document().cell("top").is_none());

    let err = s
        .apply(AgentCommand::DeleteCell {
            name: "nope".into(),
        })
        .expect_err("deleting a missing cell errors");
    assert_eq!(err.code, ErrorCode::NoSuchCell);
}

// ===== add-geometry family ==================================================

#[test]
fn add_rect_returns_a_resolvable_id() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    let ids = affected(s.apply(AgentCommand::AddRect {
        cell: "top".into(),
        layer: layer(1, 0),
        rect: rect(0, 0, 100, 100),
    }));
    assert_eq!(ids.len(), 1, "one shape, one id");
    assert_eq!(s.document().cell("top").unwrap().shapes.len(), 1);

    // The id resolves in a query: the returned shape carries the same id.
    let value = data(s.apply(AgentCommand::QueryShapes {
        cell: "top".into(),
        layer: None,
        region: None,
    }));
    let shapes = value["shapes"].as_array().expect("shapes array");
    assert_eq!(shapes.len(), 1);
    assert_eq!(shapes[0]["id"].as_u64(), Some(ids[0].0));
}

#[test]
fn add_polygon_and_path_validate_vertex_counts() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();

    // A two-vertex polygon is invalid.
    let err = s
        .apply(AgentCommand::AddPolygon {
            cell: "top".into(),
            layer: layer(2, 0),
            points: vec![PointArg { x: 0, y: 0 }, PointArg { x: 10, y: 0 }],
        })
        .expect_err("degenerate polygon rejected");
    assert_eq!(err.code, ErrorCode::InvalidArgument);

    // A three-vertex polygon is fine.
    s.apply(AgentCommand::AddPolygon {
        cell: "top".into(),
        layer: layer(2, 0),
        points: vec![
            PointArg { x: 0, y: 0 },
            PointArg { x: 10, y: 0 },
            PointArg { x: 0, y: 10 },
        ],
    })
    .expect("triangle ok");

    // A one-vertex path is invalid; a two-vertex path is fine.
    let err = s
        .apply(AgentCommand::AddPath {
            cell: "top".into(),
            layer: layer(3, 0),
            width: 4,
            points: vec![PointArg { x: 0, y: 0 }],
            endcap: None,
        })
        .expect_err("single-point path rejected");
    assert_eq!(err.code, ErrorCode::InvalidArgument);

    s.apply(AgentCommand::AddPath {
        cell: "top".into(),
        layer: layer(3, 0),
        width: 4,
        points: vec![PointArg { x: 0, y: 0 }, PointArg { x: 50, y: 0 }],
        endcap: None,
    })
    .expect("wire ok");

    assert_eq!(s.document().cell("top").unwrap().shapes.len(), 2);
}

#[test]
fn add_rect_to_missing_cell_errors() {
    let mut s = Session::new();
    let err = s
        .apply(AgentCommand::AddRect {
            cell: "ghost".into(),
            layer: layer(1, 0),
            rect: rect(0, 0, 10, 10),
        })
        .expect_err("no such cell");
    assert_eq!(err.code, ErrorCode::NoSuchCell);
}

// ===== placement family =====================================================

#[test]
fn place_instance_and_array_reference_child_cells() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell {
        name: "leaf".into(),
    })
    .unwrap();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();

    affected(s.apply(AgentCommand::PlaceInstance {
        cell: "top".into(),
        child: "leaf".into(),
        transform: TransformArg::default(),
    }));
    assert_eq!(s.document().cell("top").unwrap().instances.len(), 1);

    affected(s.apply(AgentCommand::PlaceArray {
        cell: "top".into(),
        child: "leaf".into(),
        transform: TransformArg::default(),
        columns: 3,
        rows: 2,
        column_pitch: 100,
        row_pitch: 100,
    }));
    assert_eq!(s.document().cell("top").unwrap().arrays.len(), 1);

    // A zero-dimension array is rejected.
    let err = s
        .apply(AgentCommand::PlaceArray {
            cell: "top".into(),
            child: "leaf".into(),
            transform: TransformArg::default(),
            columns: 0,
            rows: 2,
            column_pitch: 100,
            row_pitch: 100,
        })
        .expect_err("zero columns rejected");
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}

#[test]
fn transform_with_bad_magnification_is_rejected() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell {
        name: "leaf".into(),
    })
    .unwrap();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    let bad = TransformArg {
        orientation: OrientationArg::R0,
        mag_num: 1,
        mag_den: 0,
        dx: 0,
        dy: 0,
    };
    let err = s
        .apply(AgentCommand::PlaceInstance {
            cell: "top".into(),
            child: "leaf".into(),
            transform: bad,
        })
        .expect_err("zero denominator rejected");
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}

// ===== transform / delete existing shapes ===================================

#[test]
fn transform_shapes_keeps_the_id_and_moves_the_geometry() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    let id = affected(s.apply(AgentCommand::AddRect {
        cell: "top".into(),
        layer: layer(1, 0),
        rect: rect(0, 0, 10, 10),
    }))[0];

    // Translate the shape by (100, 0). The same id must still resolve, now to the
    // moved geometry.
    let out = affected(s.apply(AgentCommand::TransformShapes {
        ids: vec![id],
        transform: TransformArg {
            orientation: OrientationArg::R0,
            mag_num: 1,
            mag_den: 1,
            dx: 100,
            dy: 0,
        },
    }));
    assert_eq!(out, vec![id], "the id is preserved across a transform");

    let value = data(s.apply(AgentCommand::QueryShapes {
        cell: "top".into(),
        layer: None,
        region: None,
    }));
    let shapes = value["shapes"].as_array().unwrap();
    let moved = shapes
        .iter()
        .find(|sh| sh["id"].as_u64() == Some(id.0))
        .expect("moved shape still addressable by id");
    assert_eq!(moved["bbox"]["min"]["x"].as_i64(), Some(100));
    assert_eq!(moved["bbox"]["max"]["x"].as_i64(), Some(110));
}

#[test]
fn delete_shapes_reconciles_surviving_ids() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    // Three rects at increasing x so we can tell them apart after a delete.
    let a = affected(s.apply(AgentCommand::AddRect {
        cell: "top".into(),
        layer: layer(1, 0),
        rect: rect(0, 0, 10, 10),
    }))[0];
    let b = affected(s.apply(AgentCommand::AddRect {
        cell: "top".into(),
        layer: layer(1, 0),
        rect: rect(100, 0, 110, 10),
    }))[0];
    let c = affected(s.apply(AgentCommand::AddRect {
        cell: "top".into(),
        layer: layer(1, 0),
        rect: rect(200, 0, 210, 10),
    }))[0];

    // Delete the middle shape b. a and c must survive and keep addressing their own
    // geometry even though c's underlying slot shifted down by one.
    s.apply(AgentCommand::DeleteShapes { ids: vec![b] })
        .expect("delete b");
    assert_eq!(s.document().cell("top").unwrap().shapes.len(), 2);

    let value = data(s.apply(AgentCommand::QueryShapes {
        cell: "top".into(),
        layer: None,
        region: None,
    }));
    let shapes = value["shapes"].as_array().unwrap();
    let by_id = |id: ElementId| {
        shapes
            .iter()
            .find(|sh| sh["id"].as_u64() == Some(id.0))
            .map(|sh| sh["bbox"]["min"]["x"].as_i64().unwrap())
    };
    assert_eq!(by_id(a), Some(0), "a still points at x=0");
    assert_eq!(
        by_id(c),
        Some(200),
        "c still points at x=200 after the shift"
    );
    assert_eq!(by_id(b), None, "b is gone");

    // Deleting a stale id is a NoSuchElement error.
    let err = s
        .apply(AgentCommand::DeleteShapes { ids: vec![b] })
        .expect_err("stale id rejected");
    assert_eq!(err.code, ErrorCode::NoSuchElement);
}

// ===== query family =========================================================

#[test]
fn query_filters_by_layer_and_region() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    s.apply(AgentCommand::AddRect {
        cell: "top".into(),
        layer: layer(1, 0),
        rect: rect(0, 0, 10, 10),
    })
    .unwrap();
    s.apply(AgentCommand::AddRect {
        cell: "top".into(),
        layer: layer(2, 0),
        rect: rect(1000, 1000, 1010, 1010),
    })
    .unwrap();

    // Filter by layer 1: only the first rect.
    let v = data(s.apply(AgentCommand::QueryShapes {
        cell: "top".into(),
        layer: Some(layer(1, 0)),
        region: None,
    }));
    assert_eq!(v["shapes"].as_array().unwrap().len(), 1);

    // Filter by a region around the origin: only the first rect.
    let v = data(s.apply(AgentCommand::QueryShapes {
        cell: "top".into(),
        layer: None,
        region: Some(rect(-5, -5, 20, 20)),
    }));
    assert_eq!(v["shapes"].as_array().unwrap().len(), 1);

    // Query on a missing cell errors.
    let err = s
        .apply(AgentCommand::QueryShapes {
            cell: "ghost".into(),
            layer: None,
            region: None,
        })
        .expect_err("missing cell");
    assert_eq!(err.code, ErrorCode::NoSuchCell);
}

#[test]
fn get_cell_info_reports_counts_and_bbox() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    s.apply(AgentCommand::AddRect {
        cell: "top".into(),
        layer: layer(1, 0),
        rect: rect(0, 0, 40, 20),
    })
    .unwrap();
    let v = data(s.apply(AgentCommand::GetCellInfo { cell: "top".into() }));
    assert_eq!(v["shapes"].as_u64(), Some(1));
    assert_eq!(v["instances"].as_u64(), Some(0));
    assert_eq!(v["bbox"]["max"]["x"].as_i64(), Some(40));
    assert_eq!(v["bbox"]["max"]["y"].as_i64(), Some(20));
}

// ===== technology / DRC family ==============================================

/// A tiny technology with a single width rule that a 50-wide feature violates.
const TECH: &str = "\
technology test
dbu_per_micron 1000
layer 1 0 metal1 4488FFFF
rule width 1 0 100
";

#[test]
fn set_technology_then_list_layers() {
    let mut s = Session::new();
    s.apply(AgentCommand::SetTechnology {
        source: TECH.into(),
    })
    .expect("set tech");
    let v = data(s.apply(AgentCommand::ListLayers));
    assert_eq!(v["technology"].as_str(), Some("test"));
    assert_eq!(v["dbu_per_micron"].as_i64(), Some(1000));
    let layers = v["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0]["layer"].as_u64(), Some(1));

    // A malformed technology source is an InvalidArgument.
    let err = s
        .apply(AgentCommand::SetTechnology {
            source: "dbu_per_micron not_a_number".into(),
        })
        .expect_err("bad tech rejected");
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}

#[test]
fn run_drc_flags_a_width_violation() {
    let mut s = Session::new();
    s.apply(AgentCommand::SetTechnology {
        source: TECH.into(),
    })
    .unwrap();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    // A 50 x 50 rect on layer 1: width 50 < required 100, one violation.
    s.apply(AgentCommand::AddRect {
        cell: "top".into(),
        layer: layer(1, 0),
        rect: rect(0, 0, 50, 50),
    })
    .unwrap();

    let v = data(s.apply(AgentCommand::RunDrc {
        cell: "top".into(),
        region: None,
    }));
    assert_eq!(v["count"].as_u64(), Some(1), "one width violation");
    let violations = v["violations"].as_array().unwrap();
    assert_eq!(violations[0]["kind"].as_str(), Some("width"));
    assert_eq!(violations[0]["measured"].as_i64(), Some(50));
    assert_eq!(violations[0]["required"].as_i64(), Some(100));
}

// ===== extraction family ====================================================

#[test]
fn run_extract_finds_one_net_for_overlapping_rects() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    s.apply(AgentCommand::AddRect {
        cell: "top".into(),
        layer: layer(1, 0),
        rect: rect(0, 0, 10, 10),
    })
    .unwrap();
    s.apply(AgentCommand::AddRect {
        cell: "top".into(),
        layer: layer(1, 0),
        rect: rect(5, 5, 15, 15),
    })
    .unwrap();
    let v = data(s.apply(AgentCommand::RunExtract { cell: "top".into() }));
    assert_eq!(v["net_count"].as_u64(), Some(1), "overlapping rects merge");
    assert_eq!(v["nets"][0]["shape_count"].as_u64(), Some(2));
}

#[test]
fn netlist_compare_reports_a_short() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    // Two overlapping rects extract to one net (shapes 0 and 1 connected).
    s.apply(AgentCommand::AddRect {
        cell: "top".into(),
        layer: layer(1, 0),
        rect: rect(0, 0, 10, 10),
    })
    .unwrap();
    s.apply(AgentCommand::AddRect {
        cell: "top".into(),
        layer: layer(1, 0),
        rect: rect(5, 5, 15, 15),
    })
    .unwrap();
    // Expected netlist says they are on separate nets: the extraction shorts them.
    let expected = r#"{"nets":[{"name":"a","shapes":[0]},{"name":"b","shapes":[1]}]}"#;
    let v = data(s.apply(AgentCommand::NetlistCompare {
        cell: "top".into(),
        expected: expected.into(),
    }));
    assert_eq!(v["equivalent"].as_bool(), Some(false));
    assert_eq!(v["extra"].as_array().unwrap().len(), 1, "one short pair");

    // A malformed expected netlist is InvalidArgument.
    let err = s
        .apply(AgentCommand::NetlistCompare {
            cell: "top".into(),
            expected: "not json".into(),
        })
        .expect_err("bad expected netlist");
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}

// ===== IO family (export round-trip) ========================================

#[test]
fn export_gds_then_import_round_trips_the_geometry() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    s.apply(AgentCommand::AddRect {
        cell: "top".into(),
        layer: layer(1, 0),
        rect: rect(0, 0, 100, 100),
    })
    .unwrap();
    let bytes = blob(s.apply(AgentCommand::ExportGds));
    assert!(!bytes.is_empty(), "GDS export produced bytes");

    // Import into a fresh session and confirm the cell and shape survive.
    let mut s2 = Session::new();
    s2.apply(AgentCommand::ImportGds {
        bytes: bytes.clone(),
    })
    .expect("import gds");
    assert!(s2.document().cell("top").is_some(), "cell round-tripped");
    assert_eq!(s2.document().cell("top").unwrap().shapes.len(), 1);
}

#[test]
fn export_oasis_produces_bytes() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    s.apply(AgentCommand::AddRect {
        cell: "top".into(),
        layer: layer(7, 0),
        rect: rect(0, 0, 20, 20),
    })
    .unwrap();
    let bytes = blob(s.apply(AgentCommand::ExportOasis));
    assert!(!bytes.is_empty());
}

// ===== intent checking ======================================================

#[test]
fn check_intent_runs_the_checker() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    // A met1 rectangle the terminal lands on.
    s.apply(AgentCommand::AddRect {
        cell: "top".into(),
        layer: layer(68, 20),
        rect: RectArg {
            min: PointArg { x: 0, y: 0 },
            max: PointArg { x: 100, y: 100 },
        },
    })
    .unwrap();
    // One net with one terminal over the rectangle: trivially connected.
    let intent = r#"{"nets":[{"name":"n1","terminals":[{"name":"n1","layer":{"layer":68,"datatype":20},"region":{"min":{"x":10,"y":10},"max":{"x":20,"y":20}}}]}],"forbidden":[]}"#;
    let resp = s
        .apply(AgentCommand::CheckIntent {
            cell: "top".into(),
            intent: intent.into(),
        })
        .expect("check_intent runs against the extract checker");
    match resp {
        AgentResponse::Data { value, .. } => {
            assert!(value.get("opens").is_some(), "report has an opens field");
            assert!(value.get("shorts").is_some(), "report has a shorts field");
        }
        other => panic!("expected structured data, got {other:?}"),
    }
}

#[test]
fn check_intent_rejects_a_malformed_spec() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    let err = s
        .apply(AgentCommand::CheckIntent {
            cell: "top".into(),
            intent: "not valid json".into(),
        })
        .expect_err("a malformed intent spec is rejected");
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}

// ===== session persistence ==================================================

#[test]
fn save_then_load_session_reproduces_the_document() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    s.apply(AgentCommand::AddRect {
        cell: "top".into(),
        layer: layer(1, 0),
        rect: rect(0, 0, 30, 30),
    })
    .unwrap();
    let saved = blob(s.apply(AgentCommand::SaveSession));
    let snapshot = String::from_utf8(saved).expect("utf8 snapshot");

    let mut fresh = Session::new();
    fresh
        .apply(AgentCommand::LoadSession {
            snapshot: snapshot.clone(),
        })
        .expect("load");
    assert_eq!(
        reticle_model::document_hash(fresh.document()),
        reticle_model::document_hash(s.document()),
        "a loaded session reproduces the saved document"
    );

    // A malformed snapshot is InvalidArgument.
    let err = fresh
        .apply(AgentCommand::LoadSession {
            snapshot: "{".into(),
        })
        .expect_err("bad snapshot rejected");
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}

// ===== replay contract ======================================================

#[test]
fn replay_reproduces_the_document_hash() {
    let mut s = Session::new();
    for cmd in build_program() {
        let _ = s.apply(cmd);
    }
    let transcript = transcript_of(&s);
    // The recorded final hash matches the live document.
    assert_eq!(
        transcript.final_hash,
        reticle_model::document_hash(s.document())
    );
    // Replaying the transcript reproduces exactly that hash.
    let replayed = replay(&transcript).expect("replay");
    assert_eq!(replayed, transcript.final_hash);
    verify_replay(&transcript).expect("verify_replay holds");
}

#[test]
fn verify_replay_detects_a_tampered_hash() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    let mut transcript = transcript_of(&s);
    transcript.final_hash ^= 0xFFFF; // corrupt the recorded hash
    let err = verify_replay(&transcript).expect_err("tampered hash is caught");
    assert_eq!(err.code, ErrorCode::EngineError);
}

/// A representative program touching several command families, used by the replay
/// tests. It mixes cell creation, geometry, a placement, a transform, and a delete
/// so replay must reproduce a non-trivial document.
fn build_program() -> Vec<AgentCommand> {
    vec![
        AgentCommand::SetTechnology {
            source: TECH.into(),
        },
        AgentCommand::CreateCell {
            name: "leaf".into(),
        },
        AgentCommand::AddRect {
            cell: "leaf".into(),
            layer: layer(1, 0),
            rect: rect(0, 0, 10, 10),
        },
        AgentCommand::CreateCell { name: "top".into() },
        AgentCommand::AddRect {
            cell: "top".into(),
            layer: layer(1, 0),
            rect: rect(0, 0, 200, 5),
        },
        AgentCommand::PlaceArray {
            cell: "top".into(),
            child: "leaf".into(),
            transform: TransformArg::default(),
            columns: 4,
            rows: 1,
            column_pitch: 50,
            row_pitch: 0,
        },
    ]
}

// ===== Wave 2 editor ops (boolean, align, distribute, offset, via) ==========

/// Adds a rect to `cell` on layer 1 and returns its stable id.
fn add_rect_id(s: &mut Session, cell: &str, x0: i32, y0: i32, x1: i32, y1: i32) -> ElementId {
    let ids = affected(s.apply(AgentCommand::AddRect {
        cell: cell.into(),
        layer: layer(1, 0),
        rect: rect(x0, y0, x1, y1),
    }));
    ids[0]
}

/// A technology with a via cut layer (3/0) enclosed by met1 (1/0) by 20 and by
/// met2 (2/0) by 30, for the via-stack tests. The enclosure rule form is
/// `rule enclosure <outer> <outer_dt> <inner> <inner_dt> <value>`.
const VIA_TECH: &str = "\
technology via_test
dbu_per_micron 1000
layer 1 0 met1 4488FFFF
layer 2 0 met2 88FF44FF
layer 3 0 via 000000FF
rule enclosure 1 0 3 0 20
rule enclosure 2 0 3 0 30
";

/// Returns the bounding boxes of every shape currently in `cell`, each as
/// `(min_x, min_y, max_x, max_y)`.
fn shape_bboxes(s: &mut Session, cell: &str) -> Vec<(i64, i64, i64, i64)> {
    let v = data(s.apply(AgentCommand::QueryShapes {
        cell: cell.into(),
        layer: None,
        region: None,
    }));
    v["shapes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|sh| {
            let b = &sh["bbox"];
            (
                b["min"]["x"].as_i64().unwrap(),
                b["min"]["y"].as_i64().unwrap(),
                b["max"]["x"].as_i64().unwrap(),
                b["max"]["y"].as_i64().unwrap(),
            )
        })
        .collect()
}

#[test]
fn boolean_union_merges_two_overlapping_rects_into_one_shape() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    let a = add_rect_id(&mut s, "top", 0, 0, 100, 100);
    let b = add_rect_id(&mut s, "top", 50, 0, 150, 100);

    let out = affected(s.apply(AgentCommand::BooleanCombine {
        cell: "top".into(),
        bool_op: BooleanOpArg::Union,
        ids: vec![a, b],
        layer: layer(1, 0),
    }));
    assert_eq!(
        out.len(),
        1,
        "the union of two overlapping rects is one shape"
    );
    // Inputs consumed, one result remains, spanning the combined 0..150 x 0..100.
    let boxes = shape_bboxes(&mut s, "top");
    assert_eq!(boxes.len(), 1);
    assert_eq!(boxes[0], (0, 0, 150, 100));
}

#[test]
fn boolean_intersection_keeps_only_the_overlap() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    let a = add_rect_id(&mut s, "top", 0, 0, 100, 100);
    let b = add_rect_id(&mut s, "top", 50, 0, 150, 100);

    s.apply(AgentCommand::BooleanCombine {
        cell: "top".into(),
        bool_op: BooleanOpArg::Intersection,
        ids: vec![a, b],
        layer: layer(1, 0),
    })
    .expect("intersection ok");
    let boxes = shape_bboxes(&mut s, "top");
    assert_eq!(boxes.len(), 1);
    // The overlap is the 50..100 x 0..100 strip.
    assert_eq!(boxes[0], (50, 0, 100, 100));
}

#[test]
fn boolean_difference_subtracts_the_second_from_the_first() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    let a = add_rect_id(&mut s, "top", 0, 0, 100, 100);
    let b = add_rect_id(&mut s, "top", 50, 0, 150, 100);

    s.apply(AgentCommand::BooleanCombine {
        cell: "top".into(),
        bool_op: BooleanOpArg::Difference,
        ids: vec![a, b],
        layer: layer(1, 0),
    })
    .expect("difference ok");
    let boxes = shape_bboxes(&mut s, "top");
    assert_eq!(boxes.len(), 1);
    // A minus B leaves the left 0..50 x 0..100 strip.
    assert_eq!(boxes[0], (0, 0, 50, 100));
}

#[test]
fn boolean_writes_result_to_the_target_layer() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    let a = add_rect_id(&mut s, "top", 0, 0, 100, 100);
    let b = add_rect_id(&mut s, "top", 50, 0, 150, 100);
    // Result requested on a different layer (7/0) than the inputs (1/0).
    s.apply(AgentCommand::BooleanCombine {
        cell: "top".into(),
        bool_op: BooleanOpArg::Union,
        ids: vec![a, b],
        layer: layer(7, 0),
    })
    .expect("union ok");
    let v = data(s.apply(AgentCommand::QueryShapes {
        cell: "top".into(),
        layer: Some(layer(7, 0)),
        region: None,
    }));
    assert_eq!(
        v["shapes"].as_array().unwrap().len(),
        1,
        "result is on layer 7"
    );
}

#[test]
fn boolean_needs_two_shapes_and_rejects_non_shapes() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    let a = add_rect_id(&mut s, "top", 0, 0, 100, 100);

    // A single shape is rejected before any geometry runs.
    let err = s
        .apply(AgentCommand::BooleanCombine {
            cell: "top".into(),
            bool_op: BooleanOpArg::Union,
            ids: vec![a],
            layer: layer(1, 0),
        })
        .expect_err("one shape rejected");
    assert_eq!(err.code, ErrorCode::InvalidArgument);

    // An unknown id maps to NoSuchElement.
    let err = s
        .apply(AgentCommand::BooleanCombine {
            cell: "top".into(),
            bool_op: BooleanOpArg::Union,
            ids: vec![a, ElementId(9999)],
            layer: layer(1, 0),
        })
        .expect_err("unknown id rejected");
    assert_eq!(err.code, ErrorCode::NoSuchElement);
}

#[test]
fn boolean_rejects_shapes_from_a_different_cell() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    s.apply(AgentCommand::CreateCell {
        name: "other".into(),
    })
    .unwrap();
    let a = add_rect_id(&mut s, "top", 0, 0, 100, 100);
    let b = add_rect_id(&mut s, "other", 0, 0, 100, 100);
    let err = s
        .apply(AgentCommand::BooleanCombine {
            cell: "top".into(),
            bool_op: BooleanOpArg::Union,
            ids: vec![a, b],
            layer: layer(1, 0),
        })
        .expect_err("cross-cell selection rejected");
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}

#[test]
fn align_left_moves_shapes_to_the_leftmost_edge() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    let a = add_rect_id(&mut s, "top", 0, 0, 10, 10);
    let b = add_rect_id(&mut s, "top", 100, 100, 110, 110);

    s.apply(AgentCommand::AlignShapes {
        ids: vec![a, b],
        align: AlignArg::Left,
    })
    .expect("align left ok");
    // Both shapes now share the leftmost edge x=0.
    let boxes = shape_bboxes(&mut s, "top");
    assert!(
        boxes.iter().all(|b| b.0 == 0),
        "all left edges at x=0: {boxes:?}"
    );
    assert_eq!(boxes.len(), 2, "no shapes lost");
}

#[test]
fn align_keeps_ids_stable() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    let a = add_rect_id(&mut s, "top", 0, 0, 10, 10);
    let b = add_rect_id(&mut s, "top", 100, 100, 110, 110);
    let out = affected(s.apply(AgentCommand::AlignShapes {
        ids: vec![a, b],
        align: AlignArg::Top,
    }));
    assert_eq!(out, vec![a, b], "align returns the same ids it was given");
    // The moved id still resolves: a transform in place on b keeps addressing it.
    let moved = affected(s.apply(AgentCommand::TransformShapes {
        ids: vec![b],
        transform: TransformArg::default(),
    }));
    assert_eq!(moved, vec![b]);
}

#[test]
fn distribute_equalizes_the_gaps_between_three_shapes() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    // Three 10-wide shapes; the middle one is off-center so it must move.
    let a = add_rect_id(&mut s, "top", 0, 0, 10, 10);
    let mid = add_rect_id(&mut s, "top", 30, 0, 40, 10);
    let c = add_rect_id(&mut s, "top", 100, 0, 110, 10);

    s.apply(AgentCommand::DistributeShapes {
        ids: vec![a, mid, c],
        axis: AxisArg::Horizontal,
    })
    .expect("distribute ok");
    // Extremes stay at 0..10 and 100..110; total shape extent is 30, span is 110,
    // so each of the two gaps is (110 - 30) / 2 = 40, putting the middle at 50..60.
    let mut boxes = shape_bboxes(&mut s, "top");
    boxes.sort_unstable();
    assert_eq!(boxes[0], (0, 0, 10, 10));
    assert_eq!(boxes[1], (50, 0, 60, 10), "middle respaced to equal gaps");
    assert_eq!(boxes[2], (100, 0, 110, 10));
}

#[test]
fn distribute_needs_three_shapes() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    let a = add_rect_id(&mut s, "top", 0, 0, 10, 10);
    let b = add_rect_id(&mut s, "top", 100, 0, 110, 10);
    let err = s
        .apply(AgentCommand::DistributeShapes {
            ids: vec![a, b],
            axis: AxisArg::Horizontal,
        })
        .expect_err("two shapes rejected");
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}

#[test]
fn offset_grow_increases_a_rect_bounding_box() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    let a = add_rect_id(&mut s, "top", 0, 0, 100, 100);
    let out = affected(s.apply(AgentCommand::OffsetShapes {
        ids: vec![a],
        delta: 10,
    }));
    assert_eq!(out, vec![a], "the offset shape keeps its id");
    let boxes = shape_bboxes(&mut s, "top");
    assert_eq!(boxes.len(), 1);
    // A 10-DBU grow expands every side: -10..110 on both axes.
    assert_eq!(boxes[0], (-10, -10, 110, 110));
}

#[test]
fn offset_shrink_reduces_a_rect_and_can_erase_it() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    let a = add_rect_id(&mut s, "top", 0, 0, 100, 100);
    // Shrink by 20: the box becomes 20..80 on both axes.
    s.apply(AgentCommand::OffsetShapes {
        ids: vec![a],
        delta: -20,
    })
    .expect("shrink ok");
    let boxes = shape_bboxes(&mut s, "top");
    assert_eq!(boxes.len(), 1);
    assert_eq!(boxes[0], (20, 20, 80, 80));

    // A second shape shrunk past its own size collapses to nothing and is removed.
    let b = add_rect_id(&mut s, "top", 200, 200, 210, 210);
    s.apply(AgentCommand::OffsetShapes {
        ids: vec![b],
        delta: -100,
    })
    .expect("collapse ok");
    // Only the first (still-present) shape remains.
    assert_eq!(shape_bboxes(&mut s, "top").len(), 1);
}

#[test]
fn offset_zero_is_a_noop() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    let a = add_rect_id(&mut s, "top", 0, 0, 100, 100);
    let out = affected(s.apply(AgentCommand::OffsetShapes {
        ids: vec![a],
        delta: 0,
    }));
    assert_eq!(out, vec![a]);
    assert_eq!(shape_bboxes(&mut s, "top"), vec![(0, 0, 100, 100)]);
}

#[test]
fn via_stack_sizes_enclosures_from_technology_rules() {
    let mut s = Session::new();
    s.apply(AgentCommand::SetTechnology {
        source: VIA_TECH.into(),
    })
    .unwrap();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    let out = affected(s.apply(AgentCommand::BuildViaStack {
        cell: "top".into(),
        lower_layer: layer(1, 0),
        upper_layer: layer(2, 0),
        cut_layer: layer(3, 0),
        center: PointArg { x: 0, y: 0 },
        cut_size: 40,
        default_enclosure: 5,
    }));
    assert_eq!(out.len(), 3, "cut plus two enclosures");
    // Cut is 40 wide centered at origin: -20..20. Lower enclosure grows by the
    // met1 rule (20) to -40..40; upper by the met2 rule (30) to -50..50.
    let boxes = shape_bboxes(&mut s, "top");
    assert!(boxes.contains(&(-20, -20, 20, 20)), "cut: {boxes:?}");
    assert!(
        boxes.contains(&(-40, -40, 40, 40)),
        "lower enclosure: {boxes:?}"
    );
    assert!(
        boxes.contains(&(-50, -50, 50, 50)),
        "upper enclosure: {boxes:?}"
    );
}

#[test]
fn via_stack_falls_back_to_default_enclosure_without_a_rule() {
    let mut s = Session::new();
    // No technology rules at all: both enclosures use the default margin.
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    s.apply(AgentCommand::BuildViaStack {
        cell: "top".into(),
        lower_layer: layer(1, 0),
        upper_layer: layer(2, 0),
        cut_layer: layer(3, 0),
        center: PointArg { x: 0, y: 0 },
        cut_size: 40,
        default_enclosure: 15,
    })
    .expect("via stack ok");
    let boxes = shape_bboxes(&mut s, "top");
    // Cut -20..20; both enclosures grow by the default 15 to -35..35.
    assert!(boxes.contains(&(-20, -20, 20, 20)));
    assert_eq!(
        boxes.iter().filter(|b| **b == (-35, -35, 35, 35)).count(),
        2,
        "both enclosures use the default margin: {boxes:?}"
    );
}

#[test]
fn via_stack_rejects_a_nonpositive_cut() {
    let mut s = Session::new();
    s.apply(AgentCommand::CreateCell { name: "top".into() })
        .unwrap();
    let err = s
        .apply(AgentCommand::BuildViaStack {
            cell: "top".into(),
            lower_layer: layer(1, 0),
            upper_layer: layer(2, 0),
            cut_layer: layer(3, 0),
            center: PointArg { x: 0, y: 0 },
            cut_size: 0,
            default_enclosure: 5,
        })
        .expect_err("zero cut rejected");
    assert_eq!(err.code, ErrorCode::InvalidArgument);
}

#[test]
fn via_stack_on_missing_cell_errors() {
    let mut s = Session::new();
    let err = s
        .apply(AgentCommand::BuildViaStack {
            cell: "nope".into(),
            lower_layer: layer(1, 0),
            upper_layer: layer(2, 0),
            cut_layer: layer(3, 0),
            center: PointArg { x: 0, y: 0 },
            cut_size: 40,
            default_enclosure: 5,
        })
        .expect_err("missing cell rejected");
    assert_eq!(err.code, ErrorCode::NoSuchCell);
}
