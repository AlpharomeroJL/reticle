//! The `reticle-agent` runner binary.
//!
//! Drives one layout task through the propose-verify-correct loop against a live model,
//! then writes the four artifacts (transcript, GDS, PNG, result record). The task is
//! either a benchmark TOML file (`--task`) or an inline prompt (`--prompt` plus a
//! `--checker`). Three backends are selectable with `--backend`:
//!
//! - `anthropic` (default): the Anthropic Messages API; key from `ANTHROPIC_API_KEY`.
//! - `ollama`: an OpenAI-compatible Chat Completions endpoint (Ollama by default);
//!   configured entirely from the environment (`RETICLE_MODEL_BASE_URL`,
//!   `RETICLE_MODEL_NAME`, optional `RETICLE_MODEL_API_KEY`).
//! - `claude-code`: Claude Code driven non-interactively as an external *agent system*
//!   (not a `ModelClient`: it brings its own loop). Per task the runner writes an MCP
//!   config launching `reticle-mcp` with server-side transcript capture and a budget,
//!   runs `claude -p` over it, then replays the captured transcript and runs the checker.
//!   The `claude` and `reticle-mcp` binaries are overridable with `RETICLE_CLAUDE_BIN` /
//!   `RETICLE_MCP_BIN`. A missing or unauthenticated CLI is recorded as an honest not-run
//!   (a `<id>.notrun.json` artifact), never a fabricated pass or fail. `--model` picks the
//!   model (an alias like `sonnet` or a full id); `--command-budget` becomes the MCP
//!   server's per-session budget; `--iterations` is unused (Claude Code owns its loop).
//!
//! ```text
//! # A benchmark task file, technology resolved next to it (Anthropic backend):
//! reticle-agent --task benchmarks/layout-tasks/t1_drc_clean_met1.toml
//!
//! # An inline prompt against the DRC checker, on a given technology:
//! reticle-agent --prompt "Draw a clean met1 rectangle in a cell named top." \
//!     --checker drc_clean --technology benchmarks/layout-tasks/sky130.tech
//!
//! # A local Ollama model over the OpenAI-compatible API (env-configured):
//! RETICLE_MODEL_NAME=gpt-oss:16k reticle-agent --backend ollama --task <file>
//! ```
//!
//! The Anthropic key is read from `ANTHROPIC_API_KEY` (never a flag or a file); the
//! optional OpenAI-compatible key from `RETICLE_MODEL_API_KEY`. Exit codes: `0` the task
//! passed its check, `1` the task ran but failed its check, `2` a setup or IO error
//! prevented the run.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use clap::Parser;
use reticle_agent::run::document_summary;
use reticle_agent::{
    AnthropicModel, LoopOptions, OllamaModel, Provenance, RefinementFn, RunOutcome, run_agent_task,
    run_agent_task_refined,
};
use reticle_bench::model::ModelClient;
use reticle_bench::{BenchTask, CheckerRegistry, Tier, load_suite, load_task};

/// Run one layout task through the agent loop against an Anthropic-compatible model.
#[derive(Parser, Debug)]
#[command(name = "reticle-agent", version, about, long_about = None)]
struct Cli {
    /// A whole benchmark suite directory to run (every task, or the subset selected by
    /// `--tier`/`--task-id`) against the chosen backend, writing an aggregate results
    /// file and a Markdown summary. Mutually exclusive with `--task` and `--prompt`.
    #[arg(long, conflicts_with_all = ["task", "prompt"])]
    suite: Option<PathBuf>,

    /// When running a `--suite`, restrict to tasks in this tier.
    #[arg(long, requires = "suite")]
    tier: Option<u8>,

    /// When running a `--suite`, restrict to the task with this id.
    #[arg(long, requires = "suite")]
    task_id: Option<String>,

    /// When running a `--suite`, the directory the aggregate results JSON is written to.
    #[arg(long, default_value = "scratch/agent-suite-results")]
    results_dir: PathBuf,

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

