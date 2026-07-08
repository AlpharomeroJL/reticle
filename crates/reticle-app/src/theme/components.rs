//! The widget library every panel and dialog composes from (Wave 1, lane 1C).
//!
//! From Wave 2 forward this module is the only widget source for chrome: no
//! panel builds a raw `egui` button or frame, it calls one of these so a palette
//! or density change lands in one place. Everything here is styled from
//! [`super::tokens`] only (the check-style lint bans color and size literals
//! outside `theme/`), honors both [`Density`] rhythms, has a disabled state, and
//! carries a one-line usage example.
//!
//! # The [`Ctx`] handle
//!
//! Components do not reach into ambient `egui` state for their palette; they take
//! a [`Ctx`] (the token table, the density, and whether motion is reduced) so the
//! same widget can render at two densities in the gallery and so a caller is never
//! surprised by a hidden global. `Ctx` is `Copy`; a panel keeps one for the frame
//! and threads it into each call.
//!
//! Lane 4A extends this module additive-only in Wave 2 (new components and new
//! builder methods, never a changed signature).

use eframe::egui::{
    self, Color32, Id, Margin, Response, RichText, Sense, Stroke, StrokeKind, Vec2, epaint::Shadow,
};

use super::tokens::{DARK, Density, RADIUS_LG, RADIUS_MD, RADIUS_SM, RADIUS_XL, Tokens};

/// Everything a component needs from the theme: the semantic palette, the
/// density rhythm, and whether reduced motion is requested. Cheap to copy;
/// build one per frame (the `apply` mapping, lane 1A, stashes the active one;
/// the gallery builds [`Ctx::dark`]).
#[derive(Clone, Copy, Debug)]
pub struct Ctx {
    /// The active semantic palette.
    pub tokens: Tokens,
    /// The spatial and type rhythm.
    pub density: Density,
    /// When true, functional transitions collapse to an instant cut.
    pub reduced_motion: bool,
}

impl Ctx {
    /// The shipped dark theme at a given density, motion enabled. The gallery and
    /// tests build from here; the real app passes its resolved [`Density`].
    #[must_use]
    pub fn dark(density: Density) -> Self {
        Self {
            tokens: DARK,
            density,
            reduced_motion: false,
        }
    }

    /// Returns a copy with reduced motion set, for the accessibility setting.
    #[must_use]
    pub fn with_reduced_motion(mut self, reduced: bool) -> Self {
        self.reduced_motion = reduced;
        self
    }

    /// The corner radius as an `egui` [`CornerRadius`](egui::CornerRadius).
    fn radius(md: u8) -> egui::CornerRadius {
        egui::CornerRadius::same(md)
    }
}

/// The visual weight of a [`Button`]; primary carries the accent fill, danger the
/// destructive fill, secondary a bordered neutral fill, ghost no fill until hover.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonVariant {
    /// Accent fill with `accent_text`: the one affirmative action per view.
    Primary,
    /// Neutral widget fill with a border: the common non-primary action.
    Secondary,
    /// No fill until hover: low-emphasis actions that should not compete.
    Ghost,
    /// Danger fill: destructive or irreversible actions.
    Danger,
}

/// A text button in one of four [`ButtonVariant`] weights.
///
/// Usage: `components::Button::primary("Run DRC").show(ui, ctx)`.
#[derive(Clone, Debug)]
pub struct Button {
    text: String,
    variant: ButtonVariant,
    enabled: bool,
    min_width: f32,
}

impl Button {
    /// A button in `variant` carrying `text`.
    #[must_use]
    pub fn new(text: impl Into<String>, variant: ButtonVariant) -> Self {
        Self {
            text: text.into(),
            variant,
            enabled: true,
            min_width: 0.0,
        }
    }

    /// The affirmative primary action (accent fill).
    #[must_use]
    pub fn primary(text: impl Into<String>) -> Self {
        Self::new(text, ButtonVariant::Primary)
    }

    /// A neutral bordered action.
    #[must_use]
    pub fn secondary(text: impl Into<String>) -> Self {
        Self::new(text, ButtonVariant::Secondary)
    }

    /// A low-emphasis action (no fill until hover).
    #[must_use]
    pub fn ghost(text: impl Into<String>) -> Self {
        Self::new(text, ButtonVariant::Ghost)
    }

    /// A destructive action (danger fill).
    #[must_use]
    pub fn danger(text: impl Into<String>) -> Self {
        Self::new(text, ButtonVariant::Danger)
    }

    /// Disables the button: `text_faint` label, fill unchanged, no hover response.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Sets a minimum width so a row of buttons can align.
    #[must_use]
    pub fn min_width(mut self, w: f32) -> Self {
        self.min_width = w;
        self
    }

