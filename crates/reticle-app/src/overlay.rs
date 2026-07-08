//! The canvas overlay layout manager: collision-free chrome placement by construction.
//!
//! The canvas hosts several floating readouts on top of the geometry - the streaming
//! HUD, the minimap, a legend - and in v8.0 they were each anchored with their own
//! hardcoded offsets, so they overlapped the rulers and each other at common sizes
//! (audit AUD-01/02/08: the HUD sat under the ruler numbers and the minimap crowded
//! the top ruler's right end). This module makes those collisions *impossible by
//! construction* rather than by careful hand-tuning: overlays are placed by anchor
//! into a content region that already excludes the ruler bars, and every panel is
//! checked against the ones placed before it, so two overlays can never intersect and
//! none can ever cross a ruler.
//!
//! # Pure by construction
//!
//! Like [`crate::minimap`] and [`crate::camera`], this is egui-free coordinate math over
//! [`ScreenRect`], so the placement rules are unit-tested without a window. The app module
//! feeds it the canvas rectangle and the sizes each overlay wants, and paints into the
//! rectangles it returns; the guarantee (no two returned rects overlap, none touches a
//! ruler) is proven here in tests, not asserted by eye over a screenshot.

use crate::camera::ScreenRect;

/// Which canvas corner an overlay anchors to.
///
/// Each overlay owns a corner so the common case never collides; the placer still
/// verifies every pair, so a short or narrow canvas resolves to a stack or a drop rather
/// than an overlap.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Anchor {
    /// Top-left, below the top ruler and right of the left ruler (the streaming HUD).
    TopLeft,
    /// Top-right (the minimap overview).
    TopRight,
    /// Bottom-left.
    BottomLeft,
    /// Bottom-right (legends).
    BottomRight,
}

impl Anchor {
    /// Whether this anchor sits along the top edge (panels stack downward from it).
    #[must_use]
    fn is_top(self) -> bool {
        matches!(self, Self::TopLeft | Self::TopRight)
    }

    /// Whether this anchor sits along the left edge.
    #[must_use]
    fn is_left(self) -> bool {
        matches!(self, Self::TopLeft | Self::BottomLeft)
    }
}

/// The uniform gap between an overlay and the content edges, and between stacked
/// overlays, in screen pixels. Matches the minimap's historical margin so nothing shifts
/// visually when routed through the manager.
pub const MARGIN: f32 = 12.0;

/// Places canvas overlays into a ruler-free content region without overlaps.
///
/// Construct one per frame with [`OverlayLayout::new`] (the canvas rectangle and the
/// ruler bar width), then call [`OverlayLayout::place`] once per overlay in priority
/// order. Each call returns the rectangle to paint into, or `None` when the overlay
/// cannot fit at its anchor without crossing a ruler or overlapping a
/// higher-priority overlay already placed - visibility yields before a collision does.
#[derive(Clone, Debug)]
pub struct OverlayLayout {
    /// The canvas area minus the ruler bars and a uniform margin: the region overlays
    /// live inside. Placement never returns a rectangle outside it, so no overlay can
    /// ever paint over a ruler.
    content: ScreenRect,
    /// Rectangles already handed out this frame, so later placements can avoid them.
    placed: Vec<ScreenRect>,
}

impl OverlayLayout {
    /// Builds a layout for `canvas` whose top and left `ruler_bar` pixels are covered by
    /// the ruler bars.
    ///
    /// The content region insets the canvas by the ruler bars (top and left) and a
    /// uniform [`MARGIN`] on every side, so an overlay placed flush to any content edge
    /// still clears the rulers and the canvas border.
    #[must_use]
    pub fn new(canvas: &ScreenRect, ruler_bar: f32) -> Self {
        let left = canvas.left + ruler_bar + MARGIN;
        let top = canvas.top + ruler_bar + MARGIN;
        let width = (canvas.width - ruler_bar - 2.0 * MARGIN).max(0.0);
        let height = (canvas.height - ruler_bar - 2.0 * MARGIN).max(0.0);
        Self {
            content: ScreenRect::new(left, top, width, height),
            placed: Vec::new(),
        }
    }

