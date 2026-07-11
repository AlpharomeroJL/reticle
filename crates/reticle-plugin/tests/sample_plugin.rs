//! End-to-end proof that the committed sample plugin `fiducial-marker` (ADR 0116) loads and
//! runs under the native `wasmi` host, producing the expected staged `AddShape` edit.
//!
//! Unlike `tests/host_v0.rs` (whose fixtures are hand-authored `.wat`, compiled to binary
//! wasm at test time because the host accepts binary wasm only), this harness loads the
//! REAL, already-compiled wasm bytes committed at
//! `plugins/fiducial-marker/fiducial_marker.wasm` -- built from actual Rust guest source
//! (`plugins/fiducial-marker/src/lib.rs`) via `cargo build --release --target
//! wasm32-unknown-unknown` run inside that directory (see `RESULT.md` for the exact command,
//! toolchain, and a rebuild-reproducibility sha256 check). Nothing here fakes a run: `Host::run`
//! actually instantiates and executes the committed bytes.
#![cfg(not(target_arch = "wasm32"))]

use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{
    Cell, Document, DocumentStore, DrawShape, EditableDocument, ShapeKind, Technology,
    document_hash,
};
use reticle_plugin::host::{Host, HostContext, HostError, Limits};
use reticle_plugin::manifest::{ABI_VERSION, HostFn, Manifest, Permission};

/// The committed sample plugin's wasm bytes: a REAL compiled guest module, not a hand
/// authored `.wat` fixture and not a synthetic stand-in.
const SAMPLE_WASM: &[u8] = include_bytes!("../../../plugins/fiducial-marker/fiducial_marker.wasm");

/// The marker layer/datatype and base size the plugin stamps with; mirrors the constants in
/// `plugins/fiducial-marker/src/lib.rs`.
const MARKER_LAYER: u16 = 900;
const MARKER_DATATYPE: u16 = 0;
const BASE_SIZE: i32 = 4;

/// The manifest the plugin ships (mirrors `plugins/fiducial-marker/manifest.json`).
fn manifest() -> Manifest {
    Manifest {
        id: "dev.reticle.fiducial-marker".to_owned(),
        version: "0.1.0".to_owned(),
        api_version: ABI_VERSION,
        name: "Fiducial Marker".to_owned(),
        entry: "run".to_owned(),
        permissions: vec![Permission::ReadDocument, Permission::StageEdit],
    }
}

/// A document whose cell `TOP` holds `n` shapes, matching what the plugin queries before it
/// stages its marker.
fn base_doc(n: usize) -> EditableDocument {
    let mut cell = Cell::new("TOP");
    for i in 0..n {
        let d = i as i32;
        cell.shapes.push(DrawShape::new(
            LayerId::new(10, 0),
            ShapeKind::Rect(Rect::new(Point::new(d, d), Point::new(d + 1, d + 1))),
        ));
    }
    let mut doc = Document::new();
    doc.insert_cell(cell);
    doc.set_top_cells(vec!["TOP".to_owned()]);
    doc.set_technology(Technology::default());
    EditableDocument::new(doc)
}

/// The shipped manifest is valid and its permission set is exactly what the compiled module
/// imports: neither more (the module doesn't need `read_selection`/`read_technology`) nor
/// less (dropping either granted permission is rejected before running, proven below).
#[test]
fn manifest_matches_the_compiled_module_imports() {
    let m = manifest();
    m.validate().expect("manifest validates");
    assert!(m.abi_compatible());
    assert_eq!(
        m.permissions,
        vec![Permission::ReadDocument, Permission::StageEdit]
    );
}

