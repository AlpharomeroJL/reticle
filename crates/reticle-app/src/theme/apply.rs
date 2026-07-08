//! Mapping the semantic tokens onto an `egui::Style`.
//!
//! [`style`] builds the whole applied style for one density from
//! [`tokens::DARK`]; [`apply`] installs it on a context. This is the only place
//! that decides which egui field each token role feeds, so a palette or density
//! change is a one-file edit and the check-style lint can ban raw colors and
//! sizes everywhere else. The human-readable spec is `docs/design/tokens.md`
//! and the contrast proofs are in [`super::contrast`].
//!
//! The build starts from [`egui::Visuals::dark`] and overwrites the roles the
//! tokens own, so any egui field the design does not speak to keeps a sane dark
//! default rather than a zero.

use eframe::egui::{
    self, Color32, CornerRadius, Margin, Shadow, Stroke,
    style::{ScrollAnimation, Selection, WidgetVisuals},
};

use super::tokens::{self, Density, RADIUS_LG, RADIUS_MD, RADIUS_XL, Tokens};

/// Builds the applied [`egui::Style`] for `density`, zeroing animation when
/// `reduced_motion` is set.
///
/// Colors come from [`tokens::DARK`] (v8.1 ships one theme); spacing, the type
/// scale, and the animation length come from the [`Density`] methods. The
/// result is a complete style ready to hand to [`egui::Context::set_style_of`].
#[must_use]
pub fn style(density: Density, reduced_motion: bool) -> egui::Style {
    let mut style = egui::Style {
        visuals: visuals(&tokens::DARK),
        ..Default::default()
    };
    apply_spacing(&mut style, density);
    apply_type_scale(&mut style, density);
    // Functional motion only: transitions communicate state change and go to
    // zero under reduced motion, which also stops the smooth-scroll easing.
    style.animation_time = density.animation_time(reduced_motion);
    if reduced_motion {
        style.scroll_animation = ScrollAnimation::none();
    }
    style
}

/// Installs the dark token style on `ctx` for `density` and `reduced_motion`.
///
/// The same dark style is set for both egui themes and the theme preference is
/// pinned to Dark, so an OS "light" preference cannot resurrect the retired
/// stock-egui light look between frames: whatever egui resolves the active
/// theme to, it renders the tokened dark style.
pub fn apply(ctx: &egui::Context, density: Density, reduced_motion: bool) {
    let style = style(density, reduced_motion);
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);
    ctx.set_theme(egui::ThemePreference::Dark);
}

/// Maps the color roles onto `Visuals`, starting from the dark defaults.
fn visuals(t: &Tokens) -> egui::Visuals {
    let mut v = egui::Visuals::dark();

    // Surfaces: panels, raised windows/menus, and sunken input wells.
    v.panel_fill = t.bg_panel;
    v.window_fill = t.bg_raised;
    v.extreme_bg_color = t.bg_input;
    v.text_edit_bg_color = Some(t.bg_input);
    v.code_bg_color = t.bg_input;
    // Striped rows sit one raised step off the panel so a table reads without a
    // border grid (Linear's restraint).
    v.faint_bg_color = t.bg_raised;

    // Window and menu chrome.
    v.window_stroke = Stroke::new(1.0, t.border_strong);
    v.window_corner_radius = CornerRadius::same(RADIUS_XL);
    v.menu_corner_radius = CornerRadius::same(RADIUS_LG);
    // Elevation: two recipes only (tokens.md). 45% and 55% black map to alpha
    // 115 and 140 out of 255.
    v.popup_shadow = Shadow {
        offset: [0, 2],
        blur: 8,
        spread: 0,
        color: Color32::from_black_alpha(115),
    };
    v.window_shadow = Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(140),
    };

    // Text: primary and secondary. Widget states also carry the text color via
    // their fg_stroke below; weak text (captions, section headers) is one token
    // quieter.
    v.override_text_color = Some(t.text);
    v.weak_text_color = Some(t.text_weak);
    v.hyperlink_color = t.accent;
    v.warn_fg_color = t.warning;
    v.error_fg_color = t.danger;

    // Selection: a muted-accent fill behind selected text and selectable rows,
    // with the bright accent as the text-edit caret/outline stroke. This stroke
    // is the field egui consults for text-edit keyboard focus.
    v.selection = Selection {
        bg_fill: t.accent_muted,
        stroke: Stroke::new(1.0, t.accent),
    };

    // Widget states. egui resolves a keyboard-focused widget to the `active`
    // visuals (see `Widgets::style`: focus, press, and click all select
    // `active`), so the accent focus ring lives on `active.bg_stroke`; lane 4A
    // builds its focus affordance on that field. Hover and press only change
    // the fill, per the state rules in tokens.md.
    let border = Stroke::new(1.0, t.border);
    v.widgets.noninteractive = widget(t.bg_panel, t.bg_panel, border, t.text);
    v.widgets.inactive = widget(t.widget_bg, t.widget_bg, border, t.text);
    v.widgets.hovered = widget(t.widget_hover, t.widget_hover, border, t.text);
    v.widgets.active = widget(
        t.widget_active,
        t.widget_active,
        Stroke::new(1.5, t.focus),
        t.text,
    );
    v.widgets.open = widget(t.widget_active, t.widget_active, border, t.text);

    v
}

