//! Running the agent as a live collaborator in a real relay room.
//!
//! [`AgentCollaborator`] mirrors the agent's edits onto a
//! [`SyncDocument`](reticle_sync::SyncDocument) under
//! [`AGENT_ACTOR`]; this module carries that mirror over a real WebSocket to a
//! `reticle-server` room, so the agent edits *beside* browser humans instead of into
//! a private document. [`run_in_room`] is a native `tokio` client:
//!
//! * It connects to `ws://host/ws/{room}` and, for each scripted step, applies the
//!   batch through the collaborator (one atomic [`SyncDocument::step`](reticle_sync::SyncDocument::step))
//!   and ships the resulting `yrs` delta as **one** binary frame, wrapped in the frozen
//!   `SyncMessage` envelope by [`encode_update_frame_for`]. One agent step is one frame,
//!   so a watcher never sees a half-applied step.
//! * After each step it publishes the agent's presence (cursor at the last placement,
//!   the collaborator's amber color, its display name) as another binary frame via
//!   [`encode_presence_frame`], so a human sees where the agent is working.
//! * It applies inbound frames a peer publishes back into its own
//!   [`SyncDocument`](reticle_sync::SyncDocument), so a concurrent human edit reaches the
//!   agent's document. The relay never inspects these
//!   frames; they are the exact bytes the browser transport exchanges.
//!
//! The whole thing is deterministic given a fixed `steps` script: no model, no key, no
//! network beyond the relay socket. That is what the in-process integration test and the
//! committed demo transcript both rely on.
//!
//! This module is native-only (the `tokio` / `tokio-tungstenite` stack does not build
//! for `wasm32`); it is compiled out under `#[cfg(target_arch = "wasm32")]` at the crate
//! root.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use reticle_agent_api::{AGENT_ACTOR, AgentCommand};
use reticle_sync::{Frame, decode_frame, encode_presence_frame, encode_update_frame_for};
use tokio::time::Instant;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::collab::AgentCollaborator;

/// Why a live-room run could not proceed.
///
/// A run that merely receives no peer frames is *not* an error: it returns the
/// collaborator normally. These variants are transport failures.
#[derive(Debug)]
pub enum LiveError {
    /// The WebSocket connection to the relay could not be established.
    Connect {
        /// The relay URL that was tried.
        url: String,
        /// The underlying error.
        detail: String,
    },
    /// A frame could not be sent to the relay.
    Send {
        /// The underlying error.
        detail: String,
    },
}

impl std::fmt::Display for LiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LiveError::Connect { url, detail } => {
                write!(f, "connecting to relay {url}: {detail}")
            }
            LiveError::Send { detail } => write!(f, "sending a frame to the relay: {detail}"),
        }
    }
}

impl std::error::Error for LiveError {}

/// How a live-room run frames and paces itself.
#[derive(Clone, Debug)]
pub struct LiveConfig {
    /// The `doc_id` stamped onto each published `CrdtUpdate` frame. The relay ignores it
    /// (rooms are keyed by URL) and a viewer applies the raw bytes regardless, so it is
    /// informational; the room name is a natural choice.
    pub doc_id: String,
    /// Whether to publish a presence frame after each step. On by default so a human sees
    /// the agent's cursor and selection.
    pub publish_presence: bool,
    /// How long to wait after connecting, before the first step is published, so the
    /// relay has subscribed this client to the room (and its log replay has arrived)
    /// before it starts sending. Zero by default; a test against a freshly-bound relay
    /// sets a small grace so the exchange is deterministic.
    pub startup_grace: Duration,
    /// How long to keep reading inbound frames after the last scripted step, so a
    /// concurrent peer edit that arrives late still reaches the agent's document.
    pub linger: Duration,
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self {
            doc_id: String::new(),
            publish_presence: true,
            startup_grace: Duration::ZERO,
            linger: Duration::from_millis(500),
        }
    }
}

