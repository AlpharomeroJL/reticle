//! The technology-editor side panel and the upgraded layer manager.
//!
//! This module hosts two related right-panel sections:
//!
//! * an upgraded **layer manager** over the app's [`LayerState`]: reorder layers
//!   up and down, recolor them with a picker, choose a per-layer
//!   [`FillStyle`], and solo (hide-others), and
//! * a **technology editor** that edits a working copy of the live document's
//!   [`Technology`]: the database resolution, the layer table, and the DRC rule
//!   values, with validation that rejects invalid input (a non-positive
//!   resolution, a negative rule threshold, a non-positive stack thickness) before
//!   the edit ever reaches the document.
//!
//! # Editing model
//!
//! The editor holds a [`Technology`] *draft* seeded from the document. The user
//! edits the draft freely; nothing touches the live document until **Apply**, which
//! [`validate_technology`]s the draft and, only if it is clean, commits it with
//! [`History::set_technology`]. Setting
//! technology is not an undoable [`reticle_model::Edit`] (the edit vocabulary is
//! geometry-only), so the commit is applied directly to the document and is not on
//! the undo stack; it does bump the document revision so the canvas re-reads.
//!
//! # Round-trip
//!
//! The draft round-trips through the technology-file text format via
//! [`reticle_io::parse_technology`] and [`reticle_io::write_technology`]. **Load
//! from text** parses (rejecting a malformed file) and replaces the draft; **the
//! file preview** serializes the draft to the canonical form. Serialization is a
//! byte-stable fixpoint: re-saving an unedited file reproduces it exactly, though
//! comments and original spacing are not preserved (the parser discards them). See
//! the `write_technology` docs in `reticle-io` for the exact guarantee.

use crate::history::History;
use crate::layers::{self, FillStyle, LayerState};
use eframe::egui;
use reticle_geometry::LayerId;
use reticle_model::{RuleKind, Technology};

/// Working state for the technology editor: the draft being edited, the last
/// validation result, and the text-format scratch buffers.
#[derive(Clone, Debug, Default)]
pub struct TechEditorState {
    /// The technology being edited, seeded from the document on first show.
    draft: Technology,
    /// Whether the `draft` has been seeded from the live document yet.
    loaded: bool,
    /// Validation messages from the most recent Apply attempt (empty when the last
    /// attempt was clean or none has run).
    errors: Vec<String>,
    /// A one-line status from the most recent action (Apply, Load, Revert).
    status: String,
    /// The editable technology-file text, shown in the round-trip panel. Refreshed
    /// from the draft on "Show file" and parsed back on "Load from text".
    file_text: String,
    /// The last text-format parse error, shown under the text box (empty when none).
    file_error: String,
    /// Whether the collapsible technology-file text panel is expanded.
    show_file: bool,
}

impl TechEditorState {
    /// A fresh, unseeded editor. The first [`show`] seeds it from the document.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current draft technology (read-only), for tests and callers.
    #[must_use]
    pub fn draft(&self) -> &Technology {
        &self.draft
    }

    /// The validation messages from the last Apply attempt.
    #[must_use]
    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    /// Seeds the draft from `tech` (a clone of the document's technology) unless it
    /// has already been seeded. Idempotent after the first call.
    pub fn seed_from(&mut self, tech: &Technology) {
        if !self.loaded {
            self.reset_to(tech);
            self.loaded = true;
        }
    }

    /// Replaces the draft with a clone of `tech` and clears transient state.
    ///
    /// Used to seed the editor and to revert edits back to the document.
    pub fn reset_to(&mut self, tech: &Technology) {
        self.draft = tech.clone();
        self.errors.clear();
        self.file_error.clear();
    }

    /// Validates the draft and, if it is clean, returns the technology to commit.
    ///
    /// On any validation error the returned value is `None` and the `errors` list
    /// (see [`TechEditorState::errors`]) holds the messages; the draft is left
    /// untouched so the user can fix it.
    #[must_use]
    pub fn apply(&mut self) -> Option<Technology> {
        self.errors = validate_technology(&self.draft);
        if self.errors.is_empty() {
            self.set_status("Technology applied to the document.");
            Some(self.draft.clone())
        } else {
            let msg = format!(
                "{} validation error(s); nothing applied.",
                self.errors.len()
            );
            self.set_status(&msg);
            None
        }
    }

