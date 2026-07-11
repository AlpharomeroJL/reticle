//! End-to-end proof of the v0 calling convention against the native `wasmi` host.
//!
//! The host is native-only, so the whole test is gated off the wasm target. Each
//! test compiles a hand-authored `.wat` fixture to binary wasm with the `wat`
//! crate (the host itself accepts binary wasm only) and drives it through
//! [`Host::run`]. Together they cover the success bar: load + instantiate under
//! fuel and memory limits, a read-only query, the `StageEdit` funnel (undoable and
//! replayable), capability gating at instantiation, and panic-free rejection of
//! malformed, oversized, and resource-exhausting input.
#![cfg(not(target_arch = "wasm32"))]

use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{
    Cell, Document, DocumentStore, DrawShape, EditableDocument, Label, ShapeKind, Technology,
    document_hash,
};
use reticle_plugin::host::{Host, HostContext, HostError, Limits, SelectedShape, decode_edit_v0};
use reticle_plugin::manifest::{ABI_VERSION, HostFn, Manifest, Permission};

/// The primary v0 plugin fixture: queries then stages an edit encoding the result.
const PLUGIN_WAT: &str = include_str!("fixtures/plugins/plugin.wat");
/// A fixture that grows memory past the cap and traps.
const GROW_WAT: &str = include_str!("fixtures/plugins/grow.wat");
/// A fixture that probes all three read-only queries and stages the results.
const QUERIES_WAT: &str = include_str!("fixtures/plugins/queries.wat");

/// Compiles a `.wat` fixture to binary wasm (test-side only; the host is binary).
fn compile(wat: &str) -> Vec<u8> {
    wat::parse_str(wat).expect("fixture wat compiles to binary wasm")
}

/// A manifest granting `permissions`, targeting the host ABI, entry `run`.
fn manifest(permissions: Vec<Permission>) -> Manifest {
    Manifest {
        id: "dev.reticle.spike".to_owned(),
        version: "0.1.0".to_owned(),
        api_version: ABI_VERSION,
        name: "Spike".to_owned(),
        entry: "run".to_owned(),
        permissions,
    }
}

/// A document whose cell `TOP` holds `top_shapes` rectangles, with technology
/// resolution `dbu`.
fn base_doc(top_shapes: usize, dbu: i64) -> EditableDocument {
    let mut cell = Cell::new("TOP");
    for i in 0..top_shapes {
        let d = i as i32;
        cell.shapes.push(DrawShape::new(
            LayerId::new(10, 0),
            ShapeKind::Rect(Rect::new(Point::new(d, d), Point::new(d + 1, d + 1))),
        ));
    }
    let mut doc = Document::new();
    doc.insert_cell(cell);
    doc.set_top_cells(vec!["TOP".to_owned()]);
    doc.set_technology(Technology {
        dbu_per_micron: dbu,
        ..Technology::default()
    });
    EditableDocument::new(doc)
}

/// The core proof: the plugin loads and instantiates under the default limits, its
/// read-only query returns the real shape count, and its staged edit is applied
/// through the command/undo funnel so it is undoable and exactly reversible.
#[test]
fn loads_runs_reads_and_funnels_an_undoable_edit() {
    let host = Host::new();
    let mut doc = base_doc(2, 1000);
    let before = document_hash(doc.document());

    let m = manifest(vec![Permission::ReadDocument, Permission::StageEdit]);
    let wasm = compile(PLUGIN_WAT);
    let out = host
        .run(
            &wasm,
            &m,
            &mut doc,
            &HostContext::default(),
            &Limits::default(),
        )
        .expect("run succeeds under default limits");

    assert_eq!(out.staged.len(), 1, "one edit staged");
    assert_eq!(out.applied, 1, "the staged edit applied through the funnel");
    assert!(out.apply_errors.is_empty());
    assert!(out.fuel_consumed > 0, "the run metered fuel");

    // The edit reached the document only through EditableDocument::apply, so it is
    // on the undo stack.
    assert_eq!(doc.undo_depth(), 1, "the funneled edit is undoable");

    // TOP gained one shape; its y1 corner is the queried count (2), proving the
    // read-only query returned the real value and it flowed through the funnel.
    let top = doc.document().cell("TOP").expect("TOP exists");
    assert_eq!(top.shapes.len(), 3);
    match &top.shapes[2].kind {
        ShapeKind::Rect(r) => {
            assert_eq!(r.min, Point::new(0, 0));
            assert_eq!(r.max, Point::new(100, 2));
        }
        other => panic!("expected a rect, got {other:?}"),
    }

    // Undo restores the pre-run document byte-for-byte (exact reversibility), redo
    // reapplies it (replay).
    assert!(doc.undo(), "undo pops the funneled edit");
    assert_eq!(
        document_hash(doc.document()),
        before,
        "undo restores the original document hash"
    );
    assert!(doc.redo(), "redo reapplies it");
    assert_eq!(doc.document().cell("TOP").unwrap().shapes.len(), 3);
}

