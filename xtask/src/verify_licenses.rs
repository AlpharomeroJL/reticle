//! `xtask verify-licenses <dir>`: the redistribution license gate for staged
//! archive content.
//!
//! Before any `.rtla` archive is staged for hosting (the archive Worker under
//! `worker/archive/` serves it to the open web), its redistribution terms must be
//! on record and must actually permit redistribution. This subcommand walks a
//! staged content directory, and for every `*.rtla` archive it reads a sibling
//! NOTICE manifest (`<archive>.rtla.NOTICE`, the provenance style of
//! `corpus/tinytapeout/NOTICE.md`: a source URL and an SPDX identifier). It
//! verifies the SPDX license is on a small redistribution allowlist and EXCLUDES,
//! with a printed `STATUS EXCLUDED` line, any archive whose terms cannot be
//! verified: no manifest, no SPDX line, a compound expression it will not guess at,
//! or a license not on the allowlist.
//!
//! The gate is conservative by construction: an archive ships only when its license
//! is positively verified, so a missing or unrecognized license fails closed rather
//! than leaking to the host.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

// --- lane pipeline-manifest: library-manifest subcommand ---
use reticle_geometry::Rect;
use reticle_index::TileSource;
use reticle_index::gallery_manifest::{
    DieEntry, GalleryManifest, Landmark, License, Provenance, Source, Streaming, View,
};
use reticle_index::tile_source::MmapTileSource;
use sha2::{Digest, Sha256};
// --- end lane pipeline-manifest ---

/// Licenses whose terms permit redistributing the archive to the open web. The list
/// is deliberately small: Apache-2.0, MIT, CC-BY-4.0, the CERN-OHL family (any
/// variant), and the public-domain dedications. Anything not on it fails closed.
fn redistribution_allowed(spdx: &str) -> bool {
    // The CERN Open Hardware Licence has strong/weak/permissive variants
    // (CERN-OHL-S / -W / -P), all of which permit redistribution.
    if spdx.starts_with("CERN-OHL-") {
        return true;
    }
    matches!(
        spdx,
        "Apache-2.0"
            | "MIT"
            | "CC-BY-4.0"
            // Public-domain dedications.
            | "CC0-1.0"
            | "Unlicense"
            | "public-domain"
    )
}

/// Why an archive was excluded, or that it was verified. The message is what the
/// `STATUS` line prints, so it names the archive's problem in a way a human staging
/// content can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The manifest names an allowlisted license; the archive may be redistributed.
    Verified { license: String },
    /// The archive cannot be verified and is excluded from the staged set.
    Excluded { reason: String },
}

impl Verdict {
    pub fn is_verified(&self) -> bool {
        matches!(self, Verdict::Verified { .. })
    }
}

/// One archive's outcome.
#[derive(Debug, Clone)]
pub struct ArchiveOutcome {
    pub archive: PathBuf,
    pub source: Option<String>,
    pub verdict: Verdict,
}

/// The manifest path for an archive: the archive filename with `.NOTICE` appended,
/// e.g. `chip.rtla` -> `chip.rtla.NOTICE`.
fn manifest_path(archive: &Path) -> PathBuf {
    let mut name = archive.as_os_str().to_os_string();
    name.push(".NOTICE");
    PathBuf::from(name)
}

/// Case-insensitively strips `prefix` from the start of `line`, returning the rest.
fn strip_prefix_ci<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let head = line.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        Some(&line[prefix.len()..])
    } else {
        None
    }
}

