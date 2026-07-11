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
//! # Provenance, live and predicted
//!
//! [`PCellPanelState::selected_produce_meta`] renders [`PCellDef::produce_meta`]
//! (`generator_id`, `engine_version`, `script_ref`, `param_hash`) computed locally
//! from the in-memory definition and the current form values: a prediction of the
//! identity a produce would stamp, available before any script ever runs.
//! [`PCellPanelState::regenerate`] (the `f2f3-wiring` lane, Gate 3) is the real
//! thing: it runs the selected PCell's script through the merged sandboxed
//! producer (`reticle_script::produce`, the `pcell-produce` lane's Gate 2
//! sandbox) and records whichever it returns, a produced top cell's
//! shape/instance/array counts plus its stamped [`ProduceMeta`], or the
//! sandbox's clean rejection message ([`ProduceOutcome`]). Both
//! `reticle_script::produce` and this module's own logic are pure (no egui, no
//! I/O) except [`PCellPanelState::params_form`], so the panel's plumbing (schema
//! access, parameter round-trip, hashing, and now the sandboxed produce itself)
//! is unit-tested without a UI context; the egui-in-a-pass smoke test lives
//! alongside the other Inspector sections in `crate::app`.
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
#[cfg(not(target_arch = "wasm32"))]
use reticle_model::Technology;
#[cfg(not(target_arch = "wasm32"))]
use reticle_script::{SandboxLimits, produce};

