//! The shared conformance vector table.
//!
//! [`vectors`] returns every scripted vector the suite runs against both relays.
//! Each vector covers a specific clause of the relay contract (see the module
//! docs of [`crate`]); together they exercise late-join log replay, view-mode
//! frame drop, echo suppression, presence coalescing, uncoalesced updates, order
//! preservation, full-log replay (the room cap), two-room isolation, and the
//! binary-only rule. The table is data, so the native runner and the Durable
//! Object runner execute exactly the same scripts.

use crate::runner::{Action, Mode, Payload, Vector};

use Action::{
    Connect, ExpectSilence, ExpectUpdate, ExpectUpdates, Grace, PresenceBurst, Send, SendUpdates,
};
use Mode::{Edit, View};

/// Number of presence frames a coalescing burst sends (contract: "burst 50").
const BURST: i32 = 50;

/// Every shared conformance vector, in a stable order.
#[must_use]
pub fn vectors() -> Vec<Vector> {
    vec![
        late_join_log_replay(),
        view_mode_frame_dropped(),
        echo_suppression(),
        presence_coalescing_burst(),
        updates_never_coalesced(),
        full_log_replay_order(),
        two_room_isolation(),
        binary_frames_only(),
    ]
}

/// A late joiner replays the full room log in order, then receives live traffic.
fn late_join_log_replay() -> Vector {
    Vector {
        name: "late_join_log_replay",
        covers: "on join the full accepted-frame log replays in order before live traffic",
        actions: vec![
            Connect {
                who: "A",
                room: "replay",
                mode: Edit,
            },
            Send {
                who: "A",
                payload: Payload::Update("u1"),
            },
            Send {
                who: "A",
                payload: Payload::Update("u2"),
            },
            Grace,
            Connect {
                who: "B",
                room: "replay",
                mode: Edit,
            },
            ExpectUpdate {
                who: "B",
                marker: "u1",
            },
            ExpectUpdate {
                who: "B",
                marker: "u2",
            },
            Send {
                who: "A",
                payload: Payload::Update("u3"),
            },
            ExpectUpdate {
                who: "B",
                marker: "u3",
            },
            ExpectSilence { who: "A" },
        ],
    }
}

/// A view-mode client's frame is dropped server-side: not broadcast, not logged.
fn view_mode_frame_dropped() -> Vector {
    Vector {
        name: "view_mode_frame_dropped",
        covers: "frames from a view-mode connection are dropped: not logged, not broadcast",
        actions: vec![
            Connect {
                who: "A",
                room: "ro",
                mode: Edit,
            },
            Connect {
                who: "V",
                room: "ro",
                mode: View,
            },
            Connect {
                who: "B",
                room: "ro",
                mode: Edit,
            },
            Grace,
            Send {
                who: "A",
                payload: Payload::Update("real"),
            },
            ExpectUpdate {
                who: "V",
                marker: "real",
            },
            ExpectUpdate {
                who: "B",
                marker: "real",
            },
            // The viewer publishes a well-formed frame; it must not fan out.
            Send {
                who: "V",
                payload: Payload::Update("sneaky"),
            },
            ExpectSilence { who: "B" },
            Grace,
            // A late joiner replays only the editor's frame, never the viewer's.
            Connect {
                who: "C",
                room: "ro",
                mode: Edit,
            },
            ExpectUpdate {
                who: "C",
                marker: "real",
            },
            ExpectSilence { who: "C" },
        ],
    }
}

/// A sender never receives the echo of its own frame.
fn echo_suppression() -> Vector {
    Vector {
        name: "echo_suppression",
        covers: "a sender never receives its own echo",
        actions: vec![
            Connect {
                who: "A",
                room: "echo",
                mode: Edit,
            },
            Connect {
                who: "B",
                room: "echo",
                mode: Edit,
            },
            Grace,
            Send {
                who: "A",
                payload: Payload::Update("x"),
            },
            ExpectUpdate {
                who: "B",
                marker: "x",
            },
            ExpectSilence { who: "A" },
        ],
    }
}

