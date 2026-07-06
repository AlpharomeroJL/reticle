//! The `claude-code` backend: drive Claude Code non-interactively as an agent system.
//!
//! Unlike the [`anthropic`](crate::model) and [`ollama`](crate::ollama) backends, which
//! are [`ModelClient`](reticle_bench::model::ModelClient)s the harness's own
//! propose-verify-correct loop asks for commands one iteration at a time, Claude Code is
//! an **agent system**: it brings its own reasoning loop and tool-calling scaffold. So
//! this backend deliberately does **not** implement
//! [`ModelClient`](reticle_bench::model::ModelClient). Instead, per task, it:
//!
//! 1. Writes an MCP config JSON that launches the `reticle-mcp` binary with server-side
//!    transcript capture on (`RETICLE_MCP_TRANSCRIPT` set to a per-task JSONL path) and a
//!    command budget (`RETICLE_MCP_BUDGET`). The serial step (ADR 0051) already made
//!    `reticle-mcp` stream every applied command to that JSONL.
//! 2. Launches Claude Code non-interactively (`claude -p "<prompt>" --mcp-config <config>
//!    --strict-mcp-config --model <model> --output-format stream-json --permission-mode
//!    bypassPermissions --allowed-tools <the reticle-mcp tool names>`). One `claude`
//!    session per task; Claude Code drives the `reticle-mcp` tools itself, the server
//!    enforces the budget and captures the transcript.
//! 3. Reconstructs the resulting document by replaying the captured transcript, runs the
//!    task's [`Checker`], and records a [`ResultRecord`] with `backend = "claude-code"`
//!    and `model = <the model the run reports>`.
//! 4. If the `claude` CLI is missing, or the session fails to authenticate (or is out of
//!    quota), the task is recorded as an honest **not run** ([`NotRunRecord`]), never a
//!    fabricated pass or fail.
//!
//! # Honesty
//!
//! A not-run is never a [`ResultRecord`]: it is a distinct [`NotRunRecord`] artifact, so
//! it can never be counted as a pass or a fail in a summary. A run that completed but
//! whose document did not satisfy the checker is a real `success = false` record, exactly
//! like the other backends. Nothing here retro-edits a failure into a pass.
//!
//! # Testability
//!
//! The `claude` executable path is overridable with [`ENV_CLAUDE_BIN`]
//! (`RETICLE_CLAUDE_BIN`), defaulting to `claude` on `PATH`, and the `reticle-mcp` path
//! with [`ENV_MCP_BIN`] (`RETICLE_MCP_BIN`), defaulting to a sibling of the current
//! executable. The subprocess-spawn step is a [`ClaudeRunner`] seam: the production
//! [`SystemClaudeRunner`] spawns the real CLI, while a deterministic test injects a fake
//! runner that itself connects to the generated `--mcp-config` server over stdio and
//! applies a couple of scripted commands (proving the config is correct and the capture
//! path works), and another fake that simulates the CLI being absent (proving the not-run
//! path). The real `claude -p` smoke and the full suite are the orchestrator's step, not
//! this module's tests: they consume quota and are non-deterministic.

use std::path::{Path, PathBuf};

use reticle_agent_api::{AgentCommand, Session};
use reticle_bench::{BenchTask, CheckResult, Checker, CheckerRegistry, ResultRecord};
use serde::{Deserialize, Serialize};

/// The backend label stamped on every [`ResultRecord`] this driver produces.
pub const BACKEND_LABEL: &str = "claude-code";

/// The logical name of the MCP server in the generated config. Claude Code namespaces a
/// server's tools as `mcp__<server>__<tool>`, so this is the `<server>` segment of every
/// allowed-tool name.
pub const MCP_SERVER_NAME: &str = "reticle";

/// Environment variable overriding the `claude` executable. Defaults to `claude` (found
/// on `PATH`). A deterministic test points this at a fake CLI.
pub const ENV_CLAUDE_BIN: &str = "RETICLE_CLAUDE_BIN";

/// Environment variable overriding the `reticle-mcp` executable named in the generated
/// MCP config. Defaults to a `reticle-mcp` binary sibling to the current executable, or
/// bare `reticle-mcp` on `PATH` when that cannot be resolved.
pub const ENV_MCP_BIN: &str = "RETICLE_MCP_BIN";

/// The model this backend requests when the caller does not pass one. Claude Code accepts
/// an alias (`opus`, `sonnet`, ...) or a full model id; the alias keeps the run pinned to
/// a family without hardcoding a dated id here.
pub const DEFAULT_MODEL: &str = "sonnet";

/// The environment variable the `reticle-mcp` binary reads for its transcript sink path.
const RETICLE_MCP_TRANSCRIPT: &str = "RETICLE_MCP_TRANSCRIPT";

/// The environment variable the `reticle-mcp` binary reads for its command budget.
const RETICLE_MCP_BUDGET: &str = "RETICLE_MCP_BUDGET";

/// Resolved configuration for one Claude-Code-driven run.
#[derive(Clone, Debug)]
pub struct ClaudeCodeConfig {
    /// The `claude` executable to launch (from [`ENV_CLAUDE_BIN`], else `claude`).
    pub claude_bin: PathBuf,
    /// The `reticle-mcp` executable named in the generated MCP config (from
    /// [`ENV_MCP_BIN`], else a sibling of the current executable).
    pub mcp_bin: PathBuf,
    /// The model id or alias to request. `None` uses [`DEFAULT_MODEL`].
    pub model: Option<String>,
    /// The per-task command budget handed to `reticle-mcp` via `RETICLE_MCP_BUDGET`.
    pub command_budget: u32,
    /// The directory the per-task artifacts (MCP config, transcript, result / not-run
    /// record) are written under.
    pub out_dir: PathBuf,
    /// The suite version stamped on the [`ResultRecord`].
    pub suite_version: String,
}

