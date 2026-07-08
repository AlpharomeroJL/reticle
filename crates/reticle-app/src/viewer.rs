//! Read-only viewer sync for a shared session (ADR 0038).
//!
//! A viewer opens a share link ([`crate::share::viewer_link`]) and joins the
//! sharer's relay room read-only. This module is the window-free state machine
//! behind that view:
//!
//! * it owns a [`SyncDocument`] mirror and applies the sharer's live CRDT frames
//!   to it with [`ViewerSession::apply_frame`], but never produces a frame of its
//!   own, so it can never mutate the shared document;
//! * it records the sharer's [`Presence`] (cursor, selection, and viewport) with
//!   [`ViewerSession::apply_presence`], so the view can draw the sharer's cursor
//!   and highlight what the sharer has selected;
//! * it owns a **local** [`ViewCamera`] so the viewer pans and zooms independently
//!   of the sharer;
//! * a **follow-mode** toggle ([`ViewerSession::set_follow`]) snaps the local
//!   camera to frame the sharer's current viewport each time
//!   [`ViewerSession::sync_camera`] runs, so the viewer rides along with the
//!   sharer until they turn follow off (which leaves the camera wherever it was,
//!   ready for independent panning again).
//!
//! Everything here is pure and testable without egui or a network: frames are raw
//! `yrs` update bytes (exactly what the relay carries), presence is the frozen
//! [`Presence`] type, and the follow camera is the free function
//! [`follow_camera`]. The live browser wiring (opening the socket, pumping frames
//! into [`apply_frame`](ViewerSession::apply_frame)) is the Wave 1 end-to-end
//! gate; this module is the logic that wiring drives.

use reticle_geometry::{Point, Rect};
use reticle_sync::{Awareness, Presence, SyncDocument, SyncError};

use crate::camera::{ScreenRect, ViewCamera};

/// A palette of visually distinct collaborator colors (packed `0xRRGGBBAA`),
/// assigned to remote actors so each named cursor and session-chip avatar reads as
/// one consistent person (catalog 86). These are canvas *data* colors, not chrome
/// tokens (tokens.md keeps presence-cursor colors with the overlay code), and they
/// stay clear of the sharer's own blue (`0x2f_81_f7_ff`).
pub const COLLAB_PALETTE: [u32; 8] = [
    0xe5_48_4d_ff, // red
    0xf7_6b_15_ff, // orange
    0xff_b2_24_ff, // amber
    0x46_a7_58_ff, // green
    0x12_a5_94_ff, // teal
    0x8e_4e_c6_ff, // purple
    0xe9_3d_82_ff, // pink
    0x5e_b0_ef_ff, // sky
];

/// Deterministically maps an actor id to a stable [`COLLAB_PALETTE`] color, so the
/// same collaborator keeps the same color across frames and across peers without
/// any coordination (an FNV-1a hash of the id, folded into the palette).
#[must_use]
pub fn color_for_actor(actor: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for b in actor.bytes() {
        hash ^= u32::from(b);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    COLLAB_PALETTE[(hash as usize) % COLLAB_PALETTE.len()]
}

/// Seconds a cursor may sit still before it begins to fade.
pub const IDLE_FADE_START: f32 = 3.0;
/// Seconds of stillness by which a cursor reaches its faded floor.
pub const IDLE_FADE_END: f32 = 12.0;
/// The floor opacity an idle cursor fades to (it recedes, never vanishes).
pub const IDLE_MIN_ALPHA: f32 = 0.25;

/// The opacity multiplier for a cursor that last moved `idle` seconds ago
/// (catalog 86): full until [`IDLE_FADE_START`], easing linearly to
/// [`IDLE_MIN_ALPHA`] by [`IDLE_FADE_END`], and never below that floor so a parked
/// collaborator still reads on the canvas.
#[must_use]
pub fn idle_alpha(idle: f32) -> f32 {
    if idle <= IDLE_FADE_START {
        1.0
    } else if idle >= IDLE_FADE_END {
        IDLE_MIN_ALPHA
    } else {
        let f = (idle - IDLE_FADE_START) / (IDLE_FADE_END - IDLE_FADE_START);
        1.0 - f * (1.0 - IDLE_MIN_ALPHA)
    }
}

/// A remote participant in a shared session, for the viewer's session chip
/// (catalog 75): the actor id, a display name (falling back to a short label), and
/// the color its cursor and avatar are drawn with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Participant {
    /// The actor id this participant is keyed by.
    pub actor: String,
    /// The display name shown on the avatar and cursor label.
    pub name: String,
    /// The packed `0xRRGGBBAA` color for the avatar and cursor.
    pub color_rgba: u32,
}