/// Determinism / replay: the same plugin and the same input produce the same
/// staged edits and the same resulting document hash, every time.
#[test]
fn same_plugin_and_input_produce_the_same_result() {
    let host = Host::new();
    let wasm = compile(PLUGIN_WAT);
    let m = manifest(vec![Permission::ReadDocument, Permission::StageEdit]);

    let run_once = || {
        let mut doc = base_doc(2, 1000);
        let out = host
            .run(
                &wasm,
                &m,
                &mut doc,
                &HostContext::default(),
                &Limits::default(),
            )
            .expect("run succeeds");
        (
            document_hash(doc.document()),
            out.applied,
            format!("{:?}", out.staged),
        )
    };

    let first = run_once();
    let second = run_once();
    assert_eq!(
        first, second,
        "same plugin + input must yield the same staged edits and document"
    );
}

/// Capability gating: a plugin that imports a host function whose permission the
/// manifest does not grant is rejected before instantiation, and nothing runs.
#[test]
fn ungranted_host_fn_is_rejected_at_instantiation() {
    let host = Host::new();
    let wasm = compile(PLUGIN_WAT); // imports query_shapes AND stage_edit
    let m = manifest(vec![Permission::ReadDocument]); // StageEdit deliberately absent
    let mut doc = base_doc(2, 1000);

    let err = host
        .run(
            &wasm,
            &m,
            &mut doc,
            &HostContext::default(),
            &Limits::default(),
        )
        .expect_err("the ungranted stage_edit import must be rejected");
    assert!(
        matches!(
            err,
            HostError::PermissionDenied {
                host_fn: HostFn::StageEdit,
                permission: Permission::StageEdit,
            }
        ),
        "got {err:?}"
    );
    assert_eq!(doc.undo_depth(), 0, "nothing was applied");
}

/// Malformed and oversized wasm are clean errors, never panics.
#[test]
fn malformed_and_oversized_wasm_are_clean_errors() {
    let host = Host::new();
    let m = manifest(vec![Permission::ReadDocument, Permission::StageEdit]);
    let mut doc = base_doc(0, 1000);
    let ctx = HostContext::default();

    // Not wasm at all.
    let garbage = [0u8, 1, 2, 3, 4, 5, 6, 7];
    assert!(matches!(
        host.run(&garbage, &m, &mut doc, &ctx, &Limits::default()),
        Err(HostError::Compile(_))
    ));

    // Correct magic but a truncated header.
    let truncated = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00];
    assert!(
        host.run(&truncated, &m, &mut doc, &ctx, &Limits::default())
            .is_err()
    );

    // Oversized: a real module rejected before compilation by the size cap.
    let wasm = compile(PLUGIN_WAT);
    let tiny = Limits {
        max_wasm_bytes: 4,
        ..Limits::default()
    };
    assert!(matches!(
        host.run(&wasm, &m, &mut doc, &ctx, &tiny),
        Err(HostError::TooLarge { cap: 4, .. })
    ));

    assert_eq!(doc.undo_depth(), 0, "no failed input mutated the document");
}