    /// Replaces the one-line status message.
    fn set_status(&mut self, message: &str) {
        self.status.clear();
        self.status.push_str(message);
    }

    /// Serializes the draft into the `file_text` buffer for the round-trip preview.
    pub fn refresh_file_text(&mut self) {
        self.file_text = reticle_io::write_technology(&self.draft);
        self.file_error.clear();
    }

    /// Parses the `file_text` buffer and replaces the draft on success.
    ///
    /// On a parse error the draft is unchanged and the `file_error` message records
    /// the reason. This is the "reverse" validation direction: a malformed file is
    /// rejected on the way in, just as an invalid draft is rejected on Apply.
    pub fn load_from_text(&mut self) {
        match reticle_io::parse_technology(&self.file_text) {
            Ok(tech) => {
                self.draft = tech;
                self.errors.clear();
                self.file_error.clear();
                self.set_status("Loaded technology from text.");
            }
            Err(e) => {
                self.file_error = format!("parse error: {e}");
            }
        }
    }
}

/// Validates a technology for commit, returning one message per problem.
///
/// An empty result means the technology is valid to apply. The checks are the
/// invariants the DRC engine and the file format assume but the in-memory
/// [`Technology`] type does not enforce on its own:
///
/// * the resolution [`Technology::dbu_per_micron`] must be positive,
/// * every rule threshold must be non-negative (a negative width, spacing, area,
///   and so on is meaningless), and additionally a length-style rule
///   (`width`/`spacing`/`enclosure`/`extension`/`notch`) must be strictly positive,
/// * a two-layer rule kind (`spacing`/`enclosure`/`extension`) must carry a second
///   layer and a single-layer kind must not, and
/// * every stack entry's thickness must be positive (matching the file parser).
///
/// Layer names may be anything non-empty; an empty layer name is rejected because
/// the file format is whitespace-tokenized and could not round-trip it.
#[must_use]
pub fn validate_technology(tech: &Technology) -> Vec<String> {
    let mut errors = Vec::new();

    if tech.dbu_per_micron <= 0 {
        errors.push(format!(
            "resolution dbu_per_micron must be positive (got {})",
            tech.dbu_per_micron
        ));
    }

    for layer in &tech.layers {
        if layer.name.trim().is_empty() {
            errors.push(format!(
                "layer {}/{} has an empty name",
                layer.id.layer, layer.id.datatype
            ));
        }
    }

    for rule in &tech.rules {
        if rule.value < 0 {
            errors.push(format!(
                "rule `{}` has a negative threshold ({})",
                rule.name, rule.value
            ));
        } else if requires_positive_value(rule.kind) && rule.value == 0 {
            errors.push(format!(
                "rule `{}` ({}) must have a positive threshold",
                rule.name,
                rule_kind_label(rule.kind)
            ));
        }
        match (is_two_layer_kind(rule.kind), rule.other_layer) {
            (true, None) => errors.push(format!(
                "rule `{}` ({}) needs a second layer",
                rule.name,
                rule_kind_label(rule.kind)
            )),
            (false, Some(other)) => errors.push(format!(
                "rule `{}` ({}) must not carry a second layer (got {}/{})",
                rule.name,
                rule_kind_label(rule.kind),
                other.layer,
                other.datatype
            )),
            _ => {}
        }
    }

    for entry in &tech.stack {
        if entry.thickness_nm <= 0 {
            errors.push(format!(
                "stack entry for layer {}/{} must have a positive thickness (got {})",
                entry.layer.layer, entry.layer.datatype, entry.thickness_nm
            ));
        }
    }

    errors
}

