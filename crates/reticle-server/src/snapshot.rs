//! Immutable snapshot permalinks: freezing a room's history so a link opened later
//! always shows the same geometry, no matter how much further the live document
//! changes.
//!
//! # Design: no new mutable state
//!
//! A room's update log (see the crate docs) is already append-only: nothing
//! already recorded in it is ever rewritten or removed, and a joiner already
//! catches up by replaying it from the start. That is already exactly what an
//! immutable snapshot needs, so a snapshot here is nothing but `(room, revision)`,
//! where `revision` is a prefix length into that same log:
//!
//! * "Capturing" a snapshot is asking the relay for the room's *current* revision
//!   (`GET /snapshot/{room}`, [`revision_handler`]: a WebSocket upgrade that sends
//!   one binary frame carrying the decimal revision and closes) and remembering
//!   `(room, revision)` as the permalink. Nothing is written on the relay to do
//!   this; see [`RelayState::current_revision`].
//! * Opening the permalink later dials `GET /snapshot/{room}/at/{revision}`
//!   ([`snapshot_ws_handler`]), a WebSocket upgrade that replays exactly
//!   `log[..revision]` as binary frames, in order (the same framing a live
//!   joiner's catch-up replay already uses), then closes. It never subscribes to
//!   the room's live broadcast and never reads an inbound frame from the client,
//!   so a snapshot connection can neither see a later edit nor make one: it is a
//!   one-shot, read-only download, not a live join.
//!
//! Multiple concurrent editors can share one room (ADR 0081), so a client cannot
//! infer "the current revision" from its own publish count; it must come from the
//! relay, the one party that observes every peer's frames in the single order it
//! accepted them.
//!
//! An earlier design considered minting an opaque id and copying the log into a
//! side registry at capture time. That would add a second mutable map (with its
//! own lifetime story) to hold exactly the data the room's log already keeps
//! forever, for no behavioral gain: `(room, revision)` is already globally stable
//! (a revision's meaning never changes, since the log only grows by appending) and
//! trivially shareable as two path segments. The relay stays true to its "no
//! editing logic of its own" design (see the crate docs): it does not need to know
//! what a snapshot means, only that a prefix of an append-only log is permanent.

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::response::Response;

use crate::RelayState;

impl RelayState {
    /// The number of frames `room`'s log has accepted so far: the revision a
    /// snapshot captured right now would pin.
    ///
    /// A room nobody has joined yet has revision `0`. Reading the revision never
    /// creates the room entry as a side effect (unlike joining), so checking a
    /// snapshot can never itself start a room.
    #[must_use]
    pub fn current_revision(&self, room: &str) -> usize {
        let rooms = self.rooms.lock().expect("room registry mutex poisoned");
        rooms.get(room).map_or(0, |r| r.log.len())
    }

    /// The first `revision` frames of `room`'s log: exactly what a joiner would
    /// have been replayed had it caught up at that point in the room's history.
    ///
    /// `revision` beyond the log's current length clamps to the whole log rather
    /// than failing (a link opened while its own capture request is still in
    /// flight, or one that simply names a revision that has not happened yet,
    /// gets the most complete prefix available rather than an error). A room
    /// nobody has joined yet yields an empty log. Cloning the [`Bytes`] entries is
    /// a cheap refcount bump, not a data copy.
    fn log_prefix(&self, room: &str, revision: usize) -> Vec<Bytes> {
        let rooms = self.rooms.lock().expect("room registry mutex poisoned");
        rooms.get(room).map_or_else(Vec::new, |r| {
            let end = revision.min(r.log.len());
            r.log[..end].to_vec()
        })
    }
}

/// axum handler for `GET /snapshot/{room}`: upgrades to a WebSocket, sends one
/// binary frame carrying `room`'s current revision as an ASCII decimal integer,
/// then closes.
///
/// This is the "capture" step from a client's perspective (see the module docs):
/// mint a permalink by reading this once and remembering `(room, revision)`.
pub(crate) async fn revision_handler(
    ws: WebSocketUpgrade,
    Path(room): Path<String>,
    State(state): State<RelayState>,
) -> Response {
    ws.on_upgrade(move |socket| send_current_revision(socket, room, state))
}