/// A presence burst converges to the newest frame on both relays; the coalescing
/// relay additionally delivers strictly fewer than it received.
fn presence_coalescing_burst() -> Vector {
    Vector {
        name: "presence_coalescing_burst",
        covers: "presence coalesced per client to the newest within a window; convergence preserved",
        actions: vec![
            Connect {
                who: "A",
                room: "presence",
                mode: Edit,
            },
            Connect {
                who: "B",
                room: "presence",
                mode: Edit,
            },
            Grace,
            PresenceBurst {
                who: "A",
                observer: "B",
                count: BURST,
            },
        ],
    }
}

/// Update frames are never coalesced or dropped: all arrive, in order.
fn updates_never_coalesced() -> Vector {
    Vector {
        name: "updates_never_coalesced",
        covers: "update frames are never coalesced or dropped, and arrive in order",
        actions: vec![
            Connect {
                who: "A",
                room: "updates",
                mode: Edit,
            },
            Connect {
                who: "B",
                room: "updates",
                mode: Edit,
            },
            Grace,
            SendUpdates {
                who: "A",
                count: 20,
            },
            ExpectUpdates {
                who: "B",
                count: 20,
            },
        ],
    }
}

/// The room log replays a large run of frames to a late joiner, in exact order
/// (the observable of the room cap: the log holds and replays every frame).
fn full_log_replay_order() -> Vector {
    Vector {
        name: "full_log_replay_order",
        covers: "the room log holds and replays every accepted frame to a late joiner, in order",
        actions: vec![
            Connect {
                who: "A",
                room: "cap",
                mode: Edit,
            },
            Grace,
            SendUpdates {
                who: "A",
                count: 64,
            },
            Grace,
            Connect {
                who: "C",
                room: "cap",
                mode: Edit,
            },
            ExpectUpdates {
                who: "C",
                count: 64,
            },
        ],
    }
}

/// Two rooms are isolated: a frame in one never reaches a peer in the other.
fn two_room_isolation() -> Vector {
    Vector {
        name: "two_room_isolation",
        covers: "rooms are isolated: a frame in one room never reaches a peer in another",
        actions: vec![
            Connect {
                who: "A",
                room: "room-1",
                mode: Edit,
            },
            Connect {
                who: "C",
                room: "room-1",
                mode: Edit,
            },
            Connect {
                who: "B",
                room: "room-2",
                mode: Edit,
            },
            Grace,
            Send {
                who: "A",
                payload: Payload::Update("iso"),
            },
            ExpectUpdate {
                who: "C",
                marker: "iso",
            },
            ExpectSilence { who: "B" },
        ],
    }
}

/// Only binary frames are payloads: a text frame is ignored, binary still flows.
fn binary_frames_only() -> Vector {
    Vector {
        name: "binary_frames_only",
        covers: "only binary frames are payloads; text is ignored and never broadcast",
        actions: vec![
            Connect {
                who: "A",
                room: "binary",
                mode: Edit,
            },
            Connect {
                who: "B",
                room: "binary",
                mode: Edit,
            },
            Grace,
            Send {
                who: "A",
                payload: Payload::Text("not a payload"),
            },
            ExpectSilence { who: "B" },
            Send {
                who: "A",
                payload: Payload::Update("after"),
            },
            ExpectUpdate {
                who: "B",
                marker: "after",
            },
        ],
    }
}

/// A deliberately broken vector for the two-way (negative) test: it asserts a
/// view-mode client's frame reaches an editor, which a *correct* relay drops. It
/// must therefore FAIL against either real relay, proving the harness has teeth.
#[must_use]
pub fn broken_expects_view_frame_forwarded() -> Vector {
    Vector {
        name: "broken_expects_view_frame_forwarded",
        covers: "NEGATIVE: expects a dropped view frame to be forwarded; must fail",
        actions: vec![
            Connect {
                who: "A",
                room: "broken",
                mode: Edit,
            },
            Connect {
                who: "V",
                room: "broken",
                mode: View,
            },
            Grace,
            Send {
                who: "V",
                payload: Payload::Update("should-be-dropped"),
            },
            // A correct relay drops this, so the expectation cannot be met.
            ExpectUpdate {
                who: "A",
                marker: "should-be-dropped",
            },
        ],
    }
}
