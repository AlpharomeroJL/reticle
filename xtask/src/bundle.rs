//! `bundle-size`: measure the built web bundle and maintain the bundle ledger.
//!
//! Walks `crates/web/dist/` and records, for every `.wasm`/`.js`/`.css`/`.html`
//! artifact, the raw byte size and its gzip size at `flate2` best compression, a
//! stand-in for GitHub Pages' on-the-wire cost (close, not identical, so the
//! budget gate compares against a baseline measured the same way). Prints a
//! per-file table plus machine-readable `TOTAL_GZ=` and `WASM_GZ=` lines.
//!
//! - `--assert-delta-kb <N>` fails when the gz total exceeds the last
//!   `v8.0-baseline` row of `docs/design/bundle-ledger.md` by more than N KiB.
//! - `--append-ledger <label>` appends a measured row to that ledger (creating
//!   the file with its header if absent).
//!
//! Nothing here is hard-coded: the numbers come from whatever Trunk last wrote,
//! so `just web-build` must run first. A missing or empty dist is a failure with
//! a message telling the operator to build.

use flate2::Compression;
use flate2::write::GzEncoder;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The ledger row every delta and the budget gate compare against.
const BASELINE_LABEL: &str = "v8.0-baseline";

/// Header written when the ledger file does not exist yet. The gate is
/// self-consistent: both sides of the comparison are flate2 best compression.
const LEDGER_HEADER: &str = "# Bundle ledger\n\n\
Measured by 'cargo run -p xtask -- bundle-size' over crates/web/dist (trunk release build,\n\
wasm-opt=z, content-hashed artifacts). Gzip is flate2 at best compression: it approximates\n\
but does not equal GitHub Pages' on-the-wire compression; the +450 KB gz budget gate\n\
(just bundle-gate) is self-consistent against the v8.0-baseline row below.\n\n\
| date | commit | label | raw wasm | gz wasm | gz total | delta gz vs v8.0-baseline |\n\
|---|---|---|---|---|---|---|\n";

/// One measured dist artifact.
struct Artifact {
    /// Path relative to `dist/`, forward slashes, for stable table output.
    name: String,
    /// Size on disk, bytes.
    raw: u64,
    /// Gzip size at best compression, bytes.
    gz: u64,
}

/// The whole-bundle measurement the flags act on.
struct Measurement {
    /// Every `.wasm`/`.js`/`.css`/`.html` file under dist, sorted by name.
    artifacts: Vec<Artifact>,
    /// Sum of raw sizes.
    total_raw: u64,
    /// Sum of gzip sizes.
    total_gz: u64,
    /// Raw size of the largest `.wasm` (the app module, not the worker).
    wasm_raw: u64,
    /// Gzip size of that same largest `.wasm`.
    wasm_gz: u64,
}

/// Runs `bundle-size`: measure dist, print the table, then apply the optional
/// ledger append and budget assertion.
pub(crate) fn cmd_bundle_size(args: &[String]) -> ExitCode {
    let dist = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../crates/web/dist");
    let measurement = match measure(&dist) {
        Ok(measurement) => measurement,
        Err(err) => {
            eprintln!("bundle-size: {err}");
            return ExitCode::FAILURE;
        }
    };
    print_table(&dist, &measurement);

    // Append before asserting, so a `--append-ledger --assert-delta-kb` combined
    // invocation gates against a ledger that already includes this measurement.
    if args.iter().any(|arg| arg == "--append-ledger") {
        let Some(label) = crate::flag(args, "--append-ledger") else {
            eprintln!("bundle-size: --append-ledger needs a label, e.g. --append-ledger v8.1");
            return ExitCode::FAILURE;
        };
        if let Err(err) = append_ledger(&label, &measurement) {
            eprintln!("bundle-size: {err}");
            return ExitCode::FAILURE;
        }
    }
    if args.iter().any(|arg| arg == "--assert-delta-kb") {
        let Some(budget_kib) =
            crate::flag(args, "--assert-delta-kb").and_then(|v| v.parse::<u64>().ok())
        else {
            eprintln!("bundle-size: --assert-delta-kb needs a whole number of KiB");
            return ExitCode::FAILURE;
        };
        return assert_delta(&measurement, budget_kib);
    }
    ExitCode::SUCCESS
}