/// Sends `room`'s current revision as one binary frame and closes.
async fn send_current_revision(mut socket: WebSocket, room: String, state: RelayState) {
    let revision = state.current_revision(&room);
    if socket
        .send(Message::Binary(Bytes::from(revision.to_string())))
        .await
        .is_err()
    {
        return;
    }
    let _ = socket.send(Message::Close(None)).await;
}

/// axum handler for `GET /snapshot/{room}/at/{revision}`: upgrades to a WebSocket
/// and replays exactly `room`'s log up to `revision`, then closes.
///
/// A malformed (non-numeric) `revision` path segment is rejected by axum's `Path`
/// extractor before this handler ever runs (a `400`), so this can never panic on
/// bad input.
pub(crate) async fn snapshot_ws_handler(
    ws: WebSocketUpgrade,
    Path((room, revision)): Path<(String, usize)>,
    State(state): State<RelayState>,
) -> Response {
    ws.on_upgrade(move |socket| serve_snapshot(socket, room, revision, state))
}

/// Replays the frozen `log[..revision]` of `room` to `socket` as binary frames, in
/// order, then sends a clean close.
///
/// Unlike a live join (`handle_socket` in the crate root) this never subscribes to
/// the room's broadcast channel and never reads an inbound frame from the client: a
/// snapshot connection is a one-shot download, not a live join, so it is
/// structurally impossible for it to either observe a later edit or make one. If
/// the client disconnects mid-replay the send simply stops (the client is gone;
/// there is nothing left to do).
async fn serve_snapshot(mut socket: WebSocket, room: String, revision: usize, state: RelayState) {
    for payload in state.log_prefix(&room, revision) {
        if socket.send(Message::Binary(payload)).await.is_err() {
            return;
        }
    }
    let _ = socket.send(Message::Close(None)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unjoined_room_has_revision_zero_and_an_empty_prefix() {
        let state = RelayState::new();
        assert_eq!(state.current_revision("never-joined"), 0);
        assert!(state.log_prefix("never-joined", 0).is_empty());
        assert!(state.log_prefix("never-joined", 10).is_empty());
    }

    #[test]
    fn revision_tracks_the_log_and_a_past_prefix_never_changes() {
        let state = RelayState::new();
        let _ = state.join("room");
        state.record("room", &Bytes::from_static(b"a"));
        state.record("room", &Bytes::from_static(b"b"));
        assert_eq!(state.current_revision("room"), 2);

        let snapshot_at_2 = state.log_prefix("room", 2);
        assert_eq!(
            snapshot_at_2,
            vec![Bytes::from_static(b"a"), Bytes::from_static(b"b")]
        );

        // More edits land after the snapshot was captured...
        state.record("room", &Bytes::from_static(b"c"));
        state.record("room", &Bytes::from_static(b"d"));
        assert_eq!(state.current_revision("room"), 4, "the live room advanced");

        // ...but the earlier revision still replays exactly what it captured.
        assert_eq!(
            state.log_prefix("room", 2),
            snapshot_at_2,
            "a snapshot at revision 2 must still serve exactly revision 2"
        );
    }

    #[test]
    fn a_revision_past_the_current_log_clamps_to_the_whole_log() {
        let state = RelayState::new();
        let _ = state.join("room");
        state.record("room", &Bytes::from_static(b"only"));
        assert_eq!(
            state.log_prefix("room", 999),
            vec![Bytes::from_static(b"only")],
            "an out-of-range revision clamps rather than erroring"
        );
    }

    #[test]
    fn revision_zero_is_an_empty_prefix_even_on_a_populated_room() {
        let state = RelayState::new();
        let _ = state.join("room");
        state.record("room", &Bytes::from_static(b"a"));
        assert!(state.log_prefix("room", 0).is_empty());
    }
}
