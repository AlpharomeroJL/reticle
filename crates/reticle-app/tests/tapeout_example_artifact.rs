//! Verifies the committed worked-example tile artifacts under `examples/tapeout/`
//! (Lane 4C) are valid and in sync with the generator that produces them.
//!
//! This is native-only: it reads the committed files from the filesystem. It guards
//! three things about the checked-in proof artifact:
//!
//! * The committed transcript JSONL parses in the replay theater's format and replays
//!   to its recorded final hash (the `document_hash` replay contract).
//! * The committed GDS re-imports and is DRC-clean against the SKY130 subset.
//! * The committed files match what the current generator produces, so they cannot
//!   drift from `xtask tapeout-example` without this test failing.

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

use reticle_agent_api::{CommandRecord, Transcript, replay};
use reticle_app::tinytapeout::TT_TILE_TOP;
use reticle_app::tinytapeout_example::{worked_tile_document, worked_tile_transcript};
use reticle_drc::{DrcEngine, sky130_drc_rules};
use reticle_model::{Importer, RuleSet, document_hash};

/// The committed artifact directory: two levels up from this crate to the repo root,
/// then `examples/tapeout`.
fn artifact_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .expect("repo root is two levels above the crate")
        .join("examples")
        .join("tapeout")
}

fn gds_path() -> PathBuf {
    artifact_dir().join("tt_um_reticle_tile.gds")
}

fn transcript_path() -> PathBuf {
    artifact_dir().join("tt_um_reticle_tile.transcript.jsonl")
}

/// Parses the committed transcript JSONL (one [`CommandRecord`] per line, then a
/// `{"final_hash": <u64>}` trailer) into a [`Transcript`].
fn parse_committed_transcript() -> Transcript {
    let text = std::fs::read_to_string(transcript_path())
        .unwrap_or_else(|e| panic!("read {}: {e}", transcript_path().display()));
    let mut records: Vec<CommandRecord> = Vec::new();
    let mut final_hash: Option<u64> = None;
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<CommandRecord>(line) {
            records.push(record);
        } else if let Some(hash) = serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .as_ref()
            .and_then(|v| v.get("final_hash"))
            .and_then(serde_json::Value::as_u64)
        {
            final_hash = Some(hash);
        } else {
            panic!(
                "line {}: neither a command record nor a final_hash trailer",
                i + 1
            );
        }
    }
    Transcript {
        records,
        final_hash: final_hash.expect("committed transcript carries a final_hash trailer"),
        plan: Vec::new(),
    }
}

/// The committed transcript parses and replays to its recorded final hash.
#[test]
fn committed_transcript_replays_to_its_hash() {
    let transcript = parse_committed_transcript();
    assert_eq!(
        transcript.records.len(),
        3,
        "the build is import, run-generator, transform"
    );
    let replayed = replay(&transcript).expect("replay");
    assert_eq!(
        replayed, transcript.final_hash,
        "committed transcript must replay to its recorded final hash"
    );
}

/// The committed GDS re-imports and is DRC-clean against the SKY130 subset.
#[test]
fn committed_gds_reimports_drc_subset_clean() {
    let bytes =
        std::fs::read(gds_path()).unwrap_or_else(|e| panic!("read {}: {e}", gds_path().display()));
    let doc = reticle_io::Gds
        .import(&bytes)
        .expect("committed GDS imports");
    let cell = doc
        .cell(TT_TILE_TOP)
        .unwrap_or_else(|| panic!("committed GDS has the {TT_TILE_TOP} cell"));
    assert!(!cell.shapes.is_empty(), "the tile has geometry");
    let engine = DrcEngine::new(sky130_drc_rules());
    let violations = engine.check_cell(&doc, TT_TILE_TOP);
    assert!(
        violations.is_empty(),
        "committed GDS is not DRC-subset-clean: {} violation(s), first {:?}",
        violations.len(),
        violations.first()
    );
}

/// The committed files match what the current generator produces: the committed
/// transcript's recorded final hash equals the freshly built document's `document_hash`.
/// This fails if the committed artifact drifts from `xtask tapeout-example`.
///
/// The comparison is on `document_hash`, not on GDS bytes: GDSII embeds a wall-clock
/// modification timestamp, so a fresh export is never byte-identical to one written
/// earlier. `document_hash` is the timestamp-free determinism contract the transcript
/// replay is built on.
#[test]
fn committed_artifacts_match_the_generator() {
    let fresh_hash = document_hash(&worked_tile_document());

    // The committed transcript's recorded hash matches the fresh build's hash.
    let committed = parse_committed_transcript();
    assert_eq!(
        committed.final_hash, fresh_hash,
        "committed transcript hash differs from a fresh build; re-run `xtask tapeout-example`"
    );

    // The fresh transcript records the same final hash too (sanity on the fresh side).
    let fresh = worked_tile_transcript();
    assert_eq!(fresh.final_hash, fresh_hash);
}
