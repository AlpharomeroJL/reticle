//! The `reticle-agent` runner binary.
//!
//! Drives one layout task through the propose-verify-correct loop against a live
//! Anthropic-compatible model, then writes the four artifacts (transcript, GDS, PNG,
//! result record). The task is either a benchmark TOML file (`--task`) or an inline
//! prompt (`--prompt` plus a `--checker`).
//!
//! ```text
//! # A benchmark task file, technology resolved next to it:
//! reticle-agent --task benchmarks/layout-tasks/t1_drc_clean_met1.toml
//!
//! # An inline prompt against the DRC checker, on a given technology:
//! reticle-agent --prompt "Draw a clean met1 rectangle in a cell named top." \
//!     --checker drc_clean --technology benchmarks/layout-tasks/sky130.tech
//!
//! # Point at a local Anthropic-compatible endpoint and a specific model:
//! reticle-agent --task <file> --model claude-opus-4-8 --base-url http://localhost:8080
//! ```
//!
//! The API key is read from `ANTHROPIC_API_KEY` (never a flag or a file). Exit codes:
//! `0` the task passed its check, `1` the task ran but failed its check, `2` a setup or
//! IO error prevented the run.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use clap::Parser;
use reticle_agent::run::document_summary;
use reticle_agent::{AnthropicModel, LoopOptions, run_agent_task};
use reticle_bench::{BenchTask, CheckerRegistry, Tier, load_task};

/// Run one layout task through the agent loop against an Anthropic-compatible model.
#[derive(Parser, Debug)]
#[command(name = "reticle-agent", version, about, long_about = None)]
struct Cli {
    /// A benchmark task TOML file to run. Mutually exclusive with `--prompt`.
    #[arg(long, conflicts_with = "prompt")]
    task: Option<PathBuf>,

    /// An inline natural-language prompt to run (requires `--checker`). Mutually
    /// exclusive with `--task`.
    #[arg(long, requires = "checker")]
    prompt: Option<String>,

    /// The checker deciding pass/fail for an inline `--prompt` (e.g. `drc_clean`,
    /// `rect_present`, or `intent` with `--intent`).
    #[arg(long)]
    checker: Option<String>,

    /// A serialized connectivity intent spec (JSON) for an `intent`-checked inline
    /// prompt.
    #[arg(long)]
    intent: Option<String>,

    /// The technology file to install before the run. Optional for `--task` (defaults
    /// to the task's `technology`, resolved next to the task file); for `--prompt`,
    /// omitting it keeps the session's built-in default technology.
    #[arg(long)]
    technology: Option<PathBuf>,

    /// The model id to request.
    #[arg(long, default_value = reticle_agent::DEFAULT_MODEL)]
    model: String,

    /// The Anthropic-compatible API base URL (`/v1/messages` is appended).
    #[arg(long, default_value = reticle_agent::DEFAULT_BASE_URL)]
    base_url: String,

    /// Maximum propose-verify-correct iterations before giving up.
    #[arg(long, default_value_t = LoopOptions::default().max_iterations)]
    iterations: u32,

    /// Maximum total commands the model may apply across all iterations.
    #[arg(long, default_value_t = LoopOptions::default().command_budget)]
    command_budget: u32,

    /// A suite version string recorded in the result record.
    #[arg(long, default_value = "adhoc")]
    suite_version: String,

    /// Directory the four artifacts are written into.
    #[arg(long, default_value = "scratch/agent-runs")]
    out_dir: PathBuf,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(true) => ExitCode::SUCCESS,
        // The task ran but did not pass its check: a real, scriptable signal.
        Ok(false) => ExitCode::from(1),
        Err(message) => {
            eprintln!("reticle-agent: {message}");
            ExitCode::from(2)
        }
    }
}

