//! [`PCellDef`]: a user-defined parametric cell (a rhai script plus a parameter schema),
//! its stable content identity, and its F2 provenance.
//!
//! SCAFFOLD OWNED BY THE `pcell-params` LANE. The type, its fields, and the method
//! signatures are fixed here so the sandboxed producer (`reticle_script`, `pcell-produce`
//! lane) and the Inspector UI (`pcell-inspect` lane) compile against a stable interface.
//! The `pcell-params` lane implements the real body of [`PCellDef::validate_params`]
//! (per-field type and range checking against the [`ParamSchema`]) and any authoring
//! helpers; [`PCellDef::param_hash`] and [`PCellDef::produce_meta`] are already wired to the
//! frozen [`param_hash`](crate::param_hash) primitive.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::GenError;
use crate::pcell::hash::param_hash;
use crate::produce::ProduceMeta;
use crate::schema::ParamSchema;

/// A user-defined parametric cell: a rhai script whose top-level parameter bindings are
/// described by a [`ParamSchema`], produced into geometry (by the `pcell-produce` lane in
/// `reticle_script`) with the stable content identity [`param_hash`](crate::param_hash)
/// gives it.
///
/// A `PCellDef` is data: it names the cell, carries its parameter schema and its script
/// source, and pins the engine version that defines its geometry, so a produced instance is
/// reproducible from `(def, params)` alone.
// No `Eq`: `ParamSchema` carries `serde_json::Value` field defaults, which are only
// `PartialEq` (floats). `PartialEq` is enough for tests and de-dup.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct PCellDef {
    /// The stable registry id (for example `"user.sensor"`); the `generator_id` in the
    /// produced instance's [`ProduceMeta`] and the key in a [`PCellRegistry`](crate::PCellRegistry).
    pub id: String,
    /// A short human title for the Inspector.
    pub title: String,
    /// One-paragraph description of what the cell builds.
    pub description: String,
    /// The parameter form: the fields the script's top-level bindings expose, reusing the
    /// same [`ParamSchema`] the built-in generators and the Generate panel already speak.
    pub schema: ParamSchema,
    /// The rhai script source that builds the geometry from the parameters.
    pub script: String,
    /// The generation engine version, part of the produced-instance identity so a produce
    /// from a different engine version is a distinct instance.
    pub engine_version: String,
}

impl PCellDef {
    /// The stable content hash for `params` under this PCell's identity (id, engine
    /// version, and canonical params), lowercase-hex `SHA-256`. Keys the produced-cell
    /// cache and identifies a regenerate.
    #[must_use]
    pub fn param_hash(&self, params: &Value) -> String {
        param_hash(&self.id, &self.engine_version, params)
    }

    /// The F2 [`ProduceMeta`] provenance for a produce of `params`: this PCell's id, its
    /// engine version, its id as the `script_ref` (a user PCell always references a
    /// script), and the [`param_hash`](Self::param_hash).
    #[must_use]
    pub fn produce_meta(&self, params: &Value) -> ProduceMeta {
        ProduceMeta {
            generator_id: self.id.clone(),
            engine_version: self.engine_version.clone(),
            script_ref: Some(self.id.clone()),
            param_hash: self.param_hash(params),
        }
    }

    /// Validates `params` against this PCell's [`ParamSchema`]: every declared field must be
    /// present with a value of the right type and within its range.
    ///
    /// SCAFFOLD STUB: returns `Ok(())` so `pcell-produce`/`pcell-inspect` compile against the
    /// signature. The `pcell-params` lane replaces this with the real per-field type and
    /// range checking (mirroring [`GenParams::validate`](crate::GenParams::validate) for the
    /// built-in generators), returning a [`GenError`] that names the offending field.
    pub fn validate_params(&self, params: &Value) -> Result<(), GenError> {
        // pcell-params lane: check each `self.schema.fields` entry against `params`.
        let _ = params;
        Ok(())
    }
}