    /// Paints the button and returns its [`Response`].
    pub fn show(self, ui: &mut egui::Ui, ctx: Ctx) -> Response {
        let t = ctx.tokens;
        let font = egui::TextStyle::Button.resolve(ui.style());
        let galley = ui
            .painter()
            .layout_no_wrap(self.text, font, Color32::PLACEHOLDER);

        let padding = ctx.density.button_padding();
        let min_h = ctx.density.interact_height();
        let desired = Vec2::new(
            (galley.size().x + 2.0 * padding.x).max(self.min_width),
            (galley.size().y + 2.0 * padding.y).max(min_h),
        );
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, resp) = ui.allocate_exact_size(desired, sense);
        if !ui.is_rect_visible(rect) {
            return resp;
        }

        let hovered = self.enabled && resp.hovered();
        let down = self.enabled && resp.is_pointer_button_down_on();
        let (fill, text_color, border) =
            button_colors(&t, self.variant, hovered, down, self.enabled);

        let painter = ui.painter();
        let cr = Ctx::radius(RADIUS_MD);
        if fill.a() > 0 {
            painter.rect_filled(rect, cr, fill);
        }
        if let Some(stroke) = border {
            painter.rect_stroke(rect, cr, stroke, StrokeKind::Inside);
        }
        let text_pos = rect.center() - galley.size() / 2.0;
        painter.galley(text_pos, galley, text_color);
        if resp.has_focus() {
            painter.rect_stroke(rect, cr, Stroke::new(1.5, t.focus), StrokeKind::Inside);
        }
        resp
    }
}

/// Resolves the (fill, text, optional border) for a button variant and state,
/// using only chrome tokens. Filled variants (primary, danger) keep their
/// semantic fill; the pressed primary drops to `accent_muted` (a proven 8.21:1
/// pair with `text`). Neutral variants walk the widget-state fills.
fn button_colors(
    t: &Tokens,
    variant: ButtonVariant,
    hovered: bool,
    down: bool,
    enabled: bool,
) -> (Color32, Color32, Option<Stroke>) {
    let disabled_text = t.text_faint;
    match variant {
        ButtonVariant::Primary => {
            let (fill, text) = if down {
                (t.accent_muted, t.text)
            } else {
                (t.accent, t.accent_text)
            };
            let text = if enabled { text } else { disabled_text };
            (fill, text, None)
        }
        ButtonVariant::Danger => {
            let text = if enabled {
                t.accent_text
            } else {
                disabled_text
            };
            (t.danger, text, None)
        }
        ButtonVariant::Secondary => {
            let fill = state_fill(t, hovered, down);
            let text = if enabled { t.text } else { disabled_text };
            (fill, text, Some(Stroke::new(1.0, t.border)))
        }
        ButtonVariant::Ghost => {
            let fill = if hovered || down {
                state_fill(t, hovered, down)
            } else {
                Color32::TRANSPARENT
            };
            let text = if enabled { t.text } else { disabled_text };
            (fill, text, None)
        }
    }
}

/// The neutral widget fill for the current interaction state (tokens.md state
/// rules: hover raises to `widget_hover`, press to `widget_active`).
fn state_fill(t: &Tokens, hovered: bool, down: bool) -> Color32 {
    if down {
        t.widget_active
    } else if hovered {
        t.widget_hover
    } else {
        t.widget_bg
    }
}

/// Width, in points, of a [`Toast`] severity bar. Theme-local: too specific to
/// one component to earn a place in the global token table.
const SEVERITY_BAR_WIDTH: f32 = 3.0;
/// Thickness, in points, of a [`ProgressRow`] track and its fill.
const PROGRESS_BAR_HEIGHT: f32 = 6.0;

/// Draws the 1.5 px keyboard-focus ring tokens.md mandates for any focused
/// widget (it is never removed for aesthetics).
fn focus_ring(painter: &egui::Painter, rect: egui::Rect, cr: egui::CornerRadius, focus: Color32) {
    painter.rect_stroke(rect, cr, Stroke::new(1.5, focus), StrokeKind::Inside);
}

/// The `shadow_popup` recipe from tokens.md (menus, popovers, toasts).
fn shadow_popup() -> Shadow {
    Shadow {
        offset: [0, 2],
        blur: 8,
        spread: 0,
        // 45% black; `Color32` is premultiplied, `from_black_alpha` is the honest
        // constructor for a translucent shadow color.
        color: Color32::from_black_alpha(115),
    }
}

/// The `shadow_window` recipe from tokens.md (modals, floating panels).
fn shadow_window() -> Shadow {
    Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        // 55% black.
        color: Color32::from_black_alpha(140),
    }
}

/// A symmetric `egui` [`Margin`] from a density padding vector.
fn margin_of(pad: Vec2) -> Margin {
    Margin::symmetric(pad.x as i8, pad.y as i8)
}

/// A square icon-only button carrying a single glyph and a rich tooltip.
///
/// The glyph is passed as a `char` so this widget does not depend on
/// `super::icons` (lane 1B); a caller passes an `icons::` constant or any glyph.
/// The tooltip carries the action name, an optional [`KbdChip`] with its chord,
/// and an optional one-line description (audit AUD-18).
///
/// Usage: `components::IconButton::new(icon, "Fit").kbd("F").show(ui, ctx)`.
#[derive(Clone, Debug)]
pub struct IconButton {
    glyph: char,
    name: String,
    kbd: Option<String>,
    hint: Option<String>,
    enabled: bool,
    selected: bool,
}

