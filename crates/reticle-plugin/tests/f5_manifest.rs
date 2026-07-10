//! F5 plugin-manifest + ABI + index contract cross-test.
//!
//! The producer (the index generator + sample plugin, Phase 4) and the consumer (the
//! plugin manager UI) both build against `tests/fixtures/contracts/f5_index.json`. This
//! test pins the fixture and the untrusted-input discipline: the committed index parses
//! and validates, the host-function table maps to the right permissions, and hostile
//! manifests (wrong ABI, overlong fields, bad hash, unsorted) are rejected with a
//! structured error rather than a panic.

use reticle_plugin::manifest::{
    ABI_VERSION, HostFn, Index, IndexEntry, Manifest, ManifestError, Permission,
};

const FIXTURE: &str = include_str!("fixtures/contracts/f5_index.json");

fn sample_manifest() -> Manifest {
    Manifest {
        id: "dev.reticle.metal-fill".to_owned(),
        version: "0.1.0".to_owned(),
        api_version: ABI_VERSION,
        name: "Metal Fill".to_owned(),
        entry: "run".to_owned(),
        permissions: vec![Permission::ReadDocument, Permission::StageEdit],
    }
}

#[test]
fn f5_fixture_parses_and_validates() {
    let index: Index = serde_json::from_str(FIXTURE).expect("F5 fixture parses");
    index.validate().expect("the committed index must validate");
    assert_eq!(index.entries.len(), 1);

    let entry = &index.entries[0];
    assert!(
        entry.manifest.abi_compatible(),
        "fixture targets ABI v{ABI_VERSION}"
    );
    assert_eq!(
        entry.manifest.permissions,
        vec![Permission::ReadDocument, Permission::StageEdit]
    );
    assert_eq!(entry.wasm_sha256.len(), 64);

    // Serde round-trips exactly.
    let reserialized = serde_json::to_string(&index).expect("serialize");
    let reparsed: Index = serde_json::from_str(&reserialized).expect("reparse");
    assert_eq!(index, reparsed);
}

#[test]
fn f5_host_functions_map_to_permissions() {
    // The v0 host table: each function requires exactly the permission the manifest must
    // have been granted, and edits funnel through StageEdit (never a direct mutation).
    assert_eq!(
        HostFn::QueryShapes.required_permission(),
        Permission::ReadDocument
    );
    assert_eq!(
        HostFn::QuerySelection.required_permission(),
        Permission::ReadSelection
    );
    assert_eq!(
        HostFn::QueryTechnology.required_permission(),
        Permission::ReadTechnology
    );
    assert_eq!(
        HostFn::StageEdit.required_permission(),
        Permission::StageEdit
    );
}

#[test]
fn f5_rejects_hostile_manifests_without_panicking() {
    // Wrong ABI version.
    let mut wrong_abi = sample_manifest();
    wrong_abi.api_version = 99;
    assert!(matches!(
        wrong_abi.validate(),
        Err(ManifestError::AbiMismatch {
            wanted: 99,
            host: ABI_VERSION
        })
    ));

    // Overlong id (untrusted-input cap).
    let mut long_id = sample_manifest();
    long_id.id = "x".repeat(10_000);
    assert!(matches!(
        long_id.validate(),
        Err(ManifestError::TooLong { field: "id", .. })
    ));

    // Empty entry.
    let mut no_entry = sample_manifest();
    no_entry.entry.clear();
    assert!(matches!(
        no_entry.validate(),
        Err(ManifestError::EmptyField("entry"))
    ));

    // Duplicate permission.
    let mut dup = sample_manifest();
    dup.permissions = vec![Permission::ReadDocument, Permission::ReadDocument];
    assert!(matches!(
        dup.validate(),
        Err(ManifestError::DuplicatePermission)
    ));
}

#[test]
fn f5_index_rejects_bad_hash_and_unsorted() {
    let entry = |id: &str, hash: &str| IndexEntry {
        manifest: Manifest {
            id: id.to_owned(),
            ..sample_manifest()
        },
        wasm_sha256: hash.to_owned(),
        source: "plugins/x".to_owned(),
    };
    let good = "a".repeat(64);

    // A non-hex / wrong-length hash is rejected.
    let bad_hash = Index {
        entries: vec![entry("a.plugin", "NOTHEX")],
    };
    assert!(matches!(bad_hash.validate(), Err(ManifestError::BadHash)));

    // Out-of-order ids are rejected (the committed index must be deterministic).
    let unsorted = Index {
        entries: vec![entry("b.plugin", &good), entry("a.plugin", &good)],
    };
    assert!(matches!(unsorted.validate(), Err(ManifestError::Unsorted)));

    // A correctly-sorted, correctly-hashed index validates.
    let ok = Index {
        entries: vec![entry("a.plugin", &good), entry("b.plugin", &good)],
    };
    ok.validate().expect("a sorted, hashed index validates");
}