/// The memory cap is enforced at run time: growth past it traps the plugin.
#[test]
fn memory_growth_past_the_cap_traps() {
    let host = Host::new();
    let wasm = compile(GROW_WAT);
    let m = manifest(vec![]); // imports nothing
    let mut doc = base_doc(0, 1000);
    let limits = Limits {
        memory_bytes: 65536, // one page: the grow-by-100-pages attempt is denied
        ..Limits::default()
    };

    let err = host
        .run(&wasm, &m, &mut doc, &HostContext::default(), &limits)
        .expect_err("growth past the cap must trap");
    assert!(matches!(err, HostError::Trap(_)), "got {err:?}");
}

/// The fuel cap halts a plugin: a one-unit budget traps before it finishes.
#[test]
fn fuel_exhaustion_traps_the_plugin() {
    let host = Host::new();
    let wasm = compile(PLUGIN_WAT);
    let m = manifest(vec![Permission::ReadDocument, Permission::StageEdit]);
    let mut doc = base_doc(2, 1000);
    let limits = Limits {
        fuel: 1,
        ..Limits::default()
    };

    let err = host
        .run(&wasm, &m, &mut doc, &HostContext::default(), &limits)
        .expect_err("a one-fuel budget must trap");
    assert!(matches!(err, HostError::Trap(_)), "got {err:?}");
    assert_eq!(doc.undo_depth(), 0, "a trapped run applied nothing");
}

/// An import from the reticle namespace that is not a v0 host function is rejected.
#[test]
fn unknown_reticle_import_is_rejected() {
    let host = Host::new();
    let wat = r#"(module
        (import "reticle" "bogus" (func $b))
        (memory (export "memory") 1)
        (func (export "run")))"#;
    let wasm = compile(wat);
    let m = manifest(vec![Permission::ReadDocument, Permission::StageEdit]);
    let mut doc = base_doc(0, 1000);
    assert!(matches!(
        host.run(
            &wasm,
            &m,
            &mut doc,
            &HostContext::default(),
            &Limits::default()
        ),
        Err(HostError::UnknownImport { .. })
    ));
}

/// A module that does not export the manifest's entry is a clean error.
#[test]
fn missing_entry_export_is_a_clean_error() {
    let host = Host::new();
    let wat = r#"(module (memory (export "memory") 1) (func (export "notrun")))"#;
    let wasm = compile(wat);
    let m = manifest(vec![]);
    let mut doc = base_doc(0, 1000);
    assert!(matches!(
        host.run(
            &wasm,
            &m,
            &mut doc,
            &HostContext::default(),
            &Limits::default()
        ),
        Err(HostError::MissingEntry { .. })
    ));
}

/// The read-only query surface returns REAL data from the pre-run snapshot: the
/// document's shape count, the selection resolved against the document, and the
/// active technology's resolution. The fixture stages each result on its own layer
/// so the applied shapes read back the three values.
#[test]
fn real_query_surface_reflects_document_selection_and_technology() {
    let host = Host::new();
    let wasm = compile(QUERIES_WAT);
    let m = manifest(vec![
        Permission::ReadDocument,
        Permission::ReadSelection,
        Permission::ReadTechnology,
        Permission::StageEdit,
    ]);
    let mut doc = base_doc(2, 1000); // TOP holds 2 shapes; technology dbu = 1000

    // Two references resolve to a real shape in TOP; two do not (an out-of-range
    // index and a missing cell). query_selection must count only the two that
    // resolve, proving the answer is grounded in real document state.
    let ctx = HostContext {
        selection: vec![
            SelectedShape::new("TOP", 0),
            SelectedShape::new("TOP", 1),
            SelectedShape::new("TOP", 99),
            SelectedShape::new("MISSING", 0),
        ],
    };

    let out = host
        .run(&wasm, &m, &mut doc, &ctx, &Limits::default())
        .expect("run succeeds");
    assert_eq!(out.applied, 3, "three query-carrying edits funneled");

    let top = doc.document().cell("TOP").expect("TOP exists");
    assert_eq!(top.shapes.len(), 5, "two original shapes plus three staged");

    // Each staged shape carries one query's result in its y1 (max.y) corner, tagged
    // by layer 1/2/3.
    let value_on_layer = |layer: u16| -> i32 {
        let shape = top
            .shapes
            .iter()
            .find(|s| s.layer == LayerId::new(layer, 0))
            .expect("staged shape on the expected layer");
        match &shape.kind {
            ShapeKind::Rect(r) => r.max.y,
            other => panic!("expected a rect, got {other:?}"),
        }
    };
    assert_eq!(
        value_on_layer(1),
        2,
        "query_shapes returned the real TOP shape count"
    );
    assert_eq!(
        value_on_layer(2),
        2,
        "query_selection returned the resolved selection count"
    );
    assert_eq!(
        value_on_layer(3),
        1000,
        "query_technology returned the real dbu_per_micron"
    );
}

