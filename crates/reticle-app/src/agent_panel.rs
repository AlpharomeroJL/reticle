//! The agent panel's window-free logic: prompt, run state machine, and narration.
//!
//! The panel narrates an agent's propose-verify-correct loop without needing a
//! live model or an API key: it steps through an [`AgentStep`] feed derived from
//! a recorded [`Transcript`] (each step carries a narration line, an
//! [`AgentStatus`], the agent's cursor position, and, for `run_drc` records, the
//! parsed violation list so the canvas DRC overlay can update live).
//!
//! [`scripted_run`] builds such a transcript by driving a real
//! [`Session`] through a small scripted propose-verify-correct loop (install a
//! width rule, draw a too-thin wire, watch DRC flag it, correct it, watch DRC
//! come back clean), so everything the panel shows comes from the real engine.
//!
//! All the interesting behavior (the idle/running/stopped state machine, the
//! narration ring, deriving statuses and cursors from records, parsing
//! violations out of `run_drc` responses) is plain code here, unit-tested
//! without an egui context; the app module owns only the thin drawing glue.
//!
//! Model-free and portable: this compiles and runs on both native and
//! `wasm32-unknown-unknown`. `reticle-agent-api` builds for wasm (its `render_png`
//! command degrades to a clean error there rather than requiring the native
//! blocking GPU context), so the web build runs the real panel, not a stub.

use reticle_agent_api::args::{LayerArg, PointArg, RectArg};
use reticle_agent_api::{
    AgentCommand, AgentResponse, AgentStatus, CommandRecord, Outcome, PlanStep, Session,
    Transcript, transcript_of,
};
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{RuleKind, Violation};

/// The cell the scripted demo run draws into.
pub const AGENT_CELL: &str = "AGENT_DEMO";

/// Maximum narration lines kept; older lines are dropped from the front.
const MAX_NARRATION: usize = 200;

/// Maximum conversation entries kept; older turns are dropped from the front.
const MAX_CONVERSATION: usize = 200;

/// Default pacing of the feed, in seconds between emitted steps.
const DEFAULT_STEP_PERIOD: f32 = 0.6;

/// Who authored a [`ConversationEntry`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Speaker {
    /// A message the user typed (the initial prompt or a follow-up instruction).
    User,
    /// A line the agent (or the panel narrating on its behalf) produced.
    Agent,
}

/// One turn in the panel's conversation transcript.
///
/// This is the UI-side record of the back-and-forth: the user's prompts and
/// follow-up instructions interleaved with the agent's status lines. It is
/// distinct from the engine [`Transcript`] (which records applied commands and
/// their outcomes); a conversation entry is human-facing text, not a replayable
/// command.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ConversationEntry {
    /// Who said it.
    pub speaker: Speaker,
    /// The message text.
    pub text: String,
}

impl ConversationEntry {
    /// A user turn carrying `text`.
    #[must_use]
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            speaker: Speaker::User,
            text: text.into(),
        }
    }

    /// An agent turn carrying `text`.
    #[must_use]
    pub fn agent(text: impl Into<String>) -> Self {
        Self {
            speaker: Speaker::Agent,
            text: text.into(),
        }
    }
}

/// The agent panel's run state machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RunState {
    /// No run has started (or the panel was reset).
    #[default]
    Idle,
    /// A run is stepping through its feed.
    Running,
    /// The run halted: the feed finished or the user pressed Stop.
    Stopped,
}

/// One step of an agent run, ready to be narrated and drawn.
#[derive(Clone, Debug)]
pub struct AgentStep {
    /// The narration line for this step.
    pub narration: String,
    /// The live status after this step (iteration, step label, violation count).
    pub status: AgentStatus,
    /// Where the agent's cursor is after this step, in DBU world coordinates.
    pub cursor: Option<Point>,
    /// For `run_drc` steps, the violations parsed from the response, so the DRC
    /// overlay can update live. `None` for every other step.
    pub violations: Option<Vec<Violation>>,
}

/// The agent panel state: prompt, state machine, feed position, and narration.
#[derive(Debug)]
pub struct AgentPanelState {
    /// The prompt text the user is editing.
    pub prompt: String,
    /// The follow-up instruction the user is composing in conversation mode.
    pub followup: String,
    state: RunState,
    feed: Vec<AgentStep>,
    next: usize,
    narration: Vec<String>,
    latest: Option<AgentStatus>,
    cursor: Option<Point>,
    seconds_per_step: f32,
    acc: f32,
    transcript: Option<Transcript>,
    plan: Vec<PlanStep>,
    conversation: Vec<ConversationEntry>,
    followups: Vec<String>,
}

impl Default for AgentPanelState {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentPanelState {
    /// Creates an idle panel with an empty prompt and no feed.
    #[must_use]
    pub fn new() -> Self {
        Self {
            prompt: String::new(),
            followup: String::new(),
            state: RunState::Idle,
            feed: Vec::new(),
            next: 0,
            narration: Vec::new(),
            latest: None,
            cursor: None,
            seconds_per_step: DEFAULT_STEP_PERIOD,
            acc: 0.0,
            transcript: None,
            plan: Vec::new(),
            conversation: Vec::new(),
            followups: Vec::new(),
        }
    }

