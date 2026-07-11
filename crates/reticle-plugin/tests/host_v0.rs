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
    Cell, Document, DocumentStore, DrawShape, EditableDocument, ShapeKind, Technology,
    document_hash,
};
use reticle_plugin::host::{Host, HostContext, HostError, Limits};
use reticle_plugin::manifest::{ABI_VERSION, HostFn, Manifest, Permission};

/// The primary v0 plugin fixture: queries then stages an edit encoding the result.
const PLUGIN_WAT: &str = include_str!("fixtures/plugins/plugin.wat");
/// A fixture that grows memory past the cap and traps.
const GROW_WAT: &str = include_str!("fixtures/plugins/grow.wat");

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