impl IconButton {
    /// An icon button showing `glyph`, named `name` (the tooltip title).
    #[must_use]
    pub fn new(glyph: char, name: impl Into<String>) -> Self {
        Self {
            glyph,
            name: name.into(),
            kbd: None,
            hint: None,
            enabled: true,
            selected: false,
        }
    }

    /// Adds a keyboard-chord hint (rendered as a [`KbdChip`] in the tooltip).
    #[must_use]
    pub fn kbd(mut self, kbd: impl Into<String>) -> Self {
        self.kbd = Some(kbd.into());
        self
    }

    /// Adds a one-line description shown under the name in the tooltip.
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Disables the button (faint glyph, no hover response).
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Marks the button active (toggled on): `accent_muted` fill with `text`.
    #[must_use]
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Paints the icon button and returns its [`Response`].
    pub fn show(self, ui: &mut egui::Ui, ctx: Ctx) -> Response {
        let t = ctx.tokens;
        let size = ctx.density.interact_height();
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), sense);
        if ui.is_rect_visible(rect) {
            let hovered = self.enabled && resp.hovered();
            let down = self.enabled && resp.is_pointer_button_down_on();
            let fill = if self.selected {
                t.accent_muted
            } else if hovered || down {
                state_fill(&t, hovered, down)
            } else {
                Color32::TRANSPARENT
            };
            let fg = if !self.enabled {
                t.text_faint
            } else if self.selected {
                t.text
            } else {
                t.text_weak
            };
            let font = egui::TextStyle::Body.resolve(ui.style());
            let painter = ui.painter();
            let cr = Ctx::radius(RADIUS_MD);
            if fill.a() > 0 {
                painter.rect_filled(rect, cr, fill);
            }
            let galley = painter.layout_no_wrap(self.glyph.to_string(), font, Color32::PLACEHOLDER);
            painter.galley(rect.center() - galley.size() / 2.0, galley, fg);
            if resp.has_focus() {
                focus_ring(painter, rect, cr, t.focus);
            }
        }

        let name = self.name;
        let kbd = self.kbd;
        let hint = self.hint;
        resp.on_hover_ui(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&name).strong().color(t.text));
                if let Some(k) = &kbd {
                    KbdChip::new(k.clone()).show(ui, ctx);
                }
            });
            if let Some(h) = &hint {
                ui.label(RichText::new(h).color(t.text_weak));
            }
        })
    }
}

/// A small keyboard-chord chip: mono face, hairline border, `RADIUS_SM`.
///
/// Usage: `components::KbdChip::new("Ctrl K").show(ui, ctx)`.
#[derive(Clone, Debug)]
pub struct KbdChip {
    keys: String,
}

impl KbdChip {
    /// A chip labeled with the chord text (for example `"Ctrl K"`).
    #[must_use]
    pub fn new(keys: impl Into<String>) -> Self {
        Self { keys: keys.into() }
    }

    /// Paints the chip and returns its [`Response`].
    pub fn show(self, ui: &mut egui::Ui, ctx: Ctx) -> Response {
        let t = ctx.tokens;
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let galley = ui
            .painter()
            .layout_no_wrap(self.keys, font, Color32::PLACEHOLDER);
        let pad = ctx.density.button_padding() * 0.5;
        let (rect, resp) = ui.allocate_exact_size(galley.size() + 2.0 * pad, Sense::hover());
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let cr = Ctx::radius(RADIUS_SM);
            painter.rect_filled(rect, cr, t.widget_bg);
            painter.rect_stroke(rect, cr, Stroke::new(1.0, t.border), StrokeKind::Inside);
            painter.galley(rect.center() - galley.size() / 2.0, galley, t.text_weak);
        }
        resp
    }
}

/// A toggle chip: on shows an `accent_muted` fill with `text`, off a quiet
/// neutral fill with `text_weak`. The caller owns the flag and flips it on click.
///
/// Usage: `if components::ToggleChip::new("Snap", snap_on).show(ui, ctx).clicked() { snap_on = !snap_on }`.
#[derive(Clone, Debug)]
pub struct ToggleChip {
    label: String,
    selected: bool,
    enabled: bool,
}

impl ToggleChip {
    /// A chip labeled `label`, drawn in the `selected` state.
    #[must_use]
    pub fn new(label: impl Into<String>, selected: bool) -> Self {
        Self {
            label: label.into(),
            selected,
            enabled: true,
        }
    }

