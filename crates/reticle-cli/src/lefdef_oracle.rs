//! Cross-validate `reticle-lefdef`'s LEF/DEF import against a real EDA tool.
//!
//! `reticle-lefdef` parses a subset of LEF and DEF (ADR 0082) and lowers it to a
//! `LefDefDesign`. This module proves that import is faithful by cross-checking it
//! against `OpenROAD`, a real place-and-route tool, run over the *same* LEF/DEF files
//! inside a pinned Docker container. It mirrors the external-oracle pattern ADR 0054
//! set for the Tiny Tapeout precheck (see the sibling [`tt_precheck`](crate::tt_precheck)
//! module): pin the tool image by digest, run it non-interactively over a mounted work
//! directory, parse its structured output, and skip honestly (never fail) when Docker or
//! the image is absent.
//!
//! # The oracle tool and image
//!
//! The oracle is `OpenROAD`, bundled in `hpretl/iic-osic-tools`, the same all-in-one
//! image ADR 0054 pins for the precheck. The image tag [`ORACLE_IMAGE`] is a dated tag
//! (never `latest`) whose amd64 digest is [`ORACLE_IMAGE_DIGEST`]. Inside the container a
//! short Tcl script ([`ORACLE_TCL`]) does `read_lef` then `read_def` and prints four
//! structured facts as `ORACLE <key>=<value>` lines: the macro count, the component
//! (instance) count, the pin count, and the die area. [`parse_oracle_output`] reads those
//! lines back into [`OracleCounts`], ignoring the banner and log lines the tool also
//! prints.
//!
//! # The compared facts
//!
//! Four structural facts, chosen because both `reticle-lefdef` and `OpenROAD` expose them
//! unambiguously from the same input:
//!
//! - **macros**: LEF `MACRO` cells (`OpenROAD` library masters; `reticle-lefdef` cells
//!   other than the top design cell).
//! - **components**: DEF `COMPONENTS` placed instances.
//! - **pins**: DEF `PINS` external I/O ports.
//! - **die area**: the DEF `DIEAREA` bounding box in database units.
//!
//! Net-level routing is deliberately not compared: it is the richest and least
//! standardized part of DEF, and the four facts above already discriminate a faithful
//! import from a corrupted one. [`OracleCounts::agrees_with`] compares two count sets,
//! with a documented per-coordinate tolerance on the die area for the case where a tool
//! reports it on a different unit grid (zero is the right tolerance here: both sides read
//! DEF database units directly).
//!
//! # Honest skip
//!
//! [`run_oracle`] returns [`OracleOutcome::Skipped`] with a printable reason when Docker
//! is not on the path or the pinned image is not present locally, so a machine without the
//! multi-GB image never fails the gate: the container run is best-effort, and the parser
//! plus the committed fixtures prove the cross-check both ways with no Docker at all.

use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The pinned oracle image reference: a dated tag (`YYYY.MM`), never `latest`. This is
/// the same all-in-one image ADR 0054 pins for the Tiny Tapeout precheck; it bundles
/// `OpenROAD` (and `KLayout`, Magic, and the SKY130 PDK).
pub const ORACLE_IMAGE: &str = "hpretl/iic-osic-tools:2025.01";

/// The amd64 digest of [`ORACLE_IMAGE`], recorded so a live run is reproducible against an
/// exact image and not merely a moving tag.
pub const ORACLE_IMAGE_DIGEST: &str =
    "sha256:a51257b7d85fc75d5a690317539f9787a401d6dd28583d73dceab174ccc9e78f";

/// The oracle tool run inside the container, for the record.
pub const ORACLE_TOOL: &str = "OpenROAD";

/// The Tcl script the container runs: read the LEF, read the DEF, and print the four
/// structural facts as `ORACLE <key>=<value>` lines that [`parse_oracle_output`] reads.
///
/// It uses only `OpenROAD`'s `OpenDB` Tcl API (`read_lef`, `read_def`, and block/library
/// accessors), so it needs no PDK and no flow configuration. Wire and via geometry are not
/// inspected, so an undefined via in the DEF cannot make the read fail; the fixtures keep
/// their routing via-free for the same reason.
pub const ORACLE_TCL: &str = "\
read_lef /work/design.lef
read_def /work/design.def
set db [ord::get_db]
set block [[$db getChip] getBlock]
puts \"ORACLE components=[llength [$block getInsts]]\"
puts \"ORACLE pins=[llength [$block getBTerms]]\"
set die [$block getDieArea]
puts \"ORACLE diearea=[$die xMin] [$die yMin] [$die xMax] [$die yMax]\"
set n 0
foreach lib [$db getLibs] { set n [expr {$n + [llength [$lib getMasters]]}] }
puts \"ORACLE macros=$n\"
";

