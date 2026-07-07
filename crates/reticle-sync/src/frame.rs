//! The live-session wire codec: the single framing every peer agrees on (ADR 0058).
//!
//! A live collaboration socket ([`reticle_server`]'s `GET /ws/{room}`) carries
//! **opaque binary frames**; the relay never inspects them. This module fixes what
//! those bytes *are*: every published frame is a [`reticle_proto::v1::SyncMessage`]
//! envelope (the frozen Wave 0 collaboration message, spec Section 6), so one binary
//! channel multiplexes both a CRDT document delta ([`v1::CrdtUpdate`], carrying raw
//! `yrs` update bytes) and a [`Presence`] update. The receiver decodes each frame with
//! [`decode_frame`] and routes on the variant.
//!
//! # Why one envelope for everything
//!
//! A read-only viewer must materialize *both* the sharer's geometry and the sharer's
//! live cursor, selection, and viewport. Those are two different message types, and the
//! relay hands over a single undistinguished binary stream, so they have to be
//! self-describing on the wire. Wrapping each in the `SyncMessage` oneof does exactly
//! that: [`decode_frame`] recovers which kind arrived without any side channel or frame
//! header of our own. The envelope is the frozen proto, so this adds no new schema.
//!
//! # The contract
//!
//! * A publisher (the editor sharing a session, or the demo agent harness) encodes a
//!   CRDT delta with [`encode_update_frame`] and its presence with
//!   [`encode_presence_frame`], and sends each as one binary frame.
//! * A receiver (the app's read-only viewer session, or a watcher building a
//!   [`SyncDocument`]) decodes each binary frame with [`decode_frame`] and applies the
//!   [`Frame::Update`] raw bytes with [`SyncDocument::apply_update`] and the
//!   [`Frame::Presence`] into its awareness map.
//!
//! Everything here is pure, `cfg`-free, and builds for `wasm32`, so the browser
//! transport and the native/Tokio relay clients share exactly one codec.

use prost::Message as _;
use reticle_proto::v1;

use crate::error::{Result, SyncError};
use crate::presence::Presence;

/// The `doc_id` a bare document delta carries when no room-scoped id is supplied.
///
/// The relay keys rooms by URL path, not by this field, and a viewer applies the raw
/// `yrs` bytes regardless of `doc_id`, so it is informational; an empty string keeps
/// frames compact. Use [`encode_update_frame_for`] to stamp a specific id.
const UNNAMED_DOC: &str = "";

/// One decoded live-session message, recovered from a binary frame by [`decode_frame`].
///
/// The [`Comment`](Frame::Comment) variant carries a decoded
/// [`Comment`](crate::Comment) for completeness of the envelope; the viewer transport
/// routes only [`Update`](Frame::Update) and [`Presence`](Frame::Presence).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    /// A CRDT document delta: the raw `yrs` v1 update bytes to feed
    /// [`SyncDocument::apply_update`](crate::SyncDocument::apply_update) (or, in the app,
    /// `ViewerSession::apply_frame`, which are the same bytes).
    Update(Vec<u8>),
    /// A collaborator's presence (cursor, selection, viewport).
    Presence(Presence),
    /// A threaded comment (carried by the same envelope; not routed by the viewer).
    Comment(crate::Comment),
}

/// Wraps raw `yrs` v1 update `bytes` in a [`v1::SyncMessage`] and encodes it to the
/// binary frame a publisher sends.
///
/// The `bytes` are exactly what [`SyncDocument::encode_state_update`] or
/// [`SyncDocument::encode_update`] produced; this only nests them in the frozen
/// [`v1::CrdtUpdate`] envelope so the receiver can tell a document delta from a
/// presence update on the one binary channel. The `doc_id` and `actor` fields are left
/// empty; use [`encode_update_frame_for`] to stamp them.
///
/// [`SyncDocument::encode_state_update`]: crate::SyncDocument::encode_state_update
/// [`SyncDocument::encode_update`]: crate::SyncDocument::encode_update
#[must_use]
pub fn encode_update_frame(bytes: &[u8]) -> Vec<u8> {
    encode_update_frame_for(UNNAMED_DOC, "", bytes)
}

/// Like [`encode_update_frame`] but stamps the [`v1::CrdtUpdate`] `doc_id` and `actor`.
///
/// The relay ignores both (it keys rooms by URL and forwards opaquely), and a viewer
/// applies the raw bytes regardless, so these are informational for a receiver that
/// wants to attribute or log a delta. The wire shape is identical either way.
#[must_use]
pub fn encode_update_frame_for(doc_id: &str, actor: &str, bytes: &[u8]) -> Vec<u8> {
    let message = v1::SyncMessage {
        payload: Some(v1::sync_message::Payload::Update(v1::CrdtUpdate {
            schema_version: v1::SchemaVersion::V1 as i32,
            doc_id: doc_id.to_owned(),
            actor: actor.to_owned(),
            update: bytes.to_vec(),
        })),
    };
    message.encode_to_vec()
}

/// Encodes `presence` as the binary frame a publisher sends: its [`v1::SyncMessage`]
/// envelope (via [`Presence::to_message`]) serialized with prost.
///
/// [`Presence::to_message`]: crate::Presence::to_message
#[must_use]
pub fn encode_presence_frame(presence: &Presence) -> Vec<u8> {
    presence.to_message().encode_to_vec()
}

