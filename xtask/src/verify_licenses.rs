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

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