/// The four structural facts the oracle and the importer are compared on.
///
/// A die area is `[x_min, y_min, x_max, y_max]` in database units. `macros` and
/// `die_area` are optional so a source that does not report one leaves it out rather than
/// asserting a wrong zero; `components` and `pins` default to zero and are always set by a
/// real run.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct OracleCounts {
    /// The number of LEF `MACRO` cells, when known.
    pub macros: Option<usize>,
    /// The number of placed DEF `COMPONENTS` instances.
    pub components: usize,
    /// The number of external DEF `PINS`.
    pub pins: usize,
    /// The die-area bounding box `[x_min, y_min, x_max, y_max]` in database units, when
    /// the design declared one.
    pub die_area: Option<[i64; 4]>,
}

impl OracleCounts {
    /// Whether these counts agree with `other` on every compared fact.
    ///
    /// `components` and `pins` must match exactly. `macros` is compared only when both
    /// sides report a count (so a source that omits it does not force a mismatch). The die
    /// area is compared coordinate by coordinate, each within `die_tolerance_dbu` database
    /// units, and only when both sides report one; pass `0` to require an exact match,
    /// which is correct when both read DEF database units directly.
    #[must_use]
    pub fn agrees_with(&self, other: &Self, die_tolerance_dbu: i64) -> bool {
        if self.components != other.components || self.pins != other.pins {
            return false;
        }
        if let (Some(a), Some(b)) = (self.macros, other.macros)
            && a != b
        {
            return false;
        }
        match (self.die_area, other.die_area) {
            (Some(a), Some(b)) => a
                .iter()
                .zip(b.iter())
                .all(|(x, y)| (x - y).abs() <= die_tolerance_dbu),
            _ => true,
        }
    }
}

/// The result of asking the container oracle for a design's counts.
///
/// A run either produced counts ([`Ran`](OracleOutcome::Ran)) or was skipped because
/// Docker or the pinned image was unavailable ([`Skipped`](OracleOutcome::Skipped)). A
/// skip is never an error: it carries a printable reason so a caller can log why the live
/// cross-check did not run and continue.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum OracleOutcome {
    /// The oracle ran and reported these counts.
    Ran(OracleCounts),
    /// The oracle did not run; the string says why (no Docker, or no pinned image).
    Skipped(String),
}

impl OracleOutcome {
    /// The counts if the oracle ran, or `None` if it was skipped.
    #[must_use]
    pub fn ran(&self) -> Option<&OracleCounts> {
        match self {
            Self::Ran(counts) => Some(counts),
            Self::Skipped(_) => None,
        }
    }
}

/// Parses the oracle's structured output into [`OracleCounts`].
///
/// Reads every `ORACLE <key>=<value>` line and ignores all other output (the tool's
/// banner, `[INFO]` log lines, warnings). Unknown keys are ignored; a malformed value
/// leaves its field at the default. `die area` is four whitespace-separated integers
/// (`x_min y_min x_max y_max`); anything else leaves the die area unset.
#[must_use]
pub fn parse_oracle_output(output: &str) -> OracleCounts {
    let mut counts = OracleCounts::default();
    for line in output.lines() {
        let Some(rest) = line.trim().strip_prefix("ORACLE ") else {
            continue;
        };
        let Some((key, value)) = rest.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "macros" => counts.macros = value.parse().ok(),
            "components" => {
                if let Ok(n) = value.parse() {
                    counts.components = n;
                }
            }
            "pins" => {
                if let Ok(n) = value.parse() {
                    counts.pins = n;
                }
            }
            "diearea" => counts.die_area = parse_die_area(value),
            _ => {}
        }
    }
    counts
}

/// Parses `x_min y_min x_max y_max` into a die-area box, or `None` unless it is exactly
/// four integers.
fn parse_die_area(value: &str) -> Option<[i64; 4]> {
    let mut fields = value.split_whitespace();
    let mut out = [0i64; 4];
    for slot in &mut out {
        *slot = fields.next()?.parse().ok()?;
    }
    if fields.next().is_some() {
        return None;
    }
    Some(out)
}