/// Whether `kind` is a two-layer rule (it references a second layer).
///
/// Mirrors the token-count split in the technology-file parser: `spacing`,
/// `enclosure`, and `extension` take a second layer; the rest are single-layer.
fn is_two_layer_kind(kind: RuleKind) -> bool {
    matches!(
        kind,
        RuleKind::Spacing | RuleKind::Enclosure | RuleKind::Extension
    )
}

/// Whether `kind`'s threshold must be strictly positive (a length or area rule),
/// as opposed to merely non-negative (`density`, `angle`).
fn requires_positive_value(kind: RuleKind) -> bool {
    matches!(
        kind,
        RuleKind::Width
            | RuleKind::Spacing
            | RuleKind::Enclosure
            | RuleKind::Extension
            | RuleKind::Notch
            | RuleKind::Area
    )
}

/// A short human-readable label for a rule kind (used in validation messages and
/// the editor UI). Falls back to `rule` for any future `#[non_exhaustive]` variant.
fn rule_kind_label(kind: RuleKind) -> &'static str {
    match kind {
        RuleKind::Width => "width",
        RuleKind::Spacing => "spacing",
        RuleKind::Enclosure => "enclosure",
        RuleKind::Extension => "extension",
        RuleKind::Notch => "notch",
        RuleKind::Area => "area",
        RuleKind::Density => "density",
        RuleKind::Angle => "angle",
        _ => "rule",
    }
}

/// Packs egui's `[r, g, b, a]` byte order into a `0xRRGGBBAA` value.
fn pack_rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (u32::from(r) << 24) | (u32::from(g) << 16) | (u32::from(b) << 8) | u32::from(a)
}

/// Draws the upgraded layer manager and the technology editor at the end of the
/// right panel.
///
/// The three borrows are disjoint fields of the app (`state`, `history`,
/// `layers`), so the caller's one-line `tech_editor_panel` wrapper can hand them
/// in together. `history` is only borrowed mutably for the Apply commit.
pub fn show(
    state: &mut TechEditorState,
    history: &mut History,
    layers: &mut LayerState,
    ui: &mut egui::Ui,
) {
    state.seed_from(history.document().technology());

    layer_manager(layers, ui);
    ui.separator();
    technology_editor(state, history, ui);
}

/// Draws the upgraded layer-manager section: per-layer reorder, recolor, fill
/// style, and solo, plus show/hide-all.
fn layer_manager(layers: &mut LayerState, ui: &mut egui::Ui) {
    ui.heading("Layer manager");
    ui.horizontal(|ui| {
        if ui.small_button("Show all").clicked() {
            layers.show_all();
        }
        if ui.small_button("Hide all").clicked() {
            layers.hide_all();
        }
    });

    // Deferred mutations, so the row list is not reordered or restyled mid-walk.
    // Each is at most one per frame: a click drives exactly one action.
    let mut move_up: Option<usize> = None;
    let mut move_down: Option<usize> = None;
    let mut solo: Option<LayerId> = None;
    let mut set_visible: Option<(LayerId, bool)> = None;
    let mut recolor: Option<(LayerId, u32)> = None;
    let mut refill: Option<(LayerId, FillStyle)> = None;

    let row_count = layers.rows().len();
    egui::ScrollArea::vertical()
        .id_salt("tech_layer_manager")
        .max_height(220.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, row) in layers.rows().iter().enumerate() {
                let id = row.id;
                ui.horizontal(|ui| {
                    // Reorder controls.
                    if ui
                        .add_enabled(i > 0, egui::Button::new("^").small())
                        .on_hover_text("Move up")
                        .clicked()
                    {
                        move_up = Some(i);
                    }
                    if ui
                        .add_enabled(i + 1 < row_count, egui::Button::new("v").small())
                        .on_hover_text("Move down")
                        .clicked()
                    {
                        move_down = Some(i);
                    }

                    // Recolor via a picker button seeded with the current color.
                    let (r, g, b, a) = layers::rgba_components(row.color_rgba);
                    let mut color = egui::Color32::from_rgba_unmultiplied(r, g, b, a);
                    if ui.color_edit_button_srgba(&mut color).changed() {
                        recolor = Some((id, pack_rgba(color.r(), color.g(), color.b(), color.a())));
                    }

                    // Visibility + name.
                    let mut visible = row.visible;
                    if ui.checkbox(&mut visible, &row.name).changed() {
                        set_visible = Some((id, visible));
                    }

                    // Fill-style picker.
                    let mut fill = row.fill;
                    egui::ComboBox::from_id_salt(("tech_fill", id.layer, id.datatype))
                        .selected_text(fill.label())
                        .width(72.0)
                        .show_ui(ui, |ui| {
                            for option in FillStyle::ALL {
                                ui.selectable_value(&mut fill, option, option.label());
                            }
                        });
                    if fill != row.fill {
                        refill = Some((id, fill));
                    }

                    if ui
                        .small_button("Solo")
                        .on_hover_text("Show only this layer")
                        .clicked()
                    {
                        solo = Some(id);
                    }
                });
            }
        });

    // Apply deferred mutations after the walk.
    if let Some(i) = move_up {
        layers.move_up(i);
    }
    if let Some(i) = move_down {
        layers.move_down(i);
    }
    if let Some(id) = solo {
        layers.solo(id);
    }
    if let Some((id, visible)) = set_visible {
        layers.set_visible(id, visible);
    }
    if let Some((id, rgba)) = recolor {
        layers.set_color(id, rgba);
    }
    if let Some((id, fill)) = refill {
        layers.set_fill(id, fill);
    }
}

