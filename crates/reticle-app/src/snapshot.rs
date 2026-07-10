//! Immutable snapshot permalinks: opening a link that always shows the same
//! geometry, no matter how much the live document changes afterward.
//!
//! A snapshot permalink names a relay, a room, and a **revision**: the prefix
//! length into the room's append-only update log a "capture snapshot" action
//! recorded (`reticle-server`'s `GET /snapshot/{room}/at/{revision}`; see that
//! crate's internal `snapshot` module for the full design). Opening the permalink
//! replays exactly that frozen prefix and nothing else, so it reproduces the
//! geometry as it stood at capture time even after the live room has moved on.
//!
//! This module owns the pure, `cfg`-free half of that: composing and parsing the
//! permalink (mirroring [`crate::share`]'s live-viewer link, but carrying a
//! revision instead of naming only a room), and [`SnapshotSession`], the read-only
//! mirror that decodes the relay's replayed frames into a materialized document.
//! It never opens a socket itself: the transport that dials `reticle-server` and
//! feeds bytes into [`SnapshotSession::apply_frame`] is DOM/wasm glue analogous to
//! `crate::livesync::ViewerTransport`, following the same testable-seam split that
//! module documents (pure logic here, unit-tested without a browser; socket glue
//! is out of this module's scope).
//!
//! # Why not `crate::viewer::ViewerSession`
//!
//! [`crate::viewer::ViewerSession`] is a *live* read-only mirror: it tracks a
//! sharer's presence and offers a follow-camera that rides the sharer's viewport.
//! A snapshot has no live sharer to follow and no presence worth showing (a cursor
//! is a transient, live-only concept); it is a fixed point in history. Reusing
//! `ViewerSession` would carry that dead weight for no benefit, so
//! [`SnapshotSession`] is the plain CRDT mirror on its own, mirroring only the
//! parts of `ViewerSession` a snapshot actually needs.

use reticle_sync::{Frame, SyncDocument, SyncError, decode_frame};

/// The `?view=` value that opens a page as a snapshot permalink (mirrors
/// [`crate::share::VIEWER_VIEW`] for the live read-only viewer).
pub const SNAPSHOT_VIEW: &str = "snapshot";

/// The relay address `reticle-server` binds when unset, reused here so a snapshot
/// link composed with no explicit relay still resolves (matches
/// [`crate::share::DEFAULT_SERVER`]).
pub const DEFAULT_SERVER: &str = "127.0.0.1:3030";

/// Maps a relay spec (bare host, `http(s)://`, or an explicit `ws(s)://`) to its
/// WebSocket base, exactly as `reticle-server` expects to be dialed.
///
/// A small, self-contained copy of the scheme mapping `crate::share::room_link`
/// performs, kept local so this module does not depend on `share`'s private
/// helpers: the two evolve independently, and `share` owns the *live* room link
/// (this lane's boundary keeps `share.rs` untouched; see the module docs).
fn relay_ws_base(relay: &str) -> String {
    let trimmed = relay.trim();
    let spec = if trimmed.is_empty() {
        DEFAULT_SERVER
    } else {
        trimmed
    };
    let with_scheme = if spec.starts_with("ws://") || spec.starts_with("wss://") {
        spec.to_owned()
    } else if let Some(rest) = spec.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = spec.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        format!("ws://{spec}")
    };
    with_scheme.trim_end_matches('/').to_owned()
}

/// Sanitizes a free-form room name into a URL path segment.
///
/// A small, self-contained copy of `crate::share::room_id`'s rule (lowercase
/// ASCII, keep `[a-z0-9_-]`, collapse everything else to a single `-`, fall back
/// to `"layout"` when nothing survives), kept local for the reason
/// `relay_ws_base` documents. Idempotent, exactly like `share::room_id`.
#[must_use]
pub fn room_id(name: &str) -> String {
    let mut id = String::with_capacity(name.len());
    let mut pending_dash = false;
    for c in name.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
            if pending_dash && !id.is_empty() {
                id.push('-');
            }
            pending_dash = false;
            id.push(c);
        } else {
            pending_dash = true;
        }
    }
    let id = id.trim_matches('-');
    if id.is_empty() {
        "layout".to_owned()
    } else {
        id.to_owned()
    }
}