/// Decodes one binary frame from the live session into a [`Frame`].
///
/// The bytes must be a prost-encoded [`v1::SyncMessage`] as produced by
/// [`encode_update_frame`], [`encode_presence_frame`], or [`Presence::to_message`] /
/// [`Comment::to_message`]. Routing on the returned variant is how a receiver tells a
/// document delta from a presence update over the single opaque channel.
///
/// # Errors
///
/// Returns [`SyncError::DecodeUpdate`] if `bytes` is not a valid [`v1::SyncMessage`],
/// or [`SyncError::MissingField`] if the envelope carries no payload (an empty oneof).
///
/// [`Presence::to_message`]: crate::Presence::to_message
/// [`Comment::to_message`]: crate::Comment::to_message
pub fn decode_frame(bytes: &[u8]) -> Result<Frame> {
    let message = v1::SyncMessage::decode(bytes)
        .map_err(|e| SyncError::DecodeUpdate(format!("sync message envelope: {e}")))?;
    match message.payload {
        Some(v1::sync_message::Payload::Update(update)) => Ok(Frame::Update(update.update)),
        Some(v1::sync_message::Payload::Presence(presence)) => {
            Ok(Frame::Presence(Presence::from_proto(&presence)))
        }
        Some(v1::sync_message::Payload::Comment(comment)) => {
            Ok(Frame::Comment(crate::Comment::from_proto(&comment)))
        }
        None => Err(SyncError::MissingField("SyncMessage.payload")),
    }
}

#[cfg(test)]
mod tests {
    use super::{Frame, decode_frame, encode_presence_frame, encode_update_frame};
    use crate::{Comment, Presence, SyncDocument};
    use prost::Message as _;
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{Cell, DrawShape, ShapeKind};
    use reticle_proto::v1;

    /// A raw `yrs` state update from a doc that adds one cell with one rect.
    fn sharer_state_bytes() -> Vec<u8> {
        let mut doc = SyncDocument::new("sharer");
        let mut cell = Cell::new("top");
        cell.shapes.push(DrawShape::new(
            LayerId::new(68, 20),
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(400, 400))),
        ));
        doc.add_cell(&cell);
        doc.encode_state_update()
    }

    #[test]
    fn update_frame_round_trips_to_the_same_raw_bytes() {
        let raw = sharer_state_bytes();
        let frame = encode_update_frame(&raw);
        match decode_frame(&frame).expect("decodes") {
            Frame::Update(bytes) => assert_eq!(bytes, raw, "raw yrs bytes survive the envelope"),
            other => panic!("expected an update frame, got {other:?}"),
        }
    }

    #[test]
    fn a_decoded_update_frame_materializes_the_sharers_geometry() {
        // The whole point: a receiver decodes the frame, applies the raw bytes, and
        // sees the sharer's cell. This is exactly what the viewer transport does.
        let frame = encode_update_frame(&sharer_state_bytes());
        let Frame::Update(bytes) = decode_frame(&frame).expect("decodes") else {
            panic!("expected an update frame");
        };
        let mut peer = SyncDocument::new("viewer");
        peer.apply_update(&bytes)
            .expect("raw bytes are a valid update");
        let cell = peer.document().cell("top").expect("sharer's cell arrived");
        assert_eq!(cell.shapes.len(), 1);
    }

    #[test]
    fn presence_frame_round_trips_with_viewport() {
        let mut p = Presence::new("sharer");
        p.cursor = Point::new(120, -45);
        p.selection = vec!["top/shape-1".to_owned()];
        p.viewport = Rect::new(Point::new(-1000, -2000), Point::new(3000, 4000));

        let frame = encode_presence_frame(&p);
        match decode_frame(&frame).expect("decodes") {
            Frame::Presence(got) => assert_eq!(got, p, "the whole presence survives the frame"),
            other => panic!("expected a presence frame, got {other:?}"),
        }
    }

    #[test]
    fn comment_frame_decodes_to_a_comment() {
        let comment = Comment::root("c1", "cell/top", "alice", "looks good", 42);
        let frame = comment.to_message().encode_to_vec();
        match decode_frame(&frame).expect("decodes") {
            Frame::Comment(got) => assert_eq!(got, comment),
            other => panic!("expected a comment frame, got {other:?}"),
        }
    }

    #[test]
    fn update_and_presence_frames_are_distinguishable_on_one_channel() {
        // The two frame kinds a live session multiplexes must route to different arms
        // when interleaved on the single binary channel.
        let update = encode_update_frame(&sharer_state_bytes());
        let presence = encode_presence_frame(&Presence::new("sharer"));
        assert!(matches!(decode_frame(&update), Ok(Frame::Update(_))));
        assert!(matches!(decode_frame(&presence), Ok(Frame::Presence(_))));
    }

    #[test]
    fn garbage_bytes_are_a_decode_error_not_a_panic() {
        // A non-envelope blob must surface a clean error. (Prost is permissive about
        // trailing bytes, so we assert on a frame that decodes to an *empty* oneof,
        // which is the "no payload" error, plus a clearly-invalid varint header.)
        let empty = v1::SyncMessage { payload: None }.encode_to_vec();
        assert!(
            decode_frame(&empty).is_err(),
            "an empty envelope is rejected"
        );
        // A truncated/garbage field header is rejected too.
        assert!(decode_frame(&[0xff, 0xff, 0xff, 0xff]).is_err());
    }
}
