//! The PCell Inspector panel: render a user-authored PCell's parameter form from its
//! schema, and show its F2 provenance ([`ProduceMeta`]/`param_hash`) read-only.
//!
//! # The pieces (Phase 2; ADR 0107, lane `pcell-inspect`)
//!
//! The panel is a thin driver over [`reticle_gen`]'s PCell scaffolding: a
//! [`PCellRegistry`] of [`PCellDef`]s, each carrying a [`ParamSchema`] rendered the
//! same way the Generate panel renders a built-in generator's schema (see
//! [`crate::generate_panel`]): an [`Int`](reticle_gen::FieldType::Int) field becomes
//! a bounded [`DragValue`](egui::DragValue), a [`Bool`](reticle_gen::FieldType::Bool)
//! a checkbox, and an [`Enum`](reticle_gen::FieldType::Enum) a combo box.
//!
//! # Provenance, not production
//!
//! This lane renders [`PCellDef::produce_meta`] (`generator_id`, `engine_version`,
//! `script_ref`, `param_hash`) computed locally from the in-memory definition and
//! the current form values, and shows it read-only. It never calls
//! `reticle_script::pcell::produce`: the sandboxed rhai run is the `pcell-produce`
//! lane's Gate 2 wiring. Everything in this module is pure (no egui) except
//! [`PCellPanelState::params_form`], so the panel's plumbing (schema access,
//! parameter round-trip, hashing) is unit-tested without a UI context; the
//! egui-in-a-pass smoke test lives alongside the other Inspector sections in
//! `crate::app`.
//!
//! # The demo catalog
//!
//! No PCell-authoring flow exists yet (that is the parallel `pcell-params` lane's
//! scope; see ADR 0107), so this module seeds one illustrative [`PCellDef`]
//! (`user.sensor`) whose schema covers every field kind, exactly as a future
//! authored PCell would look to this panel. Nothing here assumes the registry holds
//! exactly one entry, so swapping the demo catalog for a real authored registry is a
//! same-shape change.

use eframe::egui;

use reticle_gen::{FieldSchema, FieldType, PCellDef, PCellRegistry, ParamSchema, ProduceMeta};

/// The one illustrative user PCell shipped until the `pcell-params` lane wires a real
/// authoring flow: a made-up "sensor" cell whose schema exercises Int, Bool, and Enum
/// fields so every widget kind [`PCellPanelState::params_form`] renders is exercised
/// against something real.
fn demo_pcell() -> PCellDef {
    PCellDef {
        id: "user.sensor".to_owned(),
        title: "Sensor (example)".to_owned(),
        description: "Illustrative user PCell: a placeholder script and schema until \
            the pcell-params lane wires real authoring."
            .to_owned(),
        schema: ParamSchema {
            generator_id: "user.sensor".to_owned(),
            title: "Sensor (example)".to_owned(),
            description: "Illustrative parameters covering every field kind.".to_owned(),
            fields: vec![
                FieldSchema::int("width", "Sensor width.", 800, 100, 5000, "dbu"),
                FieldSchema::bool("guard_ring", "Include a guard ring.", true),
                FieldSchema::enumerated(
                    "layer",
                    "Routing layer for the sensor's leads.",
                    &["li1", "met1", "met2"],
                    "met1",
                ),
            ],
        },
        // A placeholder script: only the `pcell-produce` lane's sandboxed engine will
        // ever execute this. This lane renders only the definition's schema/identity.
        script:
            "// pcell-produce lane (Gate 2) executes this under a sandbox.\ncreate_cell(\"TOP\");"
                .to_owned(),
        engine_version: "8.1.0".to_owned(),
    }
}

/// The default parameter object for `schema`: every field's own default, keyed by
/// field name, exactly the shape [`PCellDef::param_hash`] and
/// [`PCellPanelState::params_form`] both consume.
fn default_params(schema: &ParamSchema) -> serde_json::Value {
    let mut map = serde_json::Map::with_capacity(schema.fields.len());
    for field in &schema.fields {
        map.insert(field.name.clone(), field.default.clone());
    }
    serde_json::Value::Object(map)
}

/// The PCell Inspector panel's state: the user-PCell catalog, the current selection,
/// and its per-PCell typed form values.
///
/// Mirrors [`crate::generate_panel::GeneratePanelState`]: each registered
/// [`PCellDef`] keeps its own parameter object, seeded from its schema defaults, so
/// switching the selection never loses a half-filled form.
#[derive(Debug)]
pub struct PCellPanelState {
    /// The registered user PCells, addressable by id.
    registry: PCellRegistry,
    /// Every registered id, in registry order (sorted; parallel to `params`).
    ids: Vec<String>,
    /// The index into `ids` of the selected PCell.
    selected: usize,
    /// The current parameter object for each PCell, parallel to `ids`, each seeded
    /// from that PCell's schema defaults.
    params: Vec<serde_json::Value>,
}

impl Default for PCellPanelState {
    fn default() -> Self {
        let mut registry = PCellRegistry::new();
        registry.register(demo_pcell());
        let ids: Vec<String> = registry.ids().into_iter().map(str::to_owned).collect();
        let params = ids
            .iter()
            .map(|id| default_params(&registry.get(id).expect("just listed").schema))
            .collect();
        Self {
            registry,
            ids,
            selected: 0,
            params,
        }
    }
}

impl PCellPanelState {
    /// Builds the panel state from the (currently illustrative) PCell catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every registered PCell id, in display order.
    #[must_use]
    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    /// The index of the selected PCell.
    #[must_use]
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Selects the PCell at `index`, if in range.
    pub fn select(&mut self, index: usize) {
        if index < self.ids.len() {
            self.selected = index;
        }
    }

