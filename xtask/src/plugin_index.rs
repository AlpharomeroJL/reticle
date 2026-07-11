//! `xtask plugin-index <plugins-dir> <out.json>`: the F5 static plugin index generator
//! (ADR 0105, `plugin-manifest-index` lane).
//!
//! Scans `plugins-dir` for plugin subdirectories (Phase 4's `plugins/<name>/`
//! convention, `plugin-sample` lane), one level deep. Each plugin subdirectory must
//! carry a `manifest.json` (deserialized as a [`Manifest`]) and exactly one `*.wasm`
//! file directly inside it (the built, committed guest module). The generator hashes
//! the real wasm bytes (lowercase-hex SHA-256, [`sha256_hex`]), records the plugin's
//! source path, sorts the resulting entries ascending by `manifest.id` (the [`Index`]
//! contract), and writes the pretty-printed JSON.
//!
//! Reuses `reticle_plugin::manifest`'s real types rather than a hand-mirrored struct,
//! so the emitted shape cannot drift from the F5 contract: whatever this generator
//! writes deserializes through the exact same [`Index`] the manager UI and the host
//! use.
//!
//! # Fail-closed on anything unverifiable
//!
//! A plugin directory with no manifest, an unparsable or invalid manifest, zero or
//! multiple wasm files, or a final index that does not itself validate is a hard
//! error, never a silently wrong or partial index. An index entry is only ever built
//! from a manifest that parsed and validated and a wasm file that was actually read
//! and hashed; nothing here fabricates a hash for a plugin that does not exist.
//!
//! # A missing or empty `plugins-dir` is not an error
//!
//! Phase 4 is mid-flight: `plugins/` may legitimately not exist yet (the real sample
//! plugin lands via the concurrent `plugin-sample` lane). "Zero plugins currently
//! ship" is an honest, valid index, not a build failure, so a missing directory
//! yields an empty (still-valid) [`Index`] rather than an error. Re-running this
//! exact command once a plugin lands picks it up automatically, with no flag change.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use reticle_plugin::manifest::{Index, IndexEntry, Manifest};
use sha2::{Digest, Sha256};

/// The manifest file name expected directly inside each plugin subdirectory.
const MANIFEST_FILE: &str = "manifest.json";

/// SHA-256 of `bytes`, rendered as lowercase hex (64 chars): the [`IndexEntry`]'s
/// `wasm_sha256`.
fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        })
}

/// `dir`'s immediate subdirectories, sorted by path for a deterministic scan order.
/// Dotfile-style directories (e.g. a stray `.git`) are skipped. The final index is
/// sorted by `manifest.id` regardless of scan order; this only makes error messages
/// and partial-failure behavior reproducible run to run.
fn subdirectories(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            !path
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.starts_with('.'))
        })
        .collect();
    dirs.sort();
    Ok(dirs)
}

/// The single `*.wasm` file directly inside `plugin_dir` (not recursive, so a nested
/// local `target/` build directory is never considered).
///
/// Returns a message (never panics) if there is not exactly one: zero means no built
/// module was committed, more than one is ambiguous and this refuses to guess which
/// one is the real plugin.
fn find_wasm(plugin_dir: &Path) -> Result<PathBuf, String> {
    let entries = fs::read_dir(plugin_dir)
        .map_err(|e| format!("{}: cannot read directory: {e}", plugin_dir.display()))?;
    let mut wasm_files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().and_then(OsStr::to_str) == Some("wasm"))
        .collect();
    wasm_files.sort();
    match wasm_files.len() {
        0 => Err(format!(
            "{}: no .wasm file found (expected exactly one built, committed module)",
            plugin_dir.display()
        )),
        1 => Ok(wasm_files.remove(0)),
        n => Err(format!(
            "{}: {n} .wasm files found, expected exactly one (ambiguous, refusing to guess)",
            plugin_dir.display()
        )),
    }
}

