//! Integration test for `xtask plugin-index`: the F5 static plugin index generator
//! (ADR 0105, `plugin-manifest-index` lane).
//!
//! `xtask` is a binary-only crate (no library target), so this drives the real built
//! binary via `CARGO_BIN_EXE_xtask`, exactly as `xtask/tests/library_manifest.rs` does
//! for `library-manifest`, then deserializes what it writes with the real
//! `reticle_plugin::manifest::Index` (never a hand-mirrored struct) so a shape drift
//! in the F5 contract would fail this test, not silently pass.
//!
//! The fixtures under `tests/fixtures/plugins_ok` and `tests/fixtures/plugins_bad_*`
//! are synthetic: their `.wasm` files are the minimal valid empty wasm module (the
//! 8-byte `\0asm` + version-1 header, optionally with one empty custom section
//! appended) purely so the generator has real bytes to hash. They are test fixtures,
//! never claimed as real, runnable plugins.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use reticle_plugin::manifest::Index;

/// This crate's own fixtures directory (`xtask/tests/fixtures`), an absolute path
/// baked in at compile time so the test is robust to whatever directory `cargo test`
/// / `cargo nextest run` happens to run from.
fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// A process-unique counter so parallel test runs never collide on a temp path.
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A fresh, unique path under the OS temp dir; the file need not exist yet.
fn temp_path(stem: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let mut path = std::env::temp_dir();
    path.push(format!("xtask_plugin_index_{stem}_{pid}_{n}"));
    path
}

/// An RAII guard that removes a temp file on drop.
struct TempPath(PathBuf);

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Runs `xtask <args>`, returning its captured output.
fn run_xtask(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(args)
        .output()
        .expect("xtask runs")
}

/// The sorted key names of a JSON object, for a shape-parity comparison independent
/// of key order or value content.
///
/// `serde_json::Value`'s `Index` impl distinguishes `usize` (array position) from
/// `&str` (object key); a caller walking e.g. `entries[0]` must use a real `usize`
/// index there, never a stringified one, before calling this on the resulting object.
fn sorted_keys(v: &serde_json::Value) -> Vec<String> {
    let mut ks: Vec<String> = v
        .as_object()
        .unwrap_or_else(|| panic!("expected a JSON object, got {v}"))
        .keys()
        .cloned()
        .collect();
    ks.sort();
    ks
}

#[test]
fn plugin_index_builds_a_sorted_valid_index_from_the_fixture_directory() {
    let plugins_dir = fixtures_dir().join("plugins_ok");
    let out = TempPath(temp_path("ok.json"));

    let output = run_xtask(&[
        "plugin-index",
        plugins_dir.to_str().unwrap(),
        out.0.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "plugin-index failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = std::fs::read_to_string(&out.0).expect("index written");
    let index: Index = serde_json::from_str(&json).expect("F5 index parses");
    index
        .validate()
        .expect("generated index satisfies the F5 contract");

    assert_eq!(
        index.entries.len(),
        2,
        "the fixture directory has 2 plugins"
    );

    // Sorted ascending by manifest.id: "dev.reticle.test.aaa" before "...zzz", the
    // REVERSE of the directory scan order (dir-a comes first alphabetically but
    // carries the "zzz" id), so this actually exercises the sort, not just directory
    // read order happening to already match.
    assert_eq!(index.entries[0].manifest.id, "dev.reticle.test.aaa");
    assert_eq!(index.entries[1].manifest.id, "dev.reticle.test.zzz");

    // Hand-verified: `Get-FileHash -Algorithm SHA256` (PowerShell, independent of the
    // `sha2` crate under test) over the exact fixture bytes.
    assert_eq!(
        index.entries[0].manifest.name, "Aaa Test Fixture",
        "sanity: entry 0 is dir-b's manifest"
    );
    assert_eq!(
        index.entries[0].wasm_sha256,
        "93a44bbb96c751218e4c00d479e4c14358122a389acca16205b1e4d0dc5f9476",
        "dir-b/aaa.wasm sha256 (hand-verified via PowerShell Get-FileHash)"
    );
    assert_eq!(
        index.entries[1].manifest.name, "Zzz Test Fixture",
        "sanity: entry 1 is dir-a's manifest"
    );
    assert_eq!(
        index.entries[1].wasm_sha256,
        "76b54c92ad4dfa455958ee8b52624104db89ac16996be1aebaf8d85e4ddad6d7",
        "dir-a/zzz.wasm sha256 (hand-verified via PowerShell Get-FileHash)"
    );

    // `source` is forward-slash joined and names the right subdirectory, regardless
    // of the host path separator (this repo builds on Windows).
    assert!(
        !index.entries[0].source.contains('\\'),
        "source must not carry a backslash: {}",
        index.entries[0].source
    );
    assert!(index.entries[0].source.ends_with("/dir-b"));
    assert!(index.entries[1].source.ends_with("/dir-a"));

    // Serde round-trips exactly, same invariant `f5_manifest.rs` pins for the
    // hand-authored fixture.
    let reserialized = serde_json::to_string(&index).expect("serialize");
    let reparsed: Index = serde_json::from_str(&reserialized).expect("reparse");
    assert_eq!(index, reparsed);
}

#[test]
fn plugin_index_fails_closed_on_a_missing_manifest() {
    let plugins_dir = fixtures_dir().join("plugins_bad_no_manifest");
    let out = TempPath(temp_path("bad_no_manifest.json"));

    let output = run_xtask(&[
        "plugin-index",
        plugins_dir.to_str().unwrap(),
        out.0.to_str().unwrap(),
    ]);
    assert!(
        !output.status.success(),
        "a plugin directory with no manifest.json must fail closed"
    );
    assert!(!out.0.exists(), "no index is written on a failed run");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("manifest.json"), "stderr was: {stderr}");
}

#[test]
fn plugin_index_fails_closed_on_a_missing_wasm() {
    let plugins_dir = fixtures_dir().join("plugins_bad_no_wasm");
    let out = TempPath(temp_path("bad_no_wasm.json"));

    let output = run_xtask(&[
        "plugin-index",
        plugins_dir.to_str().unwrap(),
        out.0.to_str().unwrap(),
    ]);
    assert!(
        !output.status.success(),
        "a plugin directory with no .wasm file must fail closed"
    );
    assert!(!out.0.exists(), "no index is written on a failed run");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(".wasm"), "stderr was: {stderr}");
}