/// Measures every bundle artifact under `dist`. Errors when the directory is
/// missing or holds no artifacts, because a gate that measures nothing would
/// pass vacuously.
fn measure(dist: &Path) -> Result<Measurement, String> {
    let mut paths = Vec::new();
    collect_artifacts(dist, &mut paths).map_err(|err| {
        format!(
            "cannot walk {}: {err}; run `just web-build` first",
            dist.display()
        )
    })?;
    if paths.is_empty() {
        return Err(format!(
            "no wasm/js/css/html artifacts in {}; run `just web-build` first",
            dist.display()
        ));
    }

    let mut artifacts = Vec::new();
    for path in paths {
        let bytes =
            std::fs::read(&path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
        let gz =
            gzip_len(&bytes).map_err(|err| format!("gzip of {} failed: {err}", path.display()))?;
        let name = path
            .strip_prefix(dist)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        artifacts.push(Artifact {
            name,
            raw: bytes.len() as u64,
            gz,
        });
    }
    artifacts.sort_by(|a, b| a.name.cmp(&b.name));

    let total_raw = artifacts.iter().map(|a| a.raw).sum();
    let total_gz = artifacts.iter().map(|a| a.gz).sum();
    let Some(wasm) = artifacts
        .iter()
        .filter(|a| {
            Path::new(&a.name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("wasm"))
        })
        .max_by_key(|a| a.raw)
    else {
        return Err(format!(
            "no .wasm artifact in {}; the bundle is incomplete, run `just web-build`",
            dist.display()
        ));
    };
    let (wasm_raw, wasm_gz) = (wasm.raw, wasm.gz);
    Ok(Measurement {
        artifacts,
        total_raw,
        total_gz,
        wasm_raw,
        wasm_gz,
    })
}

/// Recursively collects `.wasm`/`.js`/`.css`/`.html` files. Recursion matters
/// because Trunk layouts have carried subdirectories before (e.g. snippets).
fn collect_artifacts(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_artifacts(&path, out)?;
        } else if let Some(ext) = path.extension().and_then(|ext| ext.to_str())
            && matches!(
                ext.to_ascii_lowercase().as_str(),
                "wasm" | "js" | "css" | "html"
            )
        {
            out.push(path);
        }
    }
    Ok(())
}

/// Gzip-compresses `bytes` at best compression and returns only the compressed
/// length: the output itself is discarded through a counting sink, so the big
/// wasm module never needs a second in-memory copy.
fn gzip_len(bytes: &[u8]) -> std::io::Result<u64> {
    let mut encoder = GzEncoder::new(CountingSink::default(), Compression::best());
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?.0)
}

/// Write sink that counts bytes and drops them.
#[derive(Default)]
struct CountingSink(u64);

impl std::io::Write for CountingSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len() as u64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Prints the per-file table, totals, and the `TOTAL_GZ=`/`WASM_GZ=` lines that
/// scripts parse (stable key=value, independent of table formatting).
fn print_table(dist: &Path, m: &Measurement) {
    println!("bundle-size over {}", dist.display());
    println!("  {:<44} {:>14} {:>14}", "file", "raw bytes", "gz bytes");
    for a in &m.artifacts {
        println!("  {:<44} {:>14} {:>14}", a.name, a.raw, a.gz);
    }
    println!("  {:<44} {:>14} {:>14}", "TOTAL", m.total_raw, m.total_gz);
    println!(
        "  totals: raw {} | gz {} | largest wasm raw {} gz {}",
        human_bytes(m.total_raw),
        human_bytes(m.total_gz),
        human_bytes(m.wasm_raw),
        human_bytes(m.wasm_gz)
    );
    println!("TOTAL_GZ={}", m.total_gz);
    println!("WASM_GZ={}", m.wasm_gz);
}

/// Gates the gz total against the ledger baseline plus `budget_kib`.
fn assert_delta(m: &Measurement, budget_kib: u64) -> ExitCode {
    let path = ledger_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("bundle-size: cannot read {}: {err}", path.display());
            eprintln!(
                "Append the baseline first: cargo run -p xtask -- bundle-size --append-ledger {BASELINE_LABEL}"
            );
            return ExitCode::FAILURE;
        }
    };
    let Some(baseline_gz) = baseline_gz_total(&text) else {
        eprintln!(
            "bundle-size: no '{BASELINE_LABEL}' row in {}; append it first",
            path.display()
        );
        return ExitCode::FAILURE;
    };
    let delta = m.total_gz as i64 - baseline_gz as i64;
    let delta_kib = delta as f64 / 1024.0;
    if m.total_gz > baseline_gz + budget_kib * 1024 {
        eprintln!(
            "bundle-size: FAIL, gz total {} exceeds {BASELINE_LABEL} {} by {delta_kib:+.1} KiB (budget +{budget_kib} KiB).",
            m.total_gz, baseline_gz
        );
        eprintln!(
            "Shrink the bundle or record a deliberate new baseline in docs/design/bundle-ledger.md."
        );
        return ExitCode::FAILURE;
    }
    println!(
        "bundle-size: PASS, gz total {} vs {BASELINE_LABEL} {}: {delta_kib:+.1} KiB (budget +{budget_kib} KiB).",
        m.total_gz, baseline_gz
    );
    ExitCode::SUCCESS
}

