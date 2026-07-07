//! The scripted-vector model and the executor that drives one vector against a
//! [`Target`].
//!
//! A [`Vector`] is a named list of [`Action`]s: connect a client (edit or view),
//! send a frame, assert a client receives a specific frame (or nothing), burst
//! presence, disconnect. [`run_vector`] executes it against a target using
//! `tokio_tungstenite` clients, returning `Ok(())` if every assertion held or a
//! [`Failure`] naming the first step that did not. The same vector runs against
//! the native relay and the Durable Object; the only target-aware branch is
//! presence coalescing (see [`Action::PresenceBurst`]).

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use crate::frames::{presence_frame, presence_seq, update_frame, update_marker};
use crate::target::Target;

/// Spacing between the presence frames of a burst, so a coalescing relay has a
/// window to collapse them (50 frames at 20 ms spans the "~1 s" the contract
/// specifies).
const PRESENCE_BURST_SPACING: Duration = Duration::from_millis(20);

/// The most presence frames a coalescing relay may deliver from a burst before
/// the suite considers it uncoalesced. Generous against the contract's "~10".
const COALESCE_MAX: usize = 15;

/// A connected client socket.
type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A live map of connected clients keyed by their vector name.
type Clients = HashMap<&'static str, Client>;

/// How a client joins its room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Full participant: receives and publishes.
    Edit,
    /// Read-only viewer (`?mode=view`): receives, but its frames are dropped.
    View,
}

/// The frame a [`Action::Send`] puts on the wire.
#[derive(Clone, Debug)]
pub enum Payload {
    /// An update frame (first byte `0x0A`) carrying `marker`.
    Update(&'static str),
    /// A presence frame (first byte `0x12`) for `actor` with `seq` in `cursor.x`.
    Presence {
        /// The actor the presence belongs to.
        actor: &'static str,
        /// The sequence marker carried in `cursor.x`.
        seq: i32,
    },
    /// A WebSocket text frame, which the relay must ignore (binary-only payloads).
    Text(&'static str),
}

/// One step in a [`Vector`].
///
/// Clients and rooms are named by `&'static str` so a vector reads as a script.
#[derive(Clone, Debug)]
pub enum Action {
    /// Connect `who` to `room` in `mode`, then wait the target's connect grace.
    Connect {
        /// Client name.
        who: &'static str,
        /// Room to join.
        room: &'static str,
        /// Join mode.
        mode: Mode,
    },
    /// Sleep one connect-grace period (lets the relay record prior sends before a
    /// late join reads the room log).
    Grace,
    /// `who` sends `payload`.
    Send {
        /// Client name.
        who: &'static str,
        /// The frame to send.
        payload: Payload,
    },
    /// `who` sends `count` update frames with markers `"0".."count-1"`, in order,
    /// as fast as the socket accepts them (to prove updates are never coalesced).
    SendUpdates {
        /// Client name.
        who: &'static str,
        /// Number of update frames to send.
        count: i32,
    },
    /// Assert `who` receives an update frame with `marker` next, in order.
    ExpectUpdate {
        /// Client name.
        who: &'static str,
        /// Expected update marker.
        marker: &'static str,
    },
    /// Assert `who` receives update frames `"0".."count-1"` next, in that exact
    /// order (order preservation and full-log replay, no drops).
    ExpectUpdates {
        /// Client name.
        who: &'static str,
        /// Number of update frames to expect.
        count: i32,
    },
    /// Assert `who` receives a presence frame with `seq` next, in order.
    ExpectPresence {
        /// Client name.
        who: &'static str,
        /// Expected presence sequence.
        seq: i32,
    },
    /// Assert `who` receives nothing within the target's negative timeout.
    ExpectSilence {
        /// Client name.
        who: &'static str,
    },
    /// `who` bursts presence frames `0..count` (spaced), then assert `observer`
    /// receives a coalesced-or-full run whose newest seq is `count - 1`.
    PresenceBurst {
        /// The bursting client.
        who: &'static str,
        /// The client observing the burst.
        observer: &'static str,
        /// Number of presence frames to send.
        count: i32,
    },
    /// Disconnect `who` with a clean close.
    Disconnect {
        /// Client name.
        who: &'static str,
    },
}

/// A named sequence of [`Action`]s run identically against each target.
#[derive(Clone, Debug)]
pub struct Vector {
    /// A short, unique name used in failure messages and reports.
    pub name: &'static str,
    /// What the vector proves, one line, for the report.
    pub covers: &'static str,
    /// The scripted steps.
    pub actions: Vec<Action>,
}

/// The first assertion in a vector that did not hold.
#[derive(Clone, Debug)]
pub struct Failure {
    /// The vector that failed.
    pub vector: String,
    /// The zero-based index of the failing action.
    pub step: usize,
    /// A human-readable description of the mismatch.
    pub detail: String,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "vector `{}` failed at step {}: {}",
            self.vector, self.step, self.detail
        )
    }
}

