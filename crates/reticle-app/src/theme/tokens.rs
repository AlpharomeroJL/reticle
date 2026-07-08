//! Semantic design tokens: the data half of the theme.
//!
//! These constants encode `docs/design/tokens.md` exactly; the mapping onto
//! `egui::Style` lives in `apply` (lane 1A) and the WCAG proofs in `contrast`
//! (lane 1A). Chrome tokens and canvas tokens are distinct on purpose: chrome
//! is the quiet UI shell, canvas tokens are the few named colors the canvas
//! overlays need (selection, guides, tour highlight) so data color never
//! masquerades as chrome color.

use eframe::egui::{Color32, Vec2};

/// The semantic chrome palette. One value per *role*, never per widget; the
/// `apply` mapping decides which egui fields each role feeds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tokens {
    /// Canvas clear color (kept from v8.0; the hero backdrop).
    pub bg_canvas: Color32,
    /// Side, top, and bottom panel fill.
    pub bg_panel: Color32,
    /// Menus, popovers, cards, and modal frames (one step raised).
    pub bg_raised: Color32,
    /// Text inputs, wells, and readout fields (one step sunken).
    pub bg_input: Color32,
    /// Hairline separators and widget outlines.
    pub border: Color32,
    /// Panel edges and emphasized separators.
    pub border_strong: Color32,
    /// Primary text.
    pub text: Color32,
    /// Secondary text: captions, hints, section headers.
    pub text_weak: Color32,
    /// Disabled text only (WCAG exempts disabled controls from AA).
    pub text_faint: Color32,
    /// Links, focused borders, primary-action fills.
    pub accent: Color32,
    /// Text on accent fills.
    pub accent_text: Color32,
    /// Selection fills and active-toggle backgrounds.
    pub accent_muted: Color32,
    /// Errors and destructive actions.
    pub danger: Color32,
    /// Warnings and stale indicators.
    pub warning: Color32,
    /// Confirmations and pass states.
    pub success: Color32,
    /// The keyboard-focus ring (accent hue, 1.5 px stroke).
    pub focus: Color32,
    /// Inactive widget fill.
    pub widget_bg: Color32,
    /// Hovered widget fill.
    pub widget_hover: Color32,
    /// Pressed or open widget fill.
    pub widget_active: Color32,
}

/// The one shipped theme this packet (docs/design/tokens.md records why light
/// is deferred and how its future table slots in).
pub const DARK: Tokens = Tokens {
    bg_canvas: Color32::from_rgb(16, 18, 22),
    bg_panel: Color32::from_rgb(22, 24, 29),
    bg_raised: Color32::from_rgb(28, 31, 38),
    bg_input: Color32::from_rgb(13, 15, 18),
    border: Color32::from_rgb(42, 46, 55),
    border_strong: Color32::from_rgb(61, 67, 80),
    text: Color32::from_rgb(226, 229, 234),
    text_weak: Color32::from_rgb(154, 160, 171),
    text_faint: Color32::from_rgb(106, 113, 128),
    accent: Color32::from_rgb(110, 168, 254),
    accent_text: Color32::from_rgb(12, 18, 32),
    accent_muted: Color32::from_rgb(37, 64, 107),
    danger: Color32::from_rgb(244, 112, 103),
    warning: Color32::from_rgb(229, 192, 123),
    success: Color32::from_rgb(87, 171, 90),
    focus: Color32::from_rgb(110, 168, 254),
    widget_bg: Color32::from_rgb(34, 38, 46),
    widget_hover: Color32::from_rgb(42, 47, 57),
    widget_active: Color32::from_rgb(50, 56, 69),
};