/// Whether the `docker` CLI is available on the path.
#[must_use]
pub fn docker_available() -> bool {
    Command::new("docker")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Whether `image` is present in the local Docker image store (no pull is attempted).
#[must_use]
pub fn image_present(image: &str) -> bool {
    Command::new("docker")
        .args(["image", "inspect", image])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Runs the oracle over a LEF/DEF pair in the pinned container and returns its counts.
///
/// Writes `lef`, `def`, and [`ORACLE_TCL`] into a fresh work directory, mounts it into the
/// container at `/work`, and runs `OpenROAD` over the script. Returns
/// [`OracleOutcome::Skipped`] (never an error) when Docker is not on the path or the
/// pinned image [`ORACLE_IMAGE`] is not present locally, so a machine without the image
/// does not fail. The work directory is removed when the call returns.
///
/// # Errors
///
/// Returns an [`io::Error`] if the work directory or its files cannot be written, if the
/// `docker run` cannot be launched, or if the container ran but produced no parseable
/// `ORACLE` counts (which would mean the tool itself failed, not that the container was
/// absent).
pub fn run_oracle(lef: &[u8], def: &[u8]) -> io::Result<OracleOutcome> {
    if !docker_available() {
        return Ok(OracleOutcome::Skipped(
            "docker not found on PATH (install Docker to run the LEF/DEF oracle)".to_owned(),
        ));
    }
    if !image_present(ORACLE_IMAGE) {
        return Ok(OracleOutcome::Skipped(format!(
            "pinned oracle image {ORACLE_IMAGE} not present locally (docker pull {ORACLE_IMAGE})"
        )));
    }

    let work = OracleWorkDir::create()?;
    std::fs::write(work.path.join("design.lef"), lef)?;
    std::fs::write(work.path.join("design.def"), def)?;
    std::fs::write(work.path.join("oracle.tcl"), ORACLE_TCL)?;

    // Docker Desktop accepts a Windows path with forward slashes as a bind source.
    let mount = format!("{}:/work", work.path.to_string_lossy().replace('\\', "/"));
    // `--skip` is the image entrypoint's flag to bypass its X11/VNC UI bootstrap and exec
    // the assigned command directly; without it the launcher rejects `bash` (see ADR 0054
    // and scripts/tt-precheck.ps1).
    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &mount,
            ORACLE_IMAGE,
            "--skip",
            "bash",
            "-lc",
            "cd /work && openroad -no_init -exit oracle.tcl",
        ])
        .output()?;

    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    if !combined.contains("ORACLE components=") {
        return Err(io::Error::other(format!(
            "oracle container produced no counts (exit {:?}); output:\n{combined}",
            output.status.code()
        )));
    }
    Ok(OracleOutcome::Ran(parse_oracle_output(&combined)))
}

/// A uniquely named temporary work directory, removed on drop.
struct OracleWorkDir {
    /// The directory path.
    path: PathBuf,
}

impl OracleWorkDir {
    /// Creates a fresh work directory under the system temp directory, named uniquely by
    /// process id and a per-process counter so concurrent calls do not collide.
    fn create() -> io::Result<Self> {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("reticle-lefdef-oracle-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }
}

impl Drop for OracleWorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_four_facts_and_ignores_noise() {
        // A realistic capture: the tool banner and INFO lines around the ORACLE lines.
        let output = "\
OpenROAD v2.0
[INFO ODB-0227] LEF file: /work/design.lef, created 3 layers, 2 library cells
[INFO ODB-0131]     Created 3 components and 8 component-terminals.
ORACLE components=3
ORACLE pins=2
ORACLE diearea=0 0 20000 20000
ORACLE macros=2
";
        let counts = parse_oracle_output(output);
        assert_eq!(counts.macros, Some(2));
        assert_eq!(counts.components, 3);
        assert_eq!(counts.pins, 2);
        assert_eq!(counts.die_area, Some([0, 0, 20_000, 20_000]));
    }

    #[test]
    fn die_area_needs_exactly_four_integers() {
        assert_eq!(
            parse_die_area("0 0 20000 20000"),
            Some([0, 0, 20_000, 20_000])
        );
        assert_eq!(parse_die_area("0 0 20000"), None);
        assert_eq!(parse_die_area("0 0 20000 20000 5"), None);
        assert_eq!(parse_die_area("0 0 x 20000"), None);
    }

    #[test]
    fn missing_lines_leave_defaults_not_wrong_values() {
        let counts = parse_oracle_output("ORACLE components=4\n");
        assert_eq!(counts.components, 4);
        assert_eq!(counts.pins, 0);
        assert_eq!(
            counts.macros, None,
            "an absent macro line stays None, not 0"
        );
        assert_eq!(counts.die_area, None);
    }

    #[test]
    fn agreement_is_exact_on_components_and_pins() {
        let a = OracleCounts {
            macros: Some(2),
            components: 3,
            pins: 2,
            die_area: Some([0, 0, 20_000, 20_000]),
        };
        // Identical counts agree.
        assert!(a.agrees_with(&a.clone(), 0));
        // One fewer component disagrees: this is the two-way discrimination.
        let dropped = OracleCounts {
            components: 2,
            ..a.clone()
        };
        assert!(!a.agrees_with(&dropped, 0));
    }

    #[test]
    fn die_area_tolerance_is_respected() {
        let a = OracleCounts {
            die_area: Some([0, 0, 20_000, 20_000]),
            ..OracleCounts::default()
        };
        let off_by_five = OracleCounts {
            die_area: Some([0, 0, 20_000, 20_005]),
            ..OracleCounts::default()
        };
        assert!(
            !a.agrees_with(&off_by_five, 0),
            "exact match required at tol 0"
        );
        assert!(a.agrees_with(&off_by_five, 10), "within tol 10");
    }

    #[test]
    fn macros_compared_only_when_both_report_it() {
        let with = OracleCounts {
            macros: Some(2),
            ..OracleCounts::default()
        };
        let without = OracleCounts {
            macros: None,
            ..OracleCounts::default()
        };
        // The importer always knows its macro count; the oracle might not. An absent
        // count on one side does not force a mismatch.
        assert!(with.agrees_with(&without, 0));
    }
}
