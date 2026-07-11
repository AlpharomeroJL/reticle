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
}