impl ClaudeCodeConfig {
    /// Builds a config from the environment and the given parameters.
    ///
    /// Reads [`ENV_CLAUDE_BIN`] and [`ENV_MCP_BIN`], falling back to `claude` on `PATH`
    /// and a `reticle-mcp` sibling of the current executable respectively.
    #[must_use]
    pub fn from_env(
        model: Option<String>,
        command_budget: u32,
        out_dir: PathBuf,
        suite_version: String,
    ) -> Self {
        let claude_bin =
            std::env::var_os(ENV_CLAUDE_BIN).map_or_else(|| PathBuf::from("claude"), PathBuf::from);
        let mcp_bin = std::env::var_os(ENV_MCP_BIN).map_or_else(default_mcp_bin, PathBuf::from);
        Self {
            claude_bin,
            mcp_bin,
            model,
            command_budget,
            out_dir,
            suite_version,
        }
    }

    /// The model id or alias to request, applying [`DEFAULT_MODEL`] when unset.
    #[must_use]
    pub fn model(&self) -> &str {
        self.model.as_deref().unwrap_or(DEFAULT_MODEL)
    }
}

/// Resolves the default `reticle-mcp` path: a sibling of the current executable (so the
/// two binaries built into the same target directory find each other), or bare
/// `reticle-mcp` on `PATH` when the current exe cannot be located.
fn default_mcp_bin() -> PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let candidate = dir.join(exe_name("reticle-mcp"));
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from("reticle-mcp")
}

/// The platform executable file name for a bare binary name (`reticle-mcp` becomes
/// `reticle-mcp.exe` on Windows).
fn exe_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_owned()
    }
}

/// The MCP server entry Claude Code launches for a task: the `reticle-mcp` command plus
/// the per-task transcript-capture and budget environment. Serializes as the standard
/// `{ "mcpServers": { "reticle": { "command", "args", "env" } } }` shape Claude Code's
/// `--mcp-config` accepts.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpConfig {
    /// The one server entry, under [`MCP_SERVER_NAME`].
    #[serde(rename = "mcpServers")]
    pub servers: std::collections::BTreeMap<String, McpServerEntry>,
}

/// One MCP server entry: the command to launch and the environment it runs with.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerEntry {
    /// The executable to launch (the `reticle-mcp` binary).
    pub command: String,
    /// Command-line arguments (none: `reticle-mcp` is configured entirely by env).
    #[serde(default)]
    pub args: Vec<String>,
    /// The environment for the server process: the transcript sink path and the budget.
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
}

/// Builds the MCP config that launches `reticle-mcp` with transcript capture to
/// `transcript_path` and the given command budget.
///
/// The config is what makes a Claude-Code session leave a replay-verifiable transcript:
/// `reticle-mcp` streams every applied command to `RETICLE_MCP_TRANSCRIPT` (ADR 0051) and
/// caps the session at `RETICLE_MCP_BUDGET` commands.
#[must_use]
pub fn mcp_config(mcp_bin: &Path, transcript_path: &Path, command_budget: u32) -> McpConfig {
    let mut env = std::collections::BTreeMap::new();
    env.insert(
        RETICLE_MCP_TRANSCRIPT.to_owned(),
        transcript_path.display().to_string(),
    );
    env.insert(RETICLE_MCP_BUDGET.to_owned(), command_budget.to_string());
    let entry = McpServerEntry {
        command: mcp_bin.display().to_string(),
        args: Vec::new(),
        env,
    };
    let mut servers = std::collections::BTreeMap::new();
    servers.insert(MCP_SERVER_NAME.to_owned(), entry);
    McpConfig { servers }
}

/// The fully namespaced allowed-tool names for the `--allowed-tools` flag: every
/// `reticle-mcp` tool, prefixed `mcp__<server>__` as Claude Code namespaces MCP tools.
///
/// The bare tool names come from [`reticle_mcp::tool_names`], the single source of truth,
/// so this list cannot drift from what the server actually serves.
#[must_use]
pub fn allowed_tool_names() -> Vec<String> {
    reticle_mcp::tool_names()
        .iter()
        .map(|name| format!("mcp__{MCP_SERVER_NAME}__{name}"))
        .collect()
}

/// The command line for one Claude-Code task: the program to run and its arguments.
///
/// Separated from spawning so a test can assert on the exact flags without a process. The
/// `prompt` is the task's natural-language prompt; `config_path` is the written MCP config;
/// `model` is the resolved model id or alias.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeInvocation {
    /// The executable to launch.
    pub program: PathBuf,
    /// The full argument vector (not including the program itself).
    pub args: Vec<String>,
}

/// Composes the full prompt handed to `claude -p`: the task prompt, plus a preamble that
/// tells the agent how to reach the layout (the reticle tools) and installs the task's
/// technology up front when one is provided.
///
/// The harness does not own Claude Code's session, so it cannot pre-install the technology
/// the way the loop backends do; instead it hands the technology source to the agent and
/// asks it to call `set_technology` first. When `technology_source` is empty the preamble
/// omits the technology step and the session keeps its built-in default. The task prompt is
/// always included verbatim so the checker still measures the same requirement.
#[must_use]
pub fn compose_prompt(task_prompt: &str, technology_source: &str) -> String {
    let mut out = String::new();
    out.push_str(
        "You are editing an integrated-circuit layout through the reticle MCP tools. Use \
         those tools (create_cell, add_rect, run_drc, and the rest) to complete the task; \
         coordinates are integer database units and layers are GDSII (layer, datatype) \
         pairs.\n",
    );
    if !technology_source.trim().is_empty() {
        out.push_str(
            "\nFirst install the technology by calling set_technology with exactly this \
             source:\n---\n",
        );
        out.push_str(technology_source.trim_end());
        out.push_str("\n---\n");
    }
    out.push_str("\nTask:\n");
    out.push_str(task_prompt);
    out
}