    /// The ruler-free content region overlays are placed inside.
    #[must_use]
    pub fn content(&self) -> ScreenRect {
        self.content
    }

    /// Places a `w` x `h` overlay at `anchor`, returning the rectangle to paint into.
    ///
    /// The overlay is positioned flush into its corner of the content region, then
    /// slid *along the anchor's vertical axis* (down from a top anchor, up from a bottom
    /// anchor) just past any already-placed overlay it would overlap. It returns `None`
    /// if the overlay does not fit in the content region at all, or if sliding to clear a
    /// prior overlay would push it outside the region: a lower-priority overlay hides
    /// rather than collide. A returned rectangle is guaranteed to lie inside
    /// [`content`](Self::content) and to be disjoint from every rectangle this layout has
    /// already returned.
    #[must_use]
    pub fn place(&mut self, anchor: Anchor, w: f32, h: f32) -> Option<ScreenRect> {
        // Too big to ever fit inside the content region: nothing sensible to show.
        if w > self.content.width || h > self.content.height || w <= 0.0 || h <= 0.0 {
            return None;
        }
        let left = if anchor.is_left() {
            self.content.left
        } else {
            self.content.left + self.content.width - w
        };
        // The vertical anchor edge and the far edge the slide may not cross.
        let (mut top, limit) = if anchor.is_top() {
            (self.content.top, self.content.top + self.content.height)
        } else {
            (self.content.top + self.content.height - h, self.content.top)
        };

        // Slide past any overlapping prior placement. Each collision moves the panel a
        // bounded distance (past one rectangle), and there are finitely many placed
        // rectangles, so this settles.
        let mut guard = 0;
        loop {
            let candidate = ScreenRect::new(left, top, w, h);
            let Some(hit) = self.placed.iter().find(|r| overlaps(r, &candidate)) else {
                break;
            };
            if anchor.is_top() {
                let next = hit.top + hit.height + MARGIN;
                if next + h > limit {
                    return None;
                }
                top = next;
            } else {
                let next = hit.top - MARGIN - h;
                if next < limit {
                    return None;
                }
                top = next;
            }
            guard += 1;
            if guard > self.placed.len() + 1 {
                // Defensive: the geometry above is monotone, so this is unreachable, but
                // never spin.
                return None;
            }
        }
        let rect = ScreenRect::new(left, top, w, h);
        self.placed.push(rect);
        Some(rect)
    }
}

