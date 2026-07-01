//! `perf-check`: compare fresh Criterion results against the committed baseline.
//!
//! Reads `benches/history/baseline.json` (the source-controlled reference) and the
//! latest Criterion estimates under
//! `$CARGO_TARGET_DIR/criterion/<criterion>/new/estimates.json`, then prints each
//! benchmark's measured value against its baseline and exits non-zero if any
//! benchmark regressed beyond the baseline's `tolerance_pct`.
//!
//! Nothing here is hard-coded: the numbers come from whatever Criterion last wrote,
//! so `cargo bench --workspace` must run first. A missing estimate is treated as a
//! failure (the check cannot be satisfied without fresh data) with a message telling
//! the operator to run the benchmarks.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// One benchmark's committed baseline entry.
struct Baseline {
    /// The benchmark's key in `baseline.json` (and its display name).
    key: String,
    /// The baseline typical value, in `unit`.
    value: f64,
    /// The unit the value is expressed in (`ns`, `us`/`µs`, `ms`, `s`).
    unit: String,
    /// The Criterion output sub-path, e.g. `boolean/self_union_256_squares`.
    criterion: String,
}

/// Runs `perf-check`: load the baseline, read fresh Criterion estimates, compare.
pub(crate) fn perf_check() -> ExitCode {
    let baseline_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../benches/history/baseline.json");
    let text = match std::fs::read_to_string(&baseline_path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("perf-check: cannot read {}: {err}", baseline_path.display());
            return ExitCode::FAILURE;
        }
    };
    let root: Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(err) => {
            eprintln!(
                "perf-check: invalid baseline JSON in {}: {err}",
                baseline_path.display()
            );
            return ExitCode::FAILURE;
        }
    };

    let tolerance = root
        .get("tolerance_pct")
        .and_then(Value::as_f64)
        .unwrap_or(25.0);
    let Some(benchmarks) = root.get("benchmarks").and_then(Value::as_object) else {
        eprintln!("perf-check: baseline has no \"benchmarks\" object");
        return ExitCode::FAILURE;
    };

    let mut entries: Vec<Baseline> = Vec::new();
    for (key, entry) in benchmarks {
        let (Some(value), Some(unit), Some(criterion)) = (
            entry.get("value").and_then(Value::as_f64),
            entry.get("unit").and_then(Value::as_str),
            entry.get("criterion").and_then(Value::as_str),
        ) else {
            eprintln!("perf-check: baseline entry {key} needs numeric value, unit, criterion");
            return ExitCode::FAILURE;
        };
        entries.push(Baseline {
            key: key.clone(),
            value,
            unit: unit.to_owned(),
            criterion: criterion.to_owned(),
        });
    }
    entries.sort_by(|a, b| a.key.cmp(&b.key));

    let criterion_root = criterion_root();
    println!("perf-check against {}", baseline_path.display());
    println!(
        "  host:      {}",
        root.get("host").and_then(Value::as_str).unwrap_or("?")
    );
    println!("  criterion: {}", criterion_root.display());
    println!("  tolerance: +{tolerance:.0}% over baseline\n");

    let mut regressed = false;
    let mut missing = false;
    for entry in &entries {
        let est_path = criterion_root
            .join(&entry.criterion)
            .join("new")
            .join("estimates.json");
        let Some(ns) = read_estimate_ns(&est_path) else {
            println!(
                "  {:<34} baseline={:>10.2} {:<3} measured=       (no fresh estimate)  MISSING",
                entry.key, entry.value, entry.unit
            );
            missing = true;
            continue;
        };
        let measured = from_ns(ns, &entry.unit);
        let change = (measured - entry.value) / entry.value * 100.0;
        let status = if change > tolerance {
            regressed = true;
            "REGRESSED"
        } else if change < -tolerance {
            "improved"
        } else {
            "ok"
        };
        println!(
            "  {:<34} baseline={:>10.2} {:<3} measured={:>10.2} {:<3} change={:>+7.1}%  {status}",
            entry.key, entry.value, entry.unit, measured, entry.unit, change
        );
    }

    if missing {
        eprintln!("\nperf-check: FAIL, one or more benchmarks have no fresh Criterion estimate.");
        eprintln!("Run `cargo bench --workspace` first, then `just perf-check`.");
        return ExitCode::FAILURE;
    }
    if regressed {
        eprintln!("\nperf-check: FAIL, a benchmark regressed by more than +{tolerance:.0}%.");
        return ExitCode::FAILURE;
    }
    println!("\nperf-check: PASS, no benchmark regressed beyond +{tolerance:.0}%.");
    ExitCode::SUCCESS
}

/// Criterion's output root: `$CARGO_TARGET_DIR/criterion` when the target directory
/// is relocated (as it is on this machine), else `<workspace>/target/criterion`.
fn criterion_root() -> PathBuf {
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        return PathBuf::from(dir).join("criterion");
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join("criterion")
}

/// Reads Criterion's typical estimate, in nanoseconds: the regression slope when
/// present (linear-sampled benches), otherwise the mean, the same figure Criterion
/// prints on its `time:` line. Returns `None` if the file is absent or malformed.
fn read_estimate_ns(path: &Path) -> Option<f64> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let slope = value
        .get("slope")
        .and_then(|slope| slope.get("point_estimate"))
        .and_then(Value::as_f64);
    slope.or_else(|| {
        value
            .get("mean")
            .and_then(|mean| mean.get("point_estimate"))
            .and_then(Value::as_f64)
    })
}

/// Converts nanoseconds into the baseline's unit.
fn from_ns(ns: f64, unit: &str) -> f64 {
    match unit {
        "us" | "µs" => ns / 1_000.0,
        "ms" => ns / 1_000_000.0,
        "s" => ns / 1_000_000_000.0,
        // "ns" and anything unrecognized stay in nanoseconds.
        _ => ns,
    }
}