    /// Disables the chip (faint label, no hover response).
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Paints the chip and returns its [`Response`] (`clicked()` means toggle).
    pub fn show(self, ui: &mut egui::Ui, ctx: Ctx) -> Response {
        let t = ctx.tokens;
        let font = egui::TextStyle::Button.resolve(ui.style());
        let galley = ui
            .painter()
            .layout_no_wrap(self.label, font, Color32::PLACEHOLDER);
        let pad = ctx.density.button_padding();
        let min_h = ctx.density.interact_height();
        let desired = Vec2::new(
            galley.size().x + 2.0 * pad.x,
            (galley.size().y + 2.0 * pad.y).max(min_h),
        );
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, resp) = ui.allocate_exact_size(desired, sense);
        if ui.is_rect_visible(rect) {
            let hovered = self.enabled && resp.hovered();
            let down = self.enabled && resp.is_pointer_button_down_on();
            let (fill, fg) = if self.selected {
                (
                    t.accent_muted,
                    if self.enabled { t.text } else { t.text_faint },
                )
            } else {
                let fg = if self.enabled {
                    t.text_weak
                } else {
                    t.text_faint
                };
                (state_fill(&t, hovered, down), fg)
            };
            let painter = ui.painter();
            let cr = Ctx::radius(RADIUS_SM);
            painter.rect_filled(rect, cr, fill);
            painter.galley(rect.center() - galley.size() / 2.0, galley, fg);
            if resp.has_focus() {
                focus_ring(painter, rect, cr, t.focus);
            }
        }
        resp
    }
}

/// A single-select segmented control: a row of labels where exactly one is on.
///
/// Focus the control and press Left/Right (or Up/Down) to move the selection;
/// clicking a segment selects it. The caller owns the selected index. The
/// returned [`Response`] reports `changed()` when the selection moved.
///
/// Usage: `components::Segmented::new(&["Single", "Split H", "Split V"]).show(ui, ctx, &mut mode)`.
#[derive(Clone, Debug)]
pub struct Segmented<'a> {
    labels: &'a [&'a str],
    enabled: bool,
}

impl<'a> Segmented<'a> {
    /// A segmented control over `labels`.
    #[must_use]
    pub fn new(labels: &'a [&'a str]) -> Self {
        Self {
            labels,
            enabled: true,
        }
    }

    /// Disables the control (faint labels, no interaction).
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Paints the control, updating `selected` on click or arrow key, and
    /// returns the row [`Response`].
    pub fn show(self, ui: &mut egui::Ui, ctx: Ctx, selected: &mut usize) -> Response {
        let t = ctx.tokens;
        let n = self.labels.len();
        let font = egui::TextStyle::Button.resolve(ui.style());
        let pad = ctx.density.button_padding();
        let h = ctx.density.interact_height();
        let galleys: Vec<_> = self
            .labels
            .iter()
            .map(|l| {
                ui.painter()
                    .layout_no_wrap((*l).to_owned(), font.clone(), Color32::PLACEHOLDER)
            })
            .collect();
        let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 2.0 * pad.x).collect();
        let total_w = widths.iter().sum();
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(total_w, h), sense);

        if self.enabled && n > 0 && resp.has_focus() {
            let (mut next, mut prev) = (false, false);
            ui.input(|i| {
                next = i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowDown);
                prev = i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowUp);
            });
            if next {
                *selected = (*selected + 1) % n;
                resp.mark_changed();
            }
            if prev {
                *selected = (*selected + n - 1) % n;
                resp.mark_changed();
            }
        }

        if ui.is_rect_visible(rect) {
            let cr = Ctx::radius(RADIUS_MD);
            ui.painter().rect_filled(rect, cr, t.widget_bg);
            let mut x = rect.left();
            for (i, (g, w)) in galleys.into_iter().zip(&widths).enumerate() {
                let seg_rect =
                    egui::Rect::from_min_size(egui::pos2(x, rect.top()), Vec2::new(*w, h));
                let seg_resp = ui.interact(seg_rect, resp.id.with(i), sense);
                if self.enabled && seg_resp.clicked() && *selected != i {
                    *selected = i;
                    resp.mark_changed();
                    resp.request_focus();
                }
                let sel = i == *selected;
                let fill = if sel {
                    t.accent_muted
                } else if self.enabled && seg_resp.hovered() {
                    t.widget_hover
                } else {
                    Color32::TRANSPARENT
                };
                let fg = if !self.enabled {
                    t.text_faint
                } else if sel {
                    t.text
                } else {
                    t.text_weak
                };
                let painter = ui.painter();
                if fill.a() > 0 {
                    painter.rect_filled(seg_rect, cr, fill);
                }
                painter.galley(seg_rect.center() - g.size() / 2.0, g, fg);
                x += *w;
            }
            ui.painter()
                .rect_stroke(rect, cr, Stroke::new(1.0, t.border), StrokeKind::Inside);
            if resp.has_focus() {
                focus_ring(ui.painter(), rect, cr, t.focus);
            }
        }
        resp
    }
}

/// A text input wrapped in a `bg_input` well with a hairline border that becomes
/// a `focus` ring while the field is focused.
///
/// Usage: `components::TextField::new(&mut query).hint("Filter layers").show(ui, ctx)`.
#[derive(Debug)]
pub struct TextField<'a> {
    text: &'a mut String,
    hint: Option<String>,
    enabled: bool,
    desired_width: Option<f32>,
}