/// Builds the [`ClaudeInvocation`] for a task.
///
/// The flags are the verified `claude` v2.1 surface: `-p <prompt>` (non-interactive print
/// and exit), `--mcp-config <config>` and `--strict-mcp-config` (use *only* that server),
/// `--model <model>`, `--output-format stream-json`, `--permission-mode bypassPermissions`
/// (non-interactive: the `reticle-mcp` server enforces the real budget, and only its tools
/// are reachable under `--strict-mcp-config`), and `--allowed-tools <namespaced names>`.
#[must_use]
pub fn build_invocation(
    config: &ClaudeCodeConfig,
    prompt: &str,
    config_path: &Path,
) -> ClaudeInvocation {
    let mut args = vec![
        "-p".to_owned(),
        prompt.to_owned(),
        "--mcp-config".to_owned(),
        config_path.display().to_string(),
        "--strict-mcp-config".to_owned(),
        "--model".to_owned(),
        config.model().to_owned(),
        "--output-format".to_owned(),
        "stream-json".to_owned(),
        // stream-json requires --verbose to emit the event stream in -p mode.
        "--verbose".to_owned(),
        "--permission-mode".to_owned(),
        "bypassPermissions".to_owned(),
    ];
    // `--allowed-tools` takes a space-separated list; clap on the CLI side accepts each
    // tool as its own value, so push them individually after the flag.
    args.push("--allowed-tools".to_owned());
    for tool in allowed_tool_names() {
        args.push(tool);
    }
    ClaudeInvocation {
        program: config.claude_bin.clone(),
        args,
    }
}

/// The report a [`ClaudeRunner`] returns for one session that actually ran.
#[derive(Clone, Debug)]
pub struct RunReport {
    /// The process exit code, if the process produced one.
    pub exit_code: Option<i32>,
    /// The captured stdout (the `stream-json` event stream). Parsed for the model id and
    /// for authentication / quota failures.
    pub stdout: String,
    /// The captured stderr. Parsed for authentication / quota failures.
    pub stderr: String,
}

/// Why a [`ClaudeRunner`] could not run the session at all (as opposed to running it and
/// the session failing): the CLI is absent, or could not be spawned.
#[derive(Clone, Debug)]
pub struct SpawnFailure {
    /// A human-readable reason (for the not-run record).
    pub detail: String,
}

/// The subprocess-spawn seam.
///
/// The production [`SystemClaudeRunner`] spawns the real `claude` CLI; a deterministic
/// test injects a fake that drives the MCP server itself (so the captured transcript is
/// non-empty) or simulates the CLI being absent.
pub trait ClaudeRunner {
    /// Runs one Claude-Code session for the given invocation.
    ///
    /// Returns `Ok(RunReport)` when the process was launched (whatever it then did), or
    /// `Err(SpawnFailure)` when it could not be launched at all (the CLI is missing). An
    /// authentication or quota failure is *not* a [`SpawnFailure`]: the process launched
    /// and reported the failure, which [`classify_report`] detects from the report.
    fn run(&self, invocation: &ClaudeInvocation) -> Result<RunReport, SpawnFailure>;
}

/// The production runner: spawns the real `claude` CLI and captures its output.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClaudeRunner;

impl ClaudeRunner for SystemClaudeRunner {
    fn run(&self, invocation: &ClaudeInvocation) -> Result<RunReport, SpawnFailure> {
        use std::process::Command;
        let output = Command::new(&invocation.program)
            .args(&invocation.args)
            .output();
        match output {
            Ok(out) => Ok(RunReport {
                exit_code: out.status.code(),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            }),
            Err(e) => Err(SpawnFailure {
                // A `NotFound` is the CLI-absent case; any other spawn error is also an
                // honest not-run (we could not run the session), reported with its cause.
                detail: format!("could not launch `{}`: {e}", invocation.program.display()),
            }),
        }
    }
}

/// The classification of a completed [`RunReport`]: did the session run for real, or did
/// it fail to authenticate / run out of quota (an honest not-run)?
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportClass {
    /// The session ran; carries the model id reported by the stream (or `None` if the
    /// stream did not name one, in which case the requested model id is used).
    Ran {
        /// The model id the `stream-json` events named as having served the run, if any.
        reported_model: Option<String>,
    },
    /// The session could not proceed for an authentication or quota reason; carries the
    /// reason for the not-run record.
    NotRun {
        /// The human-readable reason the session could not proceed (for the not-run
        /// record).
        reason: String,
    },
}

/// Recognized authentication / quota / credit failure phrasings that mark a session as
/// an honest not-run rather than a real run. Kept broad on purpose: a fabricated pass or
/// fail is worse than an over-eager not-run. Matched case-insensitively against the
/// combined stdout/stderr.
const NOT_RUN_MARKERS: &[&str] = &[
    "invalid api key",
    "authentication_error",
    "authentication error",
    "not authenticated",
    "please run /login",
    "please log in",
    "unauthorized",
    "401",
    "credit balance is too low",
    "insufficient credit",
    "quota",
    "rate_limit_error",
    "usage limit",
    "overloaded_error",
];

/// Classifies a [`RunReport`] into [`ReportClass`].
///
/// Authentication and quota failures are detected from the combined stdout/stderr (Claude
/// Code reports them as an error line or a `stream-json` error event). This is
/// intentionally conservative: a recognized auth/quota phrase (see `NOT_RUN_MARKERS`) is
/// treated as a not-run so a broken environment never fabricates a pass or fail. When the
/// stream names the model that served the run, it is threaded onto the record; otherwise
/// the requested id stands.
#[must_use]
pub fn classify_report(report: &RunReport) -> ReportClass {
    let haystack = format!("{}\n{}", report.stdout, report.stderr).to_lowercase();
    if let Some(marker) = NOT_RUN_MARKERS.iter().find(|m| haystack.contains(**m)) {
        return ReportClass::NotRun {
            reason: format!("claude session did not authenticate or was out of quota ({marker})"),
        };
    }
    ReportClass::Ran {
        reported_model: model_from_stream(&report.stdout),
    }
}

/// Extracts the model id from the `stream-json` event stream, if one names it.
///
/// The stream is one JSON object per line; the `system`/`init` event and the final
/// `result` event carry a `model` field. We scan for the first line with a string
/// `model` field and return it, so the recorded `model` is the one the CLI actually
/// served (which may differ from an alias like `sonnet`).
fn model_from_stream(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with('{') {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(model) = value.get("model").and_then(serde_json::Value::as_str)
            && !model.is_empty()
        {
            return Some(model.to_owned());
        }
    }
    None
}