    /// The PCell definition at catalog `index` (panics if out of range; callers
    /// iterate `0..ids().len()`, as [`GeneratePanelState::infos`](crate::generate_panel::GeneratePanelState::infos)'s callers do).
    #[must_use]
    pub fn def_at(&self, index: usize) -> &PCellDef {
        self.registry
            .get(&self.ids[index])
            .expect("catalog id always resolves in its own registry")
    }

    /// The selected PCell's definition.
    #[must_use]
    pub fn selected_def(&self) -> &PCellDef {
        self.def_at(self.selected)
    }

    /// The selected PCell's schema.
    #[must_use]
    pub fn selected_schema(&self) -> &ParamSchema {
        &self.selected_def().schema
    }

    /// The selected PCell's current parameter object (mutable).
    pub fn selected_params_mut(&mut self) -> &mut serde_json::Value {
        &mut self.params[self.selected]
    }

    /// The selected PCell's current parameter object.
    #[must_use]
    pub fn selected_params(&self) -> &serde_json::Value {
        &self.params[self.selected]
    }

    /// The F2 provenance for the selected PCell's current parameters: its
    /// [`ProduceMeta`] (`generator_id`, `engine_version`, `script_ref`,
    /// `param_hash`), computed locally from [`PCellDef::produce_meta`].
    ///
    /// Read-only: computing it never produces geometry (see the module doc); this
    /// lane does not call `reticle_script::pcell::produce`.
    #[must_use]
    pub fn selected_produce_meta(&self) -> ProduceMeta {
        self.selected_def().produce_meta(self.selected_params())
    }

    /// Resets the selected PCell's parameters to its schema defaults.
    pub fn reset_selected_to_defaults(&mut self) {
        self.params[self.selected] = default_params(self.selected_schema());
    }

    /// Renders the form for the selected PCell's parameters into `ui`, mutating the
    /// current parameter object, and returns whether any field changed this frame.
    ///
    /// Identical widget mapping to
    /// [`GeneratePanelState::params_form`](crate::generate_panel::GeneratePanelState::params_form):
    /// [`Int`](FieldType::Int) maps to a range-clamped [`DragValue`](egui::DragValue),
    /// [`Bool`](FieldType::Bool) to a checkbox, and [`Enum`](FieldType::Enum) to a
    /// combo box over the variants. The field's `doc` is shown on hover.
    pub fn params_form(&mut self, ui: &mut egui::Ui) -> bool {
        // Clone the schema fields out so `params` can be borrowed mutably in the loop
        // without also borrowing `self.registry`/`self.ids`.
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
                        egui::ComboBox::from_id_salt((field.name.as_str(), "pcell_enum"))
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
    use super::PCellPanelState;
    use reticle_gen::FieldType;

    /// The demo catalog seeds at least one PCell whose schema exercises every field
    /// kind the form renders (Int/Bool/Enum), with defaults that carry a well-formed
    /// F2 provenance hash.
    #[test]
    fn demo_catalog_covers_every_field_kind_with_working_defaults() {
        let state = PCellPanelState::new();
        assert!(!state.ids().is_empty(), "at least one demo PCell is seeded");

        let schema = state.selected_schema();
        assert!(
            schema
                .fields
                .iter()
                .any(|f| matches!(f.ty, FieldType::Int { .. })),
            "demo schema needs an Int field"
        );
        assert!(
            schema
                .fields
                .iter()
                .any(|f| matches!(f.ty, FieldType::Bool)),
            "demo schema needs a Bool field"
        );
        assert!(
            schema
                .fields
                .iter()
                .any(|f| matches!(f.ty, FieldType::Enum { .. })),
            "demo schema needs an Enum field"
        );

        let meta = state.selected_produce_meta();
        assert_eq!(meta.generator_id, state.selected_def().id);
        assert_eq!(
            meta.script_ref.as_deref(),
            Some(state.selected_def().id.as_str())
        );
        assert!(
            meta.has_valid_hash(),
            "default params hash to a valid digest"
        );
    }

    /// Editing a field's value in the params object changes the provenance hash (the
    /// exact wiring `params_form`'s widget arms mutate feeds `PCellDef::param_hash`
    /// directly), and resetting restores the original.
    #[test]
    fn editing_params_changes_the_hash_and_reset_restores_it() {
        let mut state = PCellPanelState::new();
        let before = state.selected_produce_meta().param_hash;

        // Mutate the Int field directly, exactly as `params_form`'s DragValue arm
        // would on a change.
        let field_name = state
            .selected_schema()
            .fields
            .iter()
            .find(|f| matches!(f.ty, FieldType::Int { .. }))
            .expect("an Int field exists")
            .name
            .clone();
        let current = state.selected_params()[&field_name].as_i64().unwrap_or(0);
        state.selected_params_mut()[&field_name] = serde_json::Value::from(current + 1);

        let after = state.selected_produce_meta().param_hash;
        assert_ne!(before, after, "an edited parameter changes the param_hash");

        state.reset_selected_to_defaults();
        assert_eq!(
            state.selected_produce_meta().param_hash,
            before,
            "reset restores the original hash"
        );
    }

    /// Selecting an index out of range is a no-op, matching
    /// `GeneratePanelState::select`'s bounds-checked behavior.
    #[test]
    fn select_ignores_out_of_range_index() {
        let mut state = PCellPanelState::new();
        let before = state.selected();
        state.select(9999);
        assert_eq!(state.selected(), before);
    }
}