    /// Starts a scripted run for the current prompt (see [`scripted_run`]).
    ///
    /// The prompt opens the conversation as the first user turn, so the
    /// conversation transcript begins with what was asked.
    pub fn start(&mut self) {
        let (transcript, feed) = scripted_run(&self.prompt);
        let opening = self.prompt.clone();
        self.begin(feed, Some(transcript));
        if !opening.trim().is_empty() {
            self.push_conversation(ConversationEntry::user(opening));
        }
    }

    /// Starts a run that narrates an existing transcript instead of the script.
    pub fn start_from_transcript(&mut self, transcript: &Transcript) {
        let feed = feed_from_transcript(transcript);
        self.begin(feed, Some(transcript.clone()));
    }

    /// Arms a new feed and enters [`RunState::Running`].
    ///
    /// The accumulator starts one full period in credit so the first step is
    /// emitted by the very next [`tick`](Self::tick) rather than after a delay.
    /// The conversation transcript is cleared so a fresh run starts a fresh
    /// conversation.
    fn begin(&mut self, feed: Vec<AgentStep>, transcript: Option<Transcript>) {
        self.feed = feed;
        self.next = 0;
        self.acc = self.seconds_per_step;
        self.narration.clear();
        self.push_line("run started".to_owned());
        self.latest = None;
        self.cursor = None;
        // The plan log rides on the transcript (`Transcript::plan`); surface it so the
        // panel can render the agent's stated per-iteration intent next to the run.
        self.plan = transcript
            .as_ref()
            .map_or_else(Vec::new, |t| t.plan.clone());
        self.transcript = transcript;
        self.conversation.clear();
        self.followups.clear();
        self.state = RunState::Running;
    }

    /// Stops a running feed (the Stop button). Idle or already-stopped panels
    /// are left unchanged.
    pub fn stop(&mut self) {
        if self.state != RunState::Running {
            return;
        }
        self.state = RunState::Stopped;
        self.push_line("run stopped by user".to_owned());
        if let Some(status) = &mut self.latest {
            status.running = false;
        }
    }

    // ----- conversation mode -------------------------------------------------

    /// Submits the follow-up instruction in [`followup`](Self::followup) to the
    /// running session as a new constraint, appending it to the conversation and
    /// clearing the input box.
    ///
    /// The message is appended as a user turn, followed by an agent
    /// acknowledgement turn, and the instruction is recorded in the follow-up
    /// list a Wave-3 scoped harness will forward to the live agent. On the UI
    /// side today that acknowledgement is scripted, because the panel narrates a
    /// recorded transcript rather than driving a live model; the honest seam is
    /// [`followups`](Self::followups), which a live harness consumes.
    ///
    /// A follow-up is only accepted while a run is [`Running`](RunState::Running)
    /// (an instruction has no session to attach to otherwise) and only when the
    /// trimmed text is non-empty. Returns the appended instruction on success,
    /// `None` when it was rejected (nothing was mutated in that case).
    pub fn submit_followup(&mut self) -> Option<String> {
        let text = self.followup.trim().to_owned();
        if text.is_empty() || self.state != RunState::Running {
            return None;
        }
        self.followup.clear();
        self.push_conversation(ConversationEntry::user(text.clone()));
        self.push_conversation(ConversationEntry::agent(format!(
            "acknowledged; folding \"{text}\" into the run as a new constraint"
        )));
        self.push_line(format!("follow-up: {text}"));
        self.followups.push(text.clone());
        Some(text)
    }

    /// Appends `entry` to the conversation, dropping the oldest turn above
    /// [`MAX_CONVERSATION`].
    fn push_conversation(&mut self, entry: ConversationEntry) {
        self.conversation.push(entry);
        if self.conversation.len() > MAX_CONVERSATION {
            let excess = self.conversation.len() - MAX_CONVERSATION;
            self.conversation.drain(..excess);
        }
    }

    /// Appends an agent-authored line to the conversation transcript.
    ///
    /// The app uses this to surface each verify result (DRC clean or a violation
    /// count) as a conversational turn, so the transcript reads as a dialogue and
    /// not just a raw command log.
    pub fn note_agent(&mut self, text: impl Into<String>) {
        self.push_conversation(ConversationEntry::agent(text));
    }

    /// Clears the conversation transcript, the follow-up list, and the input box.
    pub fn clear_conversation(&mut self) {
        self.conversation.clear();
        self.followups.clear();
        self.followup.clear();
    }

    /// The conversation transcript so far, oldest turn first.
    #[must_use]
    pub fn conversation(&self) -> &[ConversationEntry] {
        &self.conversation
    }

    /// The follow-up instructions submitted during this run, in order.
    ///
    /// This is the Wave-3 seam: a live scoped harness reads these and forwards
    /// them to the model as additional constraints on the running session. The
    /// UI records them here regardless, so the affordance is real even before
    /// that harness exists.
    #[must_use]
    pub fn followups(&self) -> &[String] {
        &self.followups
    }