/// An honest "not run" record for a task the Claude-Code backend could not run: the CLI
/// was absent, could not be spawned, or the session did not authenticate / had no quota.
///
/// This is deliberately **not** a [`ResultRecord`]: a not-run must never be counted as a
/// pass or a fail. It is written to its own `<id>.notrun.json` artifact and carried out of
/// the driver as [`ClaudeTaskOutcome::NotRun`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotRunRecord {
    /// The task that was not run.
    pub task_id: String,
    /// The backend that could not run it (always [`BACKEND_LABEL`]).
    pub backend: String,
    /// The model that was requested (so the row is attributable even though it did not
    /// run).
    pub model: String,
    /// The suite version the task came from.
    pub suite_version: String,
    /// Why the task was not run (CLI absent, spawn error, or auth / quota failure).
    pub reason: String,
}

/// The outcome of driving one task through Claude Code: either it ran and produced a
/// [`ResultRecord`] (with `success` reflecting the checker), or it could not be run and
/// produced an honest [`NotRunRecord`].
#[derive(Clone, Debug)]
pub enum ClaudeTaskOutcome {
    /// The session ran; the checker decided pass/fail. Carries the record and the paths
    /// of the artifacts written.
    Ran {
        /// The result record (also written to `<id>.result.json`).
        record: ResultRecord,
        /// Where the artifacts landed.
        artifacts: ClaudeArtifacts,
    },
    /// The session could not run; recorded honestly, never as a pass or fail.
    NotRun {
        /// The not-run record (also written to `<id>.notrun.json`).
        record: NotRunRecord,
        /// The MCP config path (written before the run was attempted) and the not-run
        /// record path.
        config_path: PathBuf,
        /// The not-run record path.
        notrun_path: PathBuf,
    },
}

/// The paths a completed Claude-Code run writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeArtifacts {
    /// The generated MCP config JSON.
    pub config: PathBuf,
    /// The captured session transcript JSONL (`reticle-mcp`'s server-side capture).
    pub transcript: PathBuf,
    /// The result-record JSON (a single-element array, matching the bench writer).
    pub result: PathBuf,
}

/// Why the driver could not set up or finish a run (a real IO / setup failure, distinct
/// from a not-run, which is a recorded outcome rather than an error).
#[derive(Debug)]
pub enum ClaudeError {
    /// The task named a checker the registry does not contain.
    UnknownChecker {
        /// The missing checker name.
        name: String,
    },
    /// An artifact directory or file could not be written / read.
    Io {
        /// The path involved.
        path: PathBuf,
        /// The underlying error message.
        detail: String,
    },
}

impl std::fmt::Display for ClaudeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClaudeError::UnknownChecker { name } => write!(f, "no checker named `{name}`"),
            ClaudeError::Io { path, detail } => {
                write!(f, "io on {}: {detail}", path.display())
            }
        }
    }
}

impl std::error::Error for ClaudeError {}

/// Drives one task through Claude Code using `runner`, returning the outcome.
///
/// Writes the MCP config, invokes `runner`, and then either replays the captured
/// transcript and runs the checker into a [`ResultRecord`], or records an honest
/// [`NotRunRecord`] when the CLI was absent or the session did not authenticate.
///
/// `wall_ms` is the caller-measured duration in milliseconds (tests pass a fixed value so
/// records are reproducible; the CLI measures real time).
///
/// # Errors
///
/// Returns [`ClaudeError::UnknownChecker`] if the task's checker is not registered, or
/// [`ClaudeError::Io`] if an artifact cannot be written or the transcript cannot be read.
/// A session that ran but failed its check is **not** an error (it is a `success = false`
/// record); a session that could not run is **not** an error either (it is a
/// [`NotRunRecord`]).
pub fn run_claude_code_task<R: ClaudeRunner>(
    task: &BenchTask,
    config: &ClaudeCodeConfig,
    registry: &CheckerRegistry,
    technology_source: &str,
    runner: &R,
    wall_ms: u64,
) -> Result<ClaudeTaskOutcome, ClaudeError> {
    let checker = registry
        .get(&task.checker)
        .ok_or_else(|| ClaudeError::UnknownChecker {
            name: task.checker.clone(),
        })?;

    std::fs::create_dir_all(&config.out_dir).map_err(|e| ClaudeError::Io {
        path: config.out_dir.clone(),
        detail: e.to_string(),
    })?;

    // Per-task artifact paths. The transcript path is what `reticle-mcp` captures into.
    let transcript_path = config.out_dir.join(format!("{}.transcript.jsonl", task.id));
    let config_path = config.out_dir.join(format!("{}.mcp.json", task.id));
    let result_path = config.out_dir.join(format!("{}.result.json", task.id));
    let notrun_path = config.out_dir.join(format!("{}.notrun.json", task.id));

    // A stale transcript from a previous run would corrupt this run's replay (the sink
    // appends), so remove it first. Missing is fine.
    let _ = std::fs::remove_file(&transcript_path);

    // Write the MCP config that launches reticle-mcp with capture + budget.
    let cfg = mcp_config(&config.mcp_bin, &transcript_path, config.command_budget);
    let cfg_json = serde_json::to_string_pretty(&cfg).map_err(|e| ClaudeError::Io {
        path: config_path.clone(),
        detail: format!("serializing MCP config: {e}"),
    })?;
    std::fs::write(&config_path, cfg_json).map_err(|e| ClaudeError::Io {
        path: config_path.clone(),
        detail: e.to_string(),
    })?;

    // Build and run the invocation. The prompt carries the task plus a preamble that
    // installs the technology (the harness cannot pre-install it into a session it does
    // not own).
    let prompt = compose_prompt(&task.prompt, technology_source);
    let invocation = build_invocation(config, &prompt, &config_path);
    let requested_model = config.model().to_owned();

    let report = match runner.run(&invocation) {
        Ok(report) => report,
        Err(spawn) => {
            // The CLI could not be launched: honest not-run, no fabricated record.
            let record = NotRunRecord {
                task_id: task.id.clone(),
                backend: BACKEND_LABEL.to_owned(),
                model: requested_model,
                suite_version: config.suite_version.clone(),
                reason: spawn.detail,
            };
            write_notrun(&notrun_path, &record)?;
            return Ok(ClaudeTaskOutcome::NotRun {
                record,
                config_path,
                notrun_path,
            });
        }
    };

    // The process ran; did it actually authenticate and have quota?
    match classify_report(&report) {
        ReportClass::NotRun { reason } => {
            let record = NotRunRecord {
                task_id: task.id.clone(),
                backend: BACKEND_LABEL.to_owned(),
                model: requested_model,
                suite_version: config.suite_version.clone(),
                reason,
            };
            write_notrun(&notrun_path, &record)?;
            Ok(ClaudeTaskOutcome::NotRun {
                record,
                config_path,
                notrun_path,
            })
        }
        ReportClass::Ran { reported_model } => {
            let model = reported_model.unwrap_or(requested_model);
            let record = evaluate_transcript(
                task,
                checker,
                &transcript_path,
                &model,
                &config.suite_version,
                wall_ms,
            )?;
            write_result(&result_path, &record)?;
            Ok(ClaudeTaskOutcome::Ran {
                record,
                artifacts: ClaudeArtifacts {
                    config: config_path,
                    transcript: transcript_path,
                    result: result_path,
                },
            })
        }
    }
}

