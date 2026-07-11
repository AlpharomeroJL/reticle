//! [`PCellRegistry`]: user-defined PCells addressable by id, parallel to the built-in
//! generator [`Registry`](crate::Registry).
//!
//! SCAFFOLD OWNED BY THE `pcell-params` LANE. A minimal id-keyed store is fixed here so the
//! Inspector UI (`pcell-inspect` lane) can list and fetch user PCells against a stable
//! interface. The `pcell-params` lane extends it (for example an `infos()` listing that
//! mirrors [`Registry::infos`](crate::Registry::infos), schema exposure, and de/serialization
//! of a saved PCell library).

use std::collections::BTreeMap;

use crate::pcell::PCellDef;

/// A registry of user-defined [`PCellDef`]s, addressable by id.
///
/// Parallel to the built-in generator [`Registry`](crate::Registry) but for the PCells a
/// user authors: register a definition, then list or fetch it by id to render its form or
/// produce it. A `BTreeMap` keeps `ids()` stable (sorted), which the UI relies on.
#[derive(Clone, Default, Debug)]
pub struct PCellRegistry {
    defs: BTreeMap<String, PCellDef>,
}

impl PCellRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `def` under its [`id`](PCellDef::id), replacing any existing PCell with
    /// that id.
    pub fn register(&mut self, def: PCellDef) {
        self.defs.insert(def.id.clone(), def);
    }

    /// The registered PCell with `id`, if any.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&PCellDef> {
        self.defs.get(id)
    }

    /// Every registered id, sorted.
    #[must_use]
    pub fn ids(&self) -> Vec<&str> {
        self.defs.keys().map(String::as_str).collect()
    }

    /// The number of registered PCells.
    #[must_use]
    pub fn len(&self) -> usize {
        self.defs.len()
    }

    /// Whether the registry holds no PCells.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// Every registered PCell, in id order: enough to list, describe, and render the
    /// parameter form for each.
    ///
    /// Mirrors [`Registry::infos`](crate::Registry::infos) for the built-in generators as
    /// the enumeration surface the Inspector lists user PCells from: each entry's
    /// [`id`](PCellDef::id), [`title`](PCellDef::title), [`description`](PCellDef::description),
    /// and [`schema`](PCellDef::schema) are exactly the fields
    /// [`GeneratorInfo`](crate::GeneratorInfo) exposes for a built-in generator. The return
    /// type differs (borrowed [`PCellDef`]s rather than an owned metadata struct) because,
    /// unlike the built-in [`Registry`](crate::Registry) (which stores `dyn`-erased generators behind a
    /// hidden concrete `Params` type and so needs a separate snapshot struct to expose
    /// metadata), a [`PCellRegistry`] already stores plain [`PCellDef`] values with nothing
    /// to erase, so borrowing them directly is simpler and avoids a clone per listing call.
    #[must_use]
    pub fn infos(&self) -> Vec<&PCellDef> {
        self.defs.values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{FieldSchema, ParamSchema};

    /// A minimal PCell definition with the given id and title, for registry tests that do
    /// not care about script content.
    fn def(id: &str, title: &str) -> PCellDef {
        PCellDef {
            id: id.to_owned(),
            title: title.to_owned(),
            description: format!("{title} description."),
            schema: ParamSchema {
                generator_id: id.to_owned(),
                title: title.to_owned(),
                description: format!("{title} description."),
                fields: vec![FieldSchema::bool("flag", "A flag.", false)],
            },
            script: "// no-op".to_owned(),
            engine_version: "0.1.0".to_owned(),
        }
    }

    #[test]
    fn new_registry_is_empty_and_lists_nothing() {
        let reg = PCellRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        assert!(reg.ids().is_empty());
        assert!(reg.infos().is_empty());
    }

    #[test]
    fn infos_lists_every_registered_pcell_sorted_by_id() {
        let mut reg = PCellRegistry::new();
        reg.register(def("user.zebra", "Zebra"));
        reg.register(def("user.alpha", "Alpha"));
        reg.register(def("user.mid", "Mid"));

        let infos = reg.infos();
        let ids: Vec<&str> = infos.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["user.alpha", "user.mid", "user.zebra"],
            "BTreeMap iteration is sorted by id, matching `ids()`"
        );
    }

    #[test]
    fn infos_exposes_id_title_description_and_schema() {
        let mut reg = PCellRegistry::new();
        reg.register(def("user.sensor", "Sensor"));

        let infos = reg.infos();
        assert_eq!(infos.len(), 1);
        let info = infos[0];
        assert_eq!(info.id, "user.sensor");
        assert_eq!(info.title, "Sensor");
        assert_eq!(info.description, "Sensor description.");
        assert_eq!(info.schema.fields.len(), 1);
        assert_eq!(info.schema.fields[0].name, "flag");
    }

    #[test]
    fn infos_round_trips_with_get() {
        let mut reg = PCellRegistry::new();
        reg.register(def("user.sensor", "Sensor"));

        let info = *reg.infos().first().expect("one info");
        let got = reg.get(&info.id).expect("registered");
        assert_eq!(info.id, got.id);
        assert_eq!(info.title, got.title);
        assert_eq!(info.description, got.description);
        assert_eq!(info.schema, got.schema);
        assert!(
            std::ptr::eq(info, got),
            "infos() borrows the same stored def `get` returns, no cloning"
        );
    }

    #[test]
    fn infos_len_always_matches_registry_len() {
        let mut reg = PCellRegistry::new();
        assert_eq!(reg.infos().len(), reg.len());

        reg.register(def("a", "A"));
        reg.register(def("b", "B"));
        assert_eq!(reg.infos().len(), reg.len());
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn registering_a_duplicate_id_replaces_it_in_infos() {
        let mut reg = PCellRegistry::new();
        reg.register(def("user.sensor", "Sensor v1"));
        reg.register(def("user.sensor", "Sensor v2"));

        let infos = reg.infos();
        assert_eq!(infos.len(), 1, "the second register replaces, not appends");
        assert_eq!(infos[0].title, "Sensor v2");
    }
}