    /// Advances the run by `dt` seconds, emitting any steps that come due.
    ///
    /// Returns the most recent violation list emitted during this tick (from a
    /// `run_drc` step), so the caller can refresh the DRC overlay; `None` when
    /// no verify step fired. When the feed is exhausted the state moves to
    /// [`RunState::Stopped`] and a completion line is narrated.
    pub fn tick(&mut self, dt: f32) -> Option<Vec<Violation>> {
        if self.state != RunState::Running {
            return None;
        }
        self.acc += dt.max(0.0);
        let mut update: Option<Vec<Violation>> = None;
        while self.acc >= self.seconds_per_step && self.next < self.feed.len() {
            self.acc -= self.seconds_per_step;
            let step = self.feed[self.next].clone();
            self.next += 1;
            self.push_line(step.narration);
            self.latest = Some(step.status);
            if step.cursor.is_some() {
                self.cursor = step.cursor;
            }
            if let Some(v) = step.violations {
                update = Some(v);
            }
        }
        if self.next >= self.feed.len() {
            self.state = RunState::Stopped;
            let remaining = self.latest.as_ref().map_or(0, |s| s.violations);
            self.push_line(format!("run complete: {remaining} violation(s) remaining"));
            self.acc = 0.0;
        }
        update
    }

    /// Appends a narration line, dropping the oldest above [`MAX_NARRATION`].
    fn push_line(&mut self, line: String) {
        self.narration.push(line);
        if self.narration.len() > MAX_NARRATION {
            let excess = self.narration.len() - MAX_NARRATION;
            self.narration.drain(..excess);
        }
    }

    /// The current run state.
    #[must_use]
    pub fn state(&self) -> RunState {
        self.state
    }

    /// Whether a run is currently stepping.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.state == RunState::Running
    }

    /// The narration lines emitted so far, oldest first.
    #[must_use]
    pub fn narration(&self) -> &[String] {
        &self.narration
    }

    /// The most recent status emitted by the feed, if any.
    #[must_use]
    pub fn latest_status(&self) -> Option<&AgentStatus> {
        self.latest.as_ref()
    }

    /// The agent's current cursor position in world DBU, if it has one.
    #[must_use]
    pub fn cursor(&self) -> Option<Point> {
        self.cursor
    }

    /// `(emitted, total)` feed progress.
    #[must_use]
    pub fn progress(&self) -> (usize, usize) {
        (self.next, self.feed.len())
    }

    /// The transcript backing the current (or last) run, if any.
    #[must_use]
    pub fn transcript(&self) -> Option<&Transcript> {
        self.transcript.as_ref()
    }

    /// The agent's per-iteration plan for the current (or last) run, oldest first.
    ///
    /// Each [`PlanStep`] is the stated intent at the top of one iteration (goal,
    /// intended tools, expected checks); it is narration for the viewer, not a binding
    /// contract on what the iteration did. Empty when the backing transcript carried no
    /// plan (for example a transcript recorded before the plan log existed).
    #[must_use]
    pub fn plan(&self) -> &[PlanStep] {
        &self.plan
    }

    /// Overrides the pacing (seconds between steps), clamped to at least 10 ms.
    pub fn set_step_period(&mut self, seconds: f32) {
        self.seconds_per_step = seconds.max(0.01);
    }
}

// ----- scripted demo run -----------------------------------------------------

/// The technology installed by the scripted run: one metal layer with a
/// 100 DBU minimum-width rule, so the engine has something real to flag.
const SCRIPT_TECH: &str = "technology agent-demo\n\
                           dbu_per_micron 1000\n\
                           layer 4 0 met1 3A6FD4FF\n\
                           rule width 4 0 100\n";

/// Runs the built-in scripted propose-verify-correct loop against a real
/// [`Session`] and returns its transcript plus the derived narration feed.
///
/// The script installs `SCRIPT_TECH`, creates [`AGENT_CELL`], draws a 60 DBU
/// wide wire (violating the 100 DBU width rule), verifies with `run_drc`,
/// corrects by deleting the thin wire and drawing a 400 DBU one, verifies
/// again (clean), and closes with a cell summary. The prompt seeds where the
/// geometry lands so different prompts draw in visibly different spots, and it
/// opens the narration; no model or key is involved anywhere.
#[must_use]
pub fn scripted_run(prompt: &str) -> (Transcript, Vec<AgentStep>) {
    let seed = prompt_seed(prompt);
    let ox = 23_000 + ((seed % 5) as i32) * 1_200;
    let oy = ((seed >> 3) % 5) as i32 * 1_400;
    let layer = LayerArg {
        layer: 4,
        datatype: 0,
    };
    let rect = |x0: i32, y0: i32, x1: i32, y1: i32| RectArg {
        min: PointArg { x: x0, y: y0 },
        max: PointArg { x: x1, y: y1 },
    };

    let mut session = Session::new();
    let _ = session.apply(AgentCommand::SetTechnology {
        source: SCRIPT_TECH.to_owned(),
    });
    let _ = session.apply(AgentCommand::CreateCell {
        name: AGENT_CELL.to_owned(),
    });
    // Propose: a 60 DBU wide vertical wire, thinner than the 100 DBU rule.
    let thin = session.apply(AgentCommand::AddRect {
        cell: AGENT_CELL.to_owned(),
        layer,
        rect: rect(ox, oy, ox + 60, oy + 2_000),
    });
    // Verify: the width rule flags the thin wire.
    let _ = session.apply(AgentCommand::RunDrc {
        cell: AGENT_CELL.to_owned(),
        region: None,
    });
    // Correct: remove the offending wire (by the id the engine handed back).
    if let Ok(AgentResponse::Ok { affected, .. }) = thin
        && let Some(&id) = affected.first()
    {
        let _ = session.apply(AgentCommand::DeleteShapes { ids: vec![id] });
    }
    // Propose again: a 400 DBU wide wire that satisfies the rule.
    let _ = session.apply(AgentCommand::AddRect {
        cell: AGENT_CELL.to_owned(),
        layer,
        rect: rect(ox, oy, ox + 400, oy + 2_000),
    });
    // Verify again: clean.
    let _ = session.apply(AgentCommand::RunDrc {
        cell: AGENT_CELL.to_owned(),
        region: None,
    });
    let _ = session.apply(AgentCommand::GetCellInfo {
        cell: AGENT_CELL.to_owned(),
    });

    let mut transcript = transcript_of(&session);
    // Attach a two-step plan matching the script's two propose-verify iterations, so
    // the panel's plan section has real content without a live harness. This mirrors
    // what the `reticle-agent` harness derives per iteration (goal, intended tools,
    // expected checks); it is narration for the viewer, not a binding contract.
    transcript.plan = vec![
        PlanStep {
            goal: prompt.to_owned(),
            intended_tools: vec!["add_rect".to_owned(), "run_drc".to_owned()],
            expected_checks: vec!["drc".to_owned()],
        },
        PlanStep {
            goal: prompt.to_owned(),
            intended_tools: vec![
                "delete_shapes".to_owned(),
                "add_rect".to_owned(),
                "run_drc".to_owned(),
            ],
            expected_checks: vec!["drc".to_owned()],
        },
    ];
    let mut feed = vec![AgentStep {
        narration: format!("prompt: {prompt}"),
        status: AgentStatus {
            iteration: 0,
            step: "starting".to_owned(),
            violations: 0,
            running: true,
        },
        cursor: None,
        violations: None,
    }];
    feed.extend(feed_from_transcript(&transcript));
    (transcript, feed)
}