impl<'a> TextField<'a> {
    /// A single-line field editing `text`.
    #[must_use]
    pub fn new(text: &'a mut String) -> Self {
        Self {
            text,
            hint: None,
            enabled: true,
            desired_width: None,
        }
    }

    /// Sets the placeholder shown when the field is empty.
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Disables the field.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Sets an explicit width in points.
    #[must_use]
    pub fn desired_width(mut self, width: f32) -> Self {
        self.desired_width = Some(width);
        self
    }

    /// Paints the field and returns the inner [`TextEdit`](egui::TextEdit)
    /// [`Response`].
    pub fn show(self, ui: &mut egui::Ui, ctx: Ctx) -> Response {
        let t = ctx.tokens;
        let cr = Ctx::radius(RADIUS_MD);
        let frame = egui::Frame::new()
            .fill(t.bg_input)
            .inner_margin(margin_of(ctx.density.button_padding()))
            .corner_radius(cr);
        let out = frame.show(ui, |ui| {
            let mut edit = egui::TextEdit::singleline(self.text)
                .frame(egui::Frame::NONE)
                .text_color(t.text);
            if let Some(h) = &self.hint {
                edit = edit.hint_text(h.as_str());
            }
            if let Some(w) = self.desired_width {
                edit = edit.desired_width(w);
            }
            ui.add_enabled(self.enabled, edit)
        });
        let resp = out.inner;
        let stroke = if resp.has_focus() {
            Stroke::new(1.5, t.focus)
        } else {
            Stroke::new(1.0, t.border)
        };
        ui.painter()
            .rect_stroke(out.response.rect, cr, stroke, StrokeKind::Inside);
        resp
    }
}

/// A section header: the `ui-medium` face at Body size in `text_weak`. Until the
/// Medium face lands (lane 1B) this renders as Body plus strong, chosen through
/// the text-style API rather than a size literal.
///
/// Usage: `components::SectionHeader::new("Properties").show(ui, ctx)`.
#[derive(Clone, Debug)]
pub struct SectionHeader {
    title: String,
}

impl SectionHeader {
    /// A header labeled `title`.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
        }
    }

    /// Paints the header and returns its [`Response`].
    pub fn show(self, ui: &mut egui::Ui, ctx: Ctx) -> Response {
        ui.label(
            RichText::new(self.title)
                .strong()
                .color(ctx.tokens.text_weak),
        )
    }
}

/// A collapsible section: a clickable header row with a caret plus a body that
/// fades in and out. The caller owns the remembered-open flag. The fade uses
/// [`Density::animation_time`], which is zero under reduced motion, so the
/// section snaps open with no animation when the user asks for that.
///
/// Usage: `components::Collapsible::new("ops", "Operations").show(ui, ctx, &mut open, |ui, ctx| { .. })`.
#[derive(Clone, Debug)]
pub struct Collapsible {
    id: Id,
    title: String,
}

impl Collapsible {
    /// A section identified by `id_source` (unique among siblings) titled `title`.
    #[must_use]
    pub fn new(
        id_source: impl std::hash::Hash + std::fmt::Debug,
        title: impl Into<String>,
    ) -> Self {
        Self {
            id: Id::new(id_source),
            title: title.into(),
        }
    }

    /// Paints the header and, when open, the body. Returns the header
    /// [`Response`]; toggling is applied to `open` on a header click.
    pub fn show(
        self,
        ui: &mut egui::Ui,
        ctx: Ctx,
        open: &mut bool,
        add_body: impl FnOnce(&mut egui::Ui, Ctx),
    ) -> Response {
        let t = ctx.tokens;
        // U+25BE down-pointing / U+25B8 right-pointing small triangle.
        let caret = if *open { '\u{25BE}' } else { '\u{25B8}' };
        // A full-width clickable row (the conventional collapsible affordance)
        // drawn by hand so it carries the tokens.md interaction states a bare
        // `Label` cannot: a hover fill and, above all, the always-visible 1.5px
        // keyboard-focus ring that 3C's F6 traversal lands on. `Sense::click()`
        // keeps it focusable, so Space/Enter toggles it like any button.
        let font = egui::TextStyle::Body.resolve(ui.style());
        let galley = ui.painter().layout_no_wrap(
            format!("{caret}  {}", self.title),
            font,
            Color32::PLACEHOLDER,
        );
        let pad = ctx.density.button_padding();
        let desired = Vec2::new(
            ui.available_width().max(galley.size().x + 2.0 * pad.x),
            (galley.size().y + 2.0 * pad.y).max(ctx.density.interact_height()),
        );
        let (rect, header) = ui.allocate_exact_size(desired, Sense::click());
        if header.clicked() {
            *open = !*open;
        }
        if ui.is_rect_visible(rect) {
            let cr = Ctx::radius(RADIUS_SM);
            if header.hovered() {
                ui.painter().rect_filled(rect, cr, t.widget_hover);
            }
            let text_pos = egui::pos2(rect.left() + pad.x, rect.center().y - galley.size().y / 2.0);
            ui.painter().galley(text_pos, galley, t.text_weak);
            if header.has_focus() {
                focus_ring(ui.painter(), rect, cr, t.focus);
            }
        }
        let time = ctx.density.animation_time(ctx.reduced_motion);
        let openness = ui.ctx().animate_bool_with_time(self.id, *open, time);
        if openness > 0.0 {
            ui.indent(self.id, |ui| {
                ui.scope(|ui| {
                    ui.multiply_opacity(openness);
                    add_body(ui, ctx);
                });
            });
        }
        header
    }
}

