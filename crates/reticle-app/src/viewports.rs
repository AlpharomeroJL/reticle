//! Multi-viewport split: two in-window panes over the shared document.
//!
//! The canvas can split into side-by-side or stacked panes, each with its own
//! [`ViewCamera`] over the same document (in-window panes, not OS windows, so the
//! split works identically on wasm). Exactly one pane is *focused*: it hosts the
//! live camera, the active tool, and every overlay, and it is the pane the GPU
//! paint callback renders (the retained renderer binds a single camera per frame,
//! and its paint path is owned by the render lane). Unfocused panes draw read-only
//! previews through the egui fallback painter.
//!
//! This module owns the window-free logic: the pane rectangle layout, pane
//! hit-testing, and the camera bookkeeping when focus moves or the split changes.
//! The app module only routes clicks and paints borders.

use crate::camera::{ScreenRect, ViewCamera};

/// The gap between panes, in screen pixels.
const DIVIDER_PX: f32 = 4.0;

/// How the canvas is divided into panes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Split {
    /// One pane filling the whole canvas.
    #[default]
    Single,
    /// Two panes side by side (a vertical divider).
    Horizontal,
    /// Two panes stacked (a horizontal divider).
    Vertical,
}

impl Split {
    /// The number of panes this split produces.
    #[must_use]
    pub fn pane_count(self) -> usize {
        match self {
            Self::Single => 1,
            Self::Horizontal | Self::Vertical => 2,
        }
    }

    /// A short toolbar label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Single => "Single",
            Self::Horizontal => "Split H",
            Self::Vertical => "Split V",
        }
    }

    /// Every split mode, in toolbar order.
    #[must_use]
    pub fn all() -> [Split; 3] {
        [Split::Single, Split::Horizontal, Split::Vertical]
    }
}

/// Whether a screen position lies inside `rect`.
#[must_use]
pub fn contains(rect: &ScreenRect, x: f32, y: f32) -> bool {
    x >= rect.left && x <= rect.left + rect.width && y >= rect.top && y <= rect.top + rect.height
}

/// Splits `screen` into the pane rectangles for `split`, in pane order.
///
/// Panes share the canvas evenly with a `DIVIDER_PX` gap between them; the
/// divider belongs to neither pane, so [`hit_pane`] treats it as dead space.
#[must_use]
pub fn pane_rects(screen: &ScreenRect, split: Split) -> Vec<ScreenRect> {
    match split {
        Split::Single => vec![*screen],
        Split::Horizontal => {
            let w = ((screen.width - DIVIDER_PX) / 2.0).max(1.0);
            vec![
                ScreenRect::new(screen.left, screen.top, w, screen.height),
                ScreenRect::new(screen.left + w + DIVIDER_PX, screen.top, w, screen.height),
            ]
        }
        Split::Vertical => {
            let h = ((screen.height - DIVIDER_PX) / 2.0).max(1.0);
            vec![
                ScreenRect::new(screen.left, screen.top, screen.width, h),
                ScreenRect::new(screen.left, screen.top + h + DIVIDER_PX, screen.width, h),
            ]
        }
    }
}

/// The index of the pane containing `(x, y)`, or `None` on the divider/outside.
#[must_use]
pub fn hit_pane(panes: &[ScreenRect], x: f32, y: f32) -> Option<usize> {
    panes.iter().position(|p| contains(p, x, y))
}

/// The pane state: the split mode, the focused pane, and per-pane cameras.
///
/// The app owns the *live* camera (the one the tools, status bar, and session all
/// use); this struct stores a camera slot per pane and swaps the live camera in
/// and out on focus changes, so the focused slot is only a stale mirror. That
/// keeps every existing `self.camera` consumer working untouched.
#[derive(Clone, Debug)]
pub struct Viewports {
    /// The current split mode.
    split: Split,
    /// The index of the focused pane (always valid for the current split).
    focused: usize,
    /// One camera slot per pane; the focused slot is stale while it holds focus.
    cameras: Vec<ViewCamera>,
}