/// Connects `collab` to the relay room at `ws_url`, drives each batch in `steps` as one
/// atomic mirrored step published to the room, then lingers to absorb inbound peer
/// frames. Returns the collaborator so a caller can inspect the document a concurrent
/// peer's edits reached.
///
/// Each step ships two binary frames: the step's `yrs` delta (as a `SyncMessage` update)
/// and, when [`LiveConfig::publish_presence`] is set, the agent's presence. Inbound
/// frames are
/// decoded with [`decode_frame`] and applied to the collaborator's own
/// [`SyncDocument`](reticle_sync::SyncDocument) (updates) or its awareness map
/// (presence). The scripted-batch shape is exactly the deterministic mock path the bench
/// harness drives, so a run is reproducible with no model in the loop.
///
/// # Errors
///
/// Returns [`LiveError::Connect`] if the relay socket cannot be opened, or
/// [`LiveError::Send`] if a frame cannot be shipped.
pub async fn run_in_room(
    mut collab: AgentCollaborator,
    ws_url: &str,
    steps: Vec<Vec<AgentCommand>>,
    config: &LiveConfig,
) -> Result<AgentCollaborator, LiveError> {
    let (mut socket, _response) = connect_async(ws_url)
        .await
        .map_err(|e| LiveError::Connect {
            url: ws_url.to_owned(),
            detail: e.to_string(),
        })?;

    // Let the relay subscribe this client (and replay any room log) before publishing.
    if !config.startup_grace.is_zero() {
        tokio::time::sleep(config.startup_grace).await;
    }

    // The state vector before each step, so the frame carries exactly that step's delta
    // (one atomic step = one frame).
    let mut before = collab.sync().state_vector();
    for step in &steps {
        collab.apply_step(step);

        if let Ok(delta) = collab.sync().encode_update(&before) {
            let frame = encode_update_frame_for(&config.doc_id, AGENT_ACTOR, &delta);
            send(&mut socket, frame).await?;
        }
        before = collab.sync().state_vector();

        if config.publish_presence
            && let Some(presence) = collab.sync().awareness().get(AGENT_ACTOR)
        {
            let frame = encode_presence_frame(presence);
            send(&mut socket, frame).await?;
        }
    }

    // Linger: apply whatever a concurrent peer published back to us until the window
    // closes or the socket ends.
    let deadline = Instant::now() + config.linger;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, socket.next()).await {
            Ok(Some(Ok(Message::Binary(bytes)))) => apply_inbound(&mut collab, &bytes),
            // A ping/pong/text frame is not our concern; keep listening.
            Ok(Some(Ok(_))) => {}
            // The socket closed or errored, or the linger window elapsed: stop.
            Ok(Some(Err(_)) | None) | Err(_) => break,
        }
    }

    Ok(collab)
}

// ----- the deterministic DRC-fix demo script --------------------------------

/// The technology the DRC-fix demo installs: met1 (SKY130 layer 68, datatype 20) with
/// the `m1.2` minimum-spacing rule at its cited SKY130-subset value of 140 DBU.
///
/// Kept as a small inline technology (not the whole SKY130 deck) so the demo's only
/// violation is the one it is about; `RunDrc` reads the session's installed rules, so
/// installing this is what makes the seeded rects flag.
pub const DRC_FIX_TECH: &str = "technology sky130-subset\n\
                                dbu_per_micron 1000\n\
                                layer 68 20 met1 3A6FD4FF\n\
                                rule spacing 68 20 140\n";

/// The cell the demo draws into.
pub const DRC_FIX_CELL: &str = "top";

/// The scripted, model-free command batches of the DRC-fix demo, one batch per agent
/// step.
///
/// The script is the whole "loop": install the technology and seed two met1 rectangles
/// 100 DBU apart (closer than the 140 DBU `m1.2` spacing), verify (the rule flags the
/// pair), transform the **second** rectangle (its stable `ElementId` is `2`, the second
/// element created) right by 100 DBU to a legal 200 DBU gap, then verify again (clean).
/// This is a deterministic harness driving fixed commands, **not** a live model: there is
/// no key, no network, and the same batches every run.
#[must_use]
pub fn scripted_drc_fix_steps() -> Vec<Vec<AgentCommand>> {
    use reticle_agent_api::ElementId;
    use reticle_agent_api::args::{LayerArg, PointArg, RectArg, TransformArg};

    let met1 = LayerArg {
        layer: 68,
        datatype: 20,
    };
    let rect = |x0: i32, y0: i32, x1: i32, y1: i32| RectArg {
        min: PointArg { x: x0, y: y0 },
        max: PointArg { x: x1, y: y1 },
    };
    let cell = || DRC_FIX_CELL.to_owned();

    vec![
        // Step 1, seed: two met1 rects with a 100 DBU gap (300 - 200), under the min.
        vec![
            AgentCommand::SetTechnology {
                source: DRC_FIX_TECH.to_owned(),
            },
            AgentCommand::CreateCell { name: cell() },
            AgentCommand::AddRect {
                cell: cell(),
                layer: met1,
                rect: rect(0, 0, 200, 200),
            },
            AgentCommand::AddRect {
                cell: cell(),
                layer: met1,
                rect: rect(300, 0, 500, 200),
            },
        ],
        // Step 2, verify: the spacing rule flags the pair.
        vec![AgentCommand::RunDrc {
            cell: cell(),
            region: None,
        }],
        // Step 3, correct: move the second rect right by 100 DBU (gap becomes 200 DBU).
        vec![AgentCommand::TransformShapes {
            ids: vec![ElementId(2)],
            transform: TransformArg {
                dx: 100,
                dy: 0,
                ..TransformArg::default()
            },
        }],
        // Step 4, verify again: clean.
        vec![AgentCommand::RunDrc {
            cell: cell(),
            region: None,
        }],
    ]
}