/// An empty-state block: a title that names the void and a hint that says what
/// to do next, with up to two action buttons (audit AUD-19). The block is
/// centered so it reads as a deliberate rest state, not a broken panel.
///
/// Usage: `components::EmptyState::new("No selection", "Click a shape or press A to select all").show(ui, ctx)`.
#[derive(Debug)]
pub struct EmptyState {
    title: String,
    hint: String,
    actions: Vec<Button>,
}

impl EmptyState {
    /// An empty state titled `title` with the next-step `hint` line.
    #[must_use]
    pub fn new(title: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            hint: hint.into(),
            actions: Vec::new(),
        }
    }

    /// Adds an action button (at most two are kept; extras are dropped).
    #[must_use]
    pub fn action(mut self, button: Button) -> Self {
        if self.actions.len() < 2 {
            self.actions.push(button);
        }
        self
    }

    /// Paints the block; returns the index of the action clicked this frame.
    pub fn show(self, ui: &mut egui::Ui, ctx: Ctx) -> Option<usize> {
        let t = ctx.tokens;
        let mut clicked = None;
        ui.vertical_centered(|ui| {
            ui.add_space(ctx.density.window_margin());
            ui.label(RichText::new(&self.title).strong().color(t.text));
            ui.add_space(ctx.density.item_spacing().y);
            ui.label(RichText::new(&self.hint).color(t.text_weak));
            if !self.actions.is_empty() {
                ui.add_space(ctx.density.window_margin());
                ui.horizontal(|ui| {
                    for (i, button) in self.actions.into_iter().enumerate() {
                        if button.show(ui, ctx).clicked() {
                            clicked = Some(i);
                        }
                    }
                });
            }
            ui.add_space(ctx.density.window_margin());
        });
        clicked
    }
}

/// The severity of a [`Toast`] or status message, mapped to a semantic accent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    /// Neutral information (uses the plain `accent`).
    Info,
    /// A confirmation or pass state (`success`).
    Success,
    /// A warning or stale indicator (`warning`).
    Warning,
    /// An error or destructive outcome (`danger`).
    Danger,
}

impl Severity {
    /// The accent color that keys this severity.
    #[must_use]
    pub fn accent(self, t: &Tokens) -> Color32 {
        match self {
            Severity::Info => t.accent,
            Severity::Success => t.success,
            Severity::Warning => t.warning,
            Severity::Danger => t.danger,
        }
    }
}

/// The outcome of a rendered [`Toast`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ToastResponse {
    /// Whether the close affordance was clicked this frame.
    pub closed: bool,
    /// Which action button, if any, was clicked this frame.
    pub action: Option<usize>,
}

/// A toast card: a severity accent bar, a message, up to two action buttons, and
/// a close affordance, on a raised frame with the `shadow_popup` recipe. This is
/// the visual only; the queue and expiry live in `notify.rs` and are wired in
/// Wave 2 (lane 3B).
///
/// Usage: `components::Toast::new(Severity::Danger, "Open failed").action(retry).show(ui, ctx)`.
#[derive(Debug)]
pub struct Toast {
    severity: Severity,
    message: String,
    actions: Vec<Button>,
    closable: bool,
    appear: f32,
}

impl Toast {
    /// A toast of `severity` carrying `message`.
    #[must_use]
    pub fn new(severity: Severity, message: impl Into<String>) -> Self {
        Self {
            severity,
            message: message.into(),
            actions: Vec::new(),
            closable: true,
            appear: 1.0,
        }
    }

    /// Adds an action button (at most two are kept).
    #[must_use]
    pub fn action(mut self, button: Button) -> Self {
        if self.actions.len() < 2 {
            self.actions.push(button);
        }
        self
    }