/// Draws the technology editor: resolution, layer table, rule values, and the
/// Apply / Revert / file round-trip controls.
fn technology_editor(state: &mut TechEditorState, history: &mut History, ui: &mut egui::Ui) {
    ui.heading("Technology editor");

    ui.horizontal(|ui| {
        ui.label("Name:");
        ui.text_edit_singleline(&mut state.draft.name);
    });
    ui.horizontal(|ui| {
        ui.label("dbu / micron:");
        ui.add(egui::DragValue::new(&mut state.draft.dbu_per_micron).range(1..=1_000_000));
    });

    ui.separator();
    tech_layer_table(&mut state.draft, ui);
    ui.separator();
    tech_rule_table(&mut state.draft, ui);

    ui.separator();
    tech_actions(state, history, ui);

    // Validation errors and status from the last action.
    for err in &state.errors {
        ui.colored_label(ERROR_COLOR, err);
    }
    if !state.status.is_empty() {
        ui.label(&state.status);
    }

    ui.separator();
    tech_file_panel(state, ui);
}

/// The color validation errors and parse errors are drawn in.
const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(0xD0, 0x40, 0x40);

/// Draws the editable layer table: per-layer color, layer/datatype, and name.
fn tech_layer_table(draft: &mut Technology, ui: &mut egui::Ui) {
    ui.label(format!("Layers ({})", draft.layers.len()));
    egui::ScrollArea::vertical()
        .id_salt("tech_layer_table")
        .max_height(180.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for layer in &mut draft.layers {
                ui.horizontal(|ui| {
                    let (r, g, b, a) = layers::rgba_components(layer.color_rgba);
                    let mut color = egui::Color32::from_rgba_unmultiplied(r, g, b, a);
                    if ui.color_edit_button_srgba(&mut color).changed() {
                        layer.color_rgba = pack_rgba(color.r(), color.g(), color.b(), color.a());
                    }
                    ui.add(
                        egui::DragValue::new(&mut layer.id.layer)
                            .prefix("L")
                            .range(0..=u16::MAX),
                    );
                    ui.add(
                        egui::DragValue::new(&mut layer.id.datatype)
                            .prefix("D")
                            .range(0..=u16::MAX),
                    );
                    ui.text_edit_singleline(&mut layer.name);
                });
            }
        });
}

/// Draws the editable rule table: each rule's kind and layers as context, with its
/// threshold as a non-negative drag value (full validation runs on Apply).
fn tech_rule_table(draft: &mut Technology, ui: &mut egui::Ui) {
    ui.label(format!("Rules ({})", draft.rules.len()));
    egui::ScrollArea::vertical()
        .id_salt("tech_rule_table")
        .max_height(160.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for rule in &mut draft.rules {
                ui.horizontal(|ui| {
                    ui.label(rule_target_label(rule));
                    ui.add(egui::DragValue::new(&mut rule.value).range(0..=i64::MAX));
                });
            }
        });
}

