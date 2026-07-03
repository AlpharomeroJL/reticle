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
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use reticle_bench::{
    BenchTask, CheckerRegistry, ResultRecord, RunOptions, SuiteManifest, Tier, load_suite,
    run_task, summarize, write_records,
};

mod scripts;

/// Run the Reticle agent benchmark suite against the mock model.
#[derive(Parser, Debug)]
#[command(name = "reticle-bench", version, about, long_about = None)]
struct Cli {
    /// The suite directory (holds `manifest.toml` and one `<id>.toml` per task).
    #[arg(long, default_value = "benchmarks/layout-tasks")]
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
fn run(cli: &Cli) -> Result<bool, String> {
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
