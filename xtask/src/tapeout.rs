//! The `tapeout-example` subcommand: write the worked in-repo Tiny Tapeout tile and
//! its replayable transcript to `examples/tapeout/`.
//!
//! The tile is built by [`reticle_app::tinytapeout_example`]: the Lane 4A frame,
//! seeded into an agent [`Session`](reticle_agent_api::Session) through GDS import,
//! with a `test_structure` serpentine placed into the interior by the
//! `RunGenerator`/`TransformShapes` commands. This subcommand is the thin driver that
//! re-runs that build, re-verifies it (DRC-subset-clean and transcript replay) as a
//! guard, and writes the two committed artifacts:
//!
//! * `tt_um_reticle_tile.gds`: the finished tile exported to GDSII.
//! * `tt_um_reticle_tile.transcript.jsonl`: the replayable transcript, one
//!   [`CommandRecord`](reticle_agent_api::CommandRecord) per line followed by a
//!   `{"final_hash": <u64>}` trailer, the exact format the replay theater loads.
//!
//! It is a **generator-driven, deterministic build, not a Claude Code run** (the CLI
//! is unauthenticated in this environment), and the tile is DRC-clean against the
//! SKY130 subset only; the authoritative Tiny Tapeout precheck is a separate operator
//! step (`just tt-precheck`).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use reticle_agent_api::replay;
use reticle_app::tinytapeout::TT_TILE_TOP;
use reticle_app::tinytapeout_example::{worked_tile_document, worked_tile_transcript};
use reticle_drc::{DrcEngine, sky130_drc_rules};
use reticle_io::Gds;
use reticle_model::{Exporter, RuleSet};

/// The default output directory for the committed artifacts, relative to the repo
/// root.
const DEFAULT_OUT_DIR: &str = "examples/tapeout";

/// The tile's base filename (without extension).
const TILE_STEM: &str = "tt_um_reticle_tile";

/// Handles `tapeout-example`: build the worked tile, verify it, and write the GDS and
/// transcript into `out_dir` (default `examples/tapeout`).
pub fn cmd_tapeout_example(out_dir: Option<&str>) -> ExitCode {
    let dir = PathBuf::from(out_dir.unwrap_or(DEFAULT_OUT_DIR));

    // Build the tile and its transcript through the command path.
    let doc = worked_tile_document();
    let transcript = worked_tile_transcript();

    // Guard 1: the tile is DRC-clean against the committed SKY130 subset. This is the
    // same check the committed test asserts; running it here refuses to write a dirty
    // artifact.
    let engine = DrcEngine::new(sky130_drc_rules());
    let violations = engine.check_cell(&doc, TT_TILE_TOP);
    if !violations.is_empty() {
        eprintln!(
            "refusing to write: tile is not DRC-subset-clean ({} violation(s)); first: {:?}",
            violations.len(),
            violations.first()
        );
        return ExitCode::FAILURE;
    }

    // Guard 2: the transcript replays to its recorded final hash, and that hash is the
    // hash of the document we are about to export.
    match replay(&transcript) {
        Ok(hash) if hash == transcript.final_hash => {}
        Ok(hash) => {
            eprintln!(
                "refusing to write: transcript replay hash {hash:#x} != recorded {:#x}",
                transcript.final_hash
            );
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("refusing to write: transcript replay failed: {e}");
            return ExitCode::FAILURE;
        }
    }
    if reticle_model::document_hash(&doc) != transcript.final_hash {
        eprintln!("refusing to write: exported document hash != transcript final hash");
        return ExitCode::FAILURE;
    }

    // Export the GDS.
    let gds = match Gds.export(&doc) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("GDSII export failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Serialize the transcript as JSONL (one record per line + a final_hash trailer),
    // matching the replay theater's loader format.
    let jsonl = transcript_to_jsonl(&transcript);

    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("could not create {}: {e}", dir.display());
        return ExitCode::FAILURE;
    }
    let gds_path = dir.join(format!("{TILE_STEM}.gds"));
    let jsonl_path = dir.join(format!("{TILE_STEM}.transcript.jsonl"));
    if let Err(e) = std::fs::write(&gds_path, &gds) {
        eprintln!("write {} failed: {e}", gds_path.display());
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(&jsonl_path, jsonl.as_bytes()) {
        eprintln!("write {} failed: {e}", jsonl_path.display());
        return ExitCode::FAILURE;
    }

    let cell = doc.cell(TT_TILE_TOP).expect("tile cell present");
    println!(
        "wrote {}: {} shapes in {TT_TILE_TOP}, {} bytes GDS, DRC-subset-clean",
        gds_path.display(),
        cell.shapes.len(),
        gds.len(),
    );
    println!(
        "wrote {}: {} commands, final_hash {:#x}",
        jsonl_path.display(),
        transcript.records.len(),
        transcript.final_hash,
    );
    println!(
        "NOTE: DRC-clean against the SKY130 SUBSET only; NOT verified through the real \
         TinyTapeout precheck. Run the operator step to get the authoritative verdict:"
    );
    println!("  just tt-precheck {}", display_forward_slash(&gds_path));
    ExitCode::SUCCESS
}

/// Serializes a transcript to the JSONL text format the replay theater loads: one
/// [`CommandRecord`](reticle_agent_api::CommandRecord) JSON object per line, then a
/// `{"final_hash": <u64>}` trailer line.
fn transcript_to_jsonl(transcript: &reticle_agent_api::Transcript) -> String {
    let mut text = String::new();
    for record in &transcript.records {
        text.push_str(&serde_json::to_string(record).expect("command record serializes"));
        text.push('\n');
    }
    text.push_str(&serde_json::json!({ "final_hash": transcript.final_hash }).to_string());
    text.push('\n');
    text
}

/// Renders a path with forward slashes, so the printed `just tt-precheck` command is
/// copy-pasteable regardless of the host path separator.
fn display_forward_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