/// Whether two screen rectangles share any area (touching edges do not count as
/// overlap, so panels separated by exactly [`MARGIN`] are disjoint).
#[must_use]
fn overlaps(a: &ScreenRect, b: &ScreenRect) -> bool {
    a.left < b.left + b.width
        && b.left < a.left + a.width
        && a.top < b.top + b.height
        && b.top < a.top + a.height
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULER: f32 = 18.0;

    fn canvas() -> ScreenRect {
        ScreenRect::new(100.0, 50.0, 1000.0, 700.0)
    }

    fn right(r: &ScreenRect) -> f32 {
        r.left + r.width
    }
    fn bottom(r: &ScreenRect) -> f32 {
        r.top + r.height
    }

    #[test]
    fn content_region_excludes_the_ruler_bars() {
        let layout = OverlayLayout::new(&canvas(), RULER);
        let c = layout.content();
        let cv = canvas();
        // The content starts past the ruler bars on the top and left.
        assert!(c.left >= cv.left + RULER);
        assert!(c.top >= cv.top + RULER);
        // And stays inside the canvas on the right and bottom.
        assert!(right(&c) <= right(&cv));
        assert!(bottom(&c) <= bottom(&cv));
    }

    #[test]
    fn a_placed_overlay_never_crosses_a_ruler() {
        let mut layout = OverlayLayout::new(&canvas(), RULER);
        let cv = canvas();
        for anchor in [
            Anchor::TopLeft,
            Anchor::TopRight,
            Anchor::BottomLeft,
            Anchor::BottomRight,
        ] {
            let r = OverlayLayout::new(&cv, RULER)
                .place(anchor, 200.0, 120.0)
                .expect("fits");
            assert!(r.left >= cv.left + RULER, "{anchor:?} crossed left ruler");
            assert!(r.top >= cv.top + RULER, "{anchor:?} crossed top ruler");
            assert!(right(&r) <= right(&cv), "{anchor:?} spilled right");
            assert!(bottom(&r) <= bottom(&cv), "{anchor:?} spilled bottom");
        }
        let _ = &mut layout;
    }

    #[test]
    fn corners_anchor_where_expected() {
        let mut layout = OverlayLayout::new(&canvas(), RULER);
        let c = layout.content();
        let tl = layout.place(Anchor::TopLeft, 200.0, 100.0).unwrap();
        assert!((tl.left - c.left).abs() < f32::EPSILON);
        assert!((tl.top - c.top).abs() < f32::EPSILON);

        let mut layout = OverlayLayout::new(&canvas(), RULER);
        let br = layout.place(Anchor::BottomRight, 200.0, 100.0).unwrap();
        assert!((right(&br) - right(&c)).abs() < 0.001);
        assert!((bottom(&br) - bottom(&c)).abs() < 0.001);
    }

    #[test]
    fn overlays_at_the_same_corner_stack_without_overlapping() {
        let mut layout = OverlayLayout::new(&canvas(), RULER);
        let a = layout.place(Anchor::TopLeft, 200.0, 100.0).unwrap();
        let b = layout.place(Anchor::TopLeft, 200.0, 100.0).unwrap();
        assert!(!overlaps(&a, &b), "stacked overlays must not overlap");
        // The second sits below the first with a margin.
        assert!(b.top >= bottom(&a));
    }

    #[test]
    fn opposite_corners_that_would_meet_resolve_to_no_overlap() {
        // A canvas so short that a tall top-right and a tall bottom-right overlap unless
        // the manager intervenes.
        let short = ScreenRect::new(0.0, 0.0, 400.0, 300.0);
        let mut layout = OverlayLayout::new(&short, RULER);
        let tr = layout.place(Anchor::TopRight, 180.0, 135.0).unwrap();
        let br = layout.place(Anchor::BottomRight, 180.0, 135.0);
        if let Some(br) = br {
            assert!(!overlaps(&tr, &br), "top-right and bottom-right overlapped");
        }
    }

    #[test]
    fn every_pair_of_placements_is_disjoint_under_pressure() {
        // Hammer a small canvas with many same-corner requests; whatever is handed out
        // must be pairwise disjoint and inside the content.
        let mut layout = OverlayLayout::new(&ScreenRect::new(0.0, 0.0, 500.0, 500.0), RULER);
        let c = layout.content();
        let mut out = Vec::new();
        for _ in 0..12 {
            if let Some(r) = layout.place(Anchor::TopLeft, 150.0, 90.0) {
                out.push(r);
            }
        }
        for (i, a) in out.iter().enumerate() {
            assert!(a.left >= c.left - 0.001 && a.top >= c.top - 0.001);
            assert!(right(a) <= right(&c) + 0.001 && bottom(a) <= bottom(&c) + 0.001);
            for b in &out[i + 1..] {
                assert!(!overlaps(a, b), "two handed-out overlays overlap");
            }
        }
    }

    #[test]
    fn an_overlay_larger_than_the_content_is_dropped() {
        let mut layout = OverlayLayout::new(&canvas(), RULER);
        assert!(layout.place(Anchor::TopLeft, 5000.0, 100.0).is_none());
        assert!(layout.place(Anchor::TopLeft, 100.0, 5000.0).is_none());
        assert!(layout.place(Anchor::TopLeft, 0.0, 0.0).is_none());
    }

    #[test]
    fn a_tiny_canvas_yields_no_placements() {
        // Smaller than the rulers plus margins: the content region collapses.
        let tiny = ScreenRect::new(0.0, 0.0, 20.0, 20.0);
        let mut layout = OverlayLayout::new(&tiny, RULER);
        assert!(layout.place(Anchor::TopLeft, 50.0, 50.0).is_none());
    }
}
