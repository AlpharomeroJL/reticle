//! The replay theater's playback machinery: load a transcript, play it back.
//!
//! A recorded agent session (see [`reticle_agent_api::Transcript`]) is a list of
//! commands with their outcomes plus the document hash a faithful replay must
//! reproduce. The theater re-applies those commands, one per step, to a fresh
//! in-memory [`Session`], so what the window shows *is* the engine re-executing
//! the run: geometry appears, gets flagged, gets fixed, exactly as it happened.
//! No model or API key is involved; the transcript is the whole script.
//!
//! This module owns everything that does not need an egui context and is
//! unit-tested as plain code:
//!
//! * [`parse_jsonl`], the loader for the `*.transcript.jsonl` format
//!   `reticle-agent` writes (one [`CommandRecord`] per line, then a
//!   `{"final_hash": ...}` trailer).
//! * [`ReplayTheater`], the step/play/pause/speed playback state machine over a
//!   live [`Session`], including the DRC violation list recovered from each
//!   `run_drc` record's recorded response (which feeds the canvas overlay) and
//!   the end-of-run [`HashCheck`] against the recorded final hash.
//! * [`FitView`], the world-to-window projection that frames the replayed
//!   document inside the theater's canvas.
//!
//! The app module owns only the thin window glue: transport buttons, the
//! readouts, and painting the flattened shapes through a [`FitView`].
//!
//! Native-only, like the agent panel: `reticle-agent-api` does not build for
//! `wasm32-unknown-unknown` today.

use reticle_agent_api::{AgentCommand, AgentResponse, CommandRecord, Outcome, Session};
use reticle_geometry::{Point, Rect};
use reticle_model::{DrawShape, Violation, document_hash};

use crate::agent_panel::violations_from_json;

/// Steps per second at 1x speed: one command every half second reads well.
pub const BASE_STEPS_PER_SECOND: f32 = 2.0;

/// The speed multipliers the theater UI offers.
pub const SPEEDS: [f32; 5] = [0.25, 0.5, 1.0, 2.0, 4.0];

/// A transcript line the loader could not parse.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LoadError {
    /// One-based line number of the offending line.
    pub line: usize,
    /// What went wrong with it.
    pub detail: String,
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.detail)
    }
}

/// Parses the transcript JSONL format `reticle-agent` writes.
///
/// Each non-empty line is one JSON [`CommandRecord`]; a line of the form
/// `{"final_hash": <u64>}` is the trailer recording the document hash a replay
/// must reproduce. Blank lines are skipped; anything else is a [`LoadError`]
/// naming the line. Returns the records in order plus the trailer hash, if the
/// file had one.
pub fn parse_jsonl(text: &str) -> Result<(Vec<CommandRecord>, Option<u64>), LoadError> {
    let mut records = Vec::new();
    let mut final_hash = None;
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<CommandRecord>(line) {
            Ok(record) => records.push(record),
            Err(record_err) => {
                // Not a record: accept the final-hash trailer, else report the
                // record parse error (the more informative of the two).
                let trailer: Option<u64> = serde_json::from_str::<serde_json::Value>(line)
                    .ok()
                    .as_ref()
                    .and_then(|v| v.get("final_hash"))
                    .and_then(serde_json::Value::as_u64);
                match trailer {
                    Some(hash) => final_hash = Some(hash),
                    None => {
                        return Err(LoadError {
                            line: i + 1,
                            detail: format!(
                                "neither a command record nor a final_hash trailer ({record_err})"
                            ),
                        });
                    }
                }
            }
        }
    }
    Ok((records, final_hash))
}

/// The verdict on the replayed document versus the transcript's recorded hash.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HashCheck {
    /// Playback has not reached the end of the transcript yet.
    Pending,
    /// Playback finished but the transcript carried no final hash to check.
    Unverifiable,
    /// The replayed document hash matches the recorded one.
    Match,
    /// The replayed document hash differs: the transcript is not reproducible.
    Mismatch,
}