/// A one-line description of a rule's kind and its layer(s), for the rule table.
fn rule_target_label(rule: &reticle_model::Rule) -> String {
    match rule.other_layer {
        Some(o) => format!(
            "{} {}/{} -> {}/{}",
            rule_kind_label(rule.kind),
            rule.layer.layer,
            rule.layer.datatype,
            o.layer,
            o.datatype
        ),
        None => format!(
            "{} {}/{}",
            rule_kind_label(rule.kind),
            rule.layer.layer,
            rule.layer.datatype
        ),
    }
}

/// Draws the Apply / Revert row. Apply validates and commits to the document;
/// Revert reloads the draft from the live document technology.
fn tech_actions(state: &mut TechEditorState, history: &mut History, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if ui.button("Apply").clicked()
            && let Some(tech) = state.apply()
        {
            history.set_technology(tech);
        }
        if ui.button("Revert").clicked() {
            let tech = history.document().technology().clone();
            state.reset_to(&tech);
            state.set_status("Reverted to the document technology.");
        }
    });
}

/// Draws the collapsible technology-file round-trip panel: a code editor over the
/// serialized draft, plus refresh (serialize) and load (parse) actions.
fn tech_file_panel(state: &mut TechEditorState, ui: &mut egui::Ui) {
    let just_opened = ui
        .checkbox(&mut state.show_file, "Technology file (text)")
        .changed()
        && state.show_file;
    if just_opened {
        state.refresh_file_text();
    }
    if !state.show_file {
        return;
    }

    ui.horizontal(|ui| {
        if ui.small_button("Refresh from draft").clicked() {
            state.refresh_file_text();
        }
        if ui.small_button("Load from text").clicked() {
            state.load_from_text();
        }
    });
    egui::ScrollArea::vertical()
        .id_salt("tech_file_text")
        .max_height(200.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut state.file_text)
                    .code_editor()
                    .desired_width(f32::INFINITY),
            );
        });
    if !state.file_error.is_empty() {
        ui.colored_label(ERROR_COLOR, &state.file_error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use reticle_model::{LayerInfo, Rule};

    fn tech() -> Technology {
        demo::demo_technology()
    }

    #[test]
    fn valid_technology_passes_validation() {
        assert!(validate_technology(&tech()).is_empty());
    }

    #[test]
    fn non_positive_resolution_is_rejected() {
        let mut t = tech();
        t.dbu_per_micron = 0;
        assert!(
            validate_technology(&t)
                .iter()
                .any(|e| e.contains("dbu_per_micron"))
        );
        t.dbu_per_micron = -5;
        assert!(!validate_technology(&t).is_empty());
    }

    #[test]
    fn negative_rule_value_is_rejected() {
        let mut t = tech();
        t.rules.push(Rule {
            name: "width_9_0".to_owned(),
            kind: RuleKind::Width,
            layer: LayerId::new(9, 0),
            other_layer: None,
            value: -1,
        });
        let errors = validate_technology(&t);
        assert!(errors.iter().any(|e| e.contains("negative")), "{errors:?}");
    }

    #[test]
    fn zero_width_is_rejected_but_zero_angle_is_allowed() {
        let mut t = tech();
        t.rules.push(Rule {
            name: "width_9_0".to_owned(),
            kind: RuleKind::Width,
            layer: LayerId::new(9, 0),
            other_layer: None,
            value: 0,
        });
        assert!(!validate_technology(&t).is_empty(), "zero width is invalid");

        let mut t = tech();
        t.rules.push(Rule {
            name: "angle_9_0".to_owned(),
            kind: RuleKind::Angle,
            layer: LayerId::new(9, 0),
            other_layer: None,
            value: 0,
        });
        assert!(
            validate_technology(&t).is_empty(),
            "a zero-degree angle threshold is allowed"
        );
    }

    #[test]
    fn two_layer_kind_needs_a_second_layer() {
        let mut t = tech();
        // Spacing without a second layer is fine (it is a same-layer spacing), but
        // enclosure is inherently two-layer.
        t.rules.push(Rule {
            name: "enclosure_9_0".to_owned(),
            kind: RuleKind::Enclosure,
            layer: LayerId::new(9, 0),
            other_layer: None,
            value: 10,
        });
        assert!(
            validate_technology(&t)
                .iter()
                .any(|e| e.contains("second layer"))
        );
    }

    #[test]
    fn single_layer_kind_must_not_carry_a_second_layer() {
        let mut t = tech();
        t.rules.push(Rule {
            name: "width_9_0".to_owned(),
            kind: RuleKind::Width,
            layer: LayerId::new(9, 0),
            other_layer: Some(LayerId::new(8, 0)),
            value: 10,
        });
        assert!(
            validate_technology(&t)
                .iter()
                .any(|e| e.contains("must not carry a second layer"))
        );
    }

    #[test]
    fn empty_layer_name_is_rejected() {
        let mut t = tech();
        t.layers.push(LayerInfo {
            id: LayerId::new(200, 0),
            name: "  ".to_owned(),
            color_rgba: 0xFFFF_FFFF,
            visible: true,
        });
        assert!(
            validate_technology(&t)
                .iter()
                .any(|e| e.contains("empty name"))
        );
    }

    #[test]
    fn non_positive_stack_thickness_is_rejected() {
        use reticle_model::StackEntry;
        let mut t = tech();
        t.stack.push(StackEntry {
            layer: LayerId::new(5, 0),
            z_bottom_nm: 0,
            thickness_nm: 0,
        });
        assert!(
            validate_technology(&t)
                .iter()
                .any(|e| e.contains("thickness"))
        );
    }

    #[test]
    fn apply_returns_the_draft_only_when_valid() {
        let mut state = TechEditorState::new();
        state.seed_from(&tech());
        assert!(state.apply().is_some(), "seeded demo technology is valid");
        assert!(state.errors().is_empty());

        // Corrupt the draft and confirm Apply refuses and records errors.
        state.draft.dbu_per_micron = 0;
        assert!(state.apply().is_none());
        assert!(!state.errors().is_empty());
    }

    #[test]
    fn seed_is_idempotent() {
        let mut state = TechEditorState::new();
        state.seed_from(&tech());
        state.draft.name = "edited".to_owned();
        // A second seed must not clobber in-progress edits.
        state.seed_from(&tech());
        assert_eq!(state.draft().name, "edited");
    }

    #[test]
    fn file_text_round_trips_through_draft() {
        let mut state = TechEditorState::new();
        state.seed_from(&tech());
        state.refresh_file_text();

        // Loading the just-written text back reproduces an equal draft.
        let before = state.draft().clone();
        state.load_from_text();
        let after = state.draft();
        assert_eq!(before.dbu_per_micron, after.dbu_per_micron);
        assert_eq!(before.layers, after.layers);
        assert_eq!(before.stack, after.stack);
        assert_eq!(before.rules.len(), after.rules.len());
        assert!(state.file_error.is_empty());
    }

    #[test]
    fn malformed_text_is_rejected_and_leaves_draft_intact() {
        let mut state = TechEditorState::new();
        state.seed_from(&tech());
        let before = state.draft().clone();
        state.file_text = "dbu_per_micron not_a_number\n".to_owned();
        state.load_from_text();
        assert!(!state.file_error.is_empty(), "parse error is surfaced");
        assert_eq!(
            &before,
            state.draft(),
            "a bad load must not touch the draft"
        );
    }

    #[test]
    fn edited_value_commits_and_reverts() {
        let mut state = TechEditorState::new();
        let original = tech();
        state.seed_from(&original);
        state.draft.dbu_per_micron = 2000;
        assert!(state.apply().is_some());

        // Revert restores the draft to the passed-in document technology.
        state.reset_to(&original);
        assert_eq!(state.draft().dbu_per_micron, original.dbu_per_micron);
    }
}
