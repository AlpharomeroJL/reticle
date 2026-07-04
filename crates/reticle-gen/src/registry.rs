//! The generator registry: enumerate and invoke generators by id, generically.
//!
//! The [`Registry`] maps a generator id to its type-erased handle
//! ([`ErasedGenerator`](crate::ErasedGenerator)) and its metadata. It is how the app
//! (lane 2D) builds the Generate panel and how the agent lists tools: iterate
//! [`Registry::infos`] to enumerate what exists and its schema, then call
//! [`Registry::generate`] with an id and JSON parameters to drive one, all without
//! naming any concrete generator type.

use reticle_model::{Cell, Technology};
use serde_json::Value;

use crate::error::GenError;
use crate::generator::{ErasedGenerator, GenOutput};
use crate::guard_ring::GuardRing;
use crate::schema::ParamSchema;
use crate::via_farm::ViaFarm;

/// Enumerable metadata for one registered generator: enough to list it, describe it,
/// and build its form or tool schema without invoking it.
#[derive(Clone, PartialEq, Debug)]
pub struct GeneratorInfo {
    /// The stable machine id (the registry key and tool name).
    pub id: &'static str,
    /// Human-readable title.
    pub title: &'static str,
    /// One-paragraph description of what the generator emits.
    pub description: &'static str,
    /// The parameter schema (field names, types, ranges, defaults, docs).
    pub schema: ParamSchema,
}

/// A set of generators addressable by id.
///
/// Construct it with [`Registry::with_builtins`] to get the crate's generators, or
/// [`Registry::new`] plus [`Registry::register`] to assemble a custom set (for
/// example when lanes 2B/2C add generators). Ids are unique: registering a
/// duplicate id replaces the earlier entry.
#[derive(Default)]
pub struct Registry {
    generators: Vec<Box<dyn ErasedGenerator>>,
}

impl Registry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            generators: Vec::new(),
        }
    }

    /// Creates a registry preloaded with the crate's built-in generators: the
    /// [`GuardRing`] and the [`ViaFarm`].
    #[must_use]
    pub fn with_builtins() -> Self {
        let mut reg = Self::new();
        reg.register(GuardRing);
        reg.register(ViaFarm);
        reg
    }

    /// Registers a generator, replacing any existing entry with the same id.
    ///
    /// Accepts anything implementing [`ErasedGenerator`], which every
    /// [`Generator`](crate::Generator) does through the blanket impl.
    pub fn register<G: ErasedGenerator + 'static>(&mut self, generator: G) {
        let id = generator.id();
        if let Some(slot) = self.generators.iter_mut().find(|g| g.id() == id) {
            *slot = Box::new(generator);
        } else {
            self.generators.push(Box::new(generator));
        }
    }

    /// The number of registered generators.
    #[must_use]
    pub fn len(&self) -> usize {
        self.generators.len()
    }

    /// Whether the registry holds no generators.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.generators.is_empty()
    }

    /// The ids of every registered generator, in registration order.
    #[must_use]
    pub fn ids(&self) -> Vec<&'static str> {
        self.generators.iter().map(|g| g.id()).collect()
    }

    /// Metadata for every registered generator, in registration order: the id,
    /// title, description, and parameter schema for each. This is the enumeration
    /// surface the app and the agent list from.
    #[must_use]
    pub fn infos(&self) -> Vec<GeneratorInfo> {
        self.generators
            .iter()
            .map(|g| GeneratorInfo {
                id: g.id(),
                title: g.title(),
                description: g.description(),
                schema: g.schema(),
            })
            .collect()
    }

    /// The type-erased handle for a generator id, if registered.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&dyn ErasedGenerator> {
        self.generators
            .iter()
            .find(|g| g.id() == id)
            .map(AsRef::as_ref)
    }

    /// The parameter schema for a generator id, if registered.
    #[must_use]
    pub fn schema(&self, id: &str) -> Option<ParamSchema> {
        self.get(id).map(ErasedGenerator::schema)
    }

    /// The default parameters (as JSON) for a generator id, if registered, ready to
    /// seed a form.
    #[must_use]
    pub fn default_params(&self, id: &str) -> Option<Value> {
        self.get(id).map(ErasedGenerator::default_params)
    }

    /// Validates JSON parameters against the named generator without generating.
    ///
    /// Returns [`GenError::UnknownGenerator`] if the id is not registered, otherwise
    /// the generator's own deserialize/validate result.
    pub fn validate(&self, id: &str, params: &Value) -> Result<(), GenError> {
        self.require(id)?.validate_json(params)
    }

    /// Drives a generator by id: deserializes and validates `params`, then appends
    /// the generated geometry to `cell`.
    ///
    /// Returns [`GenError::UnknownGenerator`] if the id is not registered.
    pub fn generate(
        &self,
        id: &str,
        params: &Value,
        tech: &Technology,
        cell: &mut Cell,
    ) -> Result<GenOutput, GenError> {
        self.require(id)?.generate_json(params, tech, cell)
    }

    /// Looks up a generator or returns [`GenError::UnknownGenerator`].
    fn require(&self, id: &str) -> Result<&dyn ErasedGenerator, GenError> {
        self.get(id)
            .ok_or_else(|| GenError::UnknownGenerator(id.to_owned()))
    }
}

impl core::fmt::Debug for Registry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // `dyn ErasedGenerator` is not `Debug`; list ids so the registry is still
        // inspectable in test output and logs.
        f.debug_struct("Registry")
            .field("generators", &self.ids())
            .finish()
    }
}