/// The WebSocket URL for `room`'s current revision on `relay`
/// (`reticle-server`'s `GET /snapshot/{room}`: the "capture" step).
#[must_use]
pub fn snapshot_revision_ws_link(relay: &str, room: &str) -> String {
    format!("{}/snapshot/{}", relay_ws_base(relay), room_id(room))
}

/// The WebSocket URL that opens the immutable snapshot of `room` at `revision` on
/// `relay` (`reticle-server`'s `GET /snapshot/{room}/at/{revision}`).
#[must_use]
pub fn snapshot_ws_link(relay: &str, room: &str, revision: u64) -> String {
    format!(
        "{}/snapshot/{}/at/{revision}",
        relay_ws_base(relay),
        room_id(room)
    )
}

/// The room, relay, and revision recovered from a snapshot page's query string.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SnapshotTarget {
    /// The sanitized relay room the snapshot was captured from.
    pub room: String,
    /// The relay host to dial for the frozen frames.
    pub relay: String,
    /// The captured revision: the log prefix length to replay.
    pub revision: u64,
}

/// Composes the shareable **page** URL for the snapshot of `room` at `revision` on
/// `relay`, hosted at page origin `page` (mirrors
/// [`crate::share::viewer_link`]'s shape, for a snapshot instead of a live view).
///
/// An empty `page` yields a relative `?...` query so the link resolves against
/// wherever the bundle is already loaded; an empty `relay` falls back to
/// [`DEFAULT_SERVER`]. The inverse is [`parse_snapshot_query`].
#[must_use]
pub fn emit_snapshot_link(page: &str, relay: &str, room: &str, revision: u64) -> String {
    let room = room_id(room);
    let relay_spec = {
        let trimmed = relay.trim();
        if trimmed.is_empty() {
            DEFAULT_SERVER
        } else {
            trimmed
        }
    };
    let query = format!(
        "view={SNAPSHOT_VIEW}&room={room}&relay={}&rev={revision}",
        encode_query_component(relay_spec)
    );
    let base = page.trim().trim_end_matches('/');
    if base.is_empty() {
        format!("?{query}")
    } else {
        format!("{base}/?{query}")
    }
}

/// Parses a page query string (the part after `?`) into a [`SnapshotTarget`], or
/// `None` if it does not name a snapshot (missing `view=snapshot`, an absent or
/// non-numeric `rev`, or an absent `room`).
///
/// The inverse of [`emit_snapshot_link`]. A leading `?` is tolerated (this is meant
/// to be fed `location.search` directly); every value is parsed leniently so a
/// hand-edited link is rejected as "not a target" rather than panicking.
#[must_use]
pub fn parse_snapshot_query(query: &str) -> Option<SnapshotTarget> {
    let query = query.trim_start_matches('?');
    let mut view = None;
    let mut room = None;
    let mut relay = None;
    let mut revision = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = decode_query_component(value);
        match key {
            "view" => view = Some(value),
            "room" => room = Some(value),
            "relay" => relay = Some(value),
            "rev" => revision = value.trim().parse::<u64>().ok(),
            _ => {}
        }
    }
    if view.as_deref() != Some(SNAPSHOT_VIEW) {
        return None;
    }
    let room = room_id(&room?);
    let revision = revision?;
    let relay = relay
        .map(|r| r.trim().to_owned())
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| DEFAULT_SERVER.to_owned());
    Some(SnapshotTarget {
        room,
        relay,
        revision,
    })
}

/// Percent-encodes the characters that would break a query component: `&`, `=`,
/// `%`, `#`, `?`, and whitespace. Mirrors `crate::share`'s encoder (kept local for
/// the reason [`relay_ws_base`] documents).
fn encode_query_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("%26"),
            '=' => out.push_str("%3D"),
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            ' ' => out.push_str("%20"),
            _ => out.push(c),
        }
    }
    out
}

/// Reverses [`encode_query_component`]; invalid or truncated escapes are left
/// verbatim so a hand-typed link never panics.
fn decode_query_component(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (
                bytes.get(i + 1).and_then(|b| hex_val(*b)),
                bytes.get(i + 2).and_then(|b| hex_val(*b)),
            )
        {
            out.push(char::from(h * 16 + l));
            i += 3;
            continue;
        }
        out.push(char::from(bytes[i]));
        i += 1;
    }
    out
}