/// A tiny FNV-1a hash of the prompt, used only to scatter the scripted
/// geometry so different prompts draw in different places.
fn prompt_seed(prompt: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in prompt.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash
}

// ----- deriving a feed from a transcript --------------------------------------

/// Derives the narrated step feed from a recorded transcript.
///
/// Iterations count completed `run_drc` verifications (zero-based, matching
/// [`AgentStatus::iteration`]); the violation count carries forward from the
/// most recent verify; the cursor follows the geometry each command touches;
/// and the final step reports `running = false`.
#[must_use]
pub fn feed_from_transcript(transcript: &Transcript) -> Vec<AgentStep> {
    let mut steps = Vec::with_capacity(transcript.records.len());
    let mut iteration: u32 = 0;
    let mut violation_count: u32 = 0;
    let mut cursor: Option<Point> = None;
    for (i, record) in transcript.records.iter().enumerate() {
        let mut violations: Option<Vec<Violation>> = None;
        if matches!(record.command, AgentCommand::RunDrc { .. })
            && let Outcome::Ok(AgentResponse::Data { value, .. }) = &record.outcome
        {
            let parsed = violations_from_json(value);
            violation_count = parsed.len() as u32;
            violations = Some(parsed);
        }
        if let Some(p) = command_target(&record.command) {
            cursor = Some(p);
        }
        let status = AgentStatus {
            iteration,
            step: step_label(&record.command).to_owned(),
            violations: violation_count,
            running: i + 1 < transcript.records.len(),
        };
        if matches!(record.command, AgentCommand::RunDrc { .. }) {
            iteration += 1;
        }
        steps.push(AgentStep {
            narration: narrate_record(record),
            status,
            cursor,
            violations,
        });
    }
    steps
}

/// Formats one transcript record as a single narration line.
#[must_use]
pub fn narrate_record(record: &CommandRecord) -> String {
    format!(
        "#{} {} -> {}",
        record.seq,
        describe_command(&record.command),
        outcome_note(&record.outcome)
    )
}

/// A terse, human-readable description of a command.
#[must_use]
pub fn describe_command(cmd: &AgentCommand) -> String {
    match cmd {
        AgentCommand::CreateCell { name } => format!("create_cell {name}"),
        AgentCommand::DeleteCell { name } => format!("delete_cell {name}"),
        AgentCommand::AddRect { cell, layer, rect } => format!(
            "add_rect {cell} l{}/{} ({},{})-({},{})",
            layer.layer, layer.datatype, rect.min.x, rect.min.y, rect.max.x, rect.max.y
        ),
        AgentCommand::AddPolygon { cell, points, .. } => {
            format!("add_polygon {cell} {} vertex(es)", points.len())
        }
        AgentCommand::AddPath {
            cell,
            width,
            points,
            ..
        } => format!("add_path {cell} w{width} {} vertex(es)", points.len()),
        AgentCommand::PlaceInstance { cell, child, .. } => {
            format!("place_instance {child} in {cell}")
        }
        AgentCommand::PlaceArray {
            cell,
            child,
            columns,
            rows,
            ..
        } => format!("place_array {child} {columns}x{rows} in {cell}"),
        AgentCommand::TransformShapes { ids, .. } => {
            format!("transform_shapes {} id(s)", ids.len())
        }
        AgentCommand::DeleteShapes { ids } => format!("delete_shapes {} id(s)", ids.len()),
        AgentCommand::QueryShapes { cell, .. } => format!("query_shapes {cell}"),
        AgentCommand::GetCellInfo { cell } => format!("get_cell_info {cell}"),
        AgentCommand::ListLayers => "list_layers".to_owned(),
        AgentCommand::SetTechnology { .. } => "set_technology".to_owned(),
        AgentCommand::RunDrc { cell, .. } => format!("run_drc {cell}"),
        AgentCommand::GetViolations => "get_violations".to_owned(),
        AgentCommand::RouteNet { cell, net, .. } => format!("route_net {net} in {cell}"),
        AgentCommand::RunExtract { cell } => format!("run_extract {cell}"),
        AgentCommand::CheckIntent { cell, .. } => format!("check_intent {cell}"),
        AgentCommand::NetlistCompare { cell, .. } => format!("netlist_compare {cell}"),
        AgentCommand::ExportGds => "export_gds".to_owned(),
        AgentCommand::ExportOasis => "export_oasis".to_owned(),
        AgentCommand::ImportGds { bytes } => format!("import_gds {} byte(s)", bytes.len()),
        AgentCommand::RenderPng { width, height, .. } => format!("render_png {width}x{height}"),
        AgentCommand::SaveSession => "save_session".to_owned(),
        AgentCommand::LoadSession { .. } => "load_session".to_owned(),
        // `AgentCommand` is non-exhaustive; narrate unknown future ops neutrally.
        _ => "command".to_owned(),
    }
}