/// Named canvas-overlay colors (not chrome; the technology table still owns
/// per-layer data colors). These are the colors the canvas overlays, HUDs, and
/// annotations paint with. They are a distinct namespace from [`Tokens`] so
/// data color never masquerades as chrome color, but they live here so the
/// check-style lint has one legal home for every color literal and a palette
/// change is still a one-file edit. Values are the shipped v8.0 overlay colors,
/// preserved as lane 1A drained the literals (a handful of near-duplicate grays
/// were unified to a single token). Semi-transparent members carry their alpha;
/// only the [`Tokens`] chrome pairs are contrast-tested, so overlay alpha is
/// allowed here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CanvasTokens {
    /// Background grid lines.
    pub grid_line: Color32,
    /// The emphasized world x/y axes.
    pub grid_axis: Color32,
    /// The border around an unfocused split pane and the minimap panel.
    pub pane_border: Color32,
    /// The focused-pane border in split view.
    pub pane_focus: Color32,
    /// The top/left ruler bar fill.
    pub ruler_bg: Color32,
    /// Ruler tick marks.
    pub ruler_tick: Color32,
    /// Ruler and section-plot numeric labels.
    pub hud_label: Color32,
    /// Primary text on a canvas HUD/readout panel.
    pub hud_text: Color32,
    /// Secondary/dim text on a canvas HUD panel.
    pub hud_text_dim: Color32,
    /// Translucent backdrop behind a canvas HUD/readout panel.
    pub hud_panel: Color32,
    /// The full-window dim veil behind the file-drop affordance.
    pub scrim: Color32,
    /// The dashed frame of the file-drop affordance.
    pub drop_frame: Color32,
    /// The document-bounds outline in the minimap.
    pub minimap_doc: Color32,
    /// The translucent placement boxes in the minimap.
    pub minimap_fill: Color32,
    /// The camera-viewport rectangle in the minimap.
    pub minimap_viewport: Color32,
    /// The outline of selected geometry.
    pub selection: Color32,
    /// Cell-bounding-box outline at the low-zoom level of detail.
    pub cell_box: Color32,
    /// Translucent cell-bounding-box fill.
    pub cell_box_fill: Color32,
    /// The highlighted-net outline.
    pub net_highlight: Color32,
    /// The array-tool live preview.
    pub array_preview: Color32,
    /// The generate-tool live preview.
    pub generate_preview: Color32,
    /// The active drawing tool's rubber-band preview.
    pub draw_preview: Color32,
    /// A DRC violation marker (also the diff overlay's removed color).
    pub drc_violation: Color32,
    /// The selected DRC violation marker (hotter than the rest).
    pub drc_selected: Color32,
    /// The DRC-as-you-type underline squiggle.
    pub live_drc: Color32,
    /// Diff overlay: an added shape.
    pub diff_added: Color32,
    /// Diff overlay: a removed shape.
    pub diff_removed: Color32,
    /// Diff overlay: a changed shape.
    pub diff_changed: Color32,
    /// An anchored comment pin at rest.
    pub comment_pin: Color32,
    /// The selected comment pin.
    pub comment_pin_selected: Color32,
    /// The measurement overlay (line, handles, readout).
    pub measure: Color32,
    /// Draggable ruler guides and the snap "guide" indicator.
    pub guide: Color32,
    /// Snap indicator: a vertex hit.
    pub snap_vertex: Color32,
    /// Snap indicator: an edge midpoint.
    pub snap_midpoint: Color32,
    /// Snap indicator: a bounding-box center.
    pub snap_center: Color32,
    /// Snap indicator: an edge hit.
    pub snap_edge: Color32,
    /// The agent's cursor crosshair.
    pub agent_cursor: Color32,
    /// The first-run tour's highlight box.
    pub tour_highlight: Color32,
    /// The neutral fallback fill for a layer with no table entry.
    pub layer_fallback: Color32,
}