/// The value of a single ASCII hex digit, or `None`.
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// The actor id a [`SnapshotSession`]'s mirror uses; distinct from
/// [`crate::viewer::VIEWER_ACTOR`] so a snapshot's element ids can never be
/// mistaken for a live viewer's (neither ever publishes).
pub const SNAPSHOT_ACTOR: &str = "snapshot";

/// A read-only mirror of one immutable snapshot: feed it the relay's replayed
/// frames in order with [`SnapshotSession::apply_frame`] and read
/// [`SnapshotSession::document`] to render.
#[derive(Debug)]
pub struct SnapshotSession {
    doc: SyncDocument,
    /// How many frames have been applied so far, for a simple load-progress signal.
    frames_applied: usize,
}

impl Default for SnapshotSession {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapshotSession {
    /// Creates an empty snapshot mirror.
    #[must_use]
    pub fn new() -> Self {
        Self {
            doc: SyncDocument::new(SNAPSHOT_ACTOR),
            frames_applied: 0,
        }
    }

    /// Applies one binary frame exactly as replayed by `reticle-server`'s
    /// `GET /snapshot/{room}/at/{revision}`.
    ///
    /// Each frame is the same `reticle_proto::v1::SyncMessage` envelope the live
    /// transport carries: this decodes it and merges a document delta into the
    /// mirror, exactly as `crate::livesync::route_frame` routes a live frame to
    /// [`crate::viewer::ViewerSession::apply_frame`]. A presence or comment frame
    /// (also a legal log entry; the relay logs every accepted frame regardless of
    /// kind, see `reticle_server`'s crate docs) carries no geometry and is
    /// ignored rather than erroring, since a snapshot's whole point is the frozen
    /// document, not any collaborator's transient cursor. A frame that fails to
    /// decode is likewise ignored: one malformed entry must not stop the rest of
    /// the replay from applying.
    ///
    /// # Errors
    ///
    /// Returns the [`SyncError`] from [`SyncDocument::apply_update`] if `bytes`
    /// decodes as an update frame whose payload is not a valid CRDT update.
    pub fn apply_frame(&mut self, bytes: &[u8]) -> Result<(), SyncError> {
        self.frames_applied += 1;
        match decode_frame(bytes) {
            Ok(Frame::Update(raw)) => self.doc.apply_update(&raw),
            Ok(Frame::Presence(_) | Frame::Comment(_)) | Err(_) => Ok(()),
        }
    }

    /// The mirrored document: the snapshot's geometry as of its captured revision.
    #[must_use]
    pub fn document(&self) -> &reticle_model::Document {
        self.doc.document()
    }

    /// How many frames have been applied so far (whether or not they carried
    /// geometry), for a simple "still loading" indicator while a replay streams
    /// in.
    #[must_use]
    pub fn frames_applied(&self) -> usize {
        self.frames_applied
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{Cell, DrawShape, ShapeKind};
    use reticle_sync::encode_update_frame;

    /// A raw yrs state update from a doc with cell `top` and one met1 rectangle.
    fn geometry_frame() -> Vec<u8> {
        let mut doc = SyncDocument::new("author");
        let mut cell = Cell::new("top");
        cell.shapes.push(DrawShape::new(
            LayerId::new(68, 20),
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(400, 400))),
        ));
        doc.add_cell(&cell);
        encode_update_frame(&doc.encode_state_update())
    }

    #[test]
    fn snapshot_link_round_trips_through_the_query() {
        let link = emit_snapshot_link("https://reticle.example", "relay.lab:9000", "CHIP_TOP", 42);
        assert!(link.starts_with("https://reticle.example/?"), "{link}");
        let query = link.split_once('?').expect("query").1;
        let target = parse_snapshot_query(query).expect("a snapshot target");
        assert_eq!(target.room, "chip_top");
        assert_eq!(target.relay, "relay.lab:9000");
        assert_eq!(target.revision, 42);
    }

    #[test]
    fn snapshot_link_with_empty_page_and_relay_is_relative_and_defaults() {
        let link = emit_snapshot_link("", "", "top", 7);
        // A colon is not a reserved query character (see `encode_query_component`'s
        // doc comment), so it survives unescaped, exactly as
        // `share::viewer_link_carries_view_room_and_relay` expects for the same
        // default relay spec.
        assert_eq!(link, "?view=snapshot&room=top&relay=127.0.0.1:3030&rev=7");
        let target = parse_snapshot_query(link.trim_start_matches('?')).expect("target");
        assert_eq!(target.relay, DEFAULT_SERVER);
        assert_eq!(target.revision, 7);
    }

