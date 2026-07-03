//! The `reticle-mcp` binary: a stdio Model Context Protocol server.
//!
//! Serves the Reticle agent command surface over newline-delimited JSON-RPC 2.0
//! on stdin/stdout. Point an MCP client (for example a model runtime) at this
//! executable with no arguments.
//!
//! The per-session command budget defaults to `10_000` and can be overridden
//! with the `RETICLE_MCP_BUDGET` environment variable (a positive integer). An
//! unparseable value falls back to the default.

use std::io::{self, BufReader};

use reticle_mcp::{Budget, Server};

/// The environment variable that overrides the session command budget.
const BUDGET_ENV: &str = "RETICLE_MCP_BUDGET";

fn main() -> io::Result<()> {
    let budget = std::env::var(BUDGET_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map_or_else(Budget::default, Budget::new);

    let mut server = Server::new(budget);
    let stdin = io::stdin();
    let stdout = io::stdout();
    server.run(BufReader::new(stdin.lock()), stdout.lock())
}
