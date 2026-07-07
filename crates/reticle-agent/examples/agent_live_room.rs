//! Run the agent as a live collaborator in a relay room, or emit the demo transcript.
//!
//! This is the small CLI entry for the live-room run mode. It drives the deterministic,
//! model-free DRC-fix script (see [`reticle_agent::live::scripted_drc_fix_steps`]); this
//! is a scripted harness, **not** a live model: no key, no API, the same commands every
//! run.
//!
//! ```text
//! # Join a room on a running relay and fix the seeded DRC violation live, beside humans:
//! cargo run -p reticle-agent --example agent_live_room -- --relay ws://127.0.0.1:8080 --room demo
//!
//! # Regenerate the committed demo transcript (no relay needed):
//! cargo run -p reticle-agent --example agent_live_room -- --emit examples/collab/agent_drc_fix.transcript.jsonl
//! ```

use std::process::ExitCode;

use reticle_agent::live::{
    DRC_FIX_CELL, LiveConfig, run_in_room, scripted_drc_fix_jsonl, scripted_drc_fix_steps,
};
use reticle_agent::{AgentCollaborator, Pacing};

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args) {
        Ok(Mode::Emit { path }) => emit(&path),
        Ok(Mode::Live { relay, room }) => live(&relay, &room).await,
        Ok(Mode::Help) => {
            print_usage();
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("agent_live_room: {message}\n");
            print_usage();
            ExitCode::from(2)
        }
    }
}

/// What the CLI was asked to do.
enum Mode {
    /// Write the committed demo transcript to a path.
    Emit { path: String },
    /// Join `relay`'s `room` and run the scripted demo live.
    Live { relay: String, room: String },
    /// Print usage.
    Help,
}

/// Parses the flat `--flag value` argument list.
fn parse(args: &[String]) -> Result<Mode, String> {
    let mut emit: Option<String> = None;
    let mut relay: Option<String> = None;
    let mut room: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => return Ok(Mode::Help),
            "--emit" => emit = Some(take(args, &mut i, "--emit")?),
            "--relay" => relay = Some(take(args, &mut i, "--relay")?),
            "--room" => room = Some(take(args, &mut i, "--room")?),
            other => return Err(format!("unknown argument `{other}`")),
        }
        i += 1;
    }
    if let Some(path) = emit {
        return Ok(Mode::Emit { path });
    }
    match (relay, room) {
        (Some(relay), Some(room)) => Ok(Mode::Live { relay, room }),
        (None, None) => Ok(Mode::Help),
        _ => Err("--relay and --room must be given together".to_owned()),
    }
}

/// Consumes the value following a flag at `args[*i]`, advancing `i` past it.
fn take(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

/// Writes the deterministic demo transcript JSONL to `path`.
fn emit(path: &str) -> ExitCode {
    match std::fs::write(path, scripted_drc_fix_jsonl()) {
        Ok(()) => {
            println!("wrote deterministic demo transcript to {path}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("agent_live_room: writing {path}: {e}");
            ExitCode::from(2)
        }
    }
}

/// Joins the room and runs the scripted demo live, then reports what converged.
async fn live(relay: &str, room: &str) -> ExitCode {
    let url = format!("{}/ws/{room}", relay.trim_end_matches('/'));
    let collab =
        AgentCollaborator::new(Pacing::millis(400)).with_display_name("Reticle agent (demo)");
    let config = LiveConfig {
        doc_id: room.to_owned(),
        ..LiveConfig::default()
    };
    println!("joining {url} as the agent; fixing the seeded DRC violation (scripted, no model)");
    match run_in_room(collab, &url, scripted_drc_fix_steps(), &config).await {
        Ok(collab) => {
            let shapes = collab
                .document()
                .cell(DRC_FIX_CELL)
                .map_or(0, |c| c.shapes.len());
            println!(
                "done: room {room} now holds cell `{DRC_FIX_CELL}` with {shapes} met1 shape(s)"
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("agent_live_room: {e}");
            ExitCode::from(1)
        }
    }
}

/// Prints the CLI usage.
fn print_usage() {
    eprintln!(
        "usage:\n  \
         agent_live_room --relay <ws-url> --room <name>   join a relay room and run the scripted demo\n  \
         agent_live_room --emit <path>                    write the committed demo transcript\n  \
         agent_live_room --help                           show this message"
    );
}