    #[test]
    fn parse_snapshot_query_rejects_non_snapshot_links() {
        assert!(parse_snapshot_query("view=viewer&room=top&rev=1").is_none());
        assert!(parse_snapshot_query("room=top&rev=1").is_none());
        // A snapshot view with no revision, or a non-numeric one, is not a target:
        // there is nothing to pin without a revision.
        assert!(parse_snapshot_query("view=snapshot&room=top").is_none());
        assert!(parse_snapshot_query("view=snapshot&room=top&rev=nope").is_none());
        // No room at all.
        assert!(parse_snapshot_query("view=snapshot&rev=1").is_none());
    }

    #[test]
    fn snapshot_ws_link_composes_the_relay_route() {
        assert_eq!(
            snapshot_ws_link("relay.lab:9000", "CHIP_TOP", 5),
            "ws://relay.lab:9000/snapshot/chip_top/at/5"
        );
        assert_eq!(
            snapshot_ws_link("https://relay.lab", "top", 0),
            "wss://relay.lab/snapshot/top/at/0"
        );
    }

    #[test]
    fn snapshot_revision_ws_link_composes_the_capture_route() {
        assert_eq!(
            snapshot_revision_ws_link("relay.lab:9000", "top"),
            "ws://relay.lab:9000/snapshot/top"
        );
    }

    #[test]
    fn snapshot_session_materializes_applied_geometry() {
        let mut session = SnapshotSession::new();
        assert!(session.document().cell("top").is_none());
        session.apply_frame(&geometry_frame()).expect("applies");
        let cell = session.document().cell("top").expect("cell replayed");
        assert_eq!(cell.shapes.len(), 1);
        assert_eq!(session.frames_applied(), 1);
    }

    #[test]
    fn snapshot_session_ignores_malformed_and_non_geometry_frames() {
        let mut session = SnapshotSession::new();
        // Garbage bytes: ignored, not an error, and still counted as "applied" (a
        // replay position advanced) so a progress indicator does not stall.
        session
            .apply_frame(&[0xff, 0xff, 0xff, 0xff])
            .expect("ignored, not an error");
        assert!(session.document().cell("top").is_none());

        // A presence frame carries no geometry and is ignored too.
        let presence = reticle_sync::encode_presence_frame(&reticle_sync::Presence::new("someone"));
        session.apply_frame(&presence).expect("presence ignored");
        assert!(session.document().cell("top").is_none());
        assert_eq!(session.frames_applied(), 2);

        // Real geometry still applies afterward.
        session.apply_frame(&geometry_frame()).expect("applies");
        assert!(session.document().cell("top").is_some());
    }

    #[test]
    fn permalink_round_trip_capture_serialize_open_reproduces_the_same_geometry() {
        // "Capture": the sharer's document at the moment of capture.
        let mut sharer = SyncDocument::new("sharer");
        let mut cell = Cell::new("top");
        cell.shapes.push(DrawShape::new(
            LayerId::new(68, 20),
            ShapeKind::Rect(Rect::new(Point::new(10, 10), Point::new(90, 90))),
        ));
        sharer.add_cell(&cell);
        let captured = sharer.document().clone();

        // "Serialize": exactly what the relay would have stored as the room's log
        // entry and later replay verbatim (a SyncMessage-framed CRDT update).
        let frame = encode_update_frame(&sharer.encode_state_update());

        // The live document keeps changing after the capture...
        sharer.add_cell(&Cell::new("changed-after-capture"));
        assert_ne!(sharer.document(), &captured, "the live doc moved on");

        // "Open": a fresh SnapshotSession, fed only the captured frame, reproduces
        // exactly the geometry as of capture, unaffected by the later edit.
        let mut opened = SnapshotSession::new();
        opened.apply_frame(&frame).expect("captured frame applies");
        assert_eq!(
            opened.document(),
            &captured,
            "opening the permalink reproduces the captured geometry"
        );
        assert!(
            opened.document().cell("changed-after-capture").is_none(),
            "a later live edit must never appear in an opened snapshot"
        );
    }
}