/// Appends a v0 length-prefixed name (`u16` length ++ UTF-8 bytes).
fn enc_name(out: &mut Vec<u8>, name: &str) {
    let len = u16::try_from(name.len()).expect("test name fits in u16");
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(name.as_bytes());
}

/// Appends a v0 transform (`u8` orientation ++ `i32` tx,ty ++ `u32` num,den).
fn enc_transform(out: &mut Vec<u8>, orient: u8, tx: i32, ty: i32, num: u32, den: u32) {
    out.push(orient);
    out.extend_from_slice(&tx.to_le_bytes());
    out.extend_from_slice(&ty.to_le_bytes());
    out.extend_from_slice(&num.to_le_bytes());
    out.extend_from_slice(&den.to_le_bytes());
}

/// Decodes `record`, funnels it through [`EditableDocument::apply`], checks the
/// document, then proves the command is exactly undoable and replayable: undo
/// restores the pre-edit hash and redo reapplies it.
fn funnel_check(mut doc: EditableDocument, record: &[u8], check: impl Fn(&Document)) {
    let before = document_hash(doc.document());
    let edit = decode_edit_v0(record, 256).expect("record decodes");
    doc.apply(edit)
        .expect("the decoded edit applies through the funnel");
    check(doc.document());
    assert_eq!(doc.undo_depth(), 1, "the funneled edit is undoable");
    assert!(doc.undo(), "undo pops the funneled edit");
    assert_eq!(
        document_hash(doc.document()),
        before,
        "undo restores the pre-edit document exactly"
    );
    assert!(doc.redo(), "redo reapplies the edit");
    check(doc.document());
}

/// A document whose cell `TOP` holds one shape and one label, for the remove tests.
fn doc_top_with_shape_and_label() -> EditableDocument {
    let mut cell = Cell::new("TOP");
    cell.shapes.push(DrawShape::new(
        LayerId::new(1, 0),
        ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(1, 1))),
    ));
    cell.labels
        .push(Label::new("NET", Point::new(0, 0), LayerId::new(2, 0)));
    let mut d = Document::new();
    d.insert_cell(cell);
    EditableDocument::new(d)
}

/// opcode 0x01 `AddShape`: appends a rectangle to the target cell.
#[test]
fn add_shape_opcode_funnels_and_is_undoable() {
    let mut r = vec![0x01];
    enc_name(&mut r, "TOP");
    r.extend_from_slice(&1u16.to_le_bytes()); // layer
    r.extend_from_slice(&0u16.to_le_bytes()); // datatype
    for v in [0i32, 0, 10, 20] {
        r.extend_from_slice(&v.to_le_bytes());
    }
    funnel_check(base_doc(0, 1000), &r, |d| {
        assert_eq!(d.cell("TOP").unwrap().shapes.len(), 1, "AddShape appended");
    });
}

/// opcode 0x02 `AddCell`: inserts a new empty cell.
#[test]
fn add_cell_opcode_funnels_and_is_undoable() {
    let mut r = vec![0x02];
    enc_name(&mut r, "SUB");
    funnel_check(EditableDocument::new(Document::new()), &r, |d| {
        let sub = d.cell("SUB").expect("AddCell created SUB");
        assert!(sub.shapes.is_empty(), "the new cell is empty");
    });
}