/// The status-line step label for a command (what the agent "is doing").
#[must_use]
pub fn step_label(cmd: &AgentCommand) -> &'static str {
    match cmd {
        AgentCommand::RunDrc { .. } => "verifying (drc)",
        AgentCommand::DeleteShapes { .. } | AgentCommand::TransformShapes { .. } => "correcting",
        AgentCommand::CreateCell { .. }
        | AgentCommand::AddRect { .. }
        | AgentCommand::AddPolygon { .. }
        | AgentCommand::AddPath { .. }
        | AgentCommand::PlaceInstance { .. }
        | AgentCommand::PlaceArray { .. }
        | AgentCommand::RouteNet { .. } => "proposing",
        AgentCommand::SetTechnology { .. }
        | AgentCommand::ImportGds { .. }
        | AgentCommand::LoadSession { .. } => "setting up",
        AgentCommand::QueryShapes { .. }
        | AgentCommand::GetCellInfo { .. }
        | AgentCommand::ListLayers
        | AgentCommand::GetViolations
        | AgentCommand::RunExtract { .. }
        | AgentCommand::CheckIntent { .. }
        | AgentCommand::NetlistCompare { .. } => "inspecting",
        _ => "working",
    }
}

/// Where the agent's cursor should sit while executing `cmd`, if the command
/// touches an identifiable spot in the layout.
#[must_use]
pub fn command_target(cmd: &AgentCommand) -> Option<Point> {
    match cmd {
        AgentCommand::AddRect { rect, .. } => Some(rect_center(rect)),
        AgentCommand::AddPolygon { points, .. } | AgentCommand::AddPath { points, .. } => {
            points.first().map(|p| Point::new(p.x, p.y))
        }
        AgentCommand::PlaceInstance { transform, .. }
        | AgentCommand::PlaceArray { transform, .. } => {
            Some(Point::new(transform.dx, transform.dy))
        }
        AgentCommand::RunDrc {
            region: Some(region),
            ..
        } => Some(rect_center(region)),
        AgentCommand::RouteNet { terminals, .. } => terminals.first().map(|p| Point::new(p.x, p.y)),
        _ => None,
    }
}

/// The center of a wire-format rectangle, in world DBU.
fn rect_center(rect: &RectArg) -> Point {
    Point::new(
        i32::midpoint(rect.min.x, rect.max.x),
        i32::midpoint(rect.min.y, rect.max.y),
    )
}

/// A terse note describing a record's outcome.
#[must_use]
pub fn outcome_note(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Ok(AgentResponse::Ok { revision, affected }) => {
            if affected.is_empty() {
                format!("ok rev {revision}")
            } else {
                format!("ok rev {revision}, {} id(s)", affected.len())
            }
        }
        Outcome::Ok(AgentResponse::Data { value, .. }) => value
            .get("count")
            .and_then(serde_json::Value::as_u64)
            .map_or_else(|| "data".to_owned(), |c| format!("data ({c} violation(s))")),
        Outcome::Ok(AgentResponse::Blob { bytes, .. }) => format!("blob {} byte(s)", bytes.len()),
        Outcome::Err(err) => format!("error {err}"),
        // `AgentResponse` is non-exhaustive; note unknown future shapes neutrally.
        Outcome::Ok(_) => "ok".to_owned(),
    }
}

// ----- parsing violations out of run_drc responses ----------------------------

/// Parses the violation list out of a `run_drc` response payload.
///
/// The payload shape is the one `reticle-agent-api` emits: an object with a
/// `violations` array whose items carry `rule`, `kind`, `layer`,
/// `other_layer`, `measured`, `required`, `location`, and `message`.
/// Malformed items are skipped rather than failing the whole list.
#[must_use]
pub fn violations_from_json(value: &serde_json::Value) -> Vec<Violation> {
    let Some(items) = value.get("violations").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items.iter().filter_map(violation_from_json).collect()
}