/// The replay theater: a loaded transcript and the playback position over it.
///
/// The session is rebuilt deterministically, so seeking backwards is just
/// replaying fewer commands from scratch; command application is cheap at the
/// scale of a transcript (tens to hundreds of records).
#[derive(Debug, Default)]
pub struct ReplayTheater {
    /// The loaded transcript records, in order.
    records: Vec<CommandRecord>,
    /// The trailer hash a faithful replay reproduces, when the file had one.
    expected_hash: Option<u64>,
    /// The live session the records are re-applied to.
    session: Session,
    /// How many records have been applied so far.
    applied: usize,
    /// Whether playback advances on [`tick`](Self::tick).
    playing: bool,
    /// Speed multiplier over [`BASE_STEPS_PER_SECOND`].
    speed: f32,
    /// Fractional steps accumulated toward the next application.
    acc: f32,
    /// The violation list from the most recent `run_drc` record crossed, so the
    /// theater canvas can mark them; empty before the first verify.
    last_violations: Vec<Violation>,
    /// Whether any `run_drc` record has been crossed at the current position.
    verified: bool,
}

impl ReplayTheater {
    /// An empty theater: nothing loaded, transport disabled.
    #[must_use]
    pub fn new() -> Self {
        Self {
            speed: 1.0,
            ..Self::default()
        }
    }

    /// Loads records (and the expected hash, if known) and rewinds to the start.
    pub fn load(&mut self, records: Vec<CommandRecord>, expected_hash: Option<u64>) {
        self.records = records;
        self.expected_hash = expected_hash;
        self.playing = false;
        let _ = self.seek(0);
    }

    /// Loads a whole [`Transcript`](reticle_agent_api::Transcript).
    pub fn load_transcript(&mut self, transcript: reticle_agent_api::Transcript) {
        self.load(transcript.records, Some(transcript.final_hash));
    }

    /// Parses and loads transcript JSONL text (see [`parse_jsonl`]).
    ///
    /// # Errors
    ///
    /// Returns the [`LoadError`] of the first unparsable line; the theater's
    /// current contents are left untouched in that case.
    pub fn load_jsonl(&mut self, text: &str) -> Result<(), LoadError> {
        let (records, hash) = parse_jsonl(text)?;
        self.load(records, hash);
        Ok(())
    }

    /// Whether a transcript is loaded (even an empty one counts once loaded).
    #[must_use]
    pub fn is_loaded(&self) -> bool {
        !self.records.is_empty()
    }

    /// `(applied, total)` playback progress in records.
    #[must_use]
    pub fn progress(&self) -> (usize, usize) {
        (self.applied, self.records.len())
    }

    /// Whether every record has been applied.
    #[must_use]
    pub fn at_end(&self) -> bool {
        self.applied >= self.records.len()
    }

    /// Whether playback is advancing on ticks.
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Starts playback (a no-op at the end of the transcript).
    pub fn play(&mut self) {
        if !self.at_end() {
            self.playing = true;
        }
    }

    /// Pauses playback, keeping the position.
    pub fn pause(&mut self) {
        self.playing = false;
        self.acc = 0.0;
    }