impl std::error::Error for Failure {}

/// Executes `vector` against `target`, returning `Ok(())` if every assertion
/// held or the first [`Failure`].
///
/// # Errors
///
/// Returns a [`Failure`] describing the first step whose observation did not
/// match the vector's expectation (a missing frame, a wrong marker, an
/// unexpected frame, or a coalescing-bound violation).
pub async fn run_vector(target: &Target, vector: &Vector) -> Result<(), Failure> {
    let mut clients: Clients = HashMap::new();
    for (step, action) in vector.actions.iter().enumerate() {
        run_action(target, &mut clients, action)
            .await
            .map_err(|detail| Failure {
                vector: vector.name.to_owned(),
                step,
                detail,
            })?;
    }
    for (_name, mut socket) in clients {
        let _ = socket.close(None).await;
    }
    Ok(())
}

/// Runs one action, returning a human-readable detail string on the first
/// mismatch (the caller attaches the vector name and step index).
async fn run_action(target: &Target, clients: &mut Clients, action: &Action) -> Result<(), String> {
    match action {
        Action::Connect { who, room, mode } => {
            let url = target.join_url(room, *mode == Mode::View);
            let (socket, _resp) = connect_async(&url)
                .await
                .map_err(|e| format!("connect {who} to {url}: {e}"))?;
            clients.insert(*who, socket);
            tokio::time::sleep(target.connect_grace()).await;
        }
        Action::Grace => tokio::time::sleep(target.connect_grace()).await,
        Action::Send { who, payload } => {
            let message = match payload {
                Payload::Update(marker) => Message::Binary(update_frame(marker).into()),
                Payload::Presence { actor, seq } => {
                    Message::Binary(presence_frame(actor, *seq).into())
                }
                Payload::Text(text) => Message::Text((*text).into()),
            };
            socket_mut(clients, who)?
                .send(message)
                .await
                .map_err(|e| format!("send from {who}: {e}"))?;
        }
        Action::SendUpdates { who, count } => {
            let socket = socket_mut(clients, who)?;
            for i in 0..*count {
                socket
                    .send(Message::Binary(update_frame(&i.to_string()).into()))
                    .await
                    .map_err(|e| format!("send update {i} from {who}: {e}"))?;
            }
        }
        Action::ExpectUpdate { who, marker } => {
            expect_update(socket_mut(clients, who)?, target.recv_timeout(), marker)
                .await
                .map_err(|e| format!("{who}: {e}"))?;
        }
        Action::ExpectUpdates { who, count } => {
            expect_updates(socket_mut(clients, who)?, target.recv_timeout(), *count)
                .await
                .map_err(|e| format!("{who}: {e}"))?;
        }
        Action::ExpectPresence { who, seq } => {
            expect_presence(socket_mut(clients, who)?, target.recv_timeout(), *seq)
                .await
                .map_err(|e| format!("{who}: {e}"))?;
        }
        Action::ExpectSilence { who } => {
            expect_silence(socket_mut(clients, who)?, target.negative_timeout())
                .await
                .map_err(|e| format!("{who}: {e}"))?;
        }
        Action::PresenceBurst {
            who,
            observer,
            count,
        } => run_burst(target, clients, who, observer, *count).await?,
        Action::Disconnect { who } => {
            if let Some(mut socket) = clients.remove(who) {
                let _ = socket.close(None).await;
            }
        }
    }
    Ok(())
}

/// Looks up a connected client by name.
fn socket_mut<'a>(clients: &'a mut Clients, who: &str) -> Result<&'a mut Client, String> {
    clients
        .get_mut(who)
        .ok_or_else(|| format!("unknown client {who}"))
}

/// Asserts the next binary frame on `socket` is an update carrying `marker`.
async fn expect_update(socket: &mut Client, dur: Duration, marker: &str) -> Result<(), String> {
    match recv_binary(socket, dur).await {
        Some(bytes) => {
            let got = update_marker(&bytes);
            if got.as_deref() == Some(marker) {
                Ok(())
            } else {
                Err(format!(
                    "expected update `{marker}`, got {got:?} (first byte {:?})",
                    bytes.first()
                ))
            }
        }
        None => Err(format!("expected update `{marker}`, received nothing")),
    }
}

