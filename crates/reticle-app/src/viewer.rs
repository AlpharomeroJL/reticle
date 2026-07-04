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

use reticle_geometry::Rect;
use reticle_sync::{Awareness, Presence, SyncDocument, SyncError};

use crate::camera::{ScreenRect, ViewCamera};

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
    use crate::camera::ScreenRect;
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
