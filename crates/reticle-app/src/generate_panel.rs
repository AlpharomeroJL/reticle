//! The Generate panel: pick a parameterized layout generator, fill a typed form
//! built from its schema, preview the geometry live, and place it as one undo step.
//!
//! The panel is a thin driver over [`reticle_gen`]'s registry. It lists the built-in
//! generators from [`Registry::with_builtins`], and for the selected one it renders a
//! form straight from the generator's [`ParamSchema`](reticle_gen::ParamSchema):
//! an [`Int`](reticle_gen::FieldType::Int) field becomes a bounded
//! [`DragValue`](egui::DragValue), a [`Bool`](reticle_gen::FieldType::Bool) a
//! checkbox, and an [`Enum`](reticle_gen::FieldType::Enum) a combo box. The field
//! values live in a per-generator `serde_json::Value` seeded from the generator's
//! defaults, so the form round-trips through the same JSON parameter path the agent
//! and MCP surfaces use.
//!
//! # Preview and placement
//!
//! [`GeneratePanelState::preview_shapes`] generates the current parameters into a
//! scratch [`Cell`] and returns the shapes, so the app can draw them as a live
//! overlay on the canvas as the form changes (empty on a validation error, which the
//! panel also surfaces as text). [`GeneratePanelState::placement_edits`] turns the
//! same geometry into a batch of [`Edit::AddShape`](reticle_model::Edit) the app
//! applies through `History::apply_group`, so the whole generated structure lands as
//! a single undo step: one Undo removes all of it.
//!
//! All the geometry-producing logic here is pure (no egui), so it is unit-tested
//! without a UI context; the app module owns only the thin form rendering and the
//! canvas overlay.

use eframe::egui;

use reticle_gen::{FieldType, GeneratorInfo, ParamSchema, Registry};
use reticle_model::{Cell, DrawShape, Edit, Technology};

/// The Generate panel's state: the generator catalog, the current selection, and the
/// per-generator parameter values.
///
/// The catalog ([`GeneratorInfo`] list) and the parameter values are built once from
/// the registry at construction. Selecting a different generator switches which
/// parameter set the form edits; each generator keeps its own values, so flipping
/// between generators does not lose a half-filled form.
#[derive(Debug)]
pub struct GeneratePanelState {
    /// The built-in generators, in registry order (id, title, description, schema).
    infos: Vec<GeneratorInfo>,
    /// The index into [`infos`](Self::infos) of the selected generator.
    selected: usize,
    /// The current parameter object for each generator, parallel to
    /// [`infos`](Self::infos), each seeded from that generator's schema defaults.
    params: Vec<serde_json::Value>,
    /// Whether the live preview overlay is drawn on the canvas.
    pub preview: bool,
}

impl Default for GeneratePanelState {
    fn default() -> Self {
        let registry = Registry::with_builtins();
        let infos = registry.infos();
        // Seed each generator's form with its schema defaults, so the panel opens on a
        // working example that generates unchanged.
        let params = infos
            .iter()
            .map(|info| {
                registry
                    .default_params(info.id)
                    .unwrap_or(serde_json::Value::Null)
            })
            .collect();
        Self {
            infos,
            selected: 0,
            params,
            preview: true,
        }
    }
}

impl GeneratePanelState {
    /// Builds the panel state from the built-in generator registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The generator catalog, in registry order.
    #[must_use]
    pub fn infos(&self) -> &[GeneratorInfo] {
        &self.infos
    }

    /// The index of the selected generator.
    #[must_use]
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Selects the generator at `index`, if in range.
    pub fn select(&mut self, index: usize) {
        if index < self.infos.len() {
            self.selected = index;
        }
    }

    /// The selected generator's metadata.
    #[must_use]
    pub fn selected_info(&self) -> &GeneratorInfo {
        &self.infos[self.selected]
    }

    /// The selected generator's schema.
    #[must_use]
    pub fn selected_schema(&self) -> &ParamSchema {
        &self.selected_info().schema
    }