/// The shipped canvas-overlay palette.
pub const CANVAS: CanvasTokens = CanvasTokens {
    grid_line: Color32::from_rgb(34, 38, 46),
    grid_axis: Color32::from_rgb(60, 66, 78),
    pane_border: Color32::from_rgb(70, 76, 90),
    pane_focus: Color32::from_rgb(110, 160, 255),
    ruler_bg: Color32::from_rgb(24, 27, 33),
    ruler_tick: Color32::from_rgb(90, 96, 110),
    hud_label: Color32::from_rgb(170, 176, 190),
    hud_text: Color32::from_rgb(210, 224, 240),
    hud_text_dim: Color32::from_rgb(150, 156, 170),
    hud_panel: Color32::from_rgba_unmultiplied_const(16, 18, 22, 220),
    scrim: Color32::from_rgba_unmultiplied_const(8, 10, 16, 180),
    drop_frame: Color32::from_rgb(120, 170, 255),
    minimap_doc: Color32::from_rgb(90, 100, 120),
    minimap_fill: Color32::from_rgba_unmultiplied_const(90, 120, 170, 90),
    minimap_viewport: Color32::from_rgb(255, 210, 90),
    selection: Color32::from_rgb(255, 240, 120),
    cell_box: Color32::from_rgb(120, 140, 180),
    cell_box_fill: Color32::from_rgba_unmultiplied_const(60, 80, 120, 40),
    net_highlight: Color32::from_rgb(120, 230, 255),
    array_preview: Color32::from_rgb(180, 210, 120),
    generate_preview: Color32::from_rgb(120, 190, 235),
    draw_preview: Color32::from_rgb(120, 200, 255),
    drc_violation: Color32::from_rgb(255, 90, 90),
    drc_selected: Color32::from_rgb(255, 200, 60),
    live_drc: Color32::from_rgb(255, 120, 90),
    diff_added: Color32::from_rgb(90, 220, 120),
    diff_removed: Color32::from_rgb(255, 90, 90),
    diff_changed: Color32::from_rgb(255, 190, 60),
    comment_pin: Color32::from_rgb(90, 160, 255),
    comment_pin_selected: Color32::from_rgb(255, 210, 90),
    measure: Color32::from_rgb(255, 210, 90),
    guide: Color32::from_rgb(80, 200, 220),
    snap_vertex: Color32::from_rgb(120, 230, 140),
    snap_midpoint: Color32::from_rgb(230, 200, 110),
    snap_center: Color32::from_rgb(220, 140, 220),
    snap_edge: Color32::from_rgb(120, 190, 240),
    agent_cursor: Color32::from_rgb(235, 80, 220),
    tour_highlight: Color32::from_rgb(255, 196, 0),
    layer_fallback: Color32::from_rgb(150, 150, 150),
};

/// Builds an opaque canvas color from a layer's raw `(r, g, b)` channels.
///
/// Canvas geometry is colored by the technology table, where color IS the data;
/// this is the one legal constructor for that, keeping the raw `Color32` call
/// inside the theme module so the check-style lint stays absolute everywhere
/// else.
#[must_use]
pub fn layer_rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Builds a canvas color from a layer's raw `(r, g, b, a)` channels (the fill
/// alpha is the layer's own translucency).
#[must_use]
pub fn layer_rgba(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Returns `color` at overlay alpha `a`, for a translucent fill derived from an
/// opaque overlay or layer color.
#[must_use]
pub fn with_alpha(color: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a)
}

/// Density modes: the same tokens at two spatial rhythms (4 px grid).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Density {
    /// The default rhythm.
    #[default]
    Comfortable,
    /// The tighter rhythm for small screens or dense work.
    Compact,
}

impl Density {
    /// `Spacing::item_spacing`.
    #[must_use]
    pub fn item_spacing(self) -> Vec2 {
        match self {
            Self::Comfortable => Vec2::new(8.0, 6.0),
            Self::Compact => Vec2::new(6.0, 4.0),
        }
    }

    /// `Spacing::button_padding`.
    #[must_use]
    pub fn button_padding(self) -> Vec2 {
        match self {
            Self::Comfortable => Vec2::new(10.0, 5.0),
            Self::Compact => Vec2::new(8.0, 3.0),
        }
    }

