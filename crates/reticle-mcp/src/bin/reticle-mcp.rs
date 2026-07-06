//! The `reticle-mcp` binary: a stdio Model Context Protocol server.
//!
//! Serves the Reticle agent command surface over newline-delimited JSON-RPC 2.0
//! on stdin/stdout. Point an MCP client (for example a model runtime) at this
//! executable with no arguments.
//!
//! The per-session command budget defaults to `10_000` and can be overridden
//! with the `RETICLE_MCP_BUDGET` environment variable (a positive integer). An
//! unparsable value falls back to the default.
//!
//! Set `RETICLE_MCP_TRANSCRIPT` to a file path to capture a session transcript:
//! every command the server applies is streamed there as JSONL (one record per
//! line), created then appended, so a client the harness does not control still
//! leaves a replay-verifiable, mineable transcript. Unset, nothing is captured.

use std::fs::OpenOptions;
use std::io::{self, BufReader, BufWriter};

use reticle_mcp::{Budget, Server};

/// The environment variable that overrides the session command budget.
const BUDGET_ENV: &str = "RETICLE_MCP_BUDGET";
/// The environment variable that, when set to a path, enables session-transcript
/// capture to that file (JSONL, appended).
const TRANSCRIPT_ENV: &str = "RETICLE_MCP_TRANSCRIPT";

fn main() -> io::Result<()> {
    let budget = std::env::var(BUDGET_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map_or_else(Budget::default, Budget::new);

    // Optional server-side transcript capture, on only when the env var names a file.
    // Append so re-running extends the transcript; a BufWriter keeps the per-request
    // flush cheap.
    let mut server = match std::env::var(TRANSCRIPT_ENV)
        .ok()
        .filter(|p| !p.trim().is_empty())
    {
        Some(path) => {
            let file = OpenOptions::new().create(true).append(true).open(&path)?;
            Server::with_transcript(budget, Box::new(BufWriter::new(file)))
        }
        None => Server::new(budget),
    };
    let stdin = io::stdin();
    let stdout = io::stdout();
    server.run(BufReader::new(stdin.lock()), stdout.lock())
}