    /// Which model backend to drive.
    #[arg(long, value_enum, default_value_t = Backend::Anthropic)]
    backend: Backend,

    /// The model id to request. For `--backend anthropic` this defaults to the built-in
    /// Anthropic model; for `--backend ollama` it defaults to `RETICLE_MODEL_NAME` (leave
    /// this flag unset to use the environment).
    #[arg(long)]
    model: Option<String>,

    /// The Anthropic-compatible API base URL (`/v1/messages` is appended). Only used by
    /// `--backend anthropic`; the ollama backend takes its base URL from
    /// `RETICLE_MODEL_BASE_URL`.
    #[arg(long, default_value = reticle_agent::DEFAULT_BASE_URL)]
    base_url: String,

    /// An optional quantization label (for example `Q4_K_M`) recorded on the result so
    /// local runs at different quantizations are distinguishable.
    #[arg(long)]
    quantization: Option<String>,

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

/// The selectable model backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum Backend {
    /// The Anthropic Messages API (or an Anthropic-compatible proxy).
    Anthropic,
    /// An OpenAI-compatible Chat Completions endpoint (Ollama by default).
    Ollama,
    /// Claude Code driven non-interactively as an external *agent system*: per task the
    /// runner generates an MCP config that launches `reticle-mcp` (with server-side
    /// transcript capture and a command budget), runs `claude -p` over that server, then
    /// replays the captured transcript and runs the task's checker. Unlike the other two
    /// backends this is not a `ModelClient`: Claude Code brings its own reasoning loop. A
    /// missing or unauthenticated CLI is recorded as an honest not-run, never a fabricated
    /// pass or fail. See [`reticle_agent::claude_code`].
    ClaudeCode,
}

impl Backend {
    /// The provenance label stamped on the result record.
    fn label(self) -> &'static str {
        match self {
            Backend::Anthropic => "anthropic",
            Backend::Ollama => "ollama",
            Backend::ClaudeCode => reticle_agent::claude_code::BACKEND_LABEL,
        }
    }
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

/// Dispatches to suite mode (`--suite`) or single-task mode. Returns whether every run
/// task passed its check.
fn run(cli: &Cli) -> Result<bool, String> {
    if cli.suite.is_some() {
        run_suite(cli)
    } else {
        run_single(cli)
    }
}

/// Single-task mode: loads or builds the task, builds the selected backend's model, runs
/// the loop, writes the four artifacts, and prints a summary.
fn run_single(cli: &Cli) -> Result<bool, String> {
    let (task, technology) = resolve_task(cli)?;

    // `for_task` already yields a `String` error, which `?` propagates directly.
    let registry = CheckerRegistry::for_task(&task)?;

    // The Claude-Code backend is an external agent system, not a `ModelClient`: it does
    // not run the propose-verify-correct loop. Route it to its own driver, which returns
    // a ran-or-not-run outcome rather than a loop record.
    if cli.backend == Backend::ClaudeCode {
        let outcome = run_claude_one(cli, &task, &technology, &registry)?;
        return Ok(claude_report(&task, &outcome));
    }

    let options = LoopOptions {
        max_iterations: cli.iterations,
        command_budget: cli.command_budget,
    };
    let provenance = Provenance {
        backend: cli.backend.label().to_owned(),
        quantization: cli.quantization.clone(),
    };

    // Build the selected backend and drive the loop. Both backends implement the same
    // context hook (feed the current layout before each proposal); the model types
    // differ, so each arm calls the generic driver with its own concrete model.
    let mut model = build_model(cli)?;
    let (success, record, outcome) = drive_one(
        cli,
        &task,
        &technology,
        &registry,
        options,
        &provenance,
        &mut model,
    )?;
    report(&task, &record, &outcome);
    Ok(success)
}

