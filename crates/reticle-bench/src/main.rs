//! The `reticle-bench` runner binary.
//!
//! Drives the agent benchmark suite against the deterministic mock model (the only
//! model in this lane; there is no live-model dependency). It can run a whole suite,
//! one tier of a suite, or a single task, writing the [`ResultRecord`]s as JSON and
//! printing a Markdown summary.
//!
//! [`ResultRecord`]: reticle_bench::ResultRecord
//!
//! ```text
//! reticle-bench --suite benchmarks/layout-tasks               # whole suite
//! reticle-bench --suite benchmarks/layout-tasks --tier 1      # one tier
//! reticle-bench --suite benchmarks/layout-tasks --task t1_...  # one task
//! reticle-bench promote cand_...   # promote a mined candidate into the suite
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use reticle_bench::{
    BenchTask, CheckerRegistry, ResultRecord, RunOptions, SuiteManifest, Tier, load_records,
    load_suite, render_leaderboard, run_task, summarize, validate_records, write_records,
};

mod scripts;

/// Run the Reticle agent benchmark suite against the mock model.
#[derive(Parser, Debug)]
#[command(name = "reticle-bench", version, about, long_about = None)]
struct Cli {
    /// The suite directory (holds `manifest.toml` and one `<id>.toml` per task).
    /// Global so it can also follow a subcommand.
    #[arg(long, default_value = "benchmarks/layout-tasks", global = true)]
    suite: PathBuf,

    /// Run only tasks in this tier.
    #[arg(long)]
    tier: Option<u8>,

    /// Run only the task with this id.
    #[arg(long)]
    task: Option<String>,

    /// Use the deterministic mock model. It is the default and only model in this
    /// lane; the flag is accepted for forward compatibility and explicitness.
    #[arg(long, default_value_t = true)]
    mock: bool,

    /// Directory to write the JSON result records into.
    #[arg(long, default_value = "scratch/bench-results")]
    results_dir: PathBuf,

    /// An action other than running the suite.
    #[command(subcommand)]
    command: Option<Command>,
}

/// Actions other than running the suite.
#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Promote a mined candidate into the live suite, but only if its checker
    /// passes the candidate's two-way vectors (the good document is accepted
    /// and the bad document rejected); bumps the suite manifest version.
    Promote {
        /// The candidate id (the file stem under the candidates directory).
        id: String,
        /// The candidates directory; defaults to `<suite>/candidates`.
        #[arg(long)]
        candidates: Option<PathBuf>,
    },
    /// Render the static leaderboard book page from the committed result records.
    ///
    /// This reads only committed records; it never runs the suite. The render is
    /// deterministic (same records produce the same bytes), so regenerating after new
    /// records are committed is the whole workflow for keeping the page current.
    Leaderboard {
        /// The directory tree of committed `*.result.json` records to aggregate.
        #[arg(long, default_value = "benchmarks/results")]
        results: PathBuf,
        /// Where to write the generated Markdown page. Use `-` to print to stdout.
        #[arg(long, default_value = "docs/src/leaderboard.md")]
        out: PathBuf,
    },
    /// Validate a submitted result record file (or a directory of them) against the
    /// schema the leaderboard relies on, rejecting a malformed record with a clear
    /// message. This is the submission gate an external contributor runs before opening a
    /// pull request.
    ValidateRecords {
        /// A `*.result.json` file, or a directory tree of them.
        path: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(all_passed) => {
            if all_passed {
                ExitCode::SUCCESS
            } else {
                // A task failing its check is a real signal for a benchmark run, so a
                // non-clean sweep exits non-zero for scripting.
                ExitCode::from(1)
            }
        }
        Err(message) => {
            eprintln!("reticle-bench: {message}");
            ExitCode::from(2)
        }
    }
}