/// Asserts the next `count` binary frames are updates `"0".."count-1"`, in order.
async fn expect_updates(socket: &mut Client, dur: Duration, count: i32) -> Result<(), String> {
    for i in 0..count {
        let want = i.to_string();
        expect_update(socket, dur, &want)
            .await
            .map_err(|e| format!("{e} (frame {i} of {count})"))?;
    }
    Ok(())
}

/// Asserts the next binary frame is a presence with sequence `seq`.
async fn expect_presence(socket: &mut Client, dur: Duration, seq: i32) -> Result<(), String> {
    match recv_binary(socket, dur).await {
        Some(bytes) => {
            let got = presence_seq(&bytes);
            if got == Some(seq) {
                Ok(())
            } else {
                Err(format!("expected presence seq {seq}, got {got:?}"))
            }
        }
        None => Err(format!("expected presence seq {seq}, received nothing")),
    }
}

/// Asserts no binary frame arrives within `dur`.
async fn expect_silence(socket: &mut Client, dur: Duration) -> Result<(), String> {
    if let Some(bytes) = recv_binary(socket, dur).await {
        return Err(format!(
            "expected silence, received a frame (first byte {:?})",
            bytes.first()
        ));
    }
    Ok(())
}

/// Sends a spaced presence burst on `who`, drains `observer`, and checks the
/// collected sequence numbers against the target's coalescing policy.
async fn run_burst(
    target: &Target,
    clients: &mut Clients,
    who: &str,
    observer: &str,
    count: i32,
) -> Result<(), String> {
    for seq in 0..count {
        socket_mut(clients, who)?
            .send(Message::Binary(presence_frame(who, seq).into()))
            .await
            .map_err(|e| format!("burst send from {who}: {e}"))?;
        tokio::time::sleep(PRESENCE_BURST_SPACING).await;
    }

    let socket = socket_mut(clients, observer)?;
    let mut seqs = Vec::new();
    while let Some(bytes) = recv_binary(socket, target.negative_timeout()).await {
        match presence_seq(&bytes) {
            Some(seq) => seqs.push(seq),
            None => {
                return Err(format!(
                    "{observer} received a non-presence frame during a burst (first byte {:?})",
                    bytes.first()
                ));
            }
        }
    }
    verify_burst(target, count, &seqs)
}

/// Checks the presence sequence numbers an observer collected from a burst
/// against the shared convergence invariant and the target's coalescing policy.
fn verify_burst(target: &Target, count: i32, seqs: &[i32]) -> Result<(), String> {
    if seqs.is_empty() {
        return Err(format!("burst of {count} presence frames delivered none"));
    }
    // Order preservation: sequence numbers are strictly increasing.
    if seqs.windows(2).any(|w| w[1] <= w[0]) {
        return Err(format!("burst delivered out of order: {seqs:?}"));
    }
    // Convergence: the newest presence always arrives last (last-write-wins).
    let newest = count - 1;
    if *seqs.last().expect("non-empty") != newest {
        return Err(format!(
            "burst newest seq should be {newest}, last delivered was {:?}",
            seqs.last()
        ));
    }
    let received = seqs.len();
    if target.coalesces_presence() {
        // The Durable Object collapses all-but-newest per window.
        if received >= count as usize {
            return Err(format!(
                "coalescing relay delivered {received} of {count} presence frames (expected fewer)"
            ));
        }
        if received > COALESCE_MAX {
            return Err(format!(
                "coalescing relay delivered {received} presence frames, over the {COALESCE_MAX} bound"
            ));
        }
    } else if received != count as usize {
        // The native relay forwards every presence frame.
        return Err(format!(
            "non-coalescing relay delivered {received} of {count} presence frames (expected all)"
        ));
    }
    Ok(())
}

/// Awaits the next binary payload on `client`, ignoring text/ping/pong, or
/// `None` on timeout or a closed socket. Mirrors `share_live.rs`.
async fn recv_binary(client: &mut Client, dur: Duration) -> Option<Vec<u8>> {
    let deadline = Instant::now() + dur;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match timeout(remaining, client.next()).await {
            Ok(Some(Ok(Message::Binary(bytes)))) => return Some(bytes.to_vec()),
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(_)) | None) | Err(_) => return None,
        }
    }
}