/// Builds the session chip's participant list from an [`Awareness`] map, dropping
/// `self_actor` (the local viewer never counts itself) and sorting by actor id so
/// the avatar row is stable frame to frame.
///
/// Each participant takes its published `color_rgba` when set, else a stable
/// palette color from [`color_for_actor`]; its name is the published
/// `display_name` when non-empty, else a short label derived from the actor id.
#[must_use]
pub fn participants(awareness: &Awareness, self_actor: &str) -> Vec<Participant> {
    let mut out: Vec<Participant> = awareness
        .iter()
        .filter(|(actor, _)| actor.as_str() != self_actor)
        .map(|(actor, presence)| {
            let color = if presence.color_rgba == 0 {
                color_for_actor(actor)
            } else {
                presence.color_rgba
            };
            let name = if presence.display_name.is_empty() {
                short_actor_label(actor)
            } else {
                presence.display_name.clone()
            };
            Participant {
                actor: actor.clone(),
                name,
                color_rgba: color,
            }
        })
        .collect();
    out.sort_by(|a, b| a.actor.cmp(&b.actor));
    out
}

/// A short human label for an actor with no published display name: the first
/// segment before any separator, capped so a long id does not overrun the chip.
fn short_actor_label(actor: &str) -> String {
    let head = actor
        .split(['-', ':', '_', '@'])
        .next()
        .unwrap_or(actor)
        .trim();
    let head = if head.is_empty() { actor } else { head };
    head.chars().take(12).collect()
}

/// Eases `from` a fraction `t` (0..=1) toward `to`, interpolating the center and
/// the zoom, for the smooth follow-camera transition (catalog 87). `t == 1.0`
/// lands exactly on `to` (used under reduced motion and as the snap fallback).
#[must_use]
pub fn lerp_camera(from: ViewCamera, to: ViewCamera, t: f32) -> ViewCamera {
    let t = f64::from(t.clamp(0.0, 1.0));
    let (a, b) = (from.center(), to.center());
    let cx = f64::from(a.x) + f64::from(b.x - a.x) * t;
    let cy = f64::from(a.y) + f64::from(b.y - a.y) * t;
    let ppd = from.pixels_per_dbu() + (to.pixels_per_dbu() - from.pixels_per_dbu()) * t;
    #[allow(clippy::cast_possible_truncation)]
    ViewCamera::new(Point::new(cx.round() as i32, cy.round() as i32), ppd)
}

/// The actor id the viewer's own (read-only) mirror uses.
///
/// It is deliberately distinct and never publishes, so even if it were wired to a
/// relay it could not be mistaken for the sharer.
pub const VIEWER_ACTOR: &str = "viewer";

/// A live, read-only view of a shared session.
///
/// Construct one with [`ViewerSession::new`], feed it the sharer's frames and
/// presence, and read [`ViewerSession::document`], [`ViewerSession::sharer`], and
/// [`ViewerSession::camera`] to render. It holds no socket; the caller pumps it.
#[derive(Debug)]
pub struct ViewerSession {
    /// The mirrored document the sharer's frames build up. Never edited locally.
    doc: SyncDocument,
    /// The sharer's latest presence (cursor, selection, viewport), if seen.
    awareness: Awareness,
    /// The actor id of the sharer we follow, learned from the first presence.
    sharer_actor: Option<String>,
    /// The viewer's own camera, panned and zoomed independently of the sharer.
    camera: ViewCamera,
    /// Whether follow-mode is on: the camera tracks the sharer's viewport.
    follow: bool,
}

impl Default for ViewerSession {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewerSession {
    /// Creates an empty read-only viewer with follow-mode off and a default camera.
    #[must_use]
    pub fn new() -> Self {
        Self {
            doc: SyncDocument::new(VIEWER_ACTOR),
            awareness: Awareness::new(),
            sharer_actor: None,
            camera: ViewCamera::default(),
            follow: false,
        }
    }