/// Reconstructs the document from the captured transcript, runs the checker, and builds
/// the [`ResultRecord`].
///
/// The transcript is `reticle-mcp`'s server-side capture: one [`CommandRecord`](reticle_agent_api::CommandRecord) JSON
/// object per line, no trailer. We rebuild the session by re-applying each record's
/// `command` in order (the same replay contract as
/// [`reticle_agent_api::replay`], but reading the raw per-line records the server
/// streamed rather than a whole `Transcript` object), then run the checker on the final
/// document. A missing transcript (the session ran but applied nothing, or capture was
/// somehow off) yields an empty document, which every checker fails honestly.
///
/// `first_proposal_violations` and `final_violations` are both the DRC violation count of
/// the final document: Claude Code owns its own iteration loop, so the harness does not
/// see per-proposal boundaries and reports the observable end state for both rather than
/// inventing intermediate counts. `iterations` is the number of applied commands, the
/// only iteration-like quantity the transcript exposes.
fn evaluate_transcript(
    task: &BenchTask,
    checker: &dyn Checker,
    transcript_path: &Path,
    model: &str,
    suite_version: &str,
    wall_ms: u64,
) -> Result<ResultRecord, ClaudeError> {
    let (session, applied) = replay_capture(transcript_path)?;
    let doc = session.document();
    let violations = drc_violation_count(&session);
    let success = matches!(
        checker.check(doc, &transcript_of_records(&session)),
        CheckResult::Pass
    );
    Ok(ResultRecord {
        task_id: task.id.clone(),
        model: model.to_owned(),
        suite_version: suite_version.to_owned(),
        success,
        // Claude Code drives its own loop; the transcript records applied commands, not
        // propose-verify iterations. Report the applied-command count as the closest
        // observable quantity, and the final DRC count for both violation fields.
        iterations: applied,
        first_proposal_violations: violations,
        final_violations: violations,
        wall_ms,
        backend: BACKEND_LABEL.to_owned(),
        quantization: None,
    })
}

/// Rebuilds a [`Session`] by replaying the `command` field of each JSONL line in the
/// server-captured transcript, returning the session and the number of commands applied.
///
/// A line that is not a well-formed [`CommandRecord`](reticle_agent_api::CommandRecord) (or lacks a `command`) is skipped
/// rather than aborting, so a partially written or trailer-bearing file still replays what
/// it can. A missing file is treated as an empty transcript (zero commands), not an error:
/// the session then holds an empty document the checker fails honestly.
fn replay_capture(transcript_path: &Path) -> Result<(Session, u32), ClaudeError> {
    let text = match std::fs::read_to_string(transcript_path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(ClaudeError::Io {
                path: transcript_path.to_path_buf(),
                detail: e.to_string(),
            });
        }
    };
    let mut session = Session::new();
    let mut applied = 0_u32;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(command_value) = value.get("command") else {
            continue;
        };
        let Ok(command) = serde_json::from_value::<AgentCommand>(command_value.clone()) else {
            continue;
        };
        // Re-apply; a command that failed originally fails again the same way, so the
        // replayed document matches what the direct session produced (ADR 0051).
        let _ = session.apply(command);
        applied += 1;
    }
    Ok((session, applied))
}

/// A [`Transcript`](reticle_agent_api::Transcript) view of the replayed session, so the
/// checker sees the same `(document, transcript)` pair the other backends give it. The
/// plan log is empty (Claude Code owns its own loop; the harness records no plan steps).
fn transcript_of_records(session: &Session) -> reticle_agent_api::Transcript {
    reticle_agent_api::Transcript {
        records: session.transcript().to_vec(),
        final_hash: reticle_model::document_hash(session.document()),
        plan: Vec::new(),
    }
}

/// Counts DRC violations on the session's target cell under the built-in SKY130 rule
/// subset; `0` when the document has no cell yet. Mirrors the loop's own counter so a
/// Claude-Code run's violation numbers are comparable to the other backends'.
fn drc_violation_count(session: &Session) -> u32 {
    use reticle_model::RuleSet as _;
    let doc = session.document();
    let Some(cell) = doc
        .top_cells()
        .first()
        .cloned()
        .or_else(|| doc.cells().next().map(|c| c.name.clone()))
    else {
        return 0;
    };
    let engine = reticle_drc::DrcEngine::new(reticle_drc::sky130_drc_rules());
    engine.check_cell(doc, &cell).len() as u32
}

/// Writes a [`ResultRecord`] as a single-element JSON array (matching the bench writer
/// shape, so the same reader loads Claude-Code results and other backends' results).
fn write_result(path: &Path, record: &ResultRecord) -> Result<(), ClaudeError> {
    let json = serde_json::to_string_pretty(std::slice::from_ref(record)).map_err(|e| {
        ClaudeError::Io {
            path: path.to_path_buf(),
            detail: format!("serializing result record: {e}"),
        }
    })?;
    std::fs::write(path, json).map_err(|e| ClaudeError::Io {
        path: path.to_path_buf(),
        detail: e.to_string(),
    })
}