/// The core proof: the REAL compiled wasm loads, instantiates under the default resource
/// limits, runs its read-only query, and funnels its staged `AddShape` edit through the
/// command/undo machinery so it is undoable and exactly reversible.
#[test]
fn sample_plugin_loads_runs_and_stamps_a_marker() {
    let host = Host::new();
    let existing = 3usize;
    let mut doc = base_doc(existing);
    let before = document_hash(doc.document());

    let out = host
        .run(
            SAMPLE_WASM,
            &manifest(),
            &mut doc,
            &HostContext::default(),
            &Limits::default(),
        )
        .expect("the real compiled sample plugin runs under the v0 host");

    assert_eq!(out.staged.len(), 1, "the plugin stages exactly one edit");
    assert_eq!(out.applied, 1, "the staged edit applied through the funnel");
    assert!(out.apply_errors.is_empty());
    assert!(out.fuel_consumed > 0, "the run metered real fuel");
    assert_eq!(doc.undo_depth(), 1, "the funneled edit is undoable");

    // The marker's size is BASE_SIZE + the queried shape count, proving the query returned
    // the real value and it flowed through the funnel into the applied shape.
    let expected_size = BASE_SIZE + i32::try_from(existing).unwrap();
    let top = doc.document().cell("TOP").expect("TOP exists");
    assert_eq!(top.shapes.len(), existing + 1);
    let marker = &top.shapes[existing];
    assert_eq!(marker.layer, LayerId::new(MARKER_LAYER, MARKER_DATATYPE));
    match &marker.kind {
        ShapeKind::Rect(r) => {
            assert_eq!(r.min, Point::new(0, 0));
            assert_eq!(r.max, Point::new(expected_size, expected_size));
        }
        other => panic!("expected a rect marker, got {other:?}"),
    }

    // Undo restores the pre-run document byte-for-byte (exact reversibility), redo
    // reapplies it (replay), matching the host_v0.rs proof for the .wat fixtures.
    assert!(doc.undo(), "undo pops the plugin's staged edit");
    assert_eq!(
        document_hash(doc.document()),
        before,
        "undo restores the original document hash"
    );
    assert!(doc.redo(), "redo reapplies it");
    assert_eq!(
        doc.document().cell("TOP").unwrap().shapes.len(),
        existing + 1
    );
}

/// Determinism: the same committed plugin bytes against the same input document produce the
/// same staged edit and the same resulting document hash, every time.
#[test]
fn sample_plugin_is_deterministic() {
    let host = Host::new();
    let run_once = || {
        let mut doc = base_doc(5);
        let out = host
            .run(
                SAMPLE_WASM,
                &manifest(),
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
    assert_eq!(run_once(), run_once(), "same plugin + input must reproduce");
}

/// The plugin degrades gracefully when its target cell does not exist yet: `query_shapes`
/// returns `-3` (no such cell), the guest folds that to zero per its documented behavior, and
/// it still stages a base-size marker naming `TOP`. Applying that edit against a document with
/// no `TOP` cell is then a clean `ModelError`, never a panic, exactly mirroring the host's
/// panic-free discipline over plugin-controlled effects.
#[test]
fn sample_plugin_degrades_gracefully_when_top_is_missing() {
    let host = Host::new();
    let mut doc = base_doc(0);
    doc.apply(reticle_model::Edit::RemoveCell {
        name: "TOP".to_owned(),
    })
    .expect("TOP removed for this fixture");
    assert!(doc.document().cell("TOP").is_none(), "TOP is gone");

    let out = host
        .run(
            SAMPLE_WASM,
            &manifest(),
            &mut doc,
            &HostContext::default(),
            &Limits::default(),
        )
        .expect("a missing target cell must not fault the host or trap the plugin");

    assert_eq!(out.staged.len(), 1, "the plugin still stages its marker");
    assert_eq!(out.applied, 0, "but there is no TOP cell to apply it to");
    assert_eq!(
        out.apply_errors.len(),
        1,
        "the failure is recorded, not swallowed"
    );
}

/// Capability gating applies to the real compiled module exactly as it does to the `.wat`
/// fixtures in `host_v0.rs`: dropping a permission the module's imports require is rejected
/// before any code runs, and nothing is applied.
#[test]
fn missing_stage_edit_permission_is_rejected_before_running() {
    let host = Host::new();
    let mut doc = base_doc(1);
    let mut m = manifest();
    m.permissions = vec![Permission::ReadDocument]; // StageEdit deliberately dropped

    let err = host
        .run(
            SAMPLE_WASM,
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

/// Same for `ReadDocument`: dropping it rejects the module's `query_shapes` import.
#[test]
fn missing_read_document_permission_is_rejected_before_running() {
    let host = Host::new();
    let mut doc = base_doc(1);
    let mut m = manifest();
    m.permissions = vec![Permission::StageEdit]; // ReadDocument deliberately dropped

    let err = host
        .run(
            SAMPLE_WASM,
            &m,
            &mut doc,
            &HostContext::default(),
            &Limits::default(),
        )
        .expect_err("the ungranted query_shapes import must be rejected");
    assert!(
        matches!(
            err,
            HostError::PermissionDenied {
                host_fn: HostFn::QueryShapes,
                permission: Permission::ReadDocument,
            }
        ),
        "got {err:?}"
    );
    assert_eq!(doc.undo_depth(), 0);
}