impl Default for Viewports {
    fn default() -> Self {
        Self::new()
    }
}

impl Viewports {
    /// Creates a single-pane layout.
    #[must_use]
    pub fn new() -> Self {
        Self {
            split: Split::Single,
            focused: 0,
            cameras: vec![ViewCamera::default()],
        }
    }

    /// The current split mode.
    #[must_use]
    pub fn split(&self) -> Split {
        self.split
    }

    /// The number of panes.
    #[must_use]
    pub fn pane_count(&self) -> usize {
        self.split.pane_count()
    }

    /// The index of the focused pane.
    #[must_use]
    pub fn focused(&self) -> usize {
        self.focused
    }

    /// The pane rectangles for the current split within `screen`.
    #[must_use]
    pub fn rects(&self, screen: &ScreenRect) -> Vec<ScreenRect> {
        pane_rects(screen, self.split)
    }

    /// The stored camera for a pane, or `None` when out of range.
    ///
    /// Meaningful for *unfocused* panes; the focused pane's slot is stale while
    /// the app holds the live camera.
    #[must_use]
    pub fn camera(&self, pane: usize) -> Option<&ViewCamera> {
        self.cameras.get(pane)
    }

    /// Changes the split mode, keeping the live camera with the focused view.
    ///
    /// New panes start as copies of `live` (so a fresh split shows the same view
    /// twice); removed panes drop their cameras. If the focused pane no longer
    /// exists, focus falls back to pane 0, which keeps whatever the live camera
    /// was showing, so collapsing a split never yanks the view away.
    pub fn set_split(&mut self, split: Split, live: &ViewCamera) {
        self.split = split;
        self.cameras.resize(split.pane_count(), *live);
        if self.focused >= self.cameras.len() {
            self.focused = 0;
        }
    }

    /// Resets every pane's stored camera to the default, keeping the split mode and
    /// focused pane.
    ///
    /// Called on a document switch: the stored cameras frame the old document's world
    /// coordinates, so an inactive pane would otherwise keep showing empty old-doc
    /// space. The focused pane holds the live camera, which the caller reframes on the
    /// next frame via its deferred fit; this drops the stale mirrors behind it.
    pub fn reset_cameras(&mut self) {
        for cam in &mut self.cameras {
            *cam = ViewCamera::default();
        }
    }