    /// The speed multiplier over [`BASE_STEPS_PER_SECOND`].
    #[must_use]
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// Sets the speed multiplier, clamped to a sane transport range.
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.clamp(0.05, 32.0);
    }

    /// Advances playback by `dt` seconds while playing.
    ///
    /// Applies however many whole steps come due at the current speed and
    /// auto-pauses at the end of the transcript. Returns the violation list of
    /// the most recent `run_drc` record crossed during this tick, if any, so
    /// the caller can refresh the DRC overlay.
    pub fn tick(&mut self, dt: f32) -> Option<Vec<Violation>> {
        if !self.playing || self.at_end() {
            return None;
        }
        self.acc += dt.max(0.0) * BASE_STEPS_PER_SECOND * self.speed;
        let mut update = None;
        while self.acc >= 1.0 && !self.at_end() {
            self.acc -= 1.0;
            if let Some(v) = self.step_forward() {
                update = Some(v);
            }
        }
        if self.at_end() {
            self.playing = false;
            self.acc = 0.0;
        }
        update
    }

    /// Applies the next record's command to the session (one transport step).
    ///
    /// Returns the violation list parsed from the record's recorded response
    /// when that record is a `run_drc`, `None` otherwise (including at the end
    /// of the transcript).
    pub fn step_forward(&mut self) -> Option<Vec<Violation>> {
        let record = self.records.get(self.applied)?;
        let command = record.command.clone();
        let is_drc = matches!(command, AgentCommand::RunDrc { .. });
        let update = if is_drc {
            recorded_violations(record)
        } else {
            None
        };
        // Replays reproduce failures too: a command that errored in the
        // original run errors identically here and leaves the document alone.
        let _ = self.session.apply(command);
        self.applied += 1;
        if let Some(v) = &update {
            self.last_violations.clone_from(v);
            self.verified = true;
        }
        update
    }

    /// Seeks to `position` records applied (clamped to the transcript length)
    /// by replaying from scratch, which is exact because application is
    /// deterministic.
    ///
    /// Returns the violation list of the last `run_drc` at or before the new
    /// position (`None` when none has run yet there), so the caller can set or
    /// clear the DRC overlay to match the moment being viewed.
    pub fn seek(&mut self, position: usize) -> Option<Vec<Violation>> {
        let position = position.min(self.records.len());
        self.session = Session::new();
        self.applied = 0;
        self.acc = 0.0;
        self.last_violations = Vec::new();
        self.verified = false;
        let mut update = None;
        for _ in 0..position {
            if let Some(v) = self.step_forward() {
                update = Some(v);
            }
        }
        if update.is_none() && self.verified {
            // Unreachable today (verified implies an update), but keep the
            // contract obvious: the overlay reflects the last verify crossed.
            update = Some(self.last_violations.clone());
        }
        update
    }

    /// Steps one record backwards (a seek to `applied - 1`); returns the
    /// overlay update for the new position exactly as [`seek`](Self::seek).
    pub fn step_back(&mut self) -> Option<Vec<Violation>> {
        let target = self.applied.saturating_sub(1);
        self.seek(target)
    }

    /// The record most recently applied, if any (the "now playing" line).
    #[must_use]
    pub fn current_record(&self) -> Option<&CommandRecord> {
        self.applied
            .checked_sub(1)
            .and_then(|i| self.records.get(i))
    }

    /// The replayed document at the current position.
    #[must_use]
    pub fn document(&self) -> &reticle_model::Document {
        self.session.document()
    }

    /// Total shapes drawn so far, summed over every cell's own geometry.
    #[must_use]
    pub fn shape_count(&self) -> usize {
        self.document().cells().map(|c| c.shapes.len()).sum()
    }

    /// The violations from the most recent `run_drc` crossed; empty before the
    /// first verify (see [`has_verified`](Self::has_verified)).
    #[must_use]
    pub fn last_violations(&self) -> &[Violation] {
        &self.last_violations
    }

    /// Whether any `run_drc` record has been crossed at the current position.
    #[must_use]
    pub fn has_verified(&self) -> bool {
        self.verified
    }

    /// The cell the theater should draw: the document's first declared top
    /// cell, else the cell with the most shapes (ties broken by name so the
    /// choice is deterministic), else `None` for an empty document.
    #[must_use]
    pub fn render_cell(&self) -> Option<String> {
        let doc = self.document();
        if let Some(top) = doc.top_cells().first()
            && doc.cell(top).is_some()
        {
            return Some(top.clone());
        }
        doc.cells()
            .map(|c| (c.shapes.len(), &c.name))
            .max_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(a.1)))
            .map(|(_, name)| name.clone())
    }

    /// The flattened geometry of [`render_cell`](Self::render_cell) (instances
    /// and arrays expanded), ready to paint.
    #[must_use]
    pub fn flattened_shapes(&self) -> Vec<DrawShape> {
        self.render_cell()
            .map(|cell| self.document().flatten(&cell))
            .unwrap_or_default()
    }

    /// The verdict on the replayed document hash at the current position.
    #[must_use]
    pub fn hash_check(&self) -> HashCheck {
        if !self.at_end() || self.records.is_empty() {
            return HashCheck::Pending;
        }
        match self.expected_hash {
            None => HashCheck::Unverifiable,
            Some(expected) if document_hash(self.document()) == expected => HashCheck::Match,
            Some(_) => HashCheck::Mismatch,
        }
    }
}

/// The violation list recorded in a `run_drc` record's response, if it has one.
fn recorded_violations(record: &CommandRecord) -> Option<Vec<Violation>> {
    if let Outcome::Ok(AgentResponse::Data { value, .. }) = &record.outcome {
        Some(violations_from_json(value))
    } else {
        None
    }
}

// ----- framing the replayed document in the theater window --------------------