/// Loads the suite, runs the selected tasks against the mock, writes records, and
/// prints the Markdown summary. Returns whether every run task passed.
///
/// With the `promote` subcommand, promotes a candidate instead: `Ok(false)`
/// (exit 1) is a gate refusal, `Err` (exit 2) a setup failure.
fn run(cli: &Cli) -> Result<bool, String> {
    match &cli.command {
        Some(Command::Promote { id, candidates }) => {
            return run_promote(cli, id, candidates.clone());
        }
        Some(Command::Leaderboard { results, out }) => return run_leaderboard(results, out),
        Some(Command::ValidateRecords { path }) => return run_validate(path),
        None => {}
    }
    if !cli.mock {
        return Err("only the mock model is available in this lane; pass --mock".into());
    }
    let (manifest, tasks) = load_suite(&cli.suite).map_err(|e| e.to_string())?;
    let selected = select(&tasks, cli.tier, cli.task.as_deref());
    if selected.is_empty() {
        return Err(selection_error(cli));
    }

    let mut model = scripts::sample_mock();
    let mut records: Vec<(Tier, ResultRecord)> = Vec::with_capacity(selected.len());
    for task in &selected {
        let technology = resolve_technology(&cli.suite, task)?;
        let registry = CheckerRegistry::for_task(task)?;
        let record = run_task(
            task,
            &mut model,
            &registry,
            &technology,
            &manifest.version,
            RunOptions::default(),
        )
        .map_err(|e| e.to_string())?;
        records.push((task.tier, record));
    }

    let flat: Vec<ResultRecord> = records.iter().map(|(_, r)| r.clone()).collect();
    let file_name = results_file_name(cli);
    let written = write_records(&cli.results_dir, &file_name, &flat).map_err(|e| e.to_string())?;

    let summary = summarize(&records);
    print!("{}", summary.to_markdown(&manifest.version));
    println!("\nWrote {} record(s) to {}", flat.len(), written.display());

    let all_passed = flat.iter().all(|r| r.success);
    Ok(all_passed)
}

