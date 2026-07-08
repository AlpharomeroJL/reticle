//! The hidden component gallery behind `?gallery=1` (web) and `--gallery`
//! (native): one pure render function so the visual-regression harness can
//! snapshot every component state without booting the editor.
//!
//! The signature and state shape here are FROZEN for Wave 1: lane 1D's
//! snapshot harness compiles against them from its own worktree while lane 1C
//! fills [`ui`] with the real component library. 1C may add fields to
//! [`GalleryState`] (additive) but must not change [`ui`]'s signature or the
//! [`GalleryGroup`] variants without an orchestrator ledger entry.

use eframe::egui;

use super::components::{
    Button, Collapsible, Ctx, EmptyState, IconButton, KbdChip, Modal, ProgressRow, SectionHeader,
    Segmented, Severity, TextField, Toast, ToggleChip,
};
use super::tokens::Density;

/// Which component family the gallery page shows; the visual suite snapshots
/// one image per group per density.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GalleryGroup {
    /// Buttons: primary, secondary, ghost, danger, icon buttons.
    #[default]
    Buttons,
    /// Text inputs, toggle chips, segmented controls.
    Inputs,
    /// Section headers, collapsible sections, empty-state blocks.
    Sections,
    /// Toasts, progress rows, kbd hint chips.
    Feedback,
    /// Modal dialog frames and overlay chrome.
    Overlays,
}

impl GalleryGroup {
    /// Every group, in display order.
    #[must_use]
    pub fn all() -> [GalleryGroup; 5] {
        [
            Self::Buttons,
            Self::Inputs,
            Self::Sections,
            Self::Feedback,
            Self::Overlays,
        ]
    }

    /// The tab label for this group.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Buttons => "Buttons",
            Self::Inputs => "Inputs",
            Self::Sections => "Sections",
            Self::Feedback => "Feedback",
            Self::Overlays => "Overlays",
        }
    }

    /// This group's index in [`GalleryGroup::all`].
    fn index(self) -> usize {
        Self::all().iter().position(|g| *g == self).unwrap_or(0)
    }
}

/// Mutable state behind the gallery page: the selected group plus scratch
/// values the interactive component demos bind to.
#[derive(Debug, Default)]
pub struct GalleryState {
    /// The component family currently shown.
    pub group: GalleryGroup,
    /// The density the page renders at (added by lane 1C; the snapshot harness
    /// sets it to capture one image per group per density).
    pub density: Density,
    /// Scratch text for the input demos.
    pub text_input: String,
    /// Scratch flag for toggle/chip demos.
    pub toggle_on: bool,
    /// Scratch index for the segmented-control demo.
    pub segment: usize,
    /// Scratch open flag for the collapsible-section demo.
    pub section_open: bool,
    /// Scratch fraction for progress demos.
    pub progress: f32,
}

/// Renders the gallery page for `state.group`. Pure egui over the theme's
/// components; no app or document state, which is exactly what makes it a
/// stable screenshot surface.
pub fn ui(ui: &mut egui::Ui, state: &mut GalleryState) {
    let ctx = Ctx::dark(state.density);

    // Header: the group selector and the density toggle both dogfood the
    // segmented control (scope item 7).
    let group_labels: Vec<&str> = GalleryGroup::all().iter().map(|g| g.label()).collect();
    let mut group_idx = state.group.index();
    if Segmented::new(&group_labels)
        .show(ui, ctx, &mut group_idx)
        .changed()
    {
        state.group = GalleryGroup::all()[group_idx];
    }

    let density_labels = ["Comfortable", "Compact"];
    let mut density_idx = usize::from(state.density == Density::Compact);
    if Segmented::new(&density_labels)
        .show(ui, ctx, &mut density_idx)
        .changed()
    {
        state.density = if density_idx == 1 {
            Density::Compact
        } else {
            Density::Comfortable
        };
    }

    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match state.group {
            GalleryGroup::Buttons => buttons_group(ui, ctx),
            GalleryGroup::Inputs => inputs_group(ui, ctx, state),
            GalleryGroup::Sections => sections_group(ui, ctx, state),
            GalleryGroup::Feedback => feedback_group(ui, ctx),
            GalleryGroup::Overlays => overlays_group(ui, ctx),
        });
}

/// A small labeled row: a caption on the left, then the demo widgets.
fn row(ui: &mut egui::Ui, caption: &str, contents: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal_wrapped(|ui| {
        ui.label(caption);
        contents(ui);
    });
    ui.add_space(4.0);
}

fn buttons_group(ui: &mut egui::Ui, ctx: Ctx) {
    row(ui, "Variants", |ui| {
        Button::primary("Run DRC").show(ui, ctx);
        Button::secondary("Clear").show(ui, ctx);
        Button::ghost("Details").show(ui, ctx);
        Button::danger("Delete").show(ui, ctx);
    });
    row(ui, "Disabled", |ui| {
        Button::primary("Run DRC").enabled(false).show(ui, ctx);
        Button::secondary("Clear").enabled(false).show(ui, ctx);
        Button::ghost("Details").enabled(false).show(ui, ctx);
        Button::danger("Delete").enabled(false).show(ui, ctx);
    });
    row(ui, "Icon buttons", |ui| {
        IconButton::new('+', "Add rectangle")
            .kbd("R")
            .hint("Draw a rectangle on the active layer")
            .show(ui, ctx);
        IconButton::new('\u{2261}', "Layers").kbd("L").show(ui, ctx);
        IconButton::new('\u{2317}', "Snap")
            .selected(true)
            .show(ui, ctx);
        IconButton::new('\u{00D7}', "Close")
            .enabled(false)
            .show(ui, ctx);
    });
}