/// A world-to-window projection that letterboxes a bounding box into a
/// viewport, preserving aspect ratio, with the world y axis pointing up.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FitView {
    /// Pixels per DBU.
    scale: f32,
    /// The world point mapped to the viewport center, x.
    world_cx: f32,
    /// The world point mapped to the viewport center, y.
    world_cy: f32,
    /// The viewport center, x, in local window pixels.
    view_cx: f32,
    /// The viewport center, y, in local window pixels.
    view_cy: f32,
}

impl FitView {
    /// Frames `bbox` inside a `width` x `height` viewport with `margin` pixels
    /// kept clear on every side.
    ///
    /// Degenerate inputs stay usable: a zero-extent bounding box (a single
    /// point) gets an arbitrary 1 px/DBU scale centered on the point, and a
    /// margin larger than the viewport collapses to fit-to-viewport.
    #[must_use]
    pub fn fit(bbox: Rect, width: f32, height: f32, margin: f32) -> Self {
        let world_w = bbox.width() as f32;
        let world_h = bbox.height() as f32;
        let avail_w = (width - 2.0 * margin).max(1.0);
        let avail_h = (height - 2.0 * margin).max(1.0);
        let scale = if world_w <= 0.0 && world_h <= 0.0 {
            1.0
        } else {
            let sx = if world_w > 0.0 {
                avail_w / world_w
            } else {
                f32::INFINITY
            };
            let sy = if world_h > 0.0 {
                avail_h / world_h
            } else {
                f32::INFINITY
            };
            sx.min(sy)
        };
        Self {
            scale,
            world_cx: f32::midpoint(bbox.min.x as f32, bbox.max.x as f32),
            world_cy: f32::midpoint(bbox.min.y as f32, bbox.max.y as f32),
            view_cx: width / 2.0,
            view_cy: height / 2.0,
        }
    }

    /// Maps a world point to local window pixels (y grows downward on screen).
    #[must_use]
    pub fn to_screen(&self, p: Point) -> (f32, f32) {
        (
            self.view_cx + (p.x as f32 - self.world_cx) * self.scale,
            self.view_cy - (p.y as f32 - self.world_cy) * self.scale,
        )
    }

    /// The projection scale, in pixels per DBU.
    #[must_use]
    pub fn scale(&self) -> f32 {
        self.scale
    }
}