/// Parses one violation object; `None` if required fields are missing.
fn violation_from_json(item: &serde_json::Value) -> Option<Violation> {
    let rule = item.get("rule")?.as_str()?.to_owned();
    let kind = kind_from_str(item.get("kind").and_then(|k| k.as_str()).unwrap_or("width"));
    let layer = layer_from_json(item.get("layer")?)?;
    let other_layer = item.get("other_layer").and_then(layer_from_json);
    let measured = item.get("measured").and_then(serde_json::Value::as_i64)?;
    let required = item.get("required").and_then(serde_json::Value::as_i64)?;
    let location = rect_from_json(item.get("location")?)?;
    let message = item
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_owned();
    Some(Violation {
        rule,
        kind,
        layer,
        other_layer,
        measured,
        required,
        location,
        message,
    })
}

/// Maps the wire `kind` keyword back to a [`RuleKind`], defaulting to width.
fn kind_from_str(kind: &str) -> RuleKind {
    match kind {
        "spacing" => RuleKind::Spacing,
        "enclosure" => RuleKind::Enclosure,
        "extension" => RuleKind::Extension,
        "notch" => RuleKind::Notch,
        "area" => RuleKind::Area,
        "density" => RuleKind::Density,
        "angle" => RuleKind::Angle,
        _ => RuleKind::Width,
    }
}

/// Parses a `{ "layer": u16, "datatype": u16 }` object.
fn layer_from_json(value: &serde_json::Value) -> Option<LayerId> {
    let layer = value.get("layer")?.as_u64()?;
    let datatype = value.get("datatype")?.as_u64()?;
    Some(LayerId::new(layer as u16, datatype as u16))
}

/// Parses a `{ "min": {x, y}, "max": {x, y} }` rectangle.
fn rect_from_json(value: &serde_json::Value) -> Option<Rect> {
    Some(Rect::new(
        point_from_json(value.get("min")?)?,
        point_from_json(value.get("max")?)?,
    ))
}