/// Suite mode: loads a whole suite, runs each selected task through the chosen backend
/// with a fresh model per task, writes an aggregate results JSON, and prints a Markdown
/// summary. A fresh model per task means the conversation buffer never bleeds between
/// tasks. Returns whether every run task passed.
fn run_suite(cli: &Cli) -> Result<bool, String> {
    let suite_dir = cli.suite.clone().expect("suite mode requires --suite");
    let (manifest, tasks) = load_suite(&suite_dir).map_err(|e| e.to_string())?;
    let selected: Vec<BenchTask> = tasks
        .into_iter()
        .filter(|t| cli.tier.is_none_or(|want| t.tier == Tier(want)))
        .filter(|t| cli.task_id.as_deref().is_none_or(|want| t.id == want))
        .collect();
    if selected.is_empty() {
        return Err("no task matched the suite selection".into());
    }

    // The Claude-Code backend has its own suite driver (ran-or-not-run per task, no loop).
    if cli.backend == Backend::ClaudeCode {
        return run_claude_suite(cli, &suite_dir, &manifest, &selected);
    }

    let options = LoopOptions {
        max_iterations: cli.iterations,
        command_budget: cli.command_budget,
    };
    let provenance = Provenance {
        backend: cli.backend.label().to_owned(),
        quantization: cli.quantization.clone(),
    };

    let mut rows: Vec<(Tier, reticle_bench::ResultRecord)> = Vec::with_capacity(selected.len());
    for task in &selected {
        let technology = resolve_suite_technology(&suite_dir, task)?;
        let registry = CheckerRegistry::for_task(task)?;
        // A fresh model per task keeps each task's conversation buffer independent.
        let mut model = build_model(cli)?;
        let (_success, record, outcome) = drive_one(
            cli,
            task,
            &technology,
            &registry,
            options,
            &provenance,
            &mut model,
        )?;
        report(task, &record, &outcome);
        rows.push((task.tier, record));
    }

    // Aggregate results file plus a Markdown summary, mirroring the bench writer shape.
    let flat: Vec<reticle_bench::ResultRecord> = rows.iter().map(|(_, r)| r.clone()).collect();
    let file_name = format!("suite-{}.json", provenance.backend);
    let written = reticle_bench::write_records(&cli.results_dir, &file_name, &flat)
        .map_err(|e| e.to_string())?;
    let summary = reticle_bench::summarize(&rows);
    print!("{}", summary.to_markdown(&manifest.version));
    println!("\nWrote {} record(s) to {}", flat.len(), written.display());

    Ok(flat.iter().all(|r| r.success))
}

/// Builds the Claude-Code backend config from the CLI and environment.
///
/// The `claude` and `reticle-mcp` binary paths come from the environment
/// ([`RETICLE_CLAUDE_BIN`](reticle_agent::claude_code::ENV_CLAUDE_BIN) /
/// [`RETICLE_MCP_BIN`](reticle_agent::claude_code::ENV_MCP_BIN)), defaulting to `claude` on
/// `PATH` and a sibling `reticle-mcp`. The `--model` flag chooses the model (an alias like
/// `sonnet` or a full id); the `--command-budget` flag becomes the MCP server's per-session
/// budget. Artifacts land under `--out-dir`.
fn build_claude_config(cli: &Cli) -> reticle_agent::ClaudeCodeConfig {
    reticle_agent::ClaudeCodeConfig::from_env(
        cli.model.clone(),
        cli.command_budget,
        cli.out_dir.clone(),
        cli.suite_version.clone(),
    )
}

/// Drives one task through the Claude-Code backend, measuring wall time.
fn run_claude_one(
    cli: &Cli,
    task: &BenchTask,
    technology: &str,
    registry: &CheckerRegistry,
) -> Result<reticle_agent::ClaudeTaskOutcome, String> {
    let config = build_claude_config(cli);
    let runner = reticle_agent::SystemClaudeRunner;
    let started = Instant::now();
    let outcome =
        reticle_agent::run_claude_code_task(task, &config, registry, technology, &runner, 0)
            .map_err(|e| e.to_string())?;
    let wall_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    // Re-stamp the measured wall time onto a ran record and rewrite its result artifact so
    // the on-disk record matches the printed one (a not-run carries no wall time).
    restamp_claude_wall(outcome, wall_ms)
}