/// Extracts the first `SPDX-License-Identifier:` value from a manifest's text, if
/// any. The key match is case-insensitive; the value is the trimmed remainder.
fn extract_spdx(contents: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let rest = strip_prefix_ci(line.trim(), "SPDX-License-Identifier:")?;
        let value = rest.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// Extracts the first `Source:` value from a manifest's text, for the report.
fn extract_source(contents: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let rest = strip_prefix_ci(line.trim(), "Source:")?;
        let value = rest.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// Decides one manifest's SPDX value. Compound expressions (`A OR B`, `A WITH ...`)
/// are not evaluated: the gate refuses to guess redistribution rights, so a
/// whitespace-bearing expression fails closed.
fn verdict_for_spdx(spdx: &str) -> Verdict {
    if spdx.split_whitespace().count() != 1 {
        return Verdict::Excluded {
            reason: format!("compound SPDX expression not evaluated: {spdx}"),
        };
    }
    if redistribution_allowed(spdx) {
        Verdict::Verified {
            license: spdx.to_string(),
        }
    } else {
        Verdict::Excluded {
            reason: format!("license not on redistribution allowlist: {spdx}"),
        }
    }
}

/// Verifies one archive against its sibling manifest.
fn verify_archive(archive: &Path) -> ArchiveOutcome {
    let manifest = manifest_path(archive);
    let Ok(contents) = fs::read_to_string(&manifest) else {
        return ArchiveOutcome {
            archive: archive.to_path_buf(),
            source: None,
            verdict: Verdict::Excluded {
                reason: format!(
                    "no license manifest (expected {})",
                    manifest
                        .file_name()
                        .and_then(OsStr::to_str)
                        .unwrap_or("<name>.NOTICE")
                ),
            },
        };
    };
    let source = extract_source(&contents);
    let verdict = match extract_spdx(&contents) {
        Some(spdx) => verdict_for_spdx(&spdx),
        None => Verdict::Excluded {
            reason: "manifest has no SPDX-License-Identifier line".to_string(),
        },
    };
    ArchiveOutcome {
        archive: archive.to_path_buf(),
        source,
        verdict,
    }
}

/// Walks `dir` and verifies every `*.rtla` archive in it. Entries are returned in a
/// stable (path-sorted) order so the printed report is deterministic.
pub fn verify_dir(dir: &Path) -> std::io::Result<Vec<ArchiveOutcome>> {
    let mut archives: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(OsStr::to_str) == Some("rtla"))
        .collect();
    archives.sort();
    Ok(archives.iter().map(|a| verify_archive(a)).collect())
}

/// Handles `verify-licenses <dir>`: prints a `STATUS` line per archive and exits
/// non-zero if the directory is unreadable or any archive was excluded, so a staging
/// run that would ship unverifiable content fails loudly.
pub fn cmd_verify_licenses(dir: Option<&str>) -> ExitCode {
    let Some(dir) = dir else {
        eprintln!("usage: xtask verify-licenses <dir>");
        return ExitCode::FAILURE;
    };
    let dir = Path::new(dir);
    let outcomes = match verify_dir(dir) {
        Ok(outcomes) => outcomes,
        Err(err) => {
            eprintln!("verify-licenses: cannot read {}: {err}", dir.display());
            return ExitCode::FAILURE;
        }
    };

    if outcomes.is_empty() {
        println!(
            "verify-licenses: no .rtla archives found in {}",
            dir.display()
        );
        return ExitCode::SUCCESS;
    }

    for outcome in &outcomes {
        let name = outcome
            .archive
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("<archive>");
        match &outcome.verdict {
            Verdict::Verified { license } => {
                // The provenance source, when the manifest records one, rides the
                // STATUS line so the report shows where a verified archive came from.
                let source = outcome
                    .source
                    .as_deref()
                    .map(|s| format!(" [{s}]"))
                    .unwrap_or_default();
                println!("STATUS VERIFIED {name} ({license}){source}");
            }
            Verdict::Excluded { reason } => {
                println!("STATUS EXCLUDED {name}: {reason}");
            }
        }
    }

    let verified = outcomes.iter().filter(|o| o.verdict.is_verified()).count();
    let excluded = outcomes.len() - verified;
    println!(
        "verify-licenses: {} archive(s) in {}: {verified} verified, {excluded} excluded",
        outcomes.len(),
        dir.display()
    );
    if excluded > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

// --- lane pipeline-manifest: library-manifest subcommand ---
//
// `xtask library-manifest <dir> <dies.json> <out.json>`: the F1 gallery-manifest
// generator (ADR 0101). It re-runs the same [`verify_dir`] gate `verify-licenses`
// uses (so the manifest's license verdicts are exactly what the redistribution gate
// checked, never a second guess), reads each die's real geometry back from the
// `.rtla` archive [`crate`] (`reticle convert`) actually wrote, and merges in the
// editorial metadata (name, technology, provenance, curated landmark) authored in
// `scripts/library/dies.json`. The license itself is never authored in that file:
// it always comes from the CHECKED verdict, so a hand-typed field can never claim a
// verification that did not happen.
//
// Every archive in `dir` must have a matching `dies.json` entry by id and vice versa,
// and the assembled manifest is validated ([`GalleryManifest::validate`]) before it is
// written; any mismatch or validation failure is a hard, fail-closed error rather than
// a silently wrong or partial manifest.

/// Nominal viewport edge, in pixels, a curated "fit the die" landmark targets: the
/// zoom is picked so the die's longer bbox edge fills this many pixels. Integer-only
/// (the project carries no floats in a committed record); see [`fit_view`].
const LANDMARK_TARGET_PX: i64 = 800;

/// One die's editorial and provenance metadata, authored in `scripts/library/dies.json`
/// and merged with its CHECKED license verdict and its archive's real geometry to build
/// its [`DieEntry`]. Deliberately carries no license field: see the module note above.
#[derive(Debug, Clone, serde::Deserialize)]
struct DieSpec {
    /// Must match the `.rtla` / `.rtla.NOTICE` file stem in the library directory.
    id: String,
    name: String,
    technology: String,
    repo: String,
    commit: String,
    url: String,
    /// The cell a curated landmark frames; empty means no landmark is authored for
    /// this die (and none is ever emitted for an excluded die regardless).
    #[serde(default)]
    landmark_cell: String,
    #[serde(default)]
    landmark_label: String,
}

/// Real geometry and size stats read back from a die's own `.rtla` archive: every
/// field is measured from the archive the converter actually wrote, never estimated.
#[derive(Debug, Clone, Copy)]
struct ArchiveStats {
    world: Rect,
    tile_count: u32,
    total_bytes: u64,
}

/// SHA-256 of `bytes`, rendered as lowercase hex (64 chars): the F1 manifest's
/// `License::Verified::text_sha256` and the content-hash archive-key prefix both use
/// this.
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

/// The repo's own canonical license text file for an SPDX id, for the licenses this
/// repo carries verbatim (it ships dual `MIT OR Apache-2.0`; see the workspace
/// `Cargo.toml`). `None` for an SPDX id with no local canonical text (the CERN-OHL
/// family, CC licenses, public-domain dedications); the caller falls back to hashing
/// the archive's own NOTICE text in that case, so every verified die still gets a
/// real, reproducible hash without a network fetch of upstream license text.
fn canonical_license_text_file(spdx: &str) -> Option<&'static str> {
    match spdx {
        "Apache-2.0" => Some("LICENSE-APACHE"),
        "MIT" => Some("LICENSE-MIT"),
        _ => None,
    }
}

/// The SHA-256 (lowercase hex) of the license text backing a `Verified` verdict for
/// `spdx`. Prefers the repo's own canonical license file (see
/// [`canonical_license_text_file`]), resolved as a sibling of `library_dir`'s parent
/// (every library directory this pipeline writes sits directly under the repo root);
/// falls back to hashing `notice_text` when no canonical file is known or found there.
fn license_text_sha256(library_dir: &Path, spdx: &str, notice_text: &str) -> String {
    if let Some(name) = canonical_license_text_file(spdx)
        && let Some(root) = library_dir.parent()
        && let Ok(bytes) = fs::read(root.join(name))
    {
        return sha256_hex(&bytes);
    }
    sha256_hex(notice_text.as_bytes())
}

/// Opens `archive` and reads back its real world bbox, total tile count (summed
/// across every pyramid level), and on-disk byte size.
///
/// # Errors
///
/// Returns a message naming `archive` when it cannot be opened, its header cannot be
/// read, or it cannot be stat'd; never panics on a malformed archive.
fn read_archive_stats(archive: &Path) -> Result<ArchiveStats, String> {
    let source = MmapTileSource::open(archive)
        .map_err(|e| format!("{}: cannot open archive: {e}", archive.display()))?;
    let header = pollster::block_on(source.header())
        .map_err(|e| format!("{}: cannot read archive header: {e}", archive.display()))?;
    let tiles: u64 = header
        .levels
        .iter()
        .map(|l| u64::from(l.cols) * u64::from(l.rows))
        .sum();
    let total_bytes = fs::metadata(archive)
        .map_err(|e| format!("{}: cannot stat archive: {e}", archive.display()))?
        .len();
    Ok(ArchiveStats {
        world: header.world_rect(),
        tile_count: u32::try_from(tiles).unwrap_or(u32::MAX),
        total_bytes,
    })
}

/// A landmark view centred on `world`, zoomed so its longer edge fills roughly
/// [`LANDMARK_TARGET_PX`] pixels. Integer-only, per the project's no-floats-in-a-
/// committed-record rule.
fn fit_view(world: Rect) -> View {
    let width = i64::from(world.max.x) - i64::from(world.min.x);
    let height = i64::from(world.max.y) - i64::from(world.min.y);
    let span = width.max(height).max(1);
    View {
        x_dbu: i64::from(world.min.x) + width / 2,
        y_dbu: i64::from(world.min.y) + height / 2,
        zoom_milli: (LANDMARK_TARGET_PX * 1000 / span).max(1),
    }
}

/// The archive's file name as a display string, or a placeholder if it has none
/// (never panics on an exotic path).
fn file_name_or_placeholder(path: &Path) -> String {
    path.file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("archive.rtla")
        .to_owned()
}

/// The die id an archive's file stem names, e.g. `library/sky130.inv-1.rtla` ->
/// `sky130.inv-1`.
fn archive_id(archive: &Path) -> String {
    archive
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_owned()
}

/// `dir` rendered with forward slashes, so a `notice_path` the manifest writes is
/// copy-pasteable regardless of the host path separator.
fn forward_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Builds one die's manifest entry from its CHECKED verdict, its own archive's real
/// stats, and its authored spec.
///
/// # Errors
///
/// Returns a message (never panics) when the archive cannot be read back; the caller
/// treats this as a fail-closed error rather than writing a manifest with guessed
/// data.
fn build_die_entry(
    library_dir: &Path,
    outcome: &ArchiveOutcome,
    notice_text: &str,
    spec: &DieSpec,
) -> Result<DieEntry, String> {
    let stats = read_archive_stats(&outcome.archive)?;
    let (license, streaming, landmarks) = match &outcome.verdict {
        Verdict::Verified { license: spdx } => {
            let text_sha256 = license_text_sha256(library_dir, spdx, notice_text);
            let archive_bytes = fs::read(&outcome.archive)
                .map_err(|e| format!("{}: {e}", outcome.archive.display()))?;
            let archive_key = format!(
                "{}/{}",
                &sha256_hex(&archive_bytes)[..10],
                file_name_or_placeholder(&outcome.archive)
            );
            let streaming = Some(Streaming {
                archive_key,
                tile_count: stats.tile_count,
                total_bytes: stats.total_bytes,
            });
            let landmarks = if spec.landmark_cell.is_empty() {
                Vec::new()
            } else {
                vec![Landmark {
                    label: spec.landmark_label.clone(),
                    cell: spec.landmark_cell.clone(),
                    view: fit_view(stats.world),
                    layers: Vec::new(),
                }]
            };
            (
                License::Verified {
                    spdx: spdx.clone(),
                    text_sha256,
                },
                streaming,
                landmarks,
            )
        }
        // An excluded die is never uploaded, so it carries no archive key and no
        // landmark deep link, regardless of what `dies.json` authored.
        Verdict::Excluded { reason } => (
            License::Excluded {
                reason: reason.clone(),
            },
            None,
            Vec::new(),
        ),
    };

    Ok(DieEntry {
        id: spec.id.clone(),
        name: spec.name.clone(),
        technology: spec.technology.clone(),
        width_dbu: i64::from(stats.world.max.x) - i64::from(stats.world.min.x),
        height_dbu: i64::from(stats.world.max.y) - i64::from(stats.world.min.y),
        source: Source {
            repo: spec.repo.clone(),
            commit: spec.commit.clone(),
            url: spec.url.clone(),
        },
        license,
        streaming,
        landmarks,
        provenance: Provenance {
            fetched_utc: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            converter: format!("reticle convert {}", env!("CARGO_PKG_VERSION")),
            notice_path: format!("{}/{}.rtla.NOTICE", forward_slash(library_dir), spec.id),
        },
    })
}

/// Reads and parses `scripts/library/dies.json` (or any path in that shape).
///
/// # Errors
///
/// Returns a message (never panics) if the file cannot be read or does not parse.
fn read_dies_meta(path: &Path) -> Result<Vec<DieSpec>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Handles `library-manifest <dir> <dies.json> <out.json>`: builds and writes the F1
/// [`GalleryManifest`] for every `*.rtla` archive in `dir`. See the module note above
/// for the fail-closed cross-checks this performs.
pub fn cmd_library_manifest(args: &[String]) -> ExitCode {
    let (Some(dir), Some(dies_meta), Some(out)) = (args.first(), args.get(1), args.get(2)) else {
        eprintln!("usage: xtask library-manifest <dir> <dies.json> <out.json>");
        return ExitCode::FAILURE;
    };
    let library_dir = Path::new(dir);

    let specs = match read_dies_meta(Path::new(dies_meta)) {
        Ok(specs) => specs,
        Err(e) => {
            eprintln!("library-manifest: {e}");
            return ExitCode::FAILURE;
        }
    };
    let outcomes = match verify_dir(library_dir) {
        Ok(outcomes) => outcomes,
        Err(e) => {
            eprintln!("library-manifest: cannot read {dir}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut by_id: BTreeMap<&str, &DieSpec> = specs.iter().map(|s| (s.id.as_str(), s)).collect();
    if by_id.len() != specs.len() {
        eprintln!("library-manifest: {dies_meta} names a duplicate die id");
        return ExitCode::FAILURE;
    }

    let mut dies = Vec::with_capacity(outcomes.len());
    for outcome in &outcomes {
        let id = archive_id(&outcome.archive);
        let Some(spec) = by_id.remove(id.as_str()) else {
            eprintln!(
                "library-manifest: {} has no matching entry in {dies_meta}",
                outcome.archive.display()
            );
            return ExitCode::FAILURE;
        };
        let notice_text = fs::read_to_string(manifest_path(&outcome.archive)).unwrap_or_default();
        match build_die_entry(library_dir, outcome, &notice_text, spec) {
            Ok(entry) => dies.push(entry),
            Err(e) => {
                eprintln!("library-manifest: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    if !by_id.is_empty() {
        let mut leftover: Vec<&str> = by_id.into_keys().collect();
        leftover.sort_unstable();
        eprintln!(
            "library-manifest: {dies_meta} names {} die(s) with no archive in {dir}: {}",
            leftover.len(),
            leftover.join(", ")
        );
        return ExitCode::FAILURE;
    }

    dies.sort_by(|a, b| a.id.cmp(&b.id));
    let manifest = GalleryManifest { version: 1, dies };
    if let Err(e) = manifest.validate() {
        eprintln!("library-manifest: generated manifest fails validation: {e}");
        return ExitCode::FAILURE;
    }

    let json = match serde_json::to_string_pretty(&manifest) {
        Ok(json) => json,
        Err(e) => {
            eprintln!("library-manifest: cannot serialize manifest: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = fs::write(out, json.as_bytes()) {
        eprintln!("library-manifest: cannot write {out}: {e}");
        return ExitCode::FAILURE;
    }

    let verified = manifest
        .dies
        .iter()
        .filter(|d| matches!(d.license, License::Verified { .. }))
        .count();
    println!(
        "library-manifest: wrote {out}: {} die(s), {verified} verified, {} excluded",
        manifest.dies.len(),
        manifest.dies.len() - verified,
    );
    ExitCode::SUCCESS
}
// --- end lane pipeline-manifest ---

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::Point;

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/staged")
    }

    fn verdict_for(outcomes: &[ArchiveOutcome], stem: &str) -> Verdict {
        let file = format!("{stem}.rtla");
        outcomes
            .iter()
            .find(|o| o.archive.file_name().and_then(OsStr::to_str) == Some(file.as_str()))
            .unwrap_or_else(|| panic!("no outcome for {file}"))
            .verdict
            .clone()
    }

    #[test]
    fn allowlist_accepts_the_redistributable_families() {
        assert!(redistribution_allowed("Apache-2.0"));
        assert!(redistribution_allowed("MIT"));
        assert!(redistribution_allowed("CC-BY-4.0"));
        assert!(redistribution_allowed("CC0-1.0"));
        assert!(redistribution_allowed("Unlicense"));
        assert!(redistribution_allowed("public-domain"));
        // Every CERN-OHL variant redistributes.
        assert!(redistribution_allowed("CERN-OHL-S-2.0"));
        assert!(redistribution_allowed("CERN-OHL-W-2.0"));
        assert!(redistribution_allowed("CERN-OHL-P-2.0"));
    }

    #[test]
    fn allowlist_rejects_everything_else() {
        assert!(!redistribution_allowed("LicenseRef-Proprietary"));
        assert!(!redistribution_allowed("CC-BY-NC-4.0"));
        assert!(!redistribution_allowed("NoSuchLicense-9.9"));
        assert!(!redistribution_allowed(""));
        // A near-miss on the CERN prefix must not sneak through.
        assert!(!redistribution_allowed("CERN-SOMETHING"));
    }

    #[test]
    fn extracts_spdx_and_source_case_insensitively() {
        let text = "Source: https://example.org/x\nspdx-license-identifier:  MIT  ";
        assert_eq!(extract_spdx(text).as_deref(), Some("MIT"));
        assert_eq!(
            extract_source(text).as_deref(),
            Some("https://example.org/x")
        );
    }

    #[test]
    fn compound_expressions_fail_closed() {
        assert_eq!(
            verdict_for_spdx("Apache-2.0 OR MIT"),
            Verdict::Excluded {
                reason: "compound SPDX expression not evaluated: Apache-2.0 OR MIT".to_string()
            }
        );
    }

    // The two-way gate over the committed fixtures: good manifests pass, and every
    // failure mode (unknown license, forbidden license, no SPDX line, no manifest)
    // is excluded with a clear reason, in a single run over one directory.
    #[test]
    fn verified_archives_pass() {
        let outcomes = verify_dir(&fixtures()).unwrap();
        for stem in ["good", "mit", "cern", "cc0", "ccby"] {
            let v = verdict_for(&outcomes, stem);
            assert!(v.is_verified(), "{stem} should be verified, got {v:?}");
        }
    }

    #[test]
    fn unknown_license_is_excluded() {
        assert_eq!(
            verdict_for(&verify_dir(&fixtures()).unwrap(), "unknown"),
            Verdict::Excluded {
                reason: "license not on redistribution allowlist: NoSuchLicense-9.9".to_string()
            }
        );
    }

    #[test]
    fn forbidden_license_is_excluded() {
        assert_eq!(
            verdict_for(&verify_dir(&fixtures()).unwrap(), "forbidden"),
            Verdict::Excluded {
                reason: "license not on redistribution allowlist: LicenseRef-Proprietary"
                    .to_string()
            }
        );
    }

    #[test]
    fn manifest_without_spdx_is_excluded() {
        assert_eq!(
            verdict_for(&verify_dir(&fixtures()).unwrap(), "nospdx"),
            Verdict::Excluded {
                reason: "manifest has no SPDX-License-Identifier line".to_string()
            }
        );
    }

    #[test]
    fn missing_manifest_is_excluded() {
        let v = verdict_for(&verify_dir(&fixtures()).unwrap(), "nomanifest");
        match &v {
            Verdict::Excluded { reason } => assert!(
                reason.starts_with("no license manifest"),
                "unexpected reason: {reason}"
            ),
            Verdict::Verified { .. } => panic!("expected exclusion, got {v:?}"),
        }
    }

    // --- lane pipeline-manifest: library-manifest subcommand tests ---

    #[test]
    fn sha256_hex_matches_known_test_vectors() {
        // NIST/RFC known-answer vectors (independently confirmed with `sha256sum`), so
        // the hex encoding and the hasher wiring are both pinned, not just "produces 64
        // hex chars".
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
    fn sha256_hex_is_always_64_lowercase_hex_chars() {
        for input in [&b""[..], b"abc", b"the quick brown fox"] {
            let hex = sha256_hex(input);
            assert_eq!(hex.len(), 64, "input {input:?}");
            assert!(
                hex.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "not lowercase hex: {hex}"
            );
        }
    }

    #[test]
    fn canonical_license_text_file_covers_the_repos_own_dual_license() {
        assert_eq!(
            canonical_license_text_file("Apache-2.0"),
            Some("LICENSE-APACHE")
        );
        assert_eq!(canonical_license_text_file("MIT"), Some("LICENSE-MIT"));
        // A license this repo has no local canonical text for falls back (tested via
        // `license_text_sha256` below), not to a guessed path.
        assert_eq!(canonical_license_text_file("CC0-1.0"), None);
        assert_eq!(canonical_license_text_file("CERN-OHL-S-2.0"), None);
    }

    #[test]
    fn license_text_sha256_falls_back_to_notice_text_when_no_canonical_file() {
        // CC0-1.0 has no local canonical file, so the hash must be of the NOTICE text,
        // not empty and not a hash of some other file.
        let notice = "Source: https://example.org/x\nSPDX-License-Identifier: CC0-1.0\n";
        let got = license_text_sha256(Path::new("library"), "CC0-1.0", notice);
        assert_eq!(got, sha256_hex(notice.as_bytes()));
    }

    #[test]
    fn license_text_sha256_prefers_the_repos_apache_license_file() {
        // `library_dir`'s parent is this worktree's repo root, which carries
        // `LICENSE-APACHE` (the same real file `crates/reticle-io/tests/corpus/sky130`'s
        // NOTICE.md points at), so an Apache-2.0 verdict hashes that file, not the
        // (deliberately different) NOTICE text passed alongside it.
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let library_dir = repo_root.join("library");
        let license_bytes =
            fs::read(repo_root.join("LICENSE-APACHE")).expect("repo carries LICENSE-APACHE");
        let notice = "Source: https://example.org/x\nSPDX-License-Identifier: Apache-2.0\n";
        let got = license_text_sha256(&library_dir, "Apache-2.0", notice);
        assert_eq!(got, sha256_hex(&license_bytes));
        assert_ne!(got, sha256_hex(notice.as_bytes()));
    }

    #[test]
    fn fit_view_centres_on_the_bbox_and_zooms_to_fit() {
        let world = Rect::new(Point::new(0, 0), Point::new(800, 400));
        let view = fit_view(world);
        assert_eq!(view.x_dbu, 400);
        assert_eq!(view.y_dbu, 200);
        // Longer edge (800) fills LANDMARK_TARGET_PX (800) px: zoom 1000 milli (1.0).
        assert_eq!(view.zoom_milli, 1000);
    }

    #[test]
    fn fit_view_never_zooms_to_zero_on_a_huge_span() {
        let world = Rect::new(Point::new(0, 0), Point::new(i32::MAX, i32::MAX));
        assert!(fit_view(world).zoom_milli >= 1);
    }

    #[test]
    fn archive_id_strips_the_rtla_extension() {
        assert_eq!(
            archive_id(Path::new("library/sky130.inv-1.rtla")),
            "sky130.inv-1"
        );
        assert_eq!(archive_id(Path::new("weird")), "weird");
    }

    #[test]
    fn forward_slash_normalizes_backslashes() {
        assert_eq!(forward_slash(Path::new("library/x.rtla")), "library/x.rtla");
    }

    // --- end lane pipeline-manifest ---
}
