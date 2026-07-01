//! Reticle build automation (`xtask`).
//!
//! Wave 4/5 implement the deterministic parameterized layout generator
//! (`gen-layout`, by shape count, layer count, and hierarchy depth), the offscreen
//! media-capture command (`capture-media`), and `perf-check` (compare against the
//! committed benchmark history and fail on regression).

use std::process::ExitCode;

fn main() -> ExitCode {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    match cmd.as_str() {
        "gen-layout" => {
            println!("xtask gen-layout (Wave 4 stub)");
            ExitCode::SUCCESS
        }
        "capture-media" => {
            println!("xtask capture-media (Wave 5 stub)");
            ExitCode::SUCCESS
        }
        "perf-check" => {
            println!("xtask perf-check (Wave 5 stub)");
            ExitCode::SUCCESS
        }
        "" => {
            eprintln!("usage: xtask <gen-layout|capture-media|perf-check> [options]");
            ExitCode::FAILURE
        }
        other => {
            eprintln!("unknown xtask subcommand: {other}");
            ExitCode::FAILURE
        }
    }
}
