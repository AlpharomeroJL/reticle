//! Ephemeral presence: where each collaborator's cursor, selection, and viewport
//! are, plus a small in-memory awareness map keyed by actor.

use reticle_geometry::{Point, Rect};
use reticle_proto::v1;
use std::collections::HashMap;

/// A single collaborator's live presence: cursor position, current selection, and
/// visible viewport. This is deliberately *not* stored in the CRDT — it is
/// transient session state exchanged out of band (ADR 0007).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Presence {
    /// The actor this presence belongs to.
    pub actor: String,
    /// Human-readable display name (may be empty).
    pub display_name: String,
    /// Packed `0xRRGGBBAA` cursor/selection color.
    pub color_rgba: u32,
    /// Cursor position in document (DBU) coordinates.
    pub cursor: Point,
    /// Ids of the shapes/cells this actor has selected.
    pub selection: Vec<String>,
    /// The actor's current viewport, in document coordinates.
    pub viewport: Rect,
}

impl Presence {
    /// Creates a presence for `actor` with an empty selection and a viewport and
    /// cursor at the origin.
    #[must_use]
    pub fn new(actor: impl Into<String>) -> Self {
        Self {
            actor: actor.into(),
            display_name: String::new(),
            color_rgba: 0,
            cursor: Point::ORIGIN,
            selection: Vec::new(),
            viewport: Rect::default(),
        }
    }

    /// Encodes this presence into its proto message form.
    #[must_use]
    pub fn to_proto(&self) -> v1::Presence {
        v1::Presence {
            actor: self.actor.clone(),
            display_name: self.display_name.clone(),
            color_rgba: self.color_rgba,
            cursor: Some(point_to_proto(self.cursor)),
            selection: self.selection.clone(),
            viewport: Some(rect_to_proto(self.viewport)),
        }
    }

    /// Decodes a presence from its proto message form.
    ///
    /// A missing `cursor` defaults to the origin and a missing `viewport` to an
    /// empty rectangle, so partially-populated messages still decode.
    #[must_use]
    pub fn from_proto(proto: &v1::Presence) -> Self {
        Self {
            actor: proto.actor.clone(),
            display_name: proto.display_name.clone(),
            color_rgba: proto.color_rgba,
            cursor: proto.cursor.map_or(Point::ORIGIN, point_from_proto),
            selection: proto.selection.clone(),
            viewport: proto.viewport.map_or_else(Rect::default, rect_from_proto),
        }
    }

    /// Wraps this presence in a [`v1::SyncMessage`] envelope ready to be sent on a
    /// live collaboration session.
    #[must_use]
    pub fn to_message(&self) -> v1::SyncMessage {
        v1::SyncMessage {
            payload: Some(v1::sync_message::Payload::Presence(self.to_proto())),
        }
    }
}

/// An in-memory map of the most recent [`Presence`] for each actor.
///
/// This is the "awareness" state: a peer merges incoming presence messages here
/// and reads it to render remote cursors and selections. It is intentionally
/// last-write-wins per actor and holds no CRDT metadata.
#[derive(Clone, Debug, Default)]
pub struct Awareness {
    states: HashMap<String, Presence>,
}

impl Awareness {
    /// Creates an empty awareness map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records (or replaces) the presence for its actor, returning the previous
    /// value if one was present.
    pub fn set(&mut self, presence: Presence) -> Option<Presence> {
        self.states.insert(presence.actor.clone(), presence)
    }

    /// Returns the latest presence for `actor`, if known.
    #[must_use]
    pub fn get(&self, actor: &str) -> Option<&Presence> {
        self.states.get(actor)
    }

    /// Removes and returns the presence for `actor` (for example when they
    /// disconnect).
    pub fn remove(&mut self, actor: &str) -> Option<Presence> {
        self.states.remove(actor)
    }

    /// Iterates over every known `(actor, presence)` pair.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Presence)> {
        self.states.iter()
    }

    /// The number of actors currently tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.states.len()
    }

    /// Returns `true` if no actors are tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
}

/// Encodes a [`Point`] into its proto form.
fn point_to_proto(p: Point) -> v1::Point {
    v1::Point { x: p.x, y: p.y }
}

/// Decodes a [`Point`] from its proto form.
fn point_from_proto(p: v1::Point) -> Point {
    Point::new(p.x, p.y)
}

/// Encodes a [`Rect`] into its proto form.
fn rect_to_proto(r: Rect) -> v1::Rect {
    v1::Rect {
        min: Some(point_to_proto(r.min)),
        max: Some(point_to_proto(r.max)),
    }
}

/// Decodes a [`Rect`] from its proto form, defaulting missing corners to the
/// origin.
fn rect_from_proto(r: v1::Rect) -> Rect {
    let min = r.min.map_or(Point::ORIGIN, point_from_proto);
    let max = r.max.map_or(Point::ORIGIN, point_from_proto);
    Rect::new(min, max)
}
