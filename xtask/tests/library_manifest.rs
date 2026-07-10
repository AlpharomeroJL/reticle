//! Integration test for `xtask library-manifest`: the F1 gallery-manifest generator
//! (ADR 0101, `pipeline-manifest` lane).
//!
//! `xtask` is a binary-only crate (no library target), so this drives the real built
//! binary via `CARGO_BIN_EXE_xtask`, exactly as `scripts/library/fetch-convert-verify.ps1`
//! does, then deserializes what it writes with
//! `reticle_index::gallery_manifest::GalleryManifest` and asserts the F1 contract
//! invariant: a verified die carries a streaming archive, an excluded die carries
//! none. This is the "fixture" the F1 gallery UI lane built against before the
//! pipeline landed (ADR 0101): from here on the real committed sample IS the fixture,
//! generated fresh by the real subcommand each run, not hand-written JSON.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use reticle_index::gallery_manifest::{GalleryManifest, License};

/// The repo root, derived from this crate's own manifest dir. Not CWD-dependent (an
/// absolute path baked in at compile time), so the test is robust to whatever
/// directory `cargo test` / `cargo nextest run` happens to run from.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// A process-unique counter so parallel test runs never collide on a temp path.
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A fresh, unique path under the OS temp dir; the file need not exist yet.
fn temp_path(stem: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let mut path = std::env::temp_dir();
    path.push(format!("xtask_library_manifest_{stem}_{pid}_{n}"));
    path
}

/// An RAII guard that removes a temp file or directory (recursively) on drop.
struct TempPath(PathBuf);

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs `xtask <args>`, returning its captured output.
fn run_xtask(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(args)
        .output()
        .expect("xtask runs")
}

#[test]
fn library_manifest_generates_a_valid_f1_manifest_from_the_committed_sample() {
    let out = TempPath(temp_path("gen.json"));
    let library_dir = repo_root().join("library");
    let dies_meta = repo_root().join("scripts/library/dies.json");

    let output = run_xtask(&[
        "library-manifest",
        library_dir.to_str().unwrap(),
        dies_meta.to_str().unwrap(),
        out.0.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "library-manifest failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json = std::fs::read_to_string(&out.0).expect("manifest written");
    let manifest: GalleryManifest = serde_json::from_str(&json).expect("F1 manifest parses");
    manifest
        .validate()
        .expect("generated manifest satisfies the F1 contract");

    assert_eq!(manifest.version, 1);
    assert_eq!(manifest.dies.len(), 2, "the committed sample has two dies");

    let verified: Vec<_> = manifest
        .dies
        .iter()
        .filter(|d| matches!(d.license, License::Verified { .. }))
        .collect();
    let excluded: Vec<_> = manifest
        .dies
        .iter()
        .filter(|d| matches!(d.license, License::Excluded { .. }))
        .collect();
    assert_eq!(verified.len(), 1, "exactly one sample die verifies");
    assert_eq!(excluded.len(), 1, "exactly one sample die is excluded");

    // The invariant the F1 contract exists to enforce.
    let v = verified[0];
    assert!(v.streaming.is_some(), "a verified die streams");
    assert!(
        !v.landmarks.is_empty(),
        "the verified sample carries its curated landmark"
    );
    let e = excluded[0];
    assert!(e.streaming.is_none(), "an excluded die is never uploaded");
    assert!(e.landmarks.is_empty(), "an excluded die has no deep link");

    // Real, not fabricated: the verified die is the committed SkyWater inverter
    // sample, and its size is whatever `reticle convert` actually measured.
    assert_eq!(v.id, "sky130.inv-1");
    assert_eq!(v.technology, "sky130");
    assert!(v.width_dbu > 0 && v.height_dbu > 0);
    if let License::Verified { spdx, text_sha256 } = &v.license {
        assert_eq!(spdx, "Apache-2.0");
        assert_eq!(text_sha256.len(), 64);
    }
}

#[test]
fn verify_licenses_reports_the_verified_and_excluded_split_on_the_sample() {
    let library_dir = repo_root().join("library");
    let output = run_xtask(&["verify-licenses", library_dir.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("STATUS VERIFIED"), "stdout was: {stdout}");
    assert!(stdout.contains("STATUS EXCLUDED"), "stdout was: {stdout}");
    // The sample deliberately includes one excluded entry, so the gate's own
    // fail-closed contract means this exits non-zero; see
    // `scripts/library/fetch-convert-verify.ps1`'s handling of this same exit code.
    assert!(
        !output.status.success(),
        "verify-licenses over a directory with an excluded entry must exit non-zero"
    );
}

#[test]
fn library_manifest_fails_closed_on_a_dies_json_mismatch() {
    // A `dies.json` naming a die id with no archive in the directory is a hard error,
    // never a silently incomplete manifest.
    let scratch = TempPath(temp_path("mismatch"));
    std::fs::create_dir_all(&scratch.0).expect("create temp dir");
    let dies_json = scratch.0.join("dies.json");
    std::fs::write(
        &dies_json,
        r#"[{"id":"nonexistent","name":"x","technology":"x","repo":"x","commit":"x","url":"x"}]"#,
    )
    .expect("write dies.json");
    let empty_library = scratch.0.join("empty-library");
    std::fs::create_dir_all(&empty_library).expect("create empty library dir");
    let out = TempPath(temp_path("mismatch.json"));

    let output = run_xtask(&[
        "library-manifest",
        empty_library.to_str().unwrap(),
        dies_json.to_str().unwrap(),
        out.0.to_str().unwrap(),
    ]);
    assert!(
        !output.status.success(),
        "a dies.json/archive mismatch must fail closed"
    );
    assert!(!out.0.exists(), "no manifest is written on a failed run");
}
