//! Frame builders and classifiers for the conformance vectors.
//!
//! Every frame on a live relay session is a [`reticle_proto::v1::SyncMessage`]
//! (see the `reticle_sync::frame` codec). The relay treats the bytes as opaque and fans
//! them out unchanged, so a conformance vector builds a frame with a
//! distinguishable marker, sends it, and recovers that marker from whatever the
//! far side receives. Update frames carry an ASCII marker in their raw `yrs`
//! slot; presence frames encode a sequence number in `cursor.x`. Neither is a
//! real CRDT payload: the relay never inspects them, so any bytes will do.

use reticle_geometry::Point;
use reticle_sync::{Frame, Presence, decode_frame, encode_presence_frame, encode_update_frame};

/// Builds an update frame (`SyncMessage` field 1, first byte `0x0A`) whose raw
/// payload is the ASCII bytes of `marker`, so the receiver can identify it.
#[must_use]
pub fn update_frame(marker: &str) -> Vec<u8> {
    encode_update_frame(marker.as_bytes())
}

/// Builds a presence frame (`SyncMessage` field 2, first byte `0x12`) for `actor`
/// whose `cursor.x` carries `seq`, so a receiver can order and identify bursts.
#[must_use]
pub fn presence_frame(actor: &str, seq: i32) -> Vec<u8> {
    let mut presence = Presence::new(actor);
    presence.cursor = Point::new(seq, 0);
    encode_presence_frame(&presence)
}

/// The first byte of `bytes`, which the relay uses as the sole frame classifier.
#[must_use]
pub fn tag(bytes: &[u8]) -> Option<u8> {
    bytes.first().copied()
}

/// Recovers the marker from an update frame built by [`update_frame`], or `None`
/// if `bytes` is not a decodable update frame with valid UTF-8 contents.
#[must_use]
pub fn update_marker(bytes: &[u8]) -> Option<String> {
    match decode_frame(bytes) {
        Ok(Frame::Update(raw)) => String::from_utf8(raw).ok(),
        _ => None,
    }
}

/// Recovers the sequence number from a presence frame built by
/// [`presence_frame`], or `None` if `bytes` is not a decodable presence frame.
#[must_use]
pub fn presence_seq(bytes: &[u8]) -> Option<i32> {
    match decode_frame(bytes) {
        Ok(Frame::Presence(presence)) => Some(presence.cursor.x),
        _ => None,
    }
}