    /// Sets whether the close affordance is shown (default true).
    #[must_use]
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }

    /// Sets the enter/exit progress in `0.0..=1.0`: 1.0 is fully present (the
    /// default), 0.0 fully faded out. The toast queue (lane 3B, `notify.rs`)
    /// drives this from an eased value so a toast fades in on arrival and out on
    /// dismissal; under reduced motion the queue animates it with zero time, so
    /// the toast simply appears. This component owns the opacity fade; the queue
    /// owns the timing and the toast's on-screen position.
    #[must_use]
    pub fn appear(mut self, progress: f32) -> Self {
        self.appear = progress.clamp(0.0, 1.0);
        self
    }

    /// Paints the toast and returns its [`ToastResponse`].
    pub fn show(self, ui: &mut egui::Ui, ctx: Ctx) -> ToastResponse {
        let t = ctx.tokens;
        // Fade the whole card (frame, shadow, and content) by the enter/exit
        // progress; the 1.0 default is a no-op, so a static toast is unchanged.
        if self.appear < 1.0 {
            ui.multiply_opacity(self.appear);
        }
        let mut out = ToastResponse::default();
        let frame = egui::Frame::new()
            .fill(t.bg_raised)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(Ctx::radius(RADIUS_LG))
            .shadow(shadow_popup())
            .inner_margin(margin_of(ctx.density.button_padding()));
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                let h = ui.available_height().max(ctx.density.interact_height());
                let (bar, _) =
                    ui.allocate_exact_size(Vec2::new(SEVERITY_BAR_WIDTH, h), Sense::hover());
                ui.painter()
                    .rect_filled(bar, Ctx::radius(RADIUS_SM), self.severity.accent(&t));
                ui.vertical(|ui| {
                    ui.label(RichText::new(&self.message).color(t.text));
                    if !self.actions.is_empty() {
                        ui.horizontal(|ui| {
                            for (i, button) in self.actions.into_iter().enumerate() {
                                if button.show(ui, ctx).clicked() {
                                    out.action = Some(i);
                                }
                            }
                        });
                    }
                });
                if self.closable
                    && IconButton::new('\u{00D7}', "Dismiss")
                        .show(ui, ctx)
                        .clicked()
                {
                    out.closed = true;
                }
            });
        });
        out
    }
}

/// The outcome of a rendered [`ProgressRow`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ProgressResponse {
    /// Whether the cancel affordance was clicked this frame.
    pub canceled: bool,
}

/// A progress row: a label, a determinate bar (`accent` fill over a `widget_bg`
/// track), and an optional cancel affordance.
///
/// Usage: `components::ProgressRow::new("Streaming", 0.4).cancelable(true).show(ui, ctx)`.
#[derive(Clone, Debug)]
pub struct ProgressRow {
    label: String,
    fraction: f32,
    cancelable: bool,
    anim_id: Option<Id>,
}

impl ProgressRow {
    /// A row labeled `label` filled to `fraction` (clamped to `0.0..=1.0`).
    #[must_use]
    pub fn new(label: impl Into<String>, fraction: f32) -> Self {
        Self {
            label: label.into(),
            fraction: fraction.clamp(0.0, 1.0),
            cancelable: false,
            anim_id: None,
        }
    }

    /// Sets whether a cancel affordance is shown at the end of the row.
    #[must_use]
    pub fn cancelable(mut self, cancelable: bool) -> Self {
        self.cancelable = cancelable;
        self
    }

    /// Eases the fill toward each new fraction rather than jumping, keyed by
    /// `id` (a streaming row whose fraction ticks up glides instead of
    /// snapping). The ease length is [`Density::animation_time`], so it is
    /// instant under reduced motion; without this the fill tracks the fraction
    /// exactly, frame for frame.
    ///
    /// [`Density::animation_time`]: super::tokens::Density::animation_time
    #[must_use]
    pub fn animate(mut self, id: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.anim_id = Some(Id::new(id));
        self
    }

    /// The fill fraction to paint this frame: the eased value when
    /// [`ProgressRow::animate`] is set (zero-time, hence instant, under reduced
    /// motion), otherwise the exact target.
    fn displayed_fraction(&self, egui_ctx: &egui::Context, ctx: Ctx) -> f32 {
        match self.anim_id {
            Some(id) => egui_ctx.animate_value_with_time(
                id,
                self.fraction,
                ctx.density.animation_time(ctx.reduced_motion),
            ),
            None => self.fraction,
        }
    }

    /// Paints the row and returns its [`ProgressResponse`].
    pub fn show(self, ui: &mut egui::Ui, ctx: Ctx) -> ProgressResponse {
        let t = ctx.tokens;
        let shown = self.displayed_fraction(ui.ctx(), ctx);
        let mut out = ProgressResponse::default();
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.label).color(t.text_weak));
            let cancel_w = if self.cancelable {
                ctx.density.interact_height() + ctx.density.item_spacing().x
            } else {
                0.0
            };
            let avail = (ui.available_width() - cancel_w).max(0.0);
            let (rect, _) = ui.allocate_exact_size(
                Vec2::new(avail, ctx.density.interact_height()),
                Sense::hover(),
            );
            let track = egui::Rect::from_center_size(
                rect.center(),
                Vec2::new(rect.width(), PROGRESS_BAR_HEIGHT),
            );
            let cr = Ctx::radius(RADIUS_SM);
            ui.painter().rect_filled(track, cr, t.widget_bg);
            let fill_w = track.width() * shown;
            if fill_w > 0.0 {
                let fill =
                    egui::Rect::from_min_size(track.min, Vec2::new(fill_w, PROGRESS_BAR_HEIGHT));
                ui.painter().rect_filled(fill, cr, t.accent);
            }
            if self.cancelable
                && IconButton::new('\u{00D7}', "Cancel")
                    .show(ui, ctx)
                    .clicked()
            {
                out.canceled = true;
            }
        });
        out
    }
}