    /// The selected generator's id.
    #[must_use]
    pub fn selected_id(&self) -> &'static str {
        self.selected_info().id
    }

    /// The selected generator's current parameter object (mutable).
    pub fn selected_params_mut(&mut self) -> &mut serde_json::Value {
        &mut self.params[self.selected]
    }

    /// The selected generator's current parameter object.
    #[must_use]
    pub fn selected_params(&self) -> &serde_json::Value {
        &self.params[self.selected]
    }

    /// Generates the selected generator's geometry from the current parameters into a
    /// scratch cell, using `tech`, and returns the produced shapes.
    ///
    /// Returns `Ok` with the shapes on success or `Err` with the generator's own
    /// error message (a range or cross-field violation) on failure. The app uses the
    /// `Ok` shapes for the live preview overlay and the placement, and shows the
    /// `Err` text so the user sees why a parameter is rejected.
    pub fn generate_into_scratch(&self, tech: &Technology) -> Result<Vec<DrawShape>, String> {
        let registry = Registry::with_builtins();
        let mut scratch = Cell::new("__generate_preview");
        registry
            .generate(
                self.selected_id(),
                self.selected_params(),
                tech,
                &mut scratch,
            )
            .map_err(|e| e.to_string())?;
        Ok(scratch.shapes)
    }

    /// The live-preview shapes for the current selection and parameters, or an empty
    /// list when the preview is off or the parameters do not currently generate.
    ///
    /// This never surfaces the error (the form does); it just yields nothing to draw,
    /// so a mid-edit invalid state simply shows no overlay rather than flickering an
    /// error on the canvas.
    #[must_use]
    pub fn preview_shapes(&self, tech: &Technology) -> Vec<DrawShape> {
        if !self.preview {
            return Vec::new();
        }
        self.generate_into_scratch(tech).unwrap_or_default()
    }

    /// The edits that place the generated structure into `cell` as one undo group.
    ///
    /// Each generated shape is one [`Edit::AddShape`]; applied together through
    /// `History::apply_group` they form a single logical undo step, so one Undo
    /// removes the whole generated structure. Returns the generator's error message
    /// if the parameters do not generate (the app then places nothing).
    pub fn placement_edits(&self, cell: &str, tech: &Technology) -> Result<Vec<Edit>, String> {
        let shapes = self.generate_into_scratch(tech)?;
        Ok(shapes
            .into_iter()
            .map(|shape| Edit::AddShape {
                cell: cell.to_owned(),
                shape,
            })
            .collect())
    }

    /// Resets the selected generator's parameters to its schema defaults.
    pub fn reset_selected_to_defaults(&mut self) {
        if let Some(defaults) = Registry::with_builtins().default_params(self.selected_id()) {
            self.params[self.selected] = defaults;
        }
    }

    /// Renders the form for the selected generator's parameters into `ui`, mutating
    /// the current parameter object, and returns whether any field changed this frame.
    ///
    /// Each schema field maps to one widget:
    /// [`Int`](reticle_gen::FieldType::Int) to a [`DragValue`](egui::DragValue)
    /// clamped to the field's `[min, max]`, [`Bool`](reticle_gen::FieldType::Bool) to
    /// a checkbox, and [`Enum`](reticle_gen::FieldType::Enum) to a combo box over the
    /// variants. The field's `doc` is shown on hover. Returns `true` if the user
    /// changed a value, so the caller can refresh the preview.
    pub fn params_form(&mut self, ui: &mut egui::Ui) -> bool {
        // Clone the schema fields out so the params object can be borrowed mutably in
        // the loop without also borrowing `self.infos`.
        let fields = self.selected_schema().fields.clone();
        let params = &mut self.params[self.selected];
        let mut changed = false;
        for field in &fields {
            ui.horizontal(|ui| {
                ui.label(&field.name).on_hover_text(&field.doc);
                match &field.ty {
                    FieldType::Int { min, max, step } => {
                        let mut value = params
                            .get(&field.name)
                            .and_then(serde_json::Value::as_i64)
                            .unwrap_or(0);
                        let speed = (*step as f64).max(1.0);
                        let widget = egui::DragValue::new(&mut value)
                            .speed(speed)
                            .range(*min..=*max);
                        if ui.add(widget).changed() {
                            params[&field.name] = serde_json::Value::from(value);
                            changed = true;
                        }
                        if let Some(unit) = &field.unit {
                            ui.label(unit);
                        }
                    }
                    FieldType::Bool => {
                        let mut value = params
                            .get(&field.name)
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false);
                        if ui.checkbox(&mut value, "").changed() {
                            params[&field.name] = serde_json::Value::from(value);
                            changed = true;
                        }
                    }
                    FieldType::Enum { variants } => {
                        let current = params
                            .get(&field.name)
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        egui::ComboBox::from_id_salt((field.name.as_str(), "gen_enum"))
                            .selected_text(current.clone())
                            .show_ui(ui, |ui| {
                                for variant in variants {
                                    let mut selected = current == *variant;
                                    if ui.selectable_label(selected, variant).clicked() {
                                        selected = true;
                                    }
                                    if selected && current != *variant {
                                        params[&field.name] =
                                            serde_json::Value::from(variant.clone());
                                        changed = true;
                                    }
                                }
                            });
                    }
                }
            });
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::GeneratePanelState;
    use reticle_model::{Edit, Technology};

    /// The panel lists every built-in generator, seeded with default parameters that
    /// generate a non-empty structure.
    #[test]
    fn lists_generators_with_working_defaults() {
        let state = GeneratePanelState::new();
        let tech = Technology::default();
        assert!(
            state.infos().len() >= 6,
            "the six built-in generators are listed, got {}",
            state.infos().len()
        );
        // Every generator's default parameters generate cleanly (they are the
        // schema's working example).
        for i in 0..state.infos().len() {
            let mut s = GeneratePanelState::new();
            s.select(i);
            let shapes = s
                .generate_into_scratch(&tech)
                .unwrap_or_else(|e| panic!("generator {} defaults generate: {e}", s.selected_id()));
            assert!(
                !shapes.is_empty(),
                "generator {} default produces geometry",
                s.selected_id()
            );
        }
    }

    /// Selecting a generator switches the schema and keeps each generator's own
    /// parameter set independent.
    #[test]
    fn selection_switches_schema_and_keeps_params() {
        let mut state = GeneratePanelState::new();
        // Find the via_farm and guard_ring indices by id.
        let farm = state
            .infos()
            .iter()
            .position(|i| i.id == "via_farm")
            .expect("via_farm registered");
        let ring = state
            .infos()
            .iter()
            .position(|i| i.id == "guard_ring")
            .expect("guard_ring registered");

        state.select(farm);
        assert_eq!(state.selected_id(), "via_farm");
        assert_eq!(state.selected_schema().generator_id, "via_farm");
        // Mutate via_farm's rows.
        state.selected_params_mut()["rows"] = serde_json::Value::from(5);

        state.select(ring);
        assert_eq!(state.selected_id(), "guard_ring");
        // guard_ring params are untouched by the via_farm edit.
        assert!(state.selected_params().get("rows").is_none());

        // Back to via_farm: the edit persisted.
        state.select(farm);
        assert_eq!(state.selected_params()["rows"], serde_json::Value::from(5));
    }

    /// The preview shapes match the placement edits: the same geometry drives the
    /// overlay and the undo-integrated placement, and placement is one group.
    #[test]
    fn preview_matches_placement_and_is_one_group() {
        let mut state = GeneratePanelState::new();
        let farm = state
            .infos()
            .iter()
            .position(|i| i.id == "via_farm")
            .expect("via_farm registered");
        state.select(farm);
        state.selected_params_mut()["rows"] = serde_json::Value::from(3);
        state.selected_params_mut()["cols"] = serde_json::Value::from(3);
        let tech = Technology::default();

        let preview = state.preview_shapes(&tech);
        // A 3x3 mcon farm: nine cuts plus two plates.
        assert_eq!(preview.len(), 11);

        let edits = state
            .placement_edits("top", &tech)
            .expect("placement edits");
        assert_eq!(edits.len(), preview.len(), "one AddShape per preview shape");
        // Every edit targets the requested cell and adds a shape (so one apply_group
        // over the batch is a single undo step for the whole structure).
        for edit in &edits {
            match edit {
                Edit::AddShape { cell, .. } => assert_eq!(cell, "top"),
                other => panic!("expected AddShape, got {other:?}"),
            }
        }
    }

    /// The preview is empty when the toggle is off, and empty (not a panic) when the
    /// parameters do not currently generate.
    #[test]
    fn preview_is_empty_when_off_or_invalid() {
        let mut state = GeneratePanelState::new();
        let tech = Technology::default();
        state.preview = false;
        assert!(
            state.preview_shapes(&tech).is_empty(),
            "off: nothing to draw"
        );

        state.preview = true;
        let farm = state
            .infos()
            .iter()
            .position(|i| i.id == "via_farm")
            .expect("via_farm registered");
        state.select(farm);
        // rows = 0 is out of range; the generator rejects it, so the preview yields
        // nothing rather than erroring on the canvas.
        state.selected_params_mut()["rows"] = serde_json::Value::from(0);
        assert!(
            state.preview_shapes(&tech).is_empty(),
            "invalid: no overlay"
        );
        // The error path surfaces the message for the form to show.
        assert!(state.generate_into_scratch(&tech).is_err());
    }

    /// Resetting restores the selected generator's schema defaults.
    #[test]
    fn reset_restores_defaults() {
        let mut state = GeneratePanelState::new();
        let farm = state
            .infos()
            .iter()
            .position(|i| i.id == "via_farm")
            .expect("via_farm registered");
        state.select(farm);
        let default_rows = state.selected_params()["rows"].clone();
        state.selected_params_mut()["rows"] = serde_json::Value::from(99);
        state.reset_selected_to_defaults();
        assert_eq!(state.selected_params()["rows"], default_rows);
    }
}