    /// Applies one CRDT frame (raw `yrs` v1 update bytes) from the sharer, merging
    /// it into the mirrored document.
    ///
    /// These are exactly the binary frames the relay fans out. Applying them is
    /// idempotent and commutative (the CRDT guarantees it), so a replayed backlog
    /// followed by live frames converges to the sharer's document regardless of
    /// ordering or duplication.
    ///
    /// # Errors
    ///
    /// Returns the [`SyncError`] from [`SyncDocument::apply_update`] if the bytes
    /// are not a valid update.
    pub fn apply_frame(&mut self, frame: &[u8]) -> Result<(), SyncError> {
        self.doc.apply_update(frame)
    }

    /// Records the sharer's latest [`Presence`] (cursor, selection, and viewport).
    ///
    /// The first presence seen fixes which actor is "the sharer" for follow-mode;
    /// subsequent updates from that actor refresh the viewport the camera tracks.
    /// Presence from other actors is still stored (so multi-peer sessions render
    /// every cursor) but does not steer follow-mode.
    pub fn apply_presence(&mut self, presence: Presence) {
        if self.sharer_actor.is_none() {
            self.sharer_actor = Some(presence.actor.clone());
        }
        self.awareness.set(presence);
    }

    /// The mirrored document, a materialized view the sharer's frames built up.
    #[must_use]
    pub fn document(&self) -> &reticle_model::Document {
        self.doc.document()
    }

    /// The sharer's latest presence, if any has arrived.
    #[must_use]
    pub fn sharer(&self) -> Option<&Presence> {
        self.sharer_actor
            .as_deref()
            .and_then(|actor| self.awareness.get(actor))
    }

    /// The full awareness map, so a view can draw every remote cursor and
    /// selection (not just the sharer's).
    #[must_use]
    pub fn awareness(&self) -> &Awareness {
        &self.awareness
    }

    /// The viewer's current camera (independent of the sharer unless following).
    #[must_use]
    pub fn camera(&self) -> ViewCamera {
        self.camera
    }

    /// Mutable access to the viewer's camera, for local pan and zoom.
    ///
    /// A manual pan or zoom while follow-mode is on is transient: the next
    /// [`sync_camera`](Self::sync_camera) snaps back to the sharer. Turn follow off
    /// (see [`set_follow`](Self::set_follow)) to pan and zoom freely.
    pub fn camera_mut(&mut self) -> &mut ViewCamera {
        &mut self.camera
    }

    /// Whether follow-mode is currently on.
    #[must_use]
    pub fn is_following(&self) -> bool {
        self.follow
    }

    /// Turns follow-mode on or off.
    ///
    /// Turning it *on* does not itself move the camera; the next
    /// [`sync_camera`](Self::sync_camera) (given a screen) snaps to the sharer.
    /// Turning it *off* leaves the camera exactly where it is, so the viewer
    /// resumes independent panning from the sharer's last framed viewport.
    pub fn set_follow(&mut self, follow: bool) {
        self.follow = follow;
    }

    /// Toggles follow-mode and returns the new state.
    pub fn toggle_follow(&mut self) -> bool {
        self.follow = !self.follow;
        self.follow
    }

    /// When following, snaps the local camera to frame the sharer's current
    /// viewport in `screen`; a no-op when follow-mode is off or no sharer viewport
    /// is known.
    ///
    /// Returns `true` if the camera was updated. Call this once per frame after
    /// feeding in the latest presence: with follow on, the viewer's camera then
    /// shows exactly the region the sharer sees, letterboxed to the viewer's own
    /// aspect ratio so nothing the sharer sees is cropped away.
    pub fn sync_camera(&mut self, screen: &ScreenRect) -> bool {
        if !self.follow {
            return false;
        }
        let Some(viewport) = self.sharer().map(|p| p.viewport) else {
            return false;
        };
        if viewport.is_empty() {
            return false;
        }
        self.camera = follow_camera(viewport, screen);
        true
    }

