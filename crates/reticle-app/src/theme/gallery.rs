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
}

/// Mutable state behind the gallery page: the selected group plus scratch
/// values the interactive component demos bind to.
#[derive(Debug, Default)]
pub struct GalleryState {
    /// The component family currently shown.
    pub group: GalleryGroup,
    /// Scratch text for the input demos.
    pub text_input: String,
    /// Scratch flag for toggle/chip demos.
    pub toggle_on: bool,
    /// Scratch fraction for progress demos.
    pub progress: f32,
}

/// Renders the gallery page for `state.group`. Pure egui over the theme's
/// components; no app or document state, which is exactly what makes it a
/// stable screenshot surface.
pub fn ui(ui: &mut egui::Ui, state: &mut GalleryState) {
    // Lane 1C replaces this placeholder with the component library demos; the
    // stub renders the group name so pre-1C snapshots are deterministic.
    ui.label(format!("component gallery: {}", state.group.label()));
}