#[test]
fn plugin_index_fails_closed_on_ambiguous_wasm_files() {
    let plugins_dir = fixtures_dir().join("plugins_bad_two_wasm");
    let out = TempPath(temp_path("bad_two_wasm.json"));

    let output = run_xtask(&[
        "plugin-index",
        plugins_dir.to_str().unwrap(),
        out.0.to_str().unwrap(),
    ]);
    assert!(
        !output.status.success(),
        "a plugin directory with two .wasm files must fail closed (never guess)"
    );
    assert!(!out.0.exists(), "no index is written on a failed run");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ambiguous"), "stderr was: {stderr}");
}

#[test]
fn plugin_index_on_a_nonexistent_plugins_dir_writes_an_empty_valid_index() {
    // Phase 4 is mid-flight: `plugins/` may legitimately not exist yet. This is the
    // exact scenario the real `library/plugins/index.json` generation hits today (the
    // sample plugin lands via a concurrent lane); it must succeed, not fail.
    let plugins_dir = fixtures_dir().join("this_directory_does_not_exist");
    assert!(!plugins_dir.exists(), "test precondition");
    let out = TempPath(temp_path("empty.json"));

    let output = run_xtask(&[
        "plugin-index",
        plugins_dir.to_str().unwrap(),
        out.0.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "a missing plugins dir must succeed with an empty index: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = std::fs::read_to_string(&out.0).expect("index written");
    let index: Index = serde_json::from_str(&json).expect("F5 index parses");
    index.validate().expect("an empty index validates");
    assert_eq!(index.entries.len(), 0);
}

#[test]
fn plugin_index_output_shares_field_names_with_the_f5_fixture() {
    // Shape parity against the frozen F5 fixture (ADR 0105): both the generator's
    // fresh output and the committed fixture must carry exactly the same field names
    // at the index-entry and manifest levels, confirming the generator emits the same
    // shape the manager UI and the host build against, not merely "some JSON that
    // happens to deserialize."
    const F5_FIXTURE: &str =
        include_str!("../../crates/reticle-plugin/tests/fixtures/contracts/f5_index.json");
    let fixture: serde_json::Value = serde_json::from_str(F5_FIXTURE).expect("fixture parses");

    let plugins_dir = fixtures_dir().join("plugins_ok");
    let out = TempPath(temp_path("shape.json"));
    let output = run_xtask(&[
        "plugin-index",
        plugins_dir.to_str().unwrap(),
        out.0.to_str().unwrap(),
    ]);
    assert!(output.status.success());
    let generated: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out.0).expect("index written"))
            .expect("generated index parses as JSON");

    let fixture_entry = &fixture["entries"][0];
    let generated_entry = &generated["entries"][0];
    assert_eq!(
        sorted_keys(fixture_entry),
        sorted_keys(generated_entry),
        "IndexEntry field names must match the F5 fixture"
    );
    assert_eq!(
        sorted_keys(&fixture_entry["manifest"]),
        sorted_keys(&generated_entry["manifest"]),
        "Manifest field names must match the F5 fixture"
    );
}
