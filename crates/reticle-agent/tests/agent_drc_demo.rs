//! The committed DRC-fix demo transcript is deterministic, replayable, and mirrored.
//!
//! These pin the honest, model-free demo (see [`reticle_agent::live::scripted_drc_fix_steps`]):
//! the scripted run reproduces a fixed document hash, the committed
//! `examples/collab/agent_drc_fix.transcript.jsonl` replays to that same hash through the
//! frozen replay parser, and driving the same script through an [`AgentCollaborator`]
//! mirrors the `TransformShapes` onto the CRDT (the offending rectangle actually *moves*
//! to a legal spacing). This is the whole point of closing ADR 0022's id-addressed gap.

use reticle_agent::live::{
    DRC_FIX_CELL, scripted_drc_fix_jsonl, scripted_drc_fix_steps, scripted_drc_fix_transcript,
};
use reticle_agent::{AgentCollaborator, Pacing};
use reticle_agent_api::{CommandRecord, Transcript, verify_replay};
use reticle_model::ShapeKind;

/// The document hash the DRC-fix demo reproduces. Pinned: a change here means the demo's
/// geometry or the model's hashing changed, and the committed transcript must be
/// regenerated (see the example's `--emit` mode).
const PINNED_FINAL_HASH: u64 = 3_163_482_734_529_708_122;

/// The path to the committed transcript, resolved from the crate directory so the test is
/// independent of the working directory.
fn committed_transcript_path() -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR is `crates/reticle-agent`; the committed artifact is at the repo
    // root under `examples/collab`.
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/collab/agent_drc_fix.transcript.jsonl")
}

/// Parses the replay-theater JSONL exactly as the frozen loader does (which this crate
/// cannot depend on): one [`CommandRecord`] per line, then a `{"final_hash": ...}`
/// trailer whose other fields are ignored. Returns the records and the trailer hash.
fn parse_jsonl(text: &str) -> (Vec<CommandRecord>, Option<u64>) {
    let mut records = Vec::new();
    let mut final_hash = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<CommandRecord>(line) {
            records.push(record);
        } else {
            let hash = serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .as_ref()
                .and_then(|v| v.get("final_hash"))
                .and_then(serde_json::Value::as_u64);
            assert!(
                hash.is_some(),
                "a non-record line must be the final_hash trailer: {line}"
            );
            final_hash = hash;
        }
    }
    (records, final_hash)
}

#[test]
fn the_scripted_run_reproduces_the_pinned_hash_and_replays() {
    let transcript = scripted_drc_fix_transcript();
    assert_eq!(
        transcript.final_hash, PINNED_FINAL_HASH,
        "the demo's document hash changed; regenerate the committed transcript"
    );
    verify_replay(&transcript).expect("the scripted transcript replays to its own hash");

    // The last verify recorded a clean DRC (count 0), and the first flagged one (count 1):
    // the transcript is an honest fix, not a fabricated pass.
    let drc_counts: Vec<u64> = transcript
        .records
        .iter()
        .filter(|r| matches!(r.command, reticle_agent_api::AgentCommand::RunDrc { .. }))
        .filter_map(|r| match &r.outcome {
            reticle_agent_api::Outcome::Ok(reticle_agent_api::AgentResponse::Data {
                value,
                ..
            }) => value.get("count").and_then(serde_json::Value::as_u64),
            _ => None,
        })
        .collect();
    assert_eq!(
        drc_counts,
        vec![1, 0],
        "flagged once, then clean after the fix"
    );
}

#[test]
fn the_committed_transcript_replays_to_the_pinned_hash() {
    let text = std::fs::read_to_string(committed_transcript_path())
        .expect("the committed demo transcript is present");
    let (records, final_hash) = parse_jsonl(&text);
    assert_eq!(
        final_hash,
        Some(PINNED_FINAL_HASH),
        "the committed trailer pins the demo hash"
    );
    // Replay the committed records through the frozen replay contract and assert the hash.
    let transcript = Transcript {
        records,
        final_hash: PINNED_FINAL_HASH,
        plan: Vec::new(),
    };
    verify_replay(&transcript).expect("the committed transcript replays to its recorded hash");
}

#[test]
fn the_committed_file_matches_the_generator_byte_for_byte() {
    // Regeneration is deterministic, so the committed artifact must equal what the
    // generator produces now. If this fails, re-run the example's `--emit` mode.
    let generated = scripted_drc_fix_jsonl();
    let committed = std::fs::read_to_string(committed_transcript_path())
        .expect("the committed demo transcript is present")
        .replace("\r\n", "\n");
    assert_eq!(
        generated, committed,
        "the committed transcript drifted from the generator; regenerate it"
    );
}

#[test]
fn the_transform_is_mirrored_onto_the_crdt_and_moves_the_rect() {
    // Drive the same script through a collaborator (as a live-room run does) and assert
    // the mirror moved the second rectangle to the legal gap, with no skipped ids.
    let mut collab = AgentCollaborator::new(Pacing::Instant);
    let mut any_skipped = false;
    for step in scripted_drc_fix_steps() {
        let report = collab.apply_step(&step);
        any_skipped |= !report.skipped.is_empty();
    }
    assert!(!any_skipped, "every addressed id resolved; nothing skipped");

    let cell = collab
        .document()
        .cell(DRC_FIX_CELL)
        .expect("the mirrored cell is present");
    assert_eq!(cell.shapes.len(), 2, "both rects are mirrored");

    // The moved rectangle sits at x in [400, 600] (originally [300, 500]); its partner
    // stays at [0, 200]. The gap is now 200 DBU, above the 140 DBU minimum.
    let mut xs: Vec<(i32, i32)> = cell
        .shapes
        .iter()
        .filter_map(|s| match &s.kind {
            ShapeKind::Rect(r) => Some((r.min.x, r.max.x)),
            _ => None,
        })
        .collect();
    xs.sort_unstable();
    assert_eq!(
        xs,
        vec![(0, 200), (400, 600)],
        "the transform moved the second rect to a legal spacing on the CRDT"
    );

    // The mirrored geometry matches the authoritative session's geometry: the mirror did
    // not drift from what the commands actually built. (The CRDT does not carry the
    // technology, so the whole documents are compared shape-for-shape, not by equality.)
    let session_cell = collab
        .session()
        .document()
        .cell(DRC_FIX_CELL)
        .expect("the session cell is present");
    assert_eq!(
        cell.shapes, session_cell.shapes,
        "the CRDT mirror's shapes match the session's shapes"
    );
}
