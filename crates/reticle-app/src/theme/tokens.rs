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
/// layer colors). Lane 1A moves the remaining canvas literals here as it
/// drains `app.rs`; the initial set covers the overlays the audit names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CanvasTokens {
    /// Selection outlines and handles on the canvas.
    pub selection: Color32,
    /// The focused-pane border in split view.
    pub pane_focus: Color32,
    /// Draggable ruler guides.
    pub guide: Color32,
    /// The first-run tour's highlight box.
    pub tour_highlight: Color32,
}

/// The shipped canvas-overlay palette.
pub const CANVAS: CanvasTokens = CanvasTokens {
    selection: Color32::from_rgb(110, 168, 254),
    pane_focus: Color32::from_rgb(110, 160, 255),
    guide: Color32::from_rgb(229, 192, 123),
    tour_highlight: Color32::from_rgb(255, 196, 0),
};

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
}