/// Re-stamps a ran outcome's `wall_ms` with the measured duration and rewrites its result
/// artifact; a not-run is returned unchanged.
fn restamp_claude_wall(
    outcome: reticle_agent::ClaudeTaskOutcome,
    wall_ms: u64,
) -> Result<reticle_agent::ClaudeTaskOutcome, String> {
    use reticle_agent::ClaudeTaskOutcome::{NotRun, Ran};
    match outcome {
        Ran {
            mut record,
            artifacts,
        } => {
            record.wall_ms = wall_ms;
            rewrite_result(&artifacts.result, &record)?;
            Ok(Ran { record, artifacts })
        }
        other @ NotRun { .. } => Ok(other),
    }
}

/// Suite mode for the Claude-Code backend: drives every selected task, writes the ran
/// records to an aggregate results JSON (not-runs are recorded separately, never as
/// pass/fail rows), and prints a Markdown summary plus a not-run tally. Returns whether
/// every task that *ran* passed (a suite that is entirely not-run returns `false`, since
/// nothing was verified).
fn run_claude_suite(
    cli: &Cli,
    suite_dir: &Path,
    manifest: &reticle_bench::SuiteManifest,
    selected: &[BenchTask],
) -> Result<bool, String> {
    let config = build_claude_config(cli);
    let runner = reticle_agent::SystemClaudeRunner;

    let mut rows: Vec<(Tier, reticle_bench::ResultRecord)> = Vec::new();
    let mut not_run: Vec<reticle_agent::NotRunRecord> = Vec::new();
    for task in selected {
        let technology = resolve_suite_technology(suite_dir, task)?;
        let registry = CheckerRegistry::for_task(task)?;
        let started = Instant::now();
        let outcome =
            reticle_agent::run_claude_code_task(task, &config, &registry, &technology, &runner, 0)
                .map_err(|e| e.to_string())?;
        let wall_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let outcome = restamp_claude_wall(outcome, wall_ms)?;
        claude_report(task, &outcome);
        match outcome {
            reticle_agent::ClaudeTaskOutcome::Ran { record, .. } => {
                rows.push((task.tier, record));
            }
            reticle_agent::ClaudeTaskOutcome::NotRun { record, .. } => not_run.push(record),
        }
    }

    // Aggregate results file (ran records only) plus a Markdown summary, mirroring the
    // other backends' writer shape. Not-runs are tallied separately and never counted as
    // pass/fail.
    let flat: Vec<reticle_bench::ResultRecord> = rows.iter().map(|(_, r)| r.clone()).collect();
    let file_name = format!("suite-{}.json", reticle_agent::claude_code::BACKEND_LABEL);
    let written = reticle_bench::write_records(&cli.results_dir, &file_name, &flat)
        .map_err(|e| e.to_string())?;
    let summary = reticle_bench::summarize(&rows);
    print!("{}", summary.to_markdown(&manifest.version));
    println!(
        "\nWrote {} ran record(s) to {}",
        flat.len(),
        written.display()
    );
    if !not_run.is_empty() {
        // Persist the honest not-run list so a broken environment is auditable, and print
        // a tally so a reader is not misled into thinking those tasks passed or failed.
        let notrun_file = format!(
            "suite-{}-notrun.json",
            reticle_agent::claude_code::BACKEND_LABEL
        );
        let notrun_path = write_notrun_list(&cli.results_dir, &notrun_file, &not_run)?;
        println!(
            "Not run (CLI absent or unauthenticated): {} task(s); recorded honestly in {}",
            not_run.len(),
            notrun_path.display()
        );
        for record in &not_run {
            println!("  not-run {}: {}", record.task_id, record.reason);
        }
    }

    // A task that ran and passed counts as a pass; a not-run is neither. The suite is a
    // success only if at least one task ran and every task that ran passed.
    Ok(!flat.is_empty() && flat.iter().all(|r| r.success))
}