/// Promotes candidate `id` through the two-way gate. A refusal (vectors
/// failing, unknown checker, double promotion) prints the reason and returns
/// `Ok(false)` so the process exits 1; success prints what was written.
fn run_promote(cli: &Cli, id: &str, candidates: Option<PathBuf>) -> Result<bool, String> {
    let candidates = candidates.unwrap_or_else(|| cli.suite.join("candidates"));
    match reticle_bench::mining::promote_candidate(&cli.suite, &candidates, id) {
        Ok(promotion) => {
            println!(
                "promoted `{}`: wrote {} and bumped the suite to {}",
                promotion.id,
                promotion.task_path.display(),
                promotion.new_version
            );
            Ok(true)
        }
        Err(e) if e.is_refusal() => {
            eprintln!("bench-promote refused `{id}`: {e}");
            Ok(false)
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Renders the static leaderboard from the committed records under `results` and writes
/// it to `out` (or stdout when `out` is `-`). Returns `Ok(true)` on success; a read or
/// write failure is `Err` (exit 2).
fn run_leaderboard(results: &Path, out: &Path) -> Result<bool, String> {
    let records = load_records(results).map_err(|e| e.to_string())?;
    let page = render_leaderboard(&records);
    if out == Path::new("-") {
        print!("{page}");
    } else {
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("creating {}: {e}", parent.display()))?;
        }
        std::fs::write(out, &page).map_err(|e| format!("writing {}: {e}", out.display()))?;
        println!(
            "wrote the leaderboard ({} record(s)) to {}",
            records.len(),
            out.display()
        );
    }
    Ok(true)
}

/// Validates the submitted record file or directory at `path`. A valid set prints a
/// count and returns `Ok(true)` (exit 0). A malformed submission (a bad record or a file
/// that is not a record array) is a rejection: it prints the reason and returns
/// `Ok(false)` (exit 1). A path that cannot be read at all is a setup failure, returned
/// as `Err` (exit 2), so a submission gate can tell "your record is wrong" from "I could
/// not read the file".
fn run_validate(path: &Path) -> Result<bool, String> {
    match validate_records(path) {
        Ok(records) => {
            println!(
                "validated {} result record(s) under {}",
                records.len(),
                path.display()
            );
            Ok(true)
        }
        // An unreadable path is an environment problem, not a malformed submission.
        Err(e @ reticle_bench::ValidateError::Io { .. }) => Err(e.to_string()),
        Err(e) => {
            eprintln!("validate-records: rejected: {e}");
            Ok(false)
        }
    }
}

/// Selects the tasks to run: filtered by tier and/or task id, or all when neither is
/// given.
fn select(tasks: &[BenchTask], tier: Option<u8>, task: Option<&str>) -> Vec<BenchTask> {
    tasks
        .iter()
        .filter(|t| tier.is_none_or(|want| t.tier == Tier(want)))
        .filter(|t| task.is_none_or(|want| t.id == want))
        .cloned()
        .collect()
}

/// A human-readable explanation of why no task matched the selection.
fn selection_error(cli: &Cli) -> String {
    match (cli.tier, &cli.task) {
        (Some(tier), Some(task)) => format!("no task `{task}` in tier {tier}"),
        (Some(tier), None) => format!("no tasks in tier {tier}"),
        (None, Some(task)) => format!("no task named `{task}`"),
        (None, None) => "the suite contains no tasks".into(),
    }
}

/// The result file name, derived from the selection so separate runs do not clobber.
fn results_file_name(cli: &Cli) -> String {
    match (cli.tier, &cli.task) {
        (_, Some(task)) => format!("task-{task}.json"),
        (Some(tier), None) => format!("tier-{tier}.json"),
        (None, None) => "suite.json".into(),
    }
}

/// Reads the task's technology file text, resolving `technology` relative to the
/// suite directory. An empty return keeps the session's default technology.
fn resolve_technology(suite: &Path, task: &BenchTask) -> Result<String, String> {
    let path = suite.join(&task.technology);
    std::fs::read_to_string(&path)
        .map_err(|e| format!("reading technology {}: {e}", path.display()))
}

/// The manifest is loaded for its version; expose the type so the summary heading and
/// record `suite_version` stay in sync with the loaded suite.
const _: fn() -> SuiteManifest = SuiteManifest::default;

#[cfg(test)]
mod replay_determinism_tests {
    //! Replay-hash determinism over every benchmark transcript (Wave 3 QA).
    //!
    //! Runs the whole suite against the deterministic mock and, for each task,
    //! re-applies the recorded transcript on a fresh session and asserts it
    //! reproduces the exact document hash (the replay contract). It also runs the
    //! whole suite twice and asserts the per-task hashes are identical, so the
    //! benchmark is bit-for-bit reproducible.

    use super::{resolve_technology, scripts};
    use reticle_agent_api::verify_replay;
    use reticle_bench::runner::run_task_with_transcript;
    use reticle_bench::{CheckerRegistry, ResultRecord, RunOptions, load_suite, run_task};
    use std::path::{Path, PathBuf};

    /// The workspace-root suite directory (tests run with the crate dir as CWD).
    fn suite_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/layout-tasks")
    }

    /// Runs the whole suite once, verify-replaying every transcript, and returns the
    /// per-task final document hashes in suite order.
    fn run_all_and_verify_replay() -> Vec<(String, u64)> {
        let suite = suite_dir();
        let (manifest, tasks) = load_suite(&suite).expect("load the suite");
        let mut model = scripts::sample_mock();
        let mut hashes = Vec::with_capacity(tasks.len());
        for task in &tasks {
            let technology = resolve_technology(&suite, task).expect("resolve technology");
            let registry = CheckerRegistry::for_task(task).expect("build the checker registry");
            let (record, transcript) = run_task_with_transcript(
                task,
                &mut model,
                &registry,
                &technology,
                &manifest.version,
                RunOptions::default(),
            )
            .expect("run the task");
            verify_replay(&transcript).unwrap_or_else(|e| {
                panic!(
                    "task `{}` transcript did not replay to its recorded hash: {e}",
                    record.task_id
                )
            });
            hashes.push((record.task_id, transcript.final_hash));
        }
        hashes
    }

    #[test]
    fn every_benchmark_transcript_replays_and_is_deterministic() {
        let first = run_all_and_verify_replay();
        assert!(
            first.len() >= 60,
            "the full suite is loaded (got {} tasks)",
            first.len()
        );
        let second = run_all_and_verify_replay();
        assert_eq!(
            first, second,
            "the benchmark suite is not bit-for-bit deterministic across two runs"
        );
    }

    /// Runs the whole committed suite once against the deterministic mock and returns the
    /// result records in suite order (the frozen output the leaderboard is built from).
    fn run_all_records() -> Vec<ResultRecord> {
        let suite = suite_dir();
        let (manifest, tasks) = load_suite(&suite).expect("load the suite");
        let mut model = scripts::sample_mock();
        let mut records = Vec::with_capacity(tasks.len());
        for task in &tasks {
            let technology = resolve_technology(&suite, task).expect("resolve technology");
            let registry = CheckerRegistry::for_task(task).expect("build the checker registry");
            let record = run_task(
                task,
                &mut model,
                &registry,
                &technology,
                &manifest.version,
                RunOptions::default(),
            )
            .expect("run the task");
            records.push(record);
        }
        records
    }

    #[test]
    fn every_result_record_is_byte_identical_across_two_runs() {
        // The freeze property at the record level: running the whole suite against the
        // MockModel twice must serialize to the same bytes. `wall_ms` is a monotonic step
        // count rather than a clock, and the run order is the manifest order, so nothing
        // time- or hash-order-dependent leaks into the records.
        let first = run_all_records();
        assert!(
            first.len() >= 60,
            "the full suite ran (got {} records)",
            first.len()
        );
        let second = run_all_records();
        let json_first = serde_json::to_string_pretty(&first).expect("serialize run 1");
        let json_second = serde_json::to_string_pretty(&second).expect("serialize run 2");
        assert_eq!(
            json_first, json_second,
            "the same suite and MockModel must yield byte-identical result records twice"
        );
    }
}