    /// Minimum interactive height (`Spacing::interact_size.y`); touch mode
    /// (lane 4B) raises this to [`TOUCH_INTERACT_HEIGHT`] on top of either
    /// density.
    #[must_use]
    pub fn interact_height(self) -> f32 {
        match self {
            Self::Comfortable => 28.0,
            Self::Compact => 22.0,
        }
    }

    /// The minimum interactive height for this density with touch mode either
    /// on or off (lane 4B).
    ///
    /// Touch mode raises the hit target to [`TOUCH_INTERACT_HEIGHT`] (40) on
    /// top of either density's resting [`interact_height`](Self::interact_height),
    /// so toolbar, menu, and panel controls meet the touch minimum on tablet and
    /// phone; `touch = false` keeps the density's resting height. The floor is a
    /// `max`, not a swap, so a density whose resting height ever exceeds 40 keeps
    /// its taller target rather than shrinking under touch.
    #[must_use]
    pub fn touch_interact_height(self, touch: bool) -> f32 {
        if touch {
            self.interact_height().max(TOUCH_INTERACT_HEIGHT)
        } else {
            self.interact_height()
        }
    }

    /// `Spacing::window_margin` (uniform).
    #[must_use]
    pub fn window_margin(self) -> f32 {
        match self {
            Self::Comfortable => 12.0,
            Self::Compact => 8.0,
        }
    }

    /// `Spacing::menu_margin` (uniform).
    #[must_use]
    pub fn menu_margin(self) -> f32 {
        match self {
            Self::Comfortable => 8.0,
            Self::Compact => 6.0,
        }
    }

    /// `Spacing::indent`.
    #[must_use]
    pub fn indent(self) -> f32 {
        match self {
            Self::Comfortable => 16.0,
            Self::Compact => 12.0,
        }
    }

    /// `Spacing::icon_width` (checkbox/radio/collapsing marker box).
    #[must_use]
    pub fn icon_width(self) -> f32 {
        match self {
            Self::Comfortable => 16.0,
            Self::Compact => 14.0,
        }
    }

    /// `Spacing::icon_width_inner` (the drawn glyph inside the marker box).
    #[must_use]
    pub fn icon_width_inner(self) -> f32 {
        match self {
            Self::Comfortable => 10.0,
            Self::Compact => 8.0,
        }
    }

    /// `Spacing::icon_spacing` (gap between a marker and its label).
    #[must_use]
    pub fn icon_spacing(self) -> f32 {
        match self {
            Self::Comfortable => 6.0,
            Self::Compact => 4.0,
        }
    }

    /// `Spacing::combo_height` (max height of an open combo-box popup).
    #[must_use]
    pub fn combo_height(self) -> f32 {
        match self {
            Self::Comfortable => 240.0,
            Self::Compact => 200.0,
        }
    }

    /// The stable text tag used when persisting the density.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Comfortable => "comfortable",
            Self::Compact => "compact",
        }
    }

    /// Parses a persisted density tag, defaulting to [`Density::Comfortable`].
    #[must_use]
    pub fn from_tag(tag: &str) -> Self {
        match tag.trim().to_ascii_lowercase().as_str() {
            "compact" => Self::Compact,
            _ => Self::Comfortable,
        }
    }

    /// Text sizes per egui text style: (small, body, button, heading, mono).
    #[must_use]
    pub fn type_scale(self) -> TypeScale {
        match self {
            Self::Comfortable => TypeScale {
                small: 11.0,
                body: 13.0,
                button: 13.0,
                heading: 17.0,
                mono: 12.5,
            },
            Self::Compact => TypeScale {
                small: 10.0,
                body: 12.0,
                button: 12.0,
                heading: 15.5,
                mono: 11.5,
            },
        }
    }

    /// Functional-transition length in seconds; zero when reduced motion is on.
    #[must_use]
    pub fn animation_time(self, reduced_motion: bool) -> f32 {
        if reduced_motion {
            return 0.0;
        }
        match self {
            Self::Comfortable => 0.12,
            Self::Compact => 0.10,
        }
    }
}