fn inputs_group(ui: &mut egui::Ui, ctx: Ctx, state: &mut GalleryState) {
    row(ui, "Toggle chips", |ui| {
        if ToggleChip::new("Snap", state.toggle_on)
            .show(ui, ctx)
            .clicked()
        {
            state.toggle_on = !state.toggle_on;
        }
        ToggleChip::new("Grid", true).show(ui, ctx);
        ToggleChip::new("Guides", false).show(ui, ctx);
        ToggleChip::new("Locked", false)
            .enabled(false)
            .show(ui, ctx);
    });
    row(ui, "Segmented", |ui| {
        Segmented::new(&["Single", "Split H", "Split V"]).show(ui, ctx, &mut state.segment);
    });
    row(ui, "Segmented (disabled)", |ui| {
        let mut fixed = 0;
        Segmented::new(&["Single", "Split H", "Split V"])
            .enabled(false)
            .show(ui, ctx, &mut fixed);
    });
    row(ui, "Text input", |ui| {
        TextField::new(&mut state.text_input)
            .hint("Filter layers")
            .desired_width(180.0)
            .show(ui, ctx);
    });
    row(ui, "Text input (disabled)", |ui| {
        let mut disabled = String::from("read only");
        TextField::new(&mut disabled)
            .enabled(false)
            .desired_width(180.0)
            .show(ui, ctx);
    });
}

fn sections_group(ui: &mut egui::Ui, ctx: Ctx, state: &mut GalleryState) {
    SectionHeader::new("Properties").show(ui, ctx);
    ui.label("Section header content sits under a quiet header.");
    ui.add_space(8.0);

    Collapsible::new("gallery_collapsible", "Operations").show(
        ui,
        ctx,
        &mut state.section_open,
        |ui, ctx| {
            ui.label("Boolean and transform operations live here.");
            Button::secondary("Union").show(ui, ctx);
        },
    );
    ui.add_space(8.0);

    SectionHeader::new("Empty state").show(ui, ctx);
    EmptyState::new(
        "No selection",
        "Click a shape on the canvas, or press A to select everything.",
    )
    .action(Button::primary("Select all"))
    .action(Button::ghost("Learn more"))
    .show(ui, ctx);
}

fn feedback_group(ui: &mut egui::Ui, ctx: Ctx) {
    row(ui, "Kbd chips", |ui| {
        KbdChip::new("Ctrl K").show(ui, ctx);
        KbdChip::new("F").show(ui, ctx);
        KbdChip::new("Shift ?").show(ui, ctx);
    });
    ui.add_space(8.0);

    for (severity, message) in [
        (Severity::Info, "Streaming residency at 42 percent."),
        (Severity::Success, "DRC passed with no violations."),
        (Severity::Warning, "Layer map is stale; reload to refresh."),
        (Severity::Danger, "Open failed: unsupported GDS record."),
    ] {
        Toast::new(severity, message)
            .action(Button::secondary("Retry"))
            .action(Button::ghost("Copy"))
            .show(ui, ctx);
        ui.add_space(6.0);
    }

    ui.add_space(8.0);
    ProgressRow::new("Idle", 0.0).show(ui, ctx);
    ProgressRow::new("Streaming", 0.4)
        .cancelable(true)
        .show(ui, ctx);
    ProgressRow::new("Complete", 1.0).show(ui, ctx);
}

fn overlays_group(ui: &mut egui::Ui, ctx: Ctx) {
    Modal::new("Discard changes?").show(ui, ctx, |ui, ctx| {
        ui.label("Your edits to the top cell have not been saved.");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            Button::danger("Discard").show(ui, ctx);
            Button::secondary("Cancel").show(ui, ctx);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every group renders a full frame for both densities without panicking,
    /// on a plain headless `egui::Context` (no GPU). This is the gallery's
    /// acceptance test; the pixel snapshots are lane 1D's `ui_snapshots`.
    #[test]
    fn every_group_renders_without_panic() {
        for group in GalleryGroup::all() {
            for density in [Density::Comfortable, Density::Compact] {
                let egui_ctx = egui::Context::default();
                let mut state = GalleryState {
                    group,
                    density,
                    ..Default::default()
                };
                egui_ctx.begin_pass(egui::RawInput::default());
                let mut ui = egui::Ui::new(
                    egui_ctx.clone(),
                    egui::Id::new("gallery_test"),
                    egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 1400.0),
                    )),
                );
                super::ui(&mut ui, &mut state);
                let _ = egui_ctx.end_pass();
            }
        }
    }
}