/// Parses a `{ "x": Dbu, "y": Dbu }` point; `None` if either coordinate is
/// missing or outside the `i32` DBU range.
fn point_from_json(value: &serde_json::Value) -> Option<Point> {
    Some(Point::new(
        i32::try_from(value.get("x")?.as_i64()?).ok()?,
        i32::try_from(value.get("y")?.as_i64()?).ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_agent_api::verify_replay;

    /// Drains a running panel to completion with generous ticks.
    fn drain(panel: &mut AgentPanelState) -> Option<Vec<Violation>> {
        let mut last = None;
        for _ in 0..1_000 {
            if let Some(v) = panel.tick(10.0) {
                last = Some(v);
            }
            if !panel.is_running() {
                break;
            }
        }
        last
    }

    #[test]
    fn scripted_run_transcript_is_replayable() {
        let (transcript, feed) = scripted_run("draw me a clean wire");
        assert!(!transcript.records.is_empty());
        verify_replay(&transcript).expect("the scripted transcript must replay to its hash");
        // The prompt prelude plus one step per record.
        assert_eq!(feed.len(), transcript.records.len() + 1);
    }

    #[test]
    fn scripted_run_flags_then_fixes_a_violation() {
        let (_, feed) = scripted_run("thin wire demo");
        let drc_steps: Vec<&AgentStep> = feed.iter().filter(|s| s.violations.is_some()).collect();
        assert_eq!(drc_steps.len(), 2, "two verify steps in the script");
        let first = drc_steps[0].violations.as_ref().expect("first verify");
        let second = drc_steps[1].violations.as_ref().expect("second verify");
        assert!(
            !first.is_empty(),
            "the 60 DBU wire must violate the 100 DBU width rule"
        );
        assert!(second.is_empty(), "the corrected wire must be clean");
        // The parsed violations carry the width rule and a real location.
        assert!(first[0].rule.contains("width"));
        assert!(first[0].location.width() > 0 || first[0].location.height() > 0);
    }

    #[test]
    fn state_machine_runs_to_stopped() {
        let mut panel = AgentPanelState::new();
        assert_eq!(panel.state(), RunState::Idle);
        panel.prompt = "hello".to_owned();
        panel.start();
        assert_eq!(panel.state(), RunState::Running);
        let update = drain(&mut panel);
        assert_eq!(panel.state(), RunState::Stopped);
        // The last DRC update of the run is the clean one.
        assert_eq!(update.expect("a verify step fired").len(), 0);
        let status = panel.latest_status().expect("a status was emitted");
        assert!(!status.running, "the final step reports not-running");
        assert_eq!(status.violations, 0);
        let (done, total) = panel.progress();
        assert_eq!(done, total);
        assert!(
            panel
                .narration()
                .last()
                .expect("narration exists")
                .contains("run complete")
        );
    }

    #[test]
    fn stop_halts_mid_run_and_further_ticks_do_nothing() {
        let mut panel = AgentPanelState::new();
        panel.start();
        // Emit exactly the first step (the accumulator starts with one period
        // in credit), then stop.
        let _ = panel.tick(0.0);
        let (done_before, total) = panel.progress();
        assert!(done_before >= 1 && done_before < total);
        panel.stop();
        assert_eq!(panel.state(), RunState::Stopped);
        assert!(panel.tick(100.0).is_none());
        let (done_after, _) = panel.progress();
        assert_eq!(done_before, done_after, "no steps after stop");
        assert!(
            panel
                .narration()
                .last()
                .expect("narration exists")
                .contains("stopped by user")
        );
        // Stopping again is a no-op.
        panel.stop();
        assert_eq!(panel.state(), RunState::Stopped);
    }

    #[test]
    fn tick_paces_steps_by_period() {
        let mut panel = AgentPanelState::new();
        panel.set_step_period(1.0);
        panel.start();
        let _ = panel.tick(0.0); // credit from begin() emits step 1
        assert_eq!(panel.progress().0, 1);
        let _ = panel.tick(0.4);
        assert_eq!(panel.progress().0, 1, "0.4 s is under the 1 s period");
        let _ = panel.tick(0.7);
        assert_eq!(panel.progress().0, 2, "1.1 s accumulated emits one more");
    }

    #[test]
    fn feed_derives_iterations_and_violation_counts() {
        let (transcript, _) = scripted_run("iterations");
        let feed = feed_from_transcript(&transcript);
        // Iterations only advance after a verify: the first verify runs at
        // iteration 0, the second at iteration 1.
        let drc: Vec<&AgentStep> = feed
            .iter()
            .filter(|s| s.status.step == "verifying (drc)")
            .collect();
        assert_eq!(drc.len(), 2);
        assert_eq!(drc[0].status.iteration, 0);
        assert_eq!(drc[1].status.iteration, 1);
        assert!(drc[0].status.violations > 0);
        assert_eq!(drc[1].status.violations, 0);
        // Only the last step reports not-running.
        assert!(feed.iter().rev().skip(1).all(|s| s.status.running));
        assert!(!feed.last().expect("feed non-empty").status.running);
    }

    #[test]
    fn cursor_follows_the_geometry() {
        let mut panel = AgentPanelState::new();
        panel.prompt = "cursor".to_owned();
        panel.start();
        drain(&mut panel);
        let cursor = panel.cursor().expect("the script draws geometry");
        // The scripted geometry is scattered to the right of the demo design.
        assert!(cursor.x >= 23_000);
    }

    #[test]
    fn prompt_scatters_geometry_deterministically() {
        let (a1, _) = scripted_run("prompt a");
        let (a2, _) = scripted_run("prompt a");
        assert_eq!(
            a1.final_hash, a2.final_hash,
            "same prompt, same final document"
        );
    }

    #[test]
    fn narration_is_capped() {
        let mut panel = AgentPanelState::new();
        for i in 0..(MAX_NARRATION + 50) {
            panel.push_line(format!("line {i}"));
        }
        assert_eq!(panel.narration().len(), MAX_NARRATION);
        assert_eq!(panel.narration()[0], "line 50", "oldest lines dropped");
    }

    #[test]
    fn violations_parse_from_the_wire_shape() {
        // Mirrors the JSON `reticle-agent-api` emits for run_drc.
        let value = serde_json::json!({
            "count": 1,
            "violations": [{
                "rule": "min_width_4_0",
                "kind": "width",
                "layer": { "layer": 4, "datatype": 0 },
                "other_layer": null,
                "measured": 60,
                "required": 100,
                "location": { "min": { "x": 10, "y": 20 }, "max": { "x": 70, "y": 2020 } },
                "message": "feature 60 < min width 100"
            }, {
                "rule": "malformed (skipped)"
            }]
        });
        let parsed = violations_from_json(&value);
        assert_eq!(parsed.len(), 1, "the malformed item is skipped");
        let v = &parsed[0];
        assert_eq!(v.rule, "min_width_4_0");
        assert_eq!(v.kind, RuleKind::Width);
        assert_eq!(v.layer, LayerId::new(4, 0));
        assert_eq!(v.other_layer, None);
        assert_eq!(v.measured, 60);
        assert_eq!(v.required, 100);
        assert_eq!(
            v.location,
            Rect::new(Point::new(10, 20), Point::new(70, 2020))
        );
        assert!(v.message.contains("min width"));
    }

    #[test]
    fn scripted_run_attaches_a_plan_and_panel_holds_it() {
        // The scripted run carries a two-step plan (one per propose-verify iteration);
        // starting a run must surface it on the panel state.
        let (transcript, _) = scripted_run("plan me a wire");
        assert_eq!(transcript.plan.len(), 2, "two scripted iterations");
        let mut panel = AgentPanelState::new();
        panel.prompt = "plan me a wire".to_owned();
        panel.start();
        let plan = panel.plan();
        assert_eq!(plan.len(), 2, "panel holds the plan from the transcript");
        assert_eq!(plan[0].goal, "plan me a wire");
        assert!(plan[0].intended_tools.contains(&"run_drc".to_owned()));
        assert_eq!(plan[0].expected_checks, ["drc"]);
        // The correcting iteration lists the delete then the redraw.
        assert_eq!(
            plan[1].intended_tools,
            ["delete_shapes", "add_rect", "run_drc"]
        );
    }

    #[test]
    fn start_from_transcript_surfaces_its_plan() {
        let (transcript, _) = scripted_run("carry the plan");
        let mut panel = AgentPanelState::new();
        panel.start_from_transcript(&transcript);
        assert_eq!(panel.plan().len(), transcript.plan.len());
        assert_eq!(panel.plan(), transcript.plan.as_slice());
    }

    #[test]
    fn plan_is_empty_for_a_planless_transcript() {
        // A transcript with no plan (e.g. one recorded before the plan log existed)
        // leaves the panel's plan empty rather than panicking.
        let mut transcript = scripted_run("no plan").0;
        transcript.plan.clear();
        let mut panel = AgentPanelState::new();
        panel.start_from_transcript(&transcript);
        assert!(panel.plan().is_empty());
    }

    #[test]
    fn start_from_transcript_narrates_it() {
        let (transcript, _) = scripted_run("replay me");
        let mut panel = AgentPanelState::new();
        panel.start_from_transcript(&transcript);
        assert!(panel.is_running());
        drain(&mut panel);
        assert_eq!(panel.progress().0, transcript.records.len());
        assert!(panel.transcript().is_some());
    }

    #[test]
    fn start_opens_the_conversation_with_the_prompt() {
        let mut panel = AgentPanelState::new();
        panel.prompt = "draw a clean wire".to_owned();
        panel.start();
        // The prompt is the first (user) conversation turn.
        let convo = panel.conversation();
        assert_eq!(convo.len(), 1);
        assert_eq!(convo[0].speaker, Speaker::User);
        assert_eq!(convo[0].text, "draw a clean wire");
        assert!(panel.followups().is_empty());
    }

    #[test]
    fn submit_followup_appends_a_turn_and_records_the_instruction() {
        let mut panel = AgentPanelState::new();
        panel.prompt = "start".to_owned();
        panel.start();
        assert!(panel.is_running());
        panel.followup = "  keep it on met1  ".to_owned();
        let sent = panel.submit_followup().expect("accepted while running");
        // Trimmed instruction is returned and the input box is cleared.
        assert_eq!(sent, "keep it on met1");
        assert!(panel.followup.is_empty());
        // The follow-up is recorded on the Wave-3 seam.
        assert_eq!(panel.followups(), ["keep it on met1"]);
        // Conversation now has: prompt (user), follow-up (user), ack (agent).
        let convo = panel.conversation();
        assert_eq!(convo.len(), 3);
        assert_eq!(convo[1].speaker, Speaker::User);
        assert_eq!(convo[1].text, "keep it on met1");
        assert_eq!(convo[2].speaker, Speaker::Agent);
        assert!(convo[2].text.contains("keep it on met1"));
    }

    #[test]
    fn submit_followup_rejects_empty_and_when_not_running() {
        let mut panel = AgentPanelState::new();
        // Not running: rejected, nothing recorded.
        panel.followup = "too early".to_owned();
        assert!(panel.submit_followup().is_none());
        assert!(panel.followups().is_empty());
        assert!(panel.conversation().is_empty());
        // Running but blank: rejected, input untouched-but-blank.
        panel.prompt = "go".to_owned();
        panel.start();
        panel.followup = "   ".to_owned();
        assert!(panel.submit_followup().is_none());
        assert!(panel.followups().is_empty());
    }

    #[test]
    fn note_agent_and_clear_conversation() {
        let mut panel = AgentPanelState::new();
        panel.prompt = "go".to_owned();
        panel.start();
        panel.note_agent("verified: DRC clean");
        assert_eq!(panel.conversation().len(), 2);
        assert_eq!(panel.conversation()[1].speaker, Speaker::Agent);
        // Clearing empties the conversation, the follow-up list, and the input.
        panel.followup = "draft".to_owned();
        panel.clear_conversation();
        assert!(panel.conversation().is_empty());
        assert!(panel.followups().is_empty());
        assert!(panel.followup.is_empty());
    }

    #[test]
    fn starting_a_new_run_resets_the_conversation() {
        let mut panel = AgentPanelState::new();
        panel.prompt = "first".to_owned();
        panel.start();
        panel.followup = "note".to_owned();
        panel.submit_followup().expect("running");
        assert!(panel.conversation().len() >= 2);
        // A fresh start clears the prior conversation and follow-ups.
        panel.prompt = "second".to_owned();
        panel.start();
        assert_eq!(panel.conversation().len(), 1);
        assert_eq!(panel.conversation()[0].text, "second");
        assert!(panel.followups().is_empty());
    }

    #[test]
    fn conversation_is_capped() {
        let mut panel = AgentPanelState::new();
        for i in 0..(MAX_CONVERSATION + 25) {
            panel.push_conversation(ConversationEntry::agent(format!("line {i}")));
        }
        assert_eq!(panel.conversation().len(), MAX_CONVERSATION);
        assert_eq!(panel.conversation()[0].text, "line 25", "oldest dropped");
    }

    #[test]
    fn describe_and_label_cover_the_script_commands() {
        let cmd = AgentCommand::AddRect {
            cell: "TOP".to_owned(),
            layer: LayerArg {
                layer: 4,
                datatype: 0,
            },
            rect: RectArg {
                min: PointArg { x: 0, y: 0 },
                max: PointArg { x: 10, y: 20 },
            },
        };
        assert_eq!(describe_command(&cmd), "add_rect TOP l4/0 (0,0)-(10,20)");
        assert_eq!(step_label(&cmd), "proposing");
        assert_eq!(command_target(&cmd), Some(Point::new(5, 10)));
        let drc = AgentCommand::RunDrc {
            cell: "TOP".to_owned(),
            region: None,
        };
        assert_eq!(step_label(&drc), "verifying (drc)");
        assert_eq!(command_target(&drc), None);
    }
}