/// The union of the bounding boxes of `shapes`, if there are any.
#[must_use]
pub fn shapes_bbox(shapes: &[DrawShape]) -> Option<Rect> {
    shapes
        .iter()
        .map(reticle_geometry::Shape::bounding_box)
        .reduce(|a, b| a.union(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_panel::scripted_run;

    /// The JSONL text `reticle-agent` would write for the scripted demo run.
    fn scripted_jsonl() -> (String, usize, u64) {
        let (transcript, _) = scripted_run("theater test");
        let mut text = String::new();
        for record in &transcript.records {
            text.push_str(&serde_json::to_string(record).expect("record serializes"));
            text.push('\n');
        }
        text.push_str(&serde_json::json!({ "final_hash": transcript.final_hash }).to_string());
        text.push('\n');
        (text, transcript.records.len(), transcript.final_hash)
    }

    #[test]
    fn parse_jsonl_reads_records_and_trailer() {
        let (text, count, hash) = scripted_jsonl();
        let (records, parsed_hash) = parse_jsonl(&text).expect("parses");
        assert_eq!(records.len(), count);
        assert_eq!(parsed_hash, Some(hash));
        // Sequence numbers survive the round trip in order.
        assert!(records.iter().enumerate().all(|(i, r)| r.seq == i as u64));
    }

    #[test]
    fn parse_jsonl_skips_blank_lines_and_reports_bad_ones() {
        let (text, count, _) = scripted_jsonl();
        let spaced = text.replace('\n', "\n\n");
        let (records, _) = parse_jsonl(&spaced).expect("blank lines are skipped");
        assert_eq!(records.len(), count);

        let err = parse_jsonl("{\"op\": \"not a record\"}\n").expect_err("must fail");
        assert_eq!(err.line, 1);
        assert!(err.to_string().contains("line 1"));

        let mut with_bad = text.clone();
        with_bad.push_str("garbage\n");
        let err = parse_jsonl(&with_bad).expect_err("trailing garbage fails");
        assert_eq!(err.line, text.lines().count() + 1);
    }

    #[test]
    fn empty_text_loads_as_nothing() {
        let (records, hash) = parse_jsonl("").expect("empty is fine");
        assert!(records.is_empty());
        assert_eq!(hash, None);
        let mut theater = ReplayTheater::new();
        theater.load(records, hash);
        assert!(!theater.is_loaded());
        assert_eq!(theater.hash_check(), HashCheck::Pending);
    }

    #[test]
    fn load_jsonl_error_leaves_previous_contents() {
        let (text, count, _) = scripted_jsonl();
        let mut theater = ReplayTheater::new();
        theater.load_jsonl(&text).expect("loads");
        assert_eq!(theater.progress(), (0, count));
        theater.load_jsonl("garbage").expect_err("must fail");
        assert_eq!(theater.progress(), (0, count), "old transcript kept");
    }

    #[test]
    fn stepping_replays_geometry_and_recovers_recorded_violations() {
        let (text, count, _) = scripted_jsonl();
        let mut theater = ReplayTheater::new();
        theater.load_jsonl(&text).expect("loads");
        assert_eq!(theater.shape_count(), 0);
        assert!(!theater.has_verified());

        let mut drc_updates = Vec::new();
        while !theater.at_end() {
            if let Some(v) = theater.step_forward() {
                drc_updates.push(v);
            }
        }
        assert_eq!(theater.progress(), (count, count));
        // The script verifies twice: first flagged, then clean.
        assert_eq!(drc_updates.len(), 2);
        assert!(
            !drc_updates[0].is_empty(),
            "first verify flags the thin wire"
        );
        assert!(drc_updates[1].is_empty(), "second verify is clean");
        assert!(theater.has_verified());
        assert!(theater.last_violations().is_empty());
        // The corrected wide wire is the one shape left.
        assert_eq!(theater.shape_count(), 1);
        assert_eq!(theater.hash_check(), HashCheck::Match);
        // Stepping past the end is a no-op.
        assert!(theater.step_forward().is_none());
    }

    #[test]
    fn seek_recomputes_position_shapes_and_overlay() {
        let (transcript, _) = scripted_run("seek me");
        let count = transcript.records.len();
        // Index of the first run_drc record.
        let first_drc = transcript
            .records
            .iter()
            .position(|r| matches!(r.command, AgentCommand::RunDrc { .. }))
            .expect("script verifies");
        let mut theater = ReplayTheater::new();
        theater.load_transcript(transcript);

        // Before the first verify: geometry exists, no overlay update.
        let update = theater.seek(first_drc);
        assert!(update.is_none(), "no verify crossed yet");
        assert!(!theater.has_verified());
        assert_eq!(theater.shape_count(), 1, "the thin wire is drawn");

        // Just after the first verify: the flagged list comes back.
        let update = theater.seek(first_drc + 1).expect("verify crossed");
        assert!(!update.is_empty());
        assert!(theater.has_verified());

        // Rewind to the very start.
        assert!(theater.seek(0).is_none());
        assert_eq!(theater.shape_count(), 0);
        assert_eq!(theater.progress(), (0, count));

        // Seeking past the end clamps.
        theater.seek(count + 100);
        assert_eq!(theater.progress(), (count, count));
        assert_eq!(theater.hash_check(), HashCheck::Match);

        // step_back walks one record at a time.
        theater.step_back();
        assert_eq!(theater.progress(), (count - 1, count));
        assert_eq!(theater.hash_check(), HashCheck::Pending);
    }

    #[test]
    fn tick_paces_by_speed_and_autopauses_at_the_end() {
        let (transcript, _) = scripted_run("pacing");
        let count = transcript.records.len();
        let mut theater = ReplayTheater::new();
        theater.load_transcript(transcript);

        // Paused: ticks do nothing.
        assert!(theater.tick(10.0).is_none());
        assert_eq!(theater.progress().0, 0);

        // 1x is BASE_STEPS_PER_SECOND steps per second.
        theater.play();
        assert!(theater.is_playing());
        theater.tick(1.0 / BASE_STEPS_PER_SECOND);
        assert_eq!(theater.progress().0, 1);

        // 4x covers four steps in the same second-per-base-step.
        theater.set_speed(4.0);
        theater.tick(1.0 / BASE_STEPS_PER_SECOND);
        assert_eq!(theater.progress().0, 5);

        // Pause holds the position and drops the fractional accumulator.
        theater.pause();
        assert!(theater.tick(100.0).is_none());
        assert_eq!(theater.progress().0, 5);

        // A long tick finishes the run and auto-pauses.
        theater.play();
        let _ = theater.tick(1_000.0);
        assert_eq!(theater.progress(), (count, count));
        assert!(!theater.is_playing());
        assert!(theater.current_record().is_some());

        // Play at the end stays paused rather than spinning.
        theater.play();
        assert!(!theater.is_playing());
    }

    #[test]
    fn speed_is_clamped() {
        let mut theater = ReplayTheater::new();
        theater.set_speed(0.0);
        assert!(theater.speed() >= 0.05);
        theater.set_speed(1_000.0);
        assert!(theater.speed() <= 32.0);
    }

    #[test]
    fn hash_mismatch_is_reported() {
        let (mut transcript, _) = scripted_run("tamper");
        transcript.final_hash ^= 0xBAD;
        let count = transcript.records.len();
        let mut theater = ReplayTheater::new();
        theater.load_transcript(transcript);
        theater.seek(count);
        assert_eq!(theater.hash_check(), HashCheck::Mismatch);
    }

    #[test]
    fn missing_trailer_is_unverifiable() {
        let (transcript, _) = scripted_run("no trailer");
        let count = transcript.records.len();
        let mut theater = ReplayTheater::new();
        theater.load(transcript.records, None);
        theater.seek(count);
        assert_eq!(theater.hash_check(), HashCheck::Unverifiable);
    }

    #[test]
    fn render_cell_prefers_top_then_largest() {
        let (transcript, _) = scripted_run("render cell");
        let mut theater = ReplayTheater::new();
        theater.load_transcript(transcript);
        assert_eq!(theater.render_cell(), None, "empty document");
        theater.seek(usize::MAX);
        // The scripted session never declares a top cell, so the largest
        // (only) cell wins.
        assert_eq!(
            theater.render_cell().as_deref(),
            Some(crate::agent_panel::AGENT_CELL)
        );
        assert_eq!(theater.flattened_shapes().len(), theater.shape_count());
        assert!(shapes_bbox(&theater.flattened_shapes()).is_some());
    }

    #[test]
    fn fit_view_letterboxes_and_flips_y() {
        // A wide 200 x 100 DBU box into a 400 x 400 px viewport with a 20 px
        // margin: the x extent binds, scale = 360 / 200 = 1.8.
        let bbox = Rect::new(Point::new(0, 0), Point::new(200, 100));
        let view = FitView::fit(bbox, 400.0, 400.0, 20.0);
        assert!((view.scale() - 1.8).abs() < 1e-6);
        // The box center maps to the viewport center.
        let (cx, cy) = view.to_screen(Point::new(100, 50));
        assert!((cx - 200.0).abs() < 1e-4 && (cy - 200.0).abs() < 1e-4);
        // World +y is screen -y: the top edge of the box lands above center.
        let (_, top_y) = view.to_screen(Point::new(100, 100));
        assert!(top_y < cy);
        // The extreme corners stay inside the margin.
        for corner in [bbox.min, bbox.max] {
            let (x, y) = view.to_screen(corner);
            assert!((20.0..=380.0).contains(&x), "x {x} outside margin");
            assert!((20.0..=380.0).contains(&y), "y {y} outside margin");
        }
    }

    #[test]
    fn fit_view_handles_degenerate_boxes() {
        // A single point: unit scale, centered.
        let point_box = Rect::new(Point::new(5, 5), Point::new(5, 5));
        let view = FitView::fit(point_box, 100.0, 60.0, 10.0);
        assert!((view.scale() - 1.0).abs() < 1e-6);
        let (x, y) = view.to_screen(Point::new(5, 5));
        assert!((x - 50.0).abs() < 1e-4 && (y - 30.0).abs() < 1e-4);
        // A zero-height line still frames by the x extent.
        let line_box = Rect::new(Point::new(0, 7), Point::new(50, 7));
        let view = FitView::fit(line_box, 120.0, 60.0, 10.0);
        assert!((view.scale() - 2.0).abs() < 1e-6);
    }
}