    /// Moves focus to `pane`, swapping cameras through `live`.
    ///
    /// The live camera is saved into the previously focused slot and the target
    /// pane's stored camera becomes the live one. Returns `false` (and changes
    /// nothing) when `pane` is already focused or out of range.
    pub fn focus(&mut self, pane: usize, live: &mut ViewCamera) -> bool {
        if pane == self.focused || pane >= self.cameras.len() {
            return false;
        }
        self.cameras[self.focused] = *live;
        *live = self.cameras[pane];
        self.focused = pane;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_geometry::Point;

    fn screen() -> ScreenRect {
        ScreenRect::new(10.0, 20.0, 804.0, 604.0)
    }

    fn cam(x: i32, zoom: f64) -> ViewCamera {
        ViewCamera::new(Point::new(x, 0), zoom)
    }

    #[test]
    fn single_split_is_the_whole_canvas() {
        let panes = pane_rects(&screen(), Split::Single);
        assert_eq!(panes, vec![screen()]);
    }

    #[test]
    fn horizontal_split_tiles_side_by_side() {
        let panes = pane_rects(&screen(), Split::Horizontal);
        assert_eq!(panes.len(), 2);
        // Equal widths, full height, separated by exactly the divider.
        assert!((panes[0].width - panes[1].width).abs() < 1e-3);
        assert!((panes[0].height - screen().height).abs() < 1e-3);
        let gap = panes[1].left - (panes[0].left + panes[0].width);
        assert!((gap - DIVIDER_PX).abs() < 1e-3);
        // Together they span the canvas width.
        let right = panes[1].left + panes[1].width;
        assert!((right - (screen().left + screen().width)).abs() < 0.001);
    }

    #[test]
    fn vertical_split_tiles_stacked() {
        let panes = pane_rects(&screen(), Split::Vertical);
        assert_eq!(panes.len(), 2);
        assert!((panes[0].width - screen().width).abs() < 1e-3);
        let gap = panes[1].top - (panes[0].top + panes[0].height);
        assert!((gap - DIVIDER_PX).abs() < 1e-3);
        let bottom = panes[1].top + panes[1].height;
        assert!((bottom - (screen().top + screen().height)).abs() < 0.001);
    }

    #[test]
    fn hit_pane_finds_panes_and_misses_the_divider() {
        let panes = pane_rects(&screen(), Split::Horizontal);
        let left_center = (panes[0].center_x(), panes[0].center_y());
        let right_center = (panes[1].center_x(), panes[1].center_y());
        assert_eq!(hit_pane(&panes, left_center.0, left_center.1), Some(0));
        assert_eq!(hit_pane(&panes, right_center.0, right_center.1), Some(1));
        // The divider midpoint belongs to neither pane.
        let divider_x = panes[0].left + panes[0].width + DIVIDER_PX / 2.0;
        assert_eq!(hit_pane(&panes, divider_x, panes[0].center_y()), None);
        // Outside the canvas entirely.
        assert_eq!(hit_pane(&panes, -1000.0, -1000.0), None);
    }

    #[test]
    fn focus_swaps_the_live_camera_with_the_stored_one() {
        let mut vp = Viewports::new();
        let mut live = cam(100, 2.0);
        vp.set_split(Split::Horizontal, &live);
        // Diverge pane 1's stored camera, then focus it.
        let pane1 = cam(900, 0.5);
        vp.focus(1, &mut live);
        assert_eq!(vp.focused(), 1);
        // Pane 1 started as a copy of the live camera at split time.
        assert_eq!(live, cam(100, 2.0));
        // Change the live view while pane 1 is focused, then focus back.
        live = pane1;
        assert!(vp.focus(0, &mut live));
        assert_eq!(vp.focused(), 0);
        // Pane 0 got back exactly the camera it had when it lost focus.
        assert_eq!(live, cam(100, 2.0));
        // And pane 1's slot preserved the diverged view.
        assert_eq!(vp.camera(1), Some(&pane1));
    }

    #[test]
    fn focus_rejects_self_and_out_of_range() {
        let mut vp = Viewports::new();
        let mut live = cam(0, 1.0);
        vp.set_split(Split::Vertical, &live);
        assert!(!vp.focus(0, &mut live), "already focused");
        assert!(!vp.focus(7, &mut live), "out of range");
        assert_eq!(vp.focused(), 0);
        assert_eq!(live, cam(0, 1.0));
    }

    #[test]
    fn set_split_seeds_new_panes_from_the_live_camera() {
        let mut vp = Viewports::new();
        let live = cam(42, 3.0);
        vp.set_split(Split::Horizontal, &live);
        assert_eq!(vp.pane_count(), 2);
        assert_eq!(vp.camera(1), Some(&live));
    }

    #[test]
    fn collapsing_a_split_keeps_the_focused_view_live() {
        let mut vp = Viewports::new();
        let mut live = cam(1, 1.0);
        vp.set_split(Split::Horizontal, &live);
        vp.focus(1, &mut live);
        // The user pans pane 1, then collapses to a single pane.
        live = cam(555, 4.0);
        vp.set_split(Split::Single, &live);
        assert_eq!(vp.focused(), 0, "focus falls back to pane 0");
        assert_eq!(vp.pane_count(), 1);
        // The live camera is untouched: the view the user was on survives.
        assert_eq!(live, cam(555, 4.0));
    }

    #[test]
    fn contains_checks_all_edges() {
        let r = ScreenRect::new(0.0, 0.0, 100.0, 50.0);
        assert!(contains(&r, 0.0, 0.0));
        assert!(contains(&r, 100.0, 50.0));
        assert!(!contains(&r, 100.1, 25.0));
        assert!(!contains(&r, 50.0, -0.1));
    }
}