/// Writes the honest not-run list as a pretty JSON array under `dir/file_name`.
fn write_notrun_list(
    dir: &Path,
    file_name: &str,
    records: &[reticle_agent::NotRunRecord],
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let json = serde_json::to_string_pretty(records)
        .map_err(|e| format!("serializing not-run records: {e}"))?;
    let path = dir.join(file_name);
    std::fs::write(&path, json).map_err(|e| format!("writing {}: {e}", path.display()))?;
    Ok(path)
}

/// Prints a human-readable summary of a Claude-Code task outcome and returns whether it
/// counts as a pass (a not-run is not a pass).
fn claude_report(task: &BenchTask, outcome: &reticle_agent::ClaudeTaskOutcome) -> bool {
    use reticle_agent::ClaudeTaskOutcome::{NotRun, Ran};
    match outcome {
        Ran { record, artifacts } => {
            println!("task:        {}", task.id);
            println!("backend:     {}", record.backend);
            println!("model:       {}", record.model);
            println!(
                "result:      {}",
                if record.success { "PASS" } else { "FAIL" }
            );
            println!("applied:     {}", record.iterations);
            println!("violations:  final={}", record.final_violations);
            println!("wall_ms:     {}", record.wall_ms);
            println!("artifacts:");
            println!("  mcp config {}", artifacts.config.display());
            println!("  transcript {}", artifacts.transcript.display());
            println!("  result     {}", artifacts.result.display());
            record.success
        }
        NotRun {
            record,
            notrun_path,
            ..
        } => {
            println!("task:        {}", task.id);
            println!("backend:     {}", record.backend);
            println!("model:       {}", record.model);
            println!("result:      NOT RUN");
            println!("reason:      {}", record.reason);
            println!("  not-run    {}", notrun_path.display());
            false
        }
    }
}

/// Builds the selected backend's model from the CLI and environment.
///
/// Returns a boxed [`ModelClient`] so both backends flow through one type; the concrete
/// model still implements [`WithDocumentContext`] for the loop's context hook (dispatched
/// via [`DynModel`]).
fn build_model(cli: &Cli) -> Result<DynModel, String> {
    match cli.backend {
        Backend::Anthropic => {
            // The key comes from the environment only; a missing key is a setup error.
            let mut model = AnthropicModel::from_env()
                .map_err(|e| e.to_string())?
                .with_base_url(cli.base_url.clone());
            if let Some(m) = &cli.model {
                model = model.with_model(m.clone());
            }
            Ok(DynModel::Anthropic(model))
        }
        Backend::Ollama => {
            // Configured from the environment; a `--model` flag overrides
            // RETICLE_MODEL_NAME. A missing model id is a clean setup error.
            let mut model = OllamaModel::from_env();
            if let Some(m) = &cli.model {
                model = model.with_model(m.clone());
            }
            if !model.has_model() {
                return Err(reticle_agent::OllamaBuildError::MissingModel.to_string());
            }
            Ok(DynModel::Ollama(model))
        }
        // The Claude-Code backend is not a `ModelClient` and is dispatched to its own
        // driver in `run_single`/`run_suite` before `build_model` is ever called, so this
        // arm is unreachable; return a clear error rather than panicking if the routing
        // ever regresses.
        Backend::ClaudeCode => Err(
            "internal error: the claude-code backend does not build a ModelClient; \
             it must be routed to run_claude_one / run_claude_suite"
                .to_owned(),
        ),
    }
}