    /// Eases the local camera a fraction `t` toward the sharer's framed viewport,
    /// for the smooth follow transition (catalog 87). Like
    /// [`sync_camera`](Self::sync_camera) it is a no-op when follow-mode is off or
    /// no sharer viewport is known, and returns whether the camera moved.
    ///
    /// Pass `t == 1.0` (reduced motion) to snap exactly as `sync_camera` does; a
    /// smaller `t` per frame interpolates so a viewer that clicks an avatar glides
    /// to the sharer's view instead of jumping.
    pub fn follow_step(&mut self, screen: &ScreenRect, t: f32) -> bool {
        if !self.follow {
            return false;
        }
        let Some(viewport) = self.sharer().map(|p| p.viewport) else {
            return false;
        };
        if viewport.is_empty() {
            return false;
        }
        let target = follow_camera(viewport, screen);
        self.camera = lerp_camera(self.camera, target, t);
        true
    }
}

/// The camera that frames the sharer's `viewport` (a world-space DBU rectangle)
/// inside the viewer's `screen`, centered, with the whole viewport visible.
///
/// This is the pure follow-mode math: it centers on the middle of the sharer's
/// viewport and picks the zoom that fits the viewport's width and height into the
/// screen, choosing the smaller of the two so the entire region the sharer sees
/// stays on screen (the extra space along the other axis is the letterbox from
/// differing aspect ratios). A degenerate `viewport` (zero width or height) keeps
/// a unit zoom and only centers, so a collapsed viewport still lands in the middle
/// rather than dividing by zero.
///
/// It reuses [`ViewCamera::zoom_to_fit`], the same fit logic the editor's
/// "zoom to fit" uses, so a followed viewer frames the sharer's view exactly as if
/// they had fit that rectangle themselves.
#[must_use]
pub fn follow_camera(viewport: Rect, screen: &ScreenRect) -> ViewCamera {
    let mut camera = ViewCamera::default();
    camera.zoom_to_fit(screen, viewport);
    camera
}

#[cfg(test)]
mod tests {
    use super::{ViewerSession, follow_camera};
    use crate::camera::{ScreenRect, ViewCamera};
    use reticle_geometry::{LayerId, Point, Rect};
    use reticle_model::{Cell, ShapeKind};
    use reticle_sync::{Presence, SyncDocument};

    fn screen() -> ScreenRect {
        ScreenRect::new(0.0, 0.0, 800.0, 600.0)
    }

