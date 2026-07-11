//! [`PCellDef`]: a user-defined parametric cell (a rhai script plus a parameter schema),
//! its stable content identity, and its F2 provenance.
//!
//! SCAFFOLD OWNED BY THE `pcell-params` LANE. The type, its fields, and the method
//! signatures are fixed here so the sandboxed producer (`reticle_script`, `pcell-produce`
//! lane) and the Inspector UI (`pcell-inspect` lane) compile against a stable interface.
//! The `pcell-params` lane implements the real body of [`PCellDef::validate_params`]
//! (per-field type and range checking against the [`ParamSchema`]) and any authoring
//! helpers; [`PCellDef::param_hash`] and [`PCellDef::produce_meta`] are already wired to the
//! frozen [`param_hash`] primitive.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::GenError;
use crate::pcell::hash::param_hash;
use crate::produce::ProduceMeta;
use crate::schema::{FieldType, ParamSchema};

/// A user-defined parametric cell: a rhai script whose top-level parameter bindings are
/// described by a [`ParamSchema`], produced into geometry (by the `pcell-produce` lane in
/// `reticle_script`) with the stable content identity [`param_hash`]
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
    /// present with a value of the right type and, for a bounded [`FieldType::Int`], within
    /// its declared `[min, max]` range.
    ///
    /// Checks fields in schema order and returns on the first failure, mirroring how the
    /// built-in generators' [`GenParams::validate`](crate::GenParams::validate) short-circuits
    /// on the first bad field. The error is always [`GenError::Deserialize`], whose message
    /// leads with the offending field name in backticks: unlike a built-in generator's
    /// compile-time `&'static str` field names, a PCell's field names are user-authored data
    /// (an owned `String` in [`FieldSchema`](crate::FieldSchema)), and this method is expected
    /// to run on every keystroke of a live form (see the crate docs), so turning one into a
    /// `&'static str` for [`GenError::OutOfRange`]/[`GenError::Invalid`] would mean leaking
    /// memory on every call. `Deserialize`'s owned `String` payload is the only variant that
    /// fits an owned, dynamic field name.
    pub fn validate_params(&self, params: &Value) -> Result<(), GenError> {
        for field in &self.schema.fields {
            let name = field.name.as_str();
            let value = params.get(name).ok_or_else(|| missing_field(name))?;
            match &field.ty {
                FieldType::Int { min, max, .. } => {
                    let v = value
                        .as_i64()
                        .ok_or_else(|| wrong_type(name, "an integer", value))?;
                    if v < *min || v > *max {
                        return Err(GenError::Deserialize(format!(
                            "parameter `{name}` = {v} is out of range [{min}, {max}]"
                        )));
                    }
                }
                FieldType::Bool => {
                    value
                        .as_bool()
                        .ok_or_else(|| wrong_type(name, "a boolean", value))?;
                }
                FieldType::Enum { variants } => {
                    let s = value
                        .as_str()
                        .ok_or_else(|| wrong_type(name, "a string", value))?;
                    if !variants.iter().any(|v| v.as_str() == s) {
                        return Err(GenError::Deserialize(format!(
                            "parameter `{name}` = {s:?} is not one of {variants:?}"
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

/// Builds the [`GenError`] for a schema field absent from the supplied params.
fn missing_field(name: &str) -> GenError {
    GenError::Deserialize(format!("parameter `{name}` is missing"))
}

/// Builds the [`GenError`] for a schema field present with the wrong JSON shape.
fn wrong_type(name: &str, expected: &str, actual: &Value) -> GenError {
    GenError::Deserialize(format!(
        "parameter `{name}` must be {expected}, got {actual}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::FieldSchema;
    use serde_json::json;

    /// A PCell with one field of each [`FieldType`], for exercising every branch of
    /// [`PCellDef::validate_params`].
    fn sample_def() -> PCellDef {
        PCellDef {
            id: "user.sensor".to_owned(),
            title: "Sensor".to_owned(),
            description: "A sample sensor PCell for tests.".to_owned(),
            schema: ParamSchema {
                generator_id: "user.sensor".to_owned(),
                title: "Sensor".to_owned(),
                description: "A sample sensor PCell for tests.".to_owned(),
                fields: vec![
                    FieldSchema::int("width", "Width in dbu.", 1_000, 100, 10_000, "dbu"),
                    FieldSchema::bool("mirrored", "Mirror the layout.", false),
                    FieldSchema::enumerated("layer", "Conductor layer.", &["met1", "met2"], "met1"),
                ],
            },
            script: "// no-op".to_owned(),
            engine_version: "0.1.0".to_owned(),
        }
    }

    fn valid_params() -> Value {
        json!({ "width": 2_000, "mirrored": true, "layer": "met2" })
    }

    #[test]
    fn valid_params_pass() {
        let def = sample_def();
        assert_eq!(def.validate_params(&valid_params()), Ok(()));
    }

    #[test]
    fn defaults_from_schema_pass() {
        // Every field's own declared default should itself be a valid value.
        let def = sample_def();
        let mut params = serde_json::Map::new();
        for field in &def.schema.fields {
            params.insert(field.name.clone(), field.default.clone());
        }
        assert_eq!(def.validate_params(&Value::Object(params)), Ok(()));
    }

    #[test]
    fn missing_field_is_named_in_the_error() {
        let def = sample_def();
        let mut params = valid_params();
        params.as_object_mut().expect("object").remove("mirrored");
        let err = def.validate_params(&params).expect_err("missing field");
        let msg = err.to_string();
        assert!(msg.contains("mirrored"), "names the missing field: {msg}");
        assert!(msg.contains("missing"), "says missing: {msg}");
    }

    #[test]
    fn non_object_params_reports_the_first_missing_field() {
        let def = sample_def();
        let err = def
            .validate_params(&Value::Null)
            .expect_err("no fields present");
        assert!(err.to_string().contains("width"));
    }

    #[test]
    fn wrong_type_int_is_named_in_the_error() {
        let def = sample_def();
        let mut params = valid_params();
        params["width"] = json!("not a number");
        let err = def.validate_params(&params).expect_err("wrong type");
        let msg = err.to_string();
        assert!(msg.contains("width"), "names the field: {msg}");
        assert!(msg.contains("integer"), "says the expected type: {msg}");
    }

    #[test]
    fn out_of_range_int_is_named_with_its_bounds() {
        let def = sample_def();
        let mut params = valid_params();
        params["width"] = json!(50_000);
        let err = def.validate_params(&params).expect_err("out of range");
        let msg = err.to_string();
        assert!(msg.contains("width"), "names the field: {msg}");
        assert!(msg.contains("50000"), "names the value: {msg}");
        assert!(
            msg.contains("100") && msg.contains("10000"),
            "names the bounds: {msg}"
        );
    }

    #[test]
    fn int_at_each_inclusive_bound_passes() {
        let def = sample_def();
        for edge in [100, 10_000] {
            let mut params = valid_params();
            params["width"] = json!(edge);
            assert_eq!(
                def.validate_params(&params),
                Ok(()),
                "edge {edge} is in range"
            );
        }
    }

    #[test]
    fn wrong_type_bool_is_named_in_the_error() {
        let def = sample_def();
        let mut params = valid_params();
        params["mirrored"] = json!("yes");
        let err = def.validate_params(&params).expect_err("wrong type");
        let msg = err.to_string();
        assert!(msg.contains("mirrored"), "names the field: {msg}");
        assert!(msg.contains("boolean"), "says the expected type: {msg}");
    }

    #[test]
    fn invalid_enum_variant_is_named_with_the_bad_value() {
        let def = sample_def();
        let mut params = valid_params();
        params["layer"] = json!("met9");
        let err = def.validate_params(&params).expect_err("invalid variant");
        let msg = err.to_string();
        assert!(msg.contains("layer"), "names the field: {msg}");
        assert!(msg.contains("met9"), "names the offending value: {msg}");
    }

    #[test]
    fn wrong_type_enum_is_named_in_the_error() {
        let def = sample_def();
        let mut params = valid_params();
        params["layer"] = json!(42);
        let err = def.validate_params(&params).expect_err("wrong type");
        assert!(err.to_string().contains("layer"));
    }

    #[test]
    fn first_declared_field_failure_wins_when_several_are_bad() {
        let def = sample_def();
        // Both `width` (declared first) and `layer` (declared last) are bad; the error
        // should name `width` since fields are checked in schema order.
        let params = json!({ "width": -1, "mirrored": true, "layer": "met9" });
        let err = def.validate_params(&params).expect_err("width is bad");
        assert!(err.to_string().contains("width"));
    }

    #[test]
    fn empty_schema_accepts_any_params() {
        let mut def = sample_def();
        def.schema.fields.clear();
        assert_eq!(def.validate_params(&json!({"whatever": true})), Ok(()));
        assert_eq!(def.validate_params(&Value::Null), Ok(()));
    }

    #[test]
    fn param_hash_and_produce_meta_still_agree_on_a_validated_instance() {
        // Sanity check that validation doesn't disturb the frozen identity primitives this
        // lane must leave untouched.
        let def = sample_def();
        let params = valid_params();
        def.validate_params(&params).expect("valid");
        let meta = def.produce_meta(&params);
        assert_eq!(meta.generator_id, def.id);
        assert_eq!(meta.param_hash, def.param_hash(&params));
    }
}