/// Builds one plugin's [`IndexEntry`] from its directory: reads and validates
/// `manifest.json`, hashes its one committed `.wasm` file, and records `source` as
/// `{plugins_dir_arg}/{name}` (a forward-slash join of the raw CLI argument and the
/// directory's own name, so the field is copy-pasteable regardless of the host path
/// separator; this repo builds on Windows).
///
/// # Errors
///
/// Returns a message (never panics) naming the plugin directory when the manifest is
/// missing, unparsable, or fails [`Manifest::validate`], or when [`find_wasm`] fails.
fn build_entry(plugins_dir_arg: &str, plugin_dir: &Path) -> Result<IndexEntry, String> {
    let name = plugin_dir
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| format!("{}: non-UTF-8 directory name", plugin_dir.display()))?;

    let manifest_path = plugin_dir.join(MANIFEST_FILE);
    let manifest_text = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("{}: cannot read {MANIFEST_FILE}: {e}", plugin_dir.display()))?;
    let manifest: Manifest = serde_json::from_str(&manifest_text).map_err(|e| {
        format!(
            "{}: {MANIFEST_FILE} does not parse: {e}",
            plugin_dir.display()
        )
    })?;
    manifest
        .validate()
        .map_err(|e| format!("{}: manifest invalid: {e}", plugin_dir.display()))?;

    let wasm_path = find_wasm(plugin_dir)?;
    let wasm_bytes =
        fs::read(&wasm_path).map_err(|e| format!("{}: cannot read: {e}", wasm_path.display()))?;
    let wasm_sha256 = sha256_hex(&wasm_bytes);

    // Normalize EVERY separator, not just a trailing one: `plugins_dir_arg` may be an
    // absolute Windows path (backslash-separated throughout, e.g. built via
    // `Path::join` upstream), and `source` must be copy-pasteable regardless.
    let root = plugins_dir_arg.replace('\\', "/");
    let root = root.trim_end_matches('/');
    let source = format!("{root}/{name}");

    Ok(IndexEntry {
        manifest,
        wasm_sha256,
        source,
    })
}

/// Scans `plugins_dir_arg` and builds the full [`Index`]. A missing directory yields
/// an empty (still-valid) index (see the module doc); an existing directory's every
/// subdirectory must build a valid entry or the whole run fails closed.
///
/// # Errors
///
/// Returns a message (never panics) on the first plugin directory that fails to build
/// a valid entry, or if the assembled index itself fails [`Index::validate`].
fn build_index(plugins_dir_arg: &str) -> Result<Index, String> {
    let dir = Path::new(plugins_dir_arg);
    if !dir.exists() {
        return Ok(Index::default());
    }
    let dirs = subdirectories(dir)
        .map_err(|e| format!("{}: cannot read directory: {e}", dir.display()))?;

    let mut entries = Vec::with_capacity(dirs.len());
    for plugin_dir in &dirs {
        entries.push(build_entry(plugins_dir_arg, plugin_dir)?);
    }
    entries.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));

    let index = Index { entries };
    index
        .validate()
        .map_err(|e| format!("generated index fails validation: {e}"))?;
    Ok(index)
}

/// Handles `plugin-index <plugins-dir> <out.json>`: writes the F5 static plugin index
/// built from `plugins-dir` to `out.json`. Exits non-zero (with no file written) on
/// any unverifiable input; see the module doc for what "unverifiable" covers and why
/// a missing `plugins-dir` is the one case that is not an error.
#[must_use]
pub fn cmd_plugin_index(args: &[String]) -> ExitCode {
    let (Some(plugins_dir), Some(out)) = (args.first(), args.get(1)) else {
        eprintln!("usage: xtask plugin-index <plugins-dir> <out.json>");
        return ExitCode::FAILURE;
    };

    let index = match build_index(plugins_dir) {
        Ok(index) => index,
        Err(e) => {
            eprintln!("plugin-index: {e}");
            return ExitCode::FAILURE;
        }
    };

    if !Path::new(plugins_dir).exists() {
        println!(
            "plugin-index: no plugin directory at {plugins_dir}; writing an empty index (0 entries)"
        );
    }

    let json = match serde_json::to_string_pretty(&index) {
        Ok(json) => json,
        Err(e) => {
            eprintln!("plugin-index: cannot serialize index: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Some(parent) = Path::new(out).parent()
        && !parent.as_os_str().is_empty()
        && let Err(e) = fs::create_dir_all(parent)
    {
        eprintln!("plugin-index: cannot create {}: {e}", parent.display());
        return ExitCode::FAILURE;
    }
    if let Err(e) = fs::write(out, json.as_bytes()) {
        eprintln!("plugin-index: cannot write {out}: {e}");
        return ExitCode::FAILURE;
    }

    println!(
        "plugin-index: wrote {out}: {} plugin(s) indexed",
        index.entries.len()
    );
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_known_test_vectors() {
        // Same NIST/RFC known-answer vectors verify_licenses.rs pins for its own
        // `sha256_hex`, confirming this independently-written copy hashes the same way.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn missing_plugins_dir_yields_an_empty_valid_index() {
        let index = build_index("tests/fixtures/plugins_does_not_exist").expect("not an error");
        assert_eq!(index.entries.len(), 0);
        index.validate().expect("an empty index validates");
    }
}
