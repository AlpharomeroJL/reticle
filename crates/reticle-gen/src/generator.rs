//! The generator contract: the [`Generator`] trait, its [`GenParams`] bound, the
//! [`GenOutput`] summary, and the type-erased [`ErasedGenerator`] the registry
//! drives.
//!
//! # Two layers, one contract
//!
//! [`Generator`] is the *typed* contract a concrete generator implements: it names
//! an associated [`Params`](Generator::Params) struct and turns valid parameters
//! plus a technology into geometry appended to a caller-provided [`Cell`]. Typed
//! callers (a test, another crate that knows the concrete type) use this directly
//! and get full type-checking on the parameters.
//!
//! [`ErasedGenerator`] is the *type-erased* contract the [registry](crate::Registry)
//! stores: it hides the associated type behind `serde_json::Value` parameters so the
//! app and the agent can enumerate and drive every generator uniformly without
//! naming its concrete parameter type. A blanket impl derives it from any
//! [`Generator`], so implementing the typed trait is all a new generator (lanes 2B
//! and 2C) needs to do; the erased path comes for free.

use reticle_geometry::Rect;
use reticle_model::{Cell, Technology};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::GenError;
use crate::schema::ParamSchema;

/// The parameter struct of a [`Generator`]: serde-round-trippable, defaultable, and
/// self-describing and self-validating.
///
/// A generator's parameters are a plain value type. This bound is what lets the
/// registry treat them uniformly: [`schema`](GenParams::schema) yields the
/// machine-readable form/tool description, [`validate`](GenParams::validate) is the
/// authoritative range and cross-field check, and the serde bounds let the erased
/// path move parameters as JSON.
pub trait GenParams: Serialize + DeserializeOwned + Default + Clone + core::fmt::Debug {
    /// The machine-readable schema for these parameters (field names, types,
    /// ranges, defaults, docs). Lane 2D turns this into a UI form and a tool schema.
    fn schema() -> ParamSchema;

    /// Checks the parameters against their ranges and any cross-field constraints,
    /// returning a [`GenError`] naming the offending field on failure.
    ///
    /// This is the single source of validation truth: the registry always calls it
    /// before generating, so a generator's [`generate`](Generator::generate) may
    /// assume validated input.
    fn validate(&self) -> Result<(), GenError>;
}

/// What a generation run added to the target cell.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct GenOutput {
    /// The number of [`DrawShape`](reticle_model::DrawShape)s appended to the cell.
    pub shapes_added: usize,
    /// The bounding box of the appended geometry, or `None` if nothing was added.
    pub bbox: Option<Rect>,
}

/// A parameterized layout generator: a pure function from validated parameters plus
/// a technology to geometry, appended to a caller-provided cell.
///
/// Implementors emit only geometry that is DRC-clean by construction against the
/// SKY130 subset (see the crate docs). The function is pure: it reads the parameters
/// and technology and writes shapes, touching no filesystem, GPU, or global state,
/// so it runs identically on native and in the browser.
pub trait Generator {
    /// The typed parameter struct this generator consumes.
    type Params: GenParams;

    /// A stable, unique, machine identifier (for example `"guard_ring"`). This is
    /// the registry key and the tool name a model calls.
    fn id(&self) -> &'static str;

    /// A short human-readable title (for example `"Guard ring"`).
    fn title(&self) -> &'static str;

    /// One-paragraph description of what geometry the generator emits.
    fn description(&self) -> &'static str;

    /// Appends the generated geometry to `cell` and returns a [`GenOutput`] summary.
    ///
    /// The registry guarantees `params` has already passed
    /// [`validate`](GenParams::validate) before this is called; a typed caller
    /// should validate first too. Implementors append to `cell.shapes` and must not
    /// remove or reorder shapes that were already there.
    fn generate(
        &self,
        params: &Self::Params,
        tech: &Technology,
        cell: &mut Cell,
    ) -> Result<GenOutput, GenError>;

    /// The schema for this generator's parameters, tagged with its id/title/
    /// description. The default builds it from [`GenParams::schema`]; generators do
    /// not normally override it.
    fn schema(&self) -> ParamSchema {
        let mut schema = Self::Params::schema();
        self.id().clone_into(&mut schema.generator_id);
        self.title().clone_into(&mut schema.title);
        self.description().clone_into(&mut schema.description);
        schema
    }
}

/// The type-erased face of a [`Generator`] the [registry](crate::Registry) stores.
///
/// It exposes the same capabilities as [`Generator`] but moves parameters as
/// `serde_json::Value`, so heterogeneous generators live behind one `dyn` type and
/// the app/agent can enumerate and invoke them without naming concrete parameter
/// types. A blanket impl provides it for every [`Generator`]; do not implement it by
/// hand.
pub trait ErasedGenerator {
    /// The generator's stable id (see [`Generator::id`]).
    fn id(&self) -> &'static str;
    /// The generator's title (see [`Generator::title`]).
    fn title(&self) -> &'static str;
    /// The generator's description (see [`Generator::description`]).
    fn description(&self) -> &'static str;
    /// The parameter schema (see [`Generator::schema`]).
    fn schema(&self) -> ParamSchema;
    /// The default parameters, serialized to JSON, ready to seed a form.
    fn default_params(&self) -> Value;
    /// Validates a JSON parameter blob: deserializes it into the concrete parameter
    /// type (surfacing a [`GenError::Deserialize`] on malformed input) and runs the
    /// generator's [`validate`](GenParams::validate).
    fn validate_json(&self, params: &Value) -> Result<(), GenError>;
    /// Deserializes and validates `params`, then generates into `cell`.
    fn generate_json(
        &self,
        params: &Value,
        tech: &Technology,
        cell: &mut Cell,
    ) -> Result<GenOutput, GenError>;
}

impl<G> ErasedGenerator for G
where
    G: Generator,
{
    fn id(&self) -> &'static str {
        Generator::id(self)
    }

    fn title(&self) -> &'static str {
        Generator::title(self)
    }

    fn description(&self) -> &'static str {
        Generator::description(self)
    }

    fn schema(&self) -> ParamSchema {
        Generator::schema(self)
    }

    fn default_params(&self) -> Value {
        // `Default` params are a plain value type; serializing them cannot fail in
        // practice, but fall back to JSON null rather than panicking if it ever did.
        serde_json::to_value(G::Params::default()).unwrap_or(Value::Null)
    }

    fn validate_json(&self, params: &Value) -> Result<(), GenError> {
        let parsed = parse_params::<G::Params>(params)?;
        parsed.validate()
    }

    fn generate_json(
        &self,
        params: &Value,
        tech: &Technology,
        cell: &mut Cell,
    ) -> Result<GenOutput, GenError> {
        let parsed = parse_params::<G::Params>(params)?;
        parsed.validate()?;
        self.generate(&parsed, tech, cell)
    }
}

/// Deserializes a JSON blob into a parameter type, mapping serde failures to
/// [`GenError::Deserialize`].
fn parse_params<P: GenParams>(params: &Value) -> Result<P, GenError> {
    serde_json::from_value(params.clone()).map_err(|e| GenError::Deserialize(e.to_string()))
}