/// Loads or builds the task, builds the model, runs the loop, writes artifacts, and
/// prints a summary. Returns whether the task passed its check.
fn run(cli: &Cli) -> Result<bool, String> {
    let (task, technology) = resolve_task(cli)?;

    // `for_task` already yields a `String` error, which `?` propagates directly.
    let registry = CheckerRegistry::for_task(&task)?;

    // Build the live model. The key comes from the environment only; a missing key is a
    // setup error, not a panic.
    let mut model = AnthropicModel::from_env()
        .map_err(|e| e.to_string())?
        .with_base_url(cli.base_url.clone())
        .with_model(cli.model.clone());

    let options = LoopOptions {
        max_iterations: cli.iterations,
        command_budget: cli.command_budget,
    };

    // Measure real wall time for the record (the loop itself never reads the clock).
    let started = Instant::now();
    let outcome = run_agent_task(
        &task,
        &mut model,
        &registry,
        &technology,
        &cli.suite_version,
        options,
        &cli.out_dir,
        0, // placeholder; overwritten below with the measured duration
        // Feed the model the current layout before each proposal.
        |m, session| m.set_document_context(document_summary(session)),
    )
    .map_err(|e| e.to_string())?;
    let wall_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;

    // Re-stamp the record's wall time with the measured duration and rewrite the result
    // artifact so the on-disk record matches the printed one. (The loop takes wall_ms as
    // an argument so tests can pass a fixed value; the CLI measures it here.)
    let mut record = outcome.record.clone();
    record.wall_ms = wall_ms;
    rewrite_result(&outcome.artifacts.result, &record)?;

    report(&task, &record, &outcome);
    Ok(record.success)
}

/// Resolves the CLI arguments into a task and its technology source text.
///
/// For `--task`, loads the [`BenchTask`] and reads its technology (from `--technology`
/// if given, otherwise the task's own `technology` path resolved next to the task
/// file). For `--prompt`, builds a synthetic tier-0 task and reads `--technology` if
/// present (empty text keeps the session default).
fn resolve_task(cli: &Cli) -> Result<(BenchTask, String), String> {
    if let Some(task_path) = &cli.task {
        let task = load_task(task_path).map_err(|e| e.to_string())?;
        let technology = if let Some(p) = &cli.technology {
            read_technology(p)?
        } else {
            // Resolve the task's own technology path relative to the task file.
            let base = task_path.parent().unwrap_or_else(|| Path::new("."));
            read_technology(&base.join(&task.technology))?
        };
        return Ok((task, technology));
    }

    let Some(prompt) = &cli.prompt else {
        return Err("provide either --task <file> or --prompt <text> --checker <name>".into());
    };
    let checker = cli.checker.clone().ok_or("--prompt requires --checker")?;
    let technology = if let Some(p) = &cli.technology {
        read_technology(p)?
    } else {
        String::new()
    };
    let task = BenchTask {
        id: "adhoc".into(),
        tier: Tier(0),
        prompt: prompt.clone(),
        technology: cli
            .technology
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        checker,
        intent: cli.intent.clone(),
    };
    Ok((task, technology))
}

/// Reads a technology file to text, mapping an IO failure to a message.
fn read_technology(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("reading technology {}: {e}", path.display()))
}

/// Rewrites the result artifact with the wall-time-stamped record.
fn rewrite_result(path: &Path, record: &reticle_bench::ResultRecord) -> Result<(), String> {
    let json = serde_json::to_string_pretty(std::slice::from_ref(record))
        .map_err(|e| format!("serializing result record: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("writing {}: {e}", path.display()))
}

/// Prints a human-readable summary of the run to stdout.
fn report(
    task: &BenchTask,
    record: &reticle_bench::ResultRecord,
    outcome: &reticle_agent::RunOutcome,
) {
    println!("task:        {}", task.id);
    println!("model:       {}", record.model);
    println!(
        "result:      {}",
        if record.success { "PASS" } else { "FAIL" }
    );
    println!("iterations:  {}", record.iterations);
    println!(
        "violations:  first={} final={}",
        record.first_proposal_violations, record.final_violations
    );
    println!("wall_ms:     {}", record.wall_ms);
    println!("artifacts:");
    println!("  transcript {}", outcome.artifacts.transcript.display());
    println!("  gds        {}", outcome.artifacts.gds.display());
    match &outcome.artifacts.png {
        Some(p) => println!("  png        {}", p.display()),
        None => println!("  png        (skipped: {})", outcome.render_note),
    }
    println!("  result     {}", outcome.artifacts.result.display());
}