    /// A frame from a "sharer" doc that adds a cell with one rect, as raw bytes.
    fn sharer_frame_with_cell(name: &str) -> Vec<u8> {
        let mut sharer = SyncDocument::new("sharer");
        let mut cell = Cell::new(name);
        cell.shapes.push(reticle_model::DrawShape::new(
            LayerId::new(68, 20),
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(400, 400))),
        ));
        sharer.add_cell(&cell);
        sharer.encode_state_update()
    }

    #[test]
    fn applies_sharer_frames_into_the_mirror() {
        let mut viewer = ViewerSession::new();
        assert!(viewer.document().cell("top").is_none());

        let frame = sharer_frame_with_cell("top");
        viewer.apply_frame(&frame).expect("frame applies");
        assert!(
            viewer.document().cell("top").is_some(),
            "the sharer's cell appears in the viewer's mirror"
        );
    }

    #[test]
    fn applying_the_same_frame_twice_is_idempotent() {
        let mut viewer = ViewerSession::new();
        let frame = sharer_frame_with_cell("top");
        viewer.apply_frame(&frame).expect("first apply");
        viewer
            .apply_frame(&frame)
            .expect("second apply is harmless");
        // The cell exists exactly once with its single shape (no duplication).
        let cell = viewer.document().cell("top").expect("cell present");
        assert_eq!(cell.shapes.len(), 1, "a replayed frame does not duplicate");
    }

    #[test]
    fn records_sharer_presence_cursor_and_selection() {
        let mut viewer = ViewerSession::new();
        assert!(viewer.sharer().is_none());

        let mut p = Presence::new("sharer");
        p.cursor = Point::new(120, -45);
        p.selection = vec!["top/shape-1".to_owned()];
        p.viewport = Rect::new(Point::new(0, 0), Point::new(1000, 800));
        viewer.apply_presence(p);

        let sharer = viewer.sharer().expect("sharer presence recorded");
        assert_eq!(sharer.cursor, Point::new(120, -45));
        assert_eq!(sharer.selection, vec!["top/shape-1".to_owned()]);
    }

    #[test]
    fn first_presence_fixes_the_followed_sharer() {
        let mut viewer = ViewerSession::new();
        // The first actor seen becomes the sharer; a later different actor does not
        // hijack follow-mode.
        viewer.apply_presence(Presence::new("sharer"));
        viewer.apply_presence(Presence::new("someone-else"));
        assert_eq!(viewer.sharer().map(|p| p.actor.as_str()), Some("sharer"));
        // But every actor is still tracked for rendering.
        assert_eq!(viewer.awareness().len(), 2);
    }

    #[test]
    fn follow_camera_frames_and_centers_the_sharer_viewport() {
        let viewport = Rect::new(Point::new(1000, 2000), Point::new(5000, 4000));
        let cam = follow_camera(viewport, &screen());
        // Centered on the middle of the sharer's viewport.
        assert_eq!(cam.center(), Point::new(3000, 3000));
        // The whole sharer viewport is visible in the framed camera.
        let vis = cam.visible_world_rect(&screen());
        assert!(
            vis.min.x <= viewport.min.x && vis.max.x >= viewport.max.x,
            "x not framed: {vis:?} vs {viewport:?}"
        );
        assert!(
            vis.min.y <= viewport.min.y && vis.max.y >= viewport.max.y,
            "y not framed: {vis:?} vs {viewport:?}"
        );
    }

    #[test]
    fn follow_off_keeps_the_local_camera_independent() {
        let mut viewer = ViewerSession::new();
        // The viewer pans on its own; with follow off, presence never moves it.
        viewer.camera_mut().pan_pixels(50.0, 30.0);
        let panned = viewer.camera();

        let mut p = Presence::new("sharer");
        p.viewport = Rect::new(Point::new(9000, 9000), Point::new(10000, 10000));
        viewer.apply_presence(p);

        let moved = viewer.sync_camera(&screen());
        assert!(!moved, "sync_camera is a no-op when follow is off");
        assert_eq!(viewer.camera(), panned, "the local camera is untouched");
    }

    #[test]
    fn follow_on_snaps_the_camera_to_the_sharer() {
        let mut viewer = ViewerSession::new();
        let viewport = Rect::new(Point::new(-2000, -1000), Point::new(2000, 1000));
        let mut p = Presence::new("sharer");
        p.viewport = viewport;
        viewer.apply_presence(p);

        assert!(viewer.toggle_follow(), "toggling turns follow on");
        let moved = viewer.sync_camera(&screen());
        assert!(moved, "sync_camera snaps the camera when following");
        // The viewer now frames exactly the sharer's viewport.
        assert_eq!(viewer.camera(), follow_camera(viewport, &screen()));
        assert_eq!(viewer.camera().center(), Point::ORIGIN);
    }

    #[test]
    fn follow_tracks_a_moving_sharer_viewport() {
        let mut viewer = ViewerSession::new();
        viewer.set_follow(true);

        // First viewport.
        let mut p1 = Presence::new("sharer");
        p1.viewport = Rect::new(Point::new(0, 0), Point::new(1000, 1000));
        viewer.apply_presence(p1);
        viewer.sync_camera(&screen());
        assert_eq!(viewer.camera().center(), Point::new(500, 500));

        // The sharer pans; the next presence + sync moves the viewer along.
        let mut p2 = Presence::new("sharer");
        p2.viewport = Rect::new(Point::new(4000, 4000), Point::new(5000, 5000));
        viewer.apply_presence(p2);
        viewer.sync_camera(&screen());
        assert_eq!(viewer.camera().center(), Point::new(4500, 4500));
    }

    #[test]
    fn color_for_actor_is_stable_and_within_the_palette() {
        // The same id always yields the same color, and it is always a palette entry.
        let a = super::color_for_actor("alice");
        let b = super::color_for_actor("alice");
        assert_eq!(a, b, "color is deterministic per actor");
        assert!(
            super::COLLAB_PALETTE.contains(&a),
            "color is from the palette"
        );
        // Different ids generally land on distinct colors (these three are chosen to).
        let names = ["alice", "bob", "carol"];
        let colors: Vec<u32> = names.iter().map(|n| super::color_for_actor(n)).collect();
        let mut uniq = colors.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(
            uniq.len(),
            names.len(),
            "distinct actors get distinct colors"
        );
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact comparison against the fade's constant endpoints
    fn idle_alpha_fades_from_full_to_the_floor() {
        assert_eq!(
            super::idle_alpha(0.0),
            1.0,
            "a fresh cursor is fully opaque"
        );
        assert_eq!(
            super::idle_alpha(super::IDLE_FADE_START),
            1.0,
            "still opaque at the fade start"
        );
        assert_eq!(
            super::idle_alpha(super::IDLE_FADE_END),
            super::IDLE_MIN_ALPHA,
            "reaches the floor at the fade end"
        );
        assert_eq!(
            super::idle_alpha(1_000.0),
            super::IDLE_MIN_ALPHA,
            "never fades below the floor"
        );
        // Monotonically non-increasing across the fade window.
        let mid = super::idle_alpha(f32::midpoint(super::IDLE_FADE_START, super::IDLE_FADE_END));
        assert!(mid < 1.0 && mid > super::IDLE_MIN_ALPHA, "eases in between");
    }

    #[test]
    fn participants_excludes_self_and_sorts_stably() {
        use reticle_sync::Awareness;
        let mut aw = Awareness::new();
        let mut sharer = Presence::new("sharer");
        sharer.display_name = "Ada".to_owned();
        sharer.color_rgba = 0x11_22_33_ff;
        aw.set(sharer);
        aw.set(Presence::new("zoe-42")); // no name, no color
        aw.set(Presence::new("viewer")); // the local actor, excluded

        let ps = super::participants(&aw, super::VIEWER_ACTOR);
        assert_eq!(ps.len(), 2, "the local viewer is not counted");
        // Sorted by actor id: "sharer" < "zoe-42".
        assert_eq!(ps[0].actor, "sharer");
        assert_eq!(ps[0].name, "Ada", "published name wins");
        assert_eq!(ps[0].color_rgba, 0x11_22_33_ff, "published color wins");
        assert_eq!(ps[1].actor, "zoe-42");
        assert_eq!(ps[1].name, "zoe", "name falls back to a short actor label");
        assert_eq!(
            ps[1].color_rgba,
            super::color_for_actor("zoe-42"),
            "color falls back to the palette"
        );
    }

    #[test]
    fn lerp_camera_lands_on_the_target_at_t_one() {
        let from = ViewCamera::new(Point::new(0, 0), 1.0);
        let to = ViewCamera::new(Point::new(1000, 2000), 4.0);
        let snapped = super::lerp_camera(from, to, 1.0);
        assert_eq!(snapped.center(), to.center());
        assert!((snapped.pixels_per_dbu() - to.pixels_per_dbu()).abs() < 1e-9);
        // A partial step moves toward the target without overshooting.
        let half = super::lerp_camera(from, to, 0.5);
        assert_eq!(half.center(), Point::new(500, 1000));
    }

    #[test]
    fn follow_step_eases_toward_the_sharer() {
        let mut viewer = ViewerSession::new();
        let viewport = Rect::new(Point::new(0, 0), Point::new(1000, 1000));
        let mut p = Presence::new("sharer");
        p.viewport = viewport;
        viewer.apply_presence(p);
        viewer.set_follow(true);

        // A fractional step moves but does not yet arrive.
        assert!(viewer.follow_step(&screen(), 0.25));
        let snapped = follow_camera(viewport, &screen());
        assert_ne!(
            viewer.camera().center(),
            snapped.center(),
            "a partial step has not arrived"
        );
        // t == 1.0 snaps exactly, matching sync_camera.
        assert!(viewer.follow_step(&screen(), 1.0));
        assert_eq!(viewer.camera(), snapped);
    }

    #[test]
    fn follow_with_no_sharer_viewport_is_a_no_op() {
        let mut viewer = ViewerSession::new();
        viewer.set_follow(true);
        // No presence at all: nothing to follow.
        assert!(!viewer.sync_camera(&screen()));
        // A presence with a degenerate (empty) viewport is also skipped.
        let mut p = Presence::new("sharer");
        p.viewport = Rect::new(Point::new(7, 7), Point::new(7, 7));
        viewer.apply_presence(p);
        assert!(
            !viewer.sync_camera(&screen()),
            "an empty viewport is not framed"
        );
    }
}