/// Drives the [`scripted_drc_fix_steps`] demo through a fresh [`AgentCollaborator`] and
/// returns the resulting replay-verifiable [`Transcript`](reticle_agent_api::Transcript)
/// with its wall-clock timestamps zeroed.
///
/// No relay is involved: the transcript is the collaborator's authoritative internal
/// session, so recording it here is equivalent to recording it during a live-room run
/// (the room changes who *sees* the edits, not the command history). Timestamps are
/// zeroed so the committed artifact is byte-deterministic; [`replay`](reticle_agent_api::replay)
/// reads only the commands and the final hash, never the timestamps. A short
/// [`PlanStep`](reticle_agent_api::PlanStep) narration is attached, labeled as a scripted
/// (not live-model) run.
#[must_use]
pub fn scripted_drc_fix_transcript() -> reticle_agent_api::Transcript {
    use reticle_agent_api::{PlanStep, transcript_of};

    let mut collab = AgentCollaborator::new(crate::Pacing::Instant);
    for step in scripted_drc_fix_steps() {
        collab.apply_step(&step);
    }
    let mut transcript = transcript_of(collab.session());
    for record in &mut transcript.records {
        record.ts_start_ms = 0;
        record.ts_end_ms = 0;
    }
    transcript.plan = vec![
        PlanStep {
            goal: "scripted (no live model): seed two met1 rects and verify spacing".to_owned(),
            intended_tools: vec![
                "set_technology".to_owned(),
                "create_cell".to_owned(),
                "add_rect".to_owned(),
                "run_drc".to_owned(),
            ],
            expected_checks: vec!["drc".to_owned()],
        },
        PlanStep {
            goal: "scripted (no live model): move the second rect to a legal gap and re-verify"
                .to_owned(),
            intended_tools: vec!["transform_shapes".to_owned(), "run_drc".to_owned()],
            expected_checks: vec!["drc".to_owned()],
        },
    ];
    transcript
}

/// Serializes the [`scripted_drc_fix_transcript`] to the replay-theater JSONL text:
/// one [`CommandRecord`](reticle_agent_api::CommandRecord) per line, then a trailer line
/// carrying the `final_hash` a faithful replay reproduces plus honest provenance.
///
/// The trailer is a JSON object; the replay-theater loader reads `final_hash` from it and
/// ignores the extra `plan` / `notice` fields, so the file stays a valid, replayable
/// transcript while carrying its own provenance. This is the exact byte content committed
/// at `examples/collab/agent_drc_fix.transcript.jsonl`.
///
/// # Panics
///
/// Panics only if a `CommandRecord` fails to serialize, which cannot happen for these
/// commands (serialization of the frozen types is infallible in practice).
#[must_use]
pub fn scripted_drc_fix_jsonl() -> String {
    let transcript = scripted_drc_fix_transcript();
    let mut out = String::new();
    for record in &transcript.records {
        out.push_str(&serde_json::to_string(record).expect("a command record serializes"));
        out.push('\n');
    }
    let trailer = serde_json::json!({
        "final_hash": transcript.final_hash,
        "plan": transcript.plan,
        "notice": "Deterministic scripted run of the reticle-agent DRC-fix demo in a live \
                   relay room. No live model or API key was involved: the commands are the \
                   fixed script in reticle_agent::live::scripted_drc_fix_steps. Regenerate \
                   with `cargo run -p reticle-agent --example agent_live_room -- --emit \
                   examples/collab/agent_drc_fix.transcript.jsonl`.",
    });
    out.push_str(&serde_json::to_string(&trailer).expect("the trailer serializes"));
    out.push('\n');
    out
}

/// Sends one binary frame to the relay, mapping a socket error to [`LiveError::Send`].
async fn send(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    frame: Vec<u8>,
) -> Result<(), LiveError> {
    socket
        .send(Message::Binary(frame.into()))
        .await
        .map_err(|e| LiveError::Send {
            detail: e.to_string(),
        })
}

/// Routes one inbound binary frame into the collaborator's document (a CRDT update) or
/// awareness map (a peer's presence); anything else is ignored.
fn apply_inbound(collab: &mut AgentCollaborator, bytes: &[u8]) {
    match decode_frame(bytes) {
        Ok(Frame::Update(raw)) => {
            // A peer's delta; converge our document onto it. A malformed update is
            // dropped rather than aborting the run.
            let _ = collab.sync_mut().apply_update(&raw);
        }
        Ok(Frame::Presence(presence)) => {
            collab.sync_mut().awareness_mut().set(presence);
        }
        // A comment frame, or a frame that does not decode, is not routed here.
        _ => {}
    }
}