/// A modal dialog frame: a raised card (`RADIUS_XL`, `shadow_window`) with a
/// title row and a body. [`Modal::show`] renders the frame inline (the reusable
/// recipe, used by the gallery); [`Modal::overlay`] opens it as a real modal
/// over a dim backdrop through egui's [`Modal`](egui::Modal), so the scrim and
/// focus are handled by egui rather than a hand-rolled dim layer.
///
/// Usage: `components::Modal::new("Discard changes?").overlay(ctx.egui, ctx, |ui, ctx| { .. })`.
#[derive(Clone, Debug)]
pub struct Modal {
    title: String,
}

impl Modal {
    /// A modal titled `title`.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
        }
    }

    /// The shared frame recipe (fill, border, radius, shadow, margin).
    fn frame(t: &Tokens, density: Density) -> egui::Frame {
        egui::Frame::new()
            .fill(t.bg_raised)
            .stroke(Stroke::new(1.0, t.border_strong))
            .corner_radius(Ctx::radius(RADIUS_XL))
            .shadow(shadow_window())
            .inner_margin(margin_of(Vec2::splat(density.window_margin())))
    }

    /// Renders the dialog frame inline into `ui`: a title row, then the body
    /// (which includes the button row). Returns the frame [`Response`].
    pub fn show(
        self,
        ui: &mut egui::Ui,
        ctx: Ctx,
        add_body: impl FnOnce(&mut egui::Ui, Ctx),
    ) -> Response {
        let t = ctx.tokens;
        Self::frame(&t, ctx.density)
            .show(ui, |ui| {
                ui.label(RichText::new(&self.title).strong().color(t.text));
                ui.add_space(ctx.density.item_spacing().y);
                add_body(ui, ctx);
            })
            .response
    }

    /// Opens the dialog as a real modal centered over a dim backdrop.
    pub fn overlay(
        self,
        egui_ctx: &egui::Context,
        ctx: Ctx,
        add_body: impl FnOnce(&mut egui::Ui, Ctx),
    ) -> egui::ModalResponse<()> {
        let t = ctx.tokens;
        let id = Id::new(("reticle_modal", self.title.as_str()));
        egui::Modal::new(id)
            .frame(Self::frame(&t, ctx.density))
            .show(egui_ctx, |ui| {
                ui.label(RichText::new(&self.title).strong().color(t.text));
                ui.add_space(ctx.density.item_spacing().y);
                add_body(ui, ctx);
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_button_uses_faint_text_and_keeps_fill() {
        let t = DARK;
        let (fill, text, _) = button_colors(&t, ButtonVariant::Primary, false, false, false);
        assert_eq!(text, t.text_faint);
        assert_eq!(fill, t.accent, "fill is unchanged when disabled");
    }

    #[test]
    fn ghost_is_transparent_until_hover() {
        let t = DARK;
        let (rest, _, _) = button_colors(&t, ButtonVariant::Ghost, false, false, true);
        let (hot, _, _) = button_colors(&t, ButtonVariant::Ghost, true, false, true);
        assert_eq!(rest, Color32::TRANSPARENT);
        assert_eq!(hot, t.widget_hover);
    }

    #[test]
    fn toast_appear_clamps_to_unit_range() {
        assert!((Toast::new(Severity::Info, "x").appear(1.5).appear - 1.0).abs() < f32::EPSILON);
        assert!(Toast::new(Severity::Info, "x").appear(-0.5).appear.abs() < f32::EPSILON);
        // The default is fully present, so a plain toast never fades.
        assert!((Toast::new(Severity::Info, "x").appear - 1.0).abs() < f32::EPSILON);
    }

    /// Without `.animate` the fill equals the target exactly, and with `.animate`
    /// under reduced motion (zero animation time) it is instant, equal to the
    /// target on the frame it is set. Both are the "no visible motion" paths the
    /// reduced-motion setting must guarantee.
    #[test]
    fn progress_fill_is_instant_without_animate_and_under_reduced_motion() {
        let egui_ctx = egui::Context::default();
        egui_ctx.begin_pass(egui::RawInput::default());

        let ctx = Ctx::dark(Density::Comfortable);
        let plain = ProgressRow::new("stream", 0.4);
        assert!((plain.displayed_fraction(&egui_ctx, ctx) - 0.4).abs() < f32::EPSILON);

        let ctx_reduced = ctx.with_reduced_motion(true);
        let animated = ProgressRow::new("stream", 0.6).animate("progress_test");
        assert!((animated.displayed_fraction(&egui_ctx, ctx_reduced) - 0.6).abs() < f32::EPSILON);

        let _ = egui_ctx.end_pass();
    }
}