/// Writes a [`NotRunRecord`] as a single JSON object to `<id>.notrun.json`. Distinct from
/// the results array on purpose: a not-run must never be loaded as a pass or a fail.
fn write_notrun(path: &Path, record: &NotRunRecord) -> Result<(), ClaudeError> {
    let json = serde_json::to_string_pretty(record).map_err(|e| ClaudeError::Io {
        path: path.to_path_buf(),
        detail: format!("serializing not-run record: {e}"),
    })?;
    std::fs::write(path, json).map_err(|e| ClaudeError::Io {
        path: path.to_path_buf(),
        detail: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        BACKEND_LABEL, ClaudeCodeConfig, ClaudeInvocation, ClaudeRunner, ClaudeTaskOutcome,
        MCP_SERVER_NAME, McpConfig, ReportClass, RunReport, SpawnFailure, allowed_tool_names,
        build_invocation, classify_report, compose_prompt, mcp_config, run_claude_code_task,
    };
    use reticle_bench::{BenchTask, CheckerRegistry, Tier};
    use std::io::{BufRead, BufReader, Write};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    /// A unique scratch directory for one test, keyed by name and process id.
    fn unique_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("reticle-claude-code-{tag}-{}", std::process::id()))
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A small technology the fake driving CLI installs on the server: a met1 layer plus a
    /// spacing rule, using the reticle-mcp stdio test's known-good `rule` syntax.
    const FAKE_TECH: &str = "technology demo\n\
                             dbu_per_micron 1000\n\
                             layer 68 20 met1 3A6FD490\n\
                             rule spacing 68 20 140\n";

    /// The tier-1 DRC task fixture (clean met1 rect passes `drc_clean`).
    fn drc_task() -> BenchTask {
        BenchTask {
            id: "t1_claude".into(),
            tier: Tier(1),
            prompt: "Create a cell named top and place a DRC-clean met1 rectangle.".into(),
            technology: "sky130.tech".into(),
            checker: "drc_clean".into(),
            intent: None,
            refinement: None,
        }
    }

    /// Locates the built `reticle-mcp` binary for the driving test.
    ///
    /// `CARGO_BIN_EXE_reticle-mcp` is only set for the *mcp* crate's own integration
    /// tests, not for `reticle-agent`'s unit tests, so we resolve it from the test
    /// binary's own location: a unit-test binary runs from `<target>/<profile>/deps/`, so
    /// the sibling `reticle-mcp[.exe]` is one directory up. Falls back to bare
    /// `reticle-mcp` (which a driving test would then fail to spawn, surfacing the
    /// misconfiguration rather than silently passing).
    fn built_mcp_bin() -> PathBuf {
        let exe = std::env::current_exe().expect("current test exe");
        // <target>/<profile>/deps/<test>  ->  <target>/<profile>/
        let profile_dir = exe
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let name = if cfg!(windows) {
            "reticle-mcp.exe"
        } else {
            "reticle-mcp"
        };
        let candidate = profile_dir.join(name);
        if candidate.exists() {
            candidate
        } else {
            PathBuf::from("reticle-mcp")
        }
    }

    /// A config pointing at the built `reticle-mcp` binary, with the given out dir.
    fn config_for(out_dir: PathBuf) -> ClaudeCodeConfig {
        ClaudeCodeConfig {
            claude_bin: PathBuf::from("claude-should-not-be-spawned-in-tests"),
            mcp_bin: built_mcp_bin(),
            model: Some("sonnet".into()),
            command_budget: 64,
            out_dir,
            suite_version: "test-suite".into(),
        }
    }

    #[test]
    fn mcp_config_launches_reticle_mcp_with_capture_and_budget() {
        let transcript = PathBuf::from("/tmp/some/t1.transcript.jsonl");
        let cfg = mcp_config(Path::new("/path/to/reticle-mcp"), &transcript, 42);
        let entry = cfg.servers.get(MCP_SERVER_NAME).expect("reticle server");
        assert_eq!(entry.command, "/path/to/reticle-mcp");
        assert!(
            entry.args.is_empty(),
            "reticle-mcp is configured by env only"
        );
        assert_eq!(
            entry.env.get("RETICLE_MCP_TRANSCRIPT").map(String::as_str),
            Some(transcript.display().to_string().as_str())
        );
        assert_eq!(
            entry.env.get("RETICLE_MCP_BUDGET").map(String::as_str),
            Some("42")
        );
        // Round-trips through the exact `mcpServers` shape Claude Code's --mcp-config wants.
        let json = serde_json::to_value(&cfg).unwrap();
        assert!(json.get("mcpServers").is_some(), "top-level mcpServers key");
        let back: McpConfig = serde_json::from_value(json).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn allowed_tools_are_namespaced_and_cover_core_tools() {
        let tools = allowed_tool_names();
        assert!(tools.len() > 30, "the whole reticle-mcp surface is allowed");
        for tool in &tools {
            assert!(
                tool.starts_with(&format!("mcp__{MCP_SERVER_NAME}__")),
                "every tool is namespaced for Claude Code: {tool}"
            );
        }
        // A couple of load-bearing tools appear under the namespaced form.
        assert!(tools.contains(&"mcp__reticle__create_cell".to_owned()));
        assert!(tools.contains(&"mcp__reticle__add_rect".to_owned()));
        assert!(tools.contains(&"mcp__reticle__run_drc".to_owned()));
    }

    #[test]
    fn compose_prompt_includes_task_and_optional_technology() {
        // With a technology, the preamble asks the agent to install it first, and the
        // task prompt is present verbatim.
        let with_tech = compose_prompt("Draw a met1 rect.", "technology demo\nlayer 68 20 met1 0");
        assert!(
            with_tech.contains("set_technology"),
            "asks to install the tech"
        );
        assert!(
            with_tech.contains("technology demo"),
            "carries the tech source"
        );
        assert!(
            with_tech.contains("Draw a met1 rect."),
            "keeps the task prompt"
        );
        // With no technology, the set_technology step is omitted but the task remains.
        let no_tech = compose_prompt("Draw a met1 rect.", "");
        assert!(
            !no_tech.contains("set_technology"),
            "no tech step when none is given"
        );
        assert!(no_tech.contains("Draw a met1 rect."));
    }

    #[test]
    fn invocation_carries_the_verified_flags() {
        let cfg = config_for(unique_dir("flags"));
        let inv = build_invocation(&cfg, "draw a rect", Path::new("/cfg/t1.mcp.json"));
        let a = &inv.args;
        // -p <prompt> (non-interactive print-and-exit).
        assert_eq!(a[0], "-p");
        assert_eq!(a[1], "draw a rect");
        // The MCP config plus strict-mcp-config (only that server).
        let cfg_pos = a
            .iter()
            .position(|s| s == "--mcp-config")
            .expect("mcp-config");
        assert_eq!(a[cfg_pos + 1], "/cfg/t1.mcp.json");
        assert!(a.iter().any(|s| s == "--strict-mcp-config"));
        // Model, output format, and a non-interactive permission mode.
        let model_pos = a.iter().position(|s| s == "--model").expect("model");
        assert_eq!(a[model_pos + 1], "sonnet");
        let out_pos = a.iter().position(|s| s == "--output-format").expect("fmt");
        assert_eq!(a[out_pos + 1], "stream-json");
        let perm_pos = a
            .iter()
            .position(|s| s == "--permission-mode")
            .expect("perm");
        assert_eq!(a[perm_pos + 1], "bypassPermissions");
        // --allowed-tools followed by the namespaced tool names.
        let allow_pos = a
            .iter()
            .position(|s| s == "--allowed-tools")
            .expect("allowed-tools");
        assert!(
            a[allow_pos + 1..]
                .iter()
                .any(|s| s == "mcp__reticle__create_cell"),
            "the allowed-tools list follows the flag"
        );
    }

    #[test]
    fn classify_recognizes_auth_and_quota_failures_as_not_run() {
        // An invalid-key error line.
        let auth = RunReport {
            exit_code: Some(1),
            stdout: String::new(),
            stderr: "API error: invalid api key; please run /login".into(),
        };
        assert!(matches!(classify_report(&auth), ReportClass::NotRun { .. }));

        // A credit / quota message.
        let quota = RunReport {
            exit_code: Some(1),
            stdout: "{\"type\":\"result\",\"subtype\":\"error\",\"error\":\"Your credit balance is too low\"}".into(),
            stderr: String::new(),
        };
        assert!(matches!(
            classify_report(&quota),
            ReportClass::NotRun { .. }
        ));
    }

    #[test]
    fn classify_reads_the_served_model_from_the_stream() {
        // A well-formed stream naming the served model on its init and result events.
        let report = RunReport {
            exit_code: Some(0),
            stdout: "{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-sonnet-4-6-20990101\"}\n\
                     {\"type\":\"result\",\"subtype\":\"success\",\"model\":\"claude-sonnet-4-6-20990101\"}"
                .into(),
            stderr: String::new(),
        };
        match classify_report(&report) {
            ReportClass::Ran { reported_model } => {
                assert_eq!(
                    reported_model.as_deref(),
                    Some("claude-sonnet-4-6-20990101")
                );
            }
            ReportClass::NotRun { .. } => panic!("a clean stream must classify as Ran"),
        }
    }

    /// A fake runner simulating the CLI being absent (a spawn failure). Proves the
    /// not-run path without any process.
    struct AbsentCli;

    impl ClaudeRunner for AbsentCli {
        fn run(&self, invocation: &ClaudeInvocation) -> Result<RunReport, SpawnFailure> {
            Err(SpawnFailure {
                detail: format!(
                    "could not launch `{}`: not found",
                    invocation.program.display()
                ),
            })
        }
    }

    #[test]
    fn absent_cli_yields_an_honest_not_run_never_a_fabricated_result() {
        let out_dir = unique_dir("absent");
        let cfg = config_for(out_dir.clone());
        let registry = CheckerRegistry::for_task(&drc_task()).unwrap();
        let outcome =
            run_claude_code_task(&drc_task(), &cfg, &registry, "", &AbsentCli, 0).expect("drive");
        match outcome {
            ClaudeTaskOutcome::NotRun {
                record,
                notrun_path,
                config_path,
            } => {
                assert_eq!(record.backend, BACKEND_LABEL);
                assert_eq!(record.task_id, "t1_claude");
                assert!(
                    record.reason.contains("could not launch"),
                    "the reason names the spawn failure: {}",
                    record.reason
                );
                // The not-run artifact exists and is a NotRunRecord object, NOT a
                // ResultRecord array (so it can never be counted as a pass or fail).
                assert!(notrun_path.exists(), "not-run artifact written");
                assert!(
                    config_path.exists(),
                    "config still written before the attempt"
                );
                let text = std::fs::read_to_string(&notrun_path).unwrap();
                assert!(
                    !text.trim_start().starts_with('['),
                    "a not-run must not be a results array"
                );
                assert!(text.contains("\"reason\""), "carries a reason field");
                // No result.json was written for a not-run.
                let result_path = out_dir.join("t1_claude.result.json");
                assert!(!result_path.exists(), "a not-run writes no result record");
            }
            ClaudeTaskOutcome::Ran { .. } => panic!("absent CLI must be a not-run, not a run"),
        }
        cleanup(&out_dir);
    }

    /// A fake CLI runner that behaves like Claude Code driving the reticle-mcp server: it
    /// reads the `--mcp-config` the driver generated, spawns the `reticle-mcp` command it
    /// names with that config's env (so server-side transcript capture is on), and applies
    /// a couple of scripted commands over stdio (a clean met1 rect). This proves the
    /// generated config is correct and that the captured transcript is non-empty and
    /// replays into the checked document, all without the real, non-deterministic CLI.
    struct FakeDrivingCli;

    impl FakeDrivingCli {
        /// Reads the MCP config path out of the invocation's `--mcp-config` argument.
        fn config_path(invocation: &ClaudeInvocation) -> PathBuf {
            let pos = invocation
                .args
                .iter()
                .position(|s| s == "--mcp-config")
                .expect("invocation has --mcp-config");
            PathBuf::from(&invocation.args[pos + 1])
        }

        /// Sends one JSON-RPC request and reads exactly one response line.
        fn rpc(
            stdin: &mut std::process::ChildStdin,
            stdout: &mut BufReader<std::process::ChildStdout>,
            id: i64,
            method: &str,
            params: &serde_json::Value,
        ) {
            let msg = serde_json::json!({
                "jsonrpc": "2.0", "id": id, "method": method, "params": params,
            });
            writeln!(stdin, "{msg}").expect("write rpc");
            stdin.flush().expect("flush rpc");
            let mut line = String::new();
            let n = stdout.read_line(&mut line).expect("read rpc response");
            assert!(n > 0, "server closed unexpectedly on {method}");
        }
    }

    impl ClaudeRunner for FakeDrivingCli {
        fn run(&self, invocation: &ClaudeInvocation) -> Result<RunReport, SpawnFailure> {
            // Parse the generated config to find the reticle-mcp command and its env.
            let config_path = Self::config_path(invocation);
            let cfg_text = std::fs::read_to_string(&config_path).expect("read generated config");
            let cfg: McpConfig = serde_json::from_str(&cfg_text).expect("parse generated config");
            let entry = cfg.servers.get(MCP_SERVER_NAME).expect("reticle server");

            // Spawn reticle-mcp exactly as Claude Code would: the named command, with the
            // config's env (which carries RETICLE_MCP_TRANSCRIPT + RETICLE_MCP_BUDGET).
            let mut child = Command::new(&entry.command)
                .args(&entry.args)
                .envs(&entry.env)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| SpawnFailure {
                    detail: format!("fake CLI could not spawn reticle-mcp: {e}"),
                })?;
            let mut stdin = child.stdin.take().expect("child stdin");
            let mut stdout = BufReader::new(child.stdout.take().expect("child stdout"));

            // A minimal but real driving session: initialize, install a tech with a met1
            // layer, create a cell, add a DRC-clean met1 rect, run DRC. The tech mirrors
            // the reticle-mcp stdio test's known-good syntax (`rule <kind> <l> <d> <v>`);
            // the checker itself uses the built-in SKY130 subset, so the exact tech only
            // feeds the server's own run_drc call.
            Self::rpc(
                &mut stdin,
                &mut stdout,
                1,
                "initialize",
                &serde_json::json!({}),
            );
            Self::rpc(
                &mut stdin,
                &mut stdout,
                2,
                "tools/call",
                &serde_json::json!({ "name": "set_technology", "arguments": { "source": FAKE_TECH } }),
            );
            Self::rpc(
                &mut stdin,
                &mut stdout,
                3,
                "tools/call",
                &serde_json::json!({ "name": "create_cell", "arguments": { "name": "top" } }),
            );
            Self::rpc(
                &mut stdin,
                &mut stdout,
                4,
                "tools/call",
                &serde_json::json!({ "name": "add_rect", "arguments": {
                    "cell": "top",
                    "layer": { "layer": 68, "datatype": 20 },
                    "rect": { "min": { "x": 0, "y": 0 }, "max": { "x": 500, "y": 500 } }
                }}),
            );
            Self::rpc(
                &mut stdin,
                &mut stdout,
                5,
                "tools/call",
                &serde_json::json!({ "name": "run_drc", "arguments": { "cell": "top" } }),
            );
            // Close stdin so the server's stdio loop ends, then reap it (its BufWriter
            // flushes the captured transcript on drop / process exit).
            drop(stdin);
            let _ = child.wait();

            // Emit a clean stream-json-like report naming the served model.
            Ok(RunReport {
                exit_code: Some(0),
                stdout: "{\"type\":\"system\",\"subtype\":\"init\",\"model\":\"claude-fake-model\"}\n\
                         {\"type\":\"result\",\"subtype\":\"success\",\"model\":\"claude-fake-model\"}"
                    .into(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn fake_cli_driving_the_mcp_produces_a_checked_result_labeled_claude_code() {
        let out_dir = unique_dir("driving");
        let cfg = config_for(out_dir.clone());
        let registry = CheckerRegistry::for_task(&drc_task()).unwrap();
        let outcome = run_claude_code_task(&drc_task(), &cfg, &registry, "", &FakeDrivingCli, 7)
            .expect("drive");
        match outcome {
            ClaudeTaskOutcome::Ran { record, artifacts } => {
                // The run is labeled as the Claude-Code backend, with the model the stream
                // reported (not the requested alias).
                assert_eq!(record.backend, BACKEND_LABEL);
                assert_eq!(record.model, "claude-fake-model");
                assert_eq!(record.suite_version, "test-suite");
                assert_eq!(record.wall_ms, 7);
                // The captured transcript was non-empty and replayed into a clean document,
                // so the DRC checker passes.
                assert!(
                    record.success,
                    "the scripted clean met1 rect should pass drc_clean"
                );
                assert_eq!(record.final_violations, 0);
                assert!(record.iterations >= 3, "several commands were applied");
                // The artifacts exist: config, the server-captured transcript, the result.
                assert!(artifacts.config.exists());
                assert!(
                    artifacts.transcript.exists(),
                    "server captured a transcript"
                );
                assert!(artifacts.result.exists());
                let tx = std::fs::read_to_string(&artifacts.transcript).unwrap();
                assert!(
                    tx.contains("add_rect"),
                    "the captured transcript records the applied commands"
                );
                // The result artifact is a one-element ResultRecord array.
                let text = std::fs::read_to_string(&artifacts.result).unwrap();
                let records: Vec<reticle_bench::ResultRecord> =
                    serde_json::from_str(&text).unwrap();
                assert_eq!(records.len(), 1);
                assert!(records[0].success);
                assert_eq!(records[0].backend, BACKEND_LABEL);
            }
            ClaudeTaskOutcome::NotRun { record, .. } => {
                panic!("the fake driving CLI must run, not be a not-run: {record:?}")
            }
        }
        cleanup(&out_dir);
    }

    #[test]
    fn unknown_checker_is_an_error_not_a_silent_pass() {
        let task = BenchTask {
            checker: "no_such_checker".into(),
            ..drc_task()
        };
        let out_dir = unique_dir("unknown");
        let cfg = config_for(out_dir.clone());
        // Build a registry that does NOT know the task's checker.
        let registry = CheckerRegistry::default();
        let err = run_claude_code_task(&task, &cfg, &registry, "", &AbsentCli, 0)
            .expect_err("unknown checker must be an error");
        assert!(matches!(err, super::ClaudeError::UnknownChecker { .. }));
        cleanup(&out_dir);
    }
}