/// Drives one task through the loop against `model`, measures wall time, re-stamps the
/// record's wall time, and rewrites the result artifact. Returns the pass flag, the
/// stamped record, and the run outcome (paths). Shared by single-task and suite modes.
fn drive_one(
    cli: &Cli,
    task: &BenchTask,
    technology: &str,
    registry: &CheckerRegistry,
    options: LoopOptions,
    provenance: &Provenance,
    model: &mut DynModel,
) -> Result<(bool, reticle_bench::ResultRecord, RunOutcome), String> {
    // Measure real wall time for the record (the loop itself never reads the clock).
    let started = Instant::now();
    // Feed the model the current layout before each proposal.
    let context_hook =
        |m: &mut DynModel, session: &_| m.set_document_context(document_summary(session));
    let outcome: RunOutcome = match &task.refinement {
        // An iterative-refinement task: the initial prompt drives iteration 0, then the
        // scripted follow-up constraint is folded into the model's feedback before
        // iteration 1 (after the first proposal), exactly as a live user would add a
        // constraint mid-session. This routes through lane 3C's refinement seam
        // (`run_agent_task_refined` + a `RefinementSource`) rather than restarting the run.
        Some(refinement) => {
            let refinement = refinement.clone();
            run_agent_task_refined(
                task,
                model,
                registry,
                technology,
                &cli.suite_version,
                options,
                &cli.out_dir,
                0, // placeholder; overwritten below with the measured duration
                provenance,
                context_hook,
                RefinementFn(move |iteration: u32| {
                    if iteration == 1 {
                        vec![refinement.clone()]
                    } else {
                        Vec::new()
                    }
                }),
            )
        }
        // An ordinary single-shot task runs with no mid-session refinements.
        None => run_agent_task(
            task,
            model,
            registry,
            technology,
            &cli.suite_version,
            options,
            &cli.out_dir,
            0, // placeholder; overwritten below with the measured duration
            provenance,
            context_hook,
        ),
    }
    .map_err(|e| e.to_string())?;
    let wall_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;

    // Re-stamp the record's wall time with the measured duration and rewrite the result
    // artifact so the on-disk record matches the printed one. (The loop takes wall_ms as
    // an argument so tests can pass a fixed value; the CLI measures it here.)
    let mut record = outcome.record.clone();
    record.wall_ms = wall_ms;
    rewrite_result(&outcome.artifacts.result, &record)?;
    Ok((record.success, record, outcome))
}

/// One of the two concrete backend models, unified into a single type so the loop driver
/// is not generic and both single-task and suite modes share one code path.
///
/// Delegates [`ModelClient`] and the document-context hook to whichever variant is held.
enum DynModel {
    /// The Anthropic Messages API backend.
    Anthropic(AnthropicModel),
    /// The OpenAI-compatible (Ollama) backend.
    Ollama(OllamaModel),
}

impl DynModel {
    /// Sets the document snapshot the next proposal is conditioned on (delegated to the
    /// held backend).
    fn set_document_context(&self, context: String) {
        match self {
            DynModel::Anthropic(m) => m.set_document_context(context),
            DynModel::Ollama(m) => m.set_document_context(context),
        }
    }
}

impl ModelClient for DynModel {
    fn id(&self) -> &str {
        match self {
            DynModel::Anthropic(m) => m.id(),
            DynModel::Ollama(m) => m.id(),
        }
    }

    fn propose(
        &mut self,
        task_id: &str,
        prompt: &str,
        context: &reticle_bench::model::Context,
    ) -> Vec<reticle_agent_api::AgentCommand> {
        match self {
            DynModel::Anthropic(m) => m.propose(task_id, prompt, context),
            DynModel::Ollama(m) => m.propose(task_id, prompt, context),
        }
    }
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
        // An inline `--prompt` run carries no scripted refinement; a `--task` file that
        // declares one is honored through the suite/single refinement path below.
        refinement: None,
    };
    Ok((task, technology))
}

/// Reads a technology file to text, mapping an IO failure to a message.
fn read_technology(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("reading technology {}: {e}", path.display()))
}

/// Reads a suite task's technology text, resolving its `technology` path relative to the
/// suite directory (mirrors the bench runner's resolver so both agree).
fn resolve_suite_technology(suite: &Path, task: &BenchTask) -> Result<String, String> {
    read_technology(&suite.join(&task.technology))
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
    println!("backend:     {}", record.backend);
    println!("model:       {}", record.model);
    if let Some(q) = &record.quantization {
        println!("quantization: {q}");
    }
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
