//! Confirms the repo-committed F5 plugin index (`library/plugins/index.json`, the
//! `plugin-manifest-index` lane's generator output, `xtask plugin-index`) parses and
//! validates via the real `reticle_plugin::manifest::Index` type. `Index` is
//! pure-serde with no platform-gated fields, so this same shape is what the browser
//! build's plugin-browse path deserializes; `just wasm-build` covers the wasm compile
//! side, this covers the parse-and-validate side.
//!
//! Deliberately does not assert an entry count: the committed index is regenerated as
//! more plugins land (the real sample plugin folds in via the concurrent
//! `plugin-sample` lane; see `scratch/lanes/plugin-manifest-index/RESULT.md`), so this
//! only pins the invariant that must hold at every count, including zero.

use reticle_plugin::manifest::Index;

const COMMITTED_INDEX: &str = include_str!("../../../library/plugins/index.json");

#[test]
fn committed_plugin_index_parses_and_validates() {
    let index: Index = serde_json::from_str(COMMITTED_INDEX)
        .expect("library/plugins/index.json must parse as the F5 Index shape");
    index
        .validate()
        .expect("the committed plugin index must satisfy the F5 contract");

    // Every entry (there may be zero today) carries a real 64-char lowercase-hex
    // hash and a non-empty source path. `validate()` already enforces the hash
    // shape; this just documents the same invariant without cross-referencing
    // `manifest.rs`.
    for entry in &index.entries {
        assert_eq!(entry.wasm_sha256.len(), 64);
        assert!(!entry.source.is_empty());
    }

    // Serde round-trips exactly, the same invariant `f5_manifest.rs` pins for the
    // hand-authored fixture.
    let reserialized = serde_json::to_string(&index).expect("serialize");
    let reparsed: Index = serde_json::from_str(&reserialized).expect("reparse");
    assert_eq!(index, reparsed);
}