/// Point sizes for the five egui text styles at one density.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TypeScale {
    /// `TextStyle::Small`.
    pub small: f32,
    /// `TextStyle::Body`.
    pub body: f32,
    /// `TextStyle::Button`.
    pub button: f32,
    /// `TextStyle::Heading`.
    pub heading: f32,
    /// `TextStyle::Monospace`.
    pub mono: f32,
}

/// Minimum touch-target height (lane 4B applies it over either density).
pub const TOUCH_INTERACT_HEIGHT: f32 = 40.0;

/// Corner radius for chips, kbd hints, and swatches.
pub const RADIUS_SM: u8 = 2;
/// Corner radius for buttons, inputs, and interactive widgets.
pub const RADIUS_MD: u8 = 4;
/// Corner radius for menus, popovers, and toasts.
pub const RADIUS_LG: u8 = 6;
/// Corner radius for windows, modal frames, and cards.
pub const RADIUS_XL: u8 = 8;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tested_tokens_are_opaque() {
        // Color32 is premultiplied; the contrast math (lane 1A) is only valid
        // on opaque colors, so the palette must ship fully opaque.
        let t = DARK;
        for c in [
            t.bg_canvas,
            t.bg_panel,
            t.bg_raised,
            t.bg_input,
            t.border,
            t.border_strong,
            t.text,
            t.text_weak,
            t.text_faint,
            t.accent,
            t.accent_text,
            t.accent_muted,
            t.danger,
            t.warning,
            t.success,
            t.focus,
            t.widget_bg,
            t.widget_hover,
            t.widget_active,
        ] {
            assert_eq!(c.a(), 255);
        }
    }

    #[test]
    fn densities_stay_on_the_4px_half_grid() {
        for d in [Density::Comfortable, Density::Compact] {
            for v in [
                d.item_spacing().x,
                d.item_spacing().y,
                d.button_padding().x,
                d.window_margin(),
                d.menu_margin(),
                d.indent(),
            ] {
                assert!(
                    (v * 2.0).fract().abs() < f32::EPSILON,
                    "{v} is off the 2px half-step grid"
                );
            }
        }
    }

    #[test]
    fn reduced_motion_zeroes_animation() {
        assert!(Density::Comfortable.animation_time(true).abs() < f32::EPSILON);
        assert!(Density::Comfortable.animation_time(false) > 0.0);
    }

    #[test]
    fn touch_raises_hit_targets_to_the_touch_minimum() {
        // Touch mode lifts every density to the 40px touch floor, so a tablet or
        // phone tap lands a target regardless of the resting density.
        for d in [Density::Comfortable, Density::Compact] {
            assert!(
                (d.touch_interact_height(true) - TOUCH_INTERACT_HEIGHT).abs() < f32::EPSILON,
                "touch mode must reach {TOUCH_INTERACT_HEIGHT}px on {d:?}"
            );
            // With touch off the density keeps its resting height (28 / 22), so
            // desktop chrome does not grow.
            assert!(
                (d.touch_interact_height(false) - d.interact_height()).abs() < f32::EPSILON,
                "touch off must keep the resting height on {d:?}"
            );
        }
    }

    #[test]
    fn touch_floor_clears_both_resting_densities() {
        // The `max` floor only lifts, never shrinks: both resting heights are
        // below 40, so touch always raises rather than lowering a target.
        for d in [Density::Comfortable, Density::Compact] {
            assert!(d.interact_height() < TOUCH_INTERACT_HEIGHT);
            assert!(d.touch_interact_height(true) >= d.interact_height());
        }
    }
}
