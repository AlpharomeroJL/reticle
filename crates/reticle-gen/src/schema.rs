//! The machine-readable parameter schema.
//!
//! Every generator publishes a [`ParamSchema`] describing its parameter struct:
//! the field names, their types, per-field ranges, defaults, units, and one-line
//! docs. The schema is plain serde data with no behaviour, so lane 2D can render it
//! two ways from the same source: a UI form in the Generate panel, and a
//! model-facing tool schema for the agent. Keeping the schema hand-authored (rather
//! than derived from a heavyweight reflection crate) keeps the dependency set to
//! geometry, model, drc, and serde, which is what the browser build needs.
//!
//! The schema is a *description*, not the validator: the authoritative check is the
//! generator's `validate`, which the registry always runs before generating. The
//! ranges here mirror that logic so a form can pre-validate and a model can be told
//! the bounds up front.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The full parameter schema for one generator.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ParamSchema {
    /// The generator id this schema belongs to (matches the registry key).
    pub generator_id: String,
    /// Human-readable generator title.
    pub title: String,
    /// One-paragraph description of what the generator emits.
    pub description: String,
    /// The parameter fields, in the order a form should present them.
    pub fields: Vec<FieldSchema>,
}

impl ParamSchema {
    /// Looks up a field description by its serde field name.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&FieldSchema> {
        self.fields.iter().find(|f| f.name == name)
    }
}

/// One parameter field's description.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct FieldSchema {
    /// The field name, identical to the serde key in the parameter struct.
    pub name: String,
    /// The field's value type, which drives the form widget and tool-schema type.
    #[serde(rename = "type")]
    pub ty: FieldType,
    /// One-line human-readable description of the field.
    pub doc: String,
    /// The default value, as it appears in the serialized parameter struct.
    pub default: Value,
    /// The engineering unit of a numeric field (for example `"dbu"`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

impl FieldSchema {
    /// Builds an integer field with an inclusive `[min, max]` range and a unit.
    #[must_use]
    pub fn int(name: &str, doc: &str, default: i64, min: i64, max: i64, unit: &str) -> Self {
        Self {
            name: name.to_owned(),
            ty: FieldType::Int { min, max, step: 1 },
            doc: doc.to_owned(),
            default: Value::from(default),
            unit: Some(unit.to_owned()),
        }
    }

    /// Builds a boolean field.
    #[must_use]
    pub fn bool(name: &str, doc: &str, default: bool) -> Self {
        Self {
            name: name.to_owned(),
            ty: FieldType::Bool,
            doc: doc.to_owned(),
            default: Value::from(default),
            unit: None,
        }
    }

    /// Builds an enumerated field from its string variants and default.
    #[must_use]
    pub fn enumerated(name: &str, doc: &str, variants: &[&str], default: &str) -> Self {
        Self {
            name: name.to_owned(),
            ty: FieldType::Enum {
                variants: variants.iter().map(|v| (*v).to_owned()).collect(),
            },
            doc: doc.to_owned(),
            default: Value::from(default),
            unit: None,
        }
    }
}

/// The type of a parameter field, shaped so a form and a JSON-schema-style tool
/// definition can both be generated from it.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FieldType {
    /// A bounded integer. `min`/`max` are inclusive; `step` is the form increment.
    Int {
        /// Inclusive lower bound.
        min: i64,
        /// Inclusive upper bound.
        max: i64,
        /// Increment a stepper control should use (always at least 1).
        step: i64,
    },
    /// A boolean toggle.
    Bool,
    /// A choice among a fixed set of string variants (a serde-tagged enum).
    Enum {
        /// The allowed variant strings, in presentation order.
        variants: Vec<String>,
    },
}