/// One widget state at the interactive-widget radius, with the given fill,
/// outline stroke, and foreground (text/glyph) color.
fn widget(
    bg_fill: Color32,
    weak_bg_fill: Color32,
    bg_stroke: Stroke,
    fg: Color32,
) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill,
        weak_bg_fill,
        bg_stroke,
        corner_radius: CornerRadius::same(RADIUS_MD),
        fg_stroke: Stroke::new(1.0, fg),
        expansion: 0.0,
    }
}

/// Applies the density's spacing rhythm (all values on the 4px grid).
fn apply_spacing(style: &mut egui::Style, d: Density) {
    let s = &mut style.spacing;
    s.item_spacing = d.item_spacing();
    s.button_padding = d.button_padding();
    s.interact_size.y = d.interact_height();
    s.window_margin = Margin::same(d.window_margin() as i8);
    s.menu_margin = Margin::same(d.menu_margin() as i8);
    s.indent = d.indent();
    s.icon_width = d.icon_width();
    s.icon_width_inner = d.icon_width_inner();
    s.icon_spacing = d.icon_spacing();
    s.combo_height = d.combo_height();
}

/// Applies the density's type scale to the five egui text styles. Faces stay on
/// egui's `Proportional`/`Monospace` families; lane 1B installs Inter and
/// `JetBrains Mono` into those families, so only the sizes are set here.
fn apply_type_scale(style: &mut egui::Style, d: Density) {
    use egui::{FontFamily, FontId, TextStyle};
    let ts = d.type_scale();
    style.text_styles = [
        (
            TextStyle::Small,
            FontId::new(ts.small, FontFamily::Proportional),
        ),
        (
            TextStyle::Body,
            FontId::new(ts.body, FontFamily::Proportional),
        ),
        (
            TextStyle::Button,
            FontId::new(ts.button, FontFamily::Proportional),
        ),
        (
            TextStyle::Heading,
            FontId::new(ts.heading, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(ts.mono, FontFamily::Monospace),
        ),
    ]
    .into();
}

/// The monospace HUD/readout font at `density` (numeric overlays, cursor and
/// measurement readouts, ruler labels). Canvas text routes through the type
/// scale rather than ad hoc sizes; this is the Monospace text style resolved for
/// a raw `Painter`, which has no access to the applied `Style`.
#[must_use]
pub fn hud_mono(density: Density) -> egui::FontId {
    egui::FontId::monospace(density.type_scale().mono)
}

/// The proportional HUD font at `density` (short canvas prose such as presence
/// names). The Body text style resolved for a raw `Painter`.
#[must_use]
pub fn hud_body(density: Density) -> egui::FontId {
    egui::FontId::proportional(density.type_scale().body)
}

/// The proportional HUD heading font at `density` (the large drop-affordance
/// prompt). The Heading text style resolved for a raw `Painter`.
#[must_use]
pub fn hud_heading(density: Density) -> egui::FontId {
    egui::FontId::proportional(density.type_scale().heading)
}

/// A monospace font at an explicit point size, for canvas text whose size is
/// computed rather than one of the fixed styles: cell-label text sized to the
/// label-fitting math and geometry-scaled annotations. The size is deliberately
/// caller-chosen, so this stays out of the density type scale.
#[must_use]
pub fn mono_sized(size: f32) -> egui::FontId {
    egui::FontId::monospace(size)
}

/// A proportional font at an explicit point size (a canvas annotation sized to
/// on-screen geometry, e.g. a comment pin's number scaled to its radius).
#[must_use]
pub fn proportional_sized(size: f32) -> egui::FontId {
    egui::FontId::proportional(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_densities_build_a_style() {
        for d in [Density::Comfortable, Density::Compact] {
            let s = style(d, false);
            // Surfaces mapped from tokens, not egui defaults.
            assert_eq!(s.visuals.panel_fill, tokens::DARK.bg_panel);
            assert_eq!(s.visuals.window_fill, tokens::DARK.bg_raised);
            assert_eq!(s.visuals.extreme_bg_color, tokens::DARK.bg_input);
            assert_eq!(s.visuals.text_edit_bg_color, Some(tokens::DARK.bg_input));
            // Semantic foregrounds.
            assert_eq!(s.visuals.hyperlink_color, tokens::DARK.accent);
            assert_eq!(s.visuals.warn_fg_color, tokens::DARK.warning);
            assert_eq!(s.visuals.error_fg_color, tokens::DARK.danger);
            assert_eq!(s.visuals.selection.bg_fill, tokens::DARK.accent_muted);
            // Spacing tracks the density.
            assert_eq!(s.spacing.item_spacing, d.item_spacing());
            assert!((s.spacing.interact_size.y - d.interact_height()).abs() < f32::EPSILON);
            // The five text styles are all present at the density's sizes.
            assert_eq!(s.text_styles.len(), 5);
            assert!(
                (s.text_styles[&egui::TextStyle::Body].size - d.type_scale().body).abs()
                    < f32::EPSILON
            );
        }
    }

    #[test]
    fn focus_ring_lives_on_active_widget_stroke() {
        // Lane 4A depends on this: a keyboard-focused widget resolves to the
        // `active` visuals, whose bg_stroke is the accent focus ring.
        let s = style(Density::Comfortable, false);
        assert_eq!(s.visuals.widgets.active.bg_stroke.color, tokens::DARK.focus);
        assert!((s.visuals.widgets.active.bg_stroke.width - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn reduced_motion_zeroes_animation() {
        let s = style(Density::Comfortable, true);
        assert!(s.animation_time.abs() < f32::EPSILON);
        let moving = style(Density::Comfortable, false);
        assert!(moving.animation_time > 0.0);
    }
}