/// opcode 0x03 `RemoveCell`: removes an existing cell.
#[test]
fn remove_cell_opcode_funnels_and_is_undoable() {
    let mut r = vec![0x03];
    enc_name(&mut r, "TOP");
    funnel_check(base_doc(1, 1000), &r, |d| {
        assert!(d.cell("TOP").is_none(), "RemoveCell removed TOP");
    });
}

/// opcode 0x04 `RemoveShape`: removes a shape by index.
#[test]
fn remove_shape_opcode_funnels_and_is_undoable() {
    let mut r = vec![0x04];
    enc_name(&mut r, "TOP");
    r.extend_from_slice(&0u32.to_le_bytes()); // index 0
    funnel_check(base_doc(1, 1000), &r, |d| {
        assert!(
            d.cell("TOP").unwrap().shapes.is_empty(),
            "RemoveShape removed the shape"
        );
    });
}

/// opcode 0x05 `AddInstance`: places a child cell with a full transform.
#[test]
fn add_instance_opcode_funnels_and_is_undoable() {
    let mut r = vec![0x05];
    enc_name(&mut r, "TOP");
    enc_name(&mut r, "CHILD");
    enc_transform(&mut r, 3, 10, -20, 3, 2); // R270, translate (10,-20), mag 3/2
    funnel_check(base_doc(0, 1000), &r, |d| {
        let inst = &d.cell("TOP").unwrap().instances;
        assert_eq!(inst.len(), 1, "AddInstance placed one instance");
        assert_eq!(inst[0].cell, "CHILD");
        assert_eq!(inst[0].transform.translation, Point::new(10, -20));
    });
}

/// opcode 0x06 `AddArray`: places an arrayed child cell.
#[test]
fn add_array_opcode_funnels_and_is_undoable() {
    let mut r = vec![0x06];
    enc_name(&mut r, "TOP");
    enc_name(&mut r, "BIT");
    enc_transform(&mut r, 0, 0, 0, 1, 1);
    r.extend_from_slice(&4u32.to_le_bytes()); // columns
    r.extend_from_slice(&8u32.to_le_bytes()); // rows
    r.extend_from_slice(&100i32.to_le_bytes()); // column_pitch
    r.extend_from_slice(&200i32.to_le_bytes()); // row_pitch
    funnel_check(base_doc(0, 1000), &r, |d| {
        let arr = &d.cell("TOP").unwrap().arrays;
        assert_eq!(arr.len(), 1, "AddArray placed one array");
        assert_eq!(arr[0].cell, "BIT");
        assert_eq!((arr[0].columns, arr[0].rows), (4, 8));
        assert_eq!((arr[0].column_pitch, arr[0].row_pitch), (100, 200));
    });
}

/// opcode 0x07 `AddLabel`: appends a label with position, layer, and anchor.
#[test]
fn add_label_opcode_funnels_and_is_undoable() {
    let mut r = vec![0x07];
    enc_name(&mut r, "TOP");
    enc_name(&mut r, "VDD");
    r.extend_from_slice(&5i32.to_le_bytes()); // x
    r.extend_from_slice(&6i32.to_le_bytes()); // y
    r.extend_from_slice(&2u16.to_le_bytes()); // layer
    r.extend_from_slice(&0u16.to_le_bytes()); // datatype
    r.push(0); // anchor Center
    funnel_check(base_doc(0, 1000), &r, |d| {
        let labels = &d.cell("TOP").unwrap().labels;
        assert_eq!(labels.len(), 1, "AddLabel appended a label");
        assert_eq!(labels[0].text, "VDD");
        assert_eq!(labels[0].position, Point::new(5, 6));
    });
}

/// opcode 0x08 `RemoveLabel`: removes a label by index.
#[test]
fn remove_label_opcode_funnels_and_is_undoable() {
    let mut r = vec![0x08];
    enc_name(&mut r, "TOP");
    r.extend_from_slice(&0u32.to_le_bytes()); // index 0
    funnel_check(doc_top_with_shape_and_label(), &r, |d| {
        assert!(
            d.cell("TOP").unwrap().labels.is_empty(),
            "RemoveLabel removed the label"
        );
    });
}