/// Appends one measured row to the ledger, creating the file (plus header) on
/// first use so the baseline run bootstraps the document.
fn append_ledger(label: &str, m: &Measurement) -> Result<(), String> {
    let path = ledger_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
    }
    let mut text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => LEDGER_HEADER.to_owned(),
    };

    // The baseline row's own delta is '-': it is the reference, and computing it
    // against an older baseline row would misread as a regression.
    let delta_cell = if label == BASELINE_LABEL {
        "-".to_owned()
    } else {
        let Some(baseline_gz) = baseline_gz_total(&text) else {
            return Err(format!(
                "no '{BASELINE_LABEL}' row to diff against; append it first"
            ));
        };
        signed_bytes(m.total_gz as i64 - baseline_gz as i64)
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| format!("system clock is before 1970: {err}"))?
        .as_secs();
    let row = format!(
        "| {} | {} | {label} | {} | {} | {} | {delta_cell} |\n",
        utc_date(now),
        git_short_hash()?,
        human_bytes(m.wasm_raw),
        human_bytes(m.wasm_gz),
        human_bytes(m.total_gz),
    );
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&row);
    std::fs::write(&path, text).map_err(|err| format!("cannot write {}: {err}", path.display()))?;
    println!("bundle-size: appended '{label}' to {}", path.display());
    Ok(())
}

/// The ledger lives next to the other design notes, not under scratch, because
/// its history is the point.
fn ledger_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/design/bundle-ledger.md")
}

/// Returns the gz-total bytes of the LAST `v8.0-baseline` row, so re-baselining
/// is an append (full history kept), never an edit.
fn baseline_gz_total(ledger_text: &str) -> Option<u64> {
    let mut found = None;
    for line in ledger_text.lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        // A data row splits into 9 cells (leading/trailing empties included);
        // the header and divider rows never carry the baseline label in cell 3.
        if cells.len() < 8 || cells[3] != BASELINE_LABEL {
            continue;
        }
        if let Some(first) = cells[6].split_whitespace().next()
            && let Ok(bytes) = first.parse::<u64>()
        {
            found = Some(bytes);
        }
    }
    found
}

/// The current commit's short hash, for the ledger's provenance column.
fn git_short_hash() -> Result<String, String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."))
        .output()
        .map_err(|err| format!("cannot run git rev-parse: {err}"))?;
    if !out.status.success() {
        return Err(format!(
            "git rev-parse failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Formats a size as exact bytes plus a human rendering, e.g. `8557312 (8.16 MiB)`.
fn human_bytes(bytes: u64) -> String {
    format!("{bytes} ({})", human(bytes))
}

/// Signed variant for the delta column, e.g. `+12345 (+12.06 KiB)`.
fn signed_bytes(delta: i64) -> String {
    let sign = if delta < 0 { "-" } else { "+" };
    let mag = delta.unsigned_abs();
    format!("{sign}{mag} ({sign}{})", human(mag))
}

/// Human `KiB`/`MiB` with two decimals; bytes below 1 KiB stay exact.
fn human(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= MIB {
        format!("{:.2} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.2} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}

/// Converts Unix seconds to a UTC `YYYY-MM-DD`. xtask deliberately has no date
/// dependency; this is Howard Hinnant's `civil_from_days`, exact for all of
/// the proleptic Gregorian calendar.
fn utc_date(secs_since_epoch: u64) -> String {
    let days = (secs_since_epoch / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::{baseline_gz_total, human_bytes, signed_bytes, utc_date};

    #[test]
    fn utc_date_known_values() {
        // Epoch, a leap day, and a post-2038 date (the ledger outlives i32 time_t).
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(951_782_400), "2000-02-29");
        assert_eq!(utc_date(2_222_222_222), "2040-06-02");
    }

    #[test]
    fn human_rendering_matches_ledger_examples() {
        assert_eq!(human_bytes(8_557_312), "8557312 (8.16 MiB)");
        assert_eq!(human_bytes(512), "512 (512 B)");
        assert_eq!(signed_bytes(-2048), "-2048 (-2.00 KiB)");
    }

    #[test]
    fn baseline_row_is_found_last_wins() {
        let ledger = "| date | commit | label | raw wasm | gz wasm | gz total | delta |\n\
                      |---|---|---|---|---|---|---|\n\
                      | 2026-07-08 | abc1234 | v8.0-baseline | 1 (1 B) | 1 (1 B) | 100 (100 B) | - |\n\
                      | 2026-07-09 | abc1235 | v8.1 | 1 (1 B) | 1 (1 B) | 120 (120 B) | +20 (+20 B) |\n\
                      | 2026-07-10 | abc1236 | v8.0-baseline | 1 (1 B) | 1 (1 B) | 130 (130 B) | - |\n";
        assert_eq!(baseline_gz_total(ledger), Some(130));
        assert_eq!(baseline_gz_total("no table here"), None);
    }
}