/// The one illustrative user PCell shipped until the `pcell-params` lane wires a real
/// authoring flow: a made-up "sensor" cell whose schema exercises Int, Bool, and Enum
/// fields so every widget kind [`PCellPanelState::params_form`] renders is exercised
/// against something real.
fn demo_pcell() -> PCellDef {
    PCellDef {
        id: "user.sensor".to_owned(),
        title: "Sensor (example)".to_owned(),
        description: "Illustrative user PCell: a demo script and schema until the \
            pcell-params lane wires real user authoring."
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
        // A real, runnable script: `PCellPanelState::regenerate` (the `f2f3-wiring`
        // lane) actually executes this through the merged sandboxed producer. A
        // square body on the chosen layer, sized by `width`, plus an outer square
        // standing in for a guard ring when `guard_ring` is set, so every schema
        // field genuinely drives the produced geometry and its `param_hash`. SKY130
        // layer numbers (li1 67/20, met1 68/20, met2 69/20), matching
        // `reticle_extract::intent_check`'s layer table, so this is real geometry on
        // real conductor layers, not arbitrary numbers.
        script: r#"create_cell("TOP");
let ld = if layer == "li1" { 67 } else if layer == "met2" { 69 } else { 68 };
add_rect("TOP", ld, 20, 0, 0, width, width);
if guard_ring {
    add_rect("TOP", ld, 20, -100, -100, width + 100, width + 100);
}
set_top_cells(["TOP"]);
"#
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

/// The outcome of a REAL sandboxed produce of a PCell (see
/// [`PCellPanelState::regenerate`]): either the produced top cell's
/// shape/instance/array counts plus its stamped F2 [`ProduceMeta`], or the
/// sandbox's clean rejection message. The sandbox (`reticle_script::produce`)
/// never panics on any script or parameter input, so `Failed` always carries a
/// readable diagnostic, never a crash.
#[derive(Clone, PartialEq, Debug)]
pub enum ProduceOutcome {
    /// The sandboxed run produced a top cell.
    Produced {
        /// The produced top cell's own (unflattened) shape count.
        shape_count: usize,
        /// The produced top cell's instance-placement count.
        instance_count: usize,
        /// The produced top cell's array-placement count.
        array_count: usize,
        /// The F2 provenance the sandboxed run stamped over the effective
        /// parameters: `generator_id`, `engine_version`, `script_ref`, and
        /// `param_hash`.
        meta: ProduceMeta,
    },
    /// The sandbox rejected the script or parameters: invalid parameters, a
    /// script compile/runtime error, a sandbox resource limit, or no top cell.
    /// The message is `reticle_script::ProduceError`'s `Display` text.
    Failed(String),
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
    /// The last real sandboxed-produce outcome for each PCell, parallel to `ids`.
    /// `None` until [`PCellPanelState::regenerate`] has run for that PCell since it
    /// was last selected; cleared again by an edited parameter
    /// ([`PCellPanelState::params_form`]) or a reset
    /// ([`PCellPanelState::reset_selected_to_defaults`]) so a shown outcome always
    /// reflects the parameters currently in the form, never a stale one.
    produced: Vec<Option<ProduceOutcome>>,
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
        let produced = vec![None; ids.len()];
        Self {
            registry,
            ids,
            selected: 0,
            params,
            produced,
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

    /// The predicted F2 provenance for the selected PCell's current parameters:
    /// its [`ProduceMeta`] (`generator_id`, `engine_version`, `script_ref`,
    /// `param_hash`), computed locally from [`PCellDef::produce_meta`].
    ///
    /// A prediction, not a produce: computing it never runs the script (see the
    /// module doc). [`PCellPanelState::regenerate`] is the real thing.
    #[must_use]
    pub fn selected_produce_meta(&self) -> ProduceMeta {
        self.selected_def().produce_meta(self.selected_params())
    }

    /// The selected PCell's last real sandboxed-produce outcome, if
    /// [`PCellPanelState::regenerate`] has run since the selection or its
    /// parameters last changed.
    #[must_use]
    pub fn selected_produce_outcome(&self) -> Option<&ProduceOutcome> {
        self.produced[self.selected].as_ref()
    }

    /// Runs a REAL sandboxed produce of the selected PCell over its current
    /// parameters (`reticle_script::produce`, under `limits` against `tech`),
    /// records the outcome, and returns it.
    ///
    /// This is the live counterpart to
    /// [`PCellPanelState::selected_produce_meta`]: rather than only predicting the
    /// F2 provenance from the definition, it actually runs the selected PCell's
    /// script through the merged sandboxed producer and keeps whichever it
    /// returns, a produced top cell's shape/instance/array counts plus its
    /// stamped [`ProduceMeta`], or the sandbox's clean rejection message. Never
    /// panics: `reticle_script::produce` is itself panic-free on any script or
    /// parameter input (see its docs).
    ///
    /// Native-only: the rhai sandbox is kept out of the wasm bundle (ADR 0115), so
    /// the browser shows the predicted provenance and defers live produce to the
    /// desktop app, mirroring the native-only agent runner.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn regenerate(&mut self, tech: &Technology, limits: SandboxLimits) -> &ProduceOutcome {
        // Both borrows are shared (`selected_def`/`selected_params` take `&self`),
        // so this scope ends before `self.produced[..]` needs `&mut self`; no
        // clone of the definition or parameters is needed just to call `produce`.
        let outcome = match produce(self.selected_def(), self.selected_params(), tech, limits) {
            Ok((cell, meta)) => ProduceOutcome::Produced {
                shape_count: cell.shapes.len(),
                instance_count: cell.instances.len(),
                array_count: cell.arrays.len(),
                meta,
            },
            Err(e) => ProduceOutcome::Failed(e.to_string()),
        };
        self.produced[self.selected] = Some(outcome);
        self.produced[self.selected]
            .as_ref()
            .expect("just inserted Some above")
    }

    /// Resets the selected PCell's parameters to its schema defaults, and clears
    /// its recorded produce outcome (which described the parameters before the
    /// reset).
    pub fn reset_selected_to_defaults(&mut self) {
        self.params[self.selected] = default_params(self.selected_schema());
        self.produced[self.selected] = None;
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
        if changed {
            // The shown produce outcome (if any) described the parameters before
            // this edit; clear it so `crate::app`'s Inspector never displays a
            // stale real result next to freshly edited, not-yet-regenerated
            // parameters.
            self.produced[self.selected] = None;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::{PCellPanelState, ProduceOutcome};
    use reticle_gen::FieldType;
    use reticle_model::Technology;
    use reticle_script::SandboxLimits;

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

    /// A fresh state has never been produced.
    #[test]
    fn new_state_has_no_produce_outcome_yet() {
        let state = PCellPanelState::new();
        assert!(state.selected_produce_outcome().is_none());
    }

    /// `regenerate` runs the REAL sandboxed producer (not a local prediction): it
    /// yields real geometry and its stamped `param_hash` equals
    /// `PCellDef::effective_param_hash` over the current parameters, the exact
    /// identity a cache-aware caller would key a regenerate lookup on.
    #[test]
    fn regenerate_runs_the_real_sandbox_and_matches_effective_param_hash() {
        let mut state = PCellPanelState::new();
        let tech = Technology::default();
        let expected_hash = state
            .selected_def()
            .effective_param_hash(state.selected_params());

        let outcome = state.regenerate(&tech, SandboxLimits::default());
        match outcome {
            ProduceOutcome::Produced {
                shape_count, meta, ..
            } => {
                assert!(*shape_count >= 1, "the demo script draws real geometry");
                assert_eq!(meta.generator_id, "user.sensor");
                assert_eq!(
                    meta.param_hash, expected_hash,
                    "produce's stamped hash must equal effective_param_hash"
                );
            }
            ProduceOutcome::Failed(msg) => {
                panic!("expected the demo PCell to produce, sandbox rejected it: {msg}");
            }
        }
        assert!(
            state.selected_produce_outcome().is_some(),
            "the outcome is recorded and retrievable"
        );
    }

    /// The demo script's `guard_ring` parameter genuinely drives the produced
    /// geometry (not a fixed/faked shape count): turning it on adds a second
    /// shape.
    #[test]
    fn regenerate_shape_count_is_param_driven() {
        let mut state = PCellPanelState::new();
        let tech = Technology::default();

        state.selected_params_mut()["guard_ring"] = serde_json::Value::from(false);
        let without = match state.regenerate(&tech, SandboxLimits::default()) {
            ProduceOutcome::Produced { shape_count, .. } => *shape_count,
            ProduceOutcome::Failed(msg) => panic!("sandbox rejected: {msg}"),
        };

        state.selected_params_mut()["guard_ring"] = serde_json::Value::from(true);
        let with = match state.regenerate(&tech, SandboxLimits::default()) {
            ProduceOutcome::Produced { shape_count, .. } => *shape_count,
            ProduceOutcome::Failed(msg) => panic!("sandbox rejected: {msg}"),
        };

        assert_eq!(without, 1, "guard_ring off draws just the body rect");
        assert_eq!(with, 2, "guard_ring on adds the outer rect");
    }

    /// A reset after a regenerate clears the recorded outcome: it described the
    /// pre-reset parameters, so showing it after would be stale, not real.
    #[test]
    fn reset_after_regenerate_clears_the_stale_outcome() {
        let mut state = PCellPanelState::new();
        let tech = Technology::default();
        state.regenerate(&tech, SandboxLimits::default());
        assert!(state.selected_produce_outcome().is_some());

        state.reset_selected_to_defaults();
        assert!(
            state.selected_produce_outcome().is_none(),
            "a reset invalidates the stale outcome"
        );
    }
}
