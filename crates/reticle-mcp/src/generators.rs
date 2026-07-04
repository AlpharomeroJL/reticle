//! Generator tools: one MCP tool per parameterized layout generator.
//!
//! Beyond the one-command-per-tool surface in [`crate::tools`], the built-in
//! [`reticle_gen`] generators are advertised as their own family of tools. Iterating
//! [`reticle_gen::Registry::with_builtins`]'s [`infos`](reticle_gen::Registry::infos)
//! yields one [`ToolSpec`] per generator, named for the
//! generator id (`guard_ring`, `via_farm`, `pad_ring`, `seal_ring`, `fill`,
//! `test_structure`), described by the generator's title and description, and
//! schema'd by converting the generator's own [`ParamSchema`]
//! into a tight model-facing JSON Schema that carries every field's type, range, and
//! default.
//!
//! A generator tool differs from a command tool in one way: it needs a target cell,
//! which is not one of the generator's parameters. The tool schema therefore adds a
//! required `cell` string alongside the generator's fields, and
//! [`to_generator_command`] splits that `cell` back out and folds the remaining
//! fields into a [`AgentCommand::RunGenerator`] carrying the generator id and the
//! parameter object. The generator validates the parameters itself when the command
//! is applied, so a bad value returns an `invalid_argument` tool error.

use serde_json::{Map, Value, json};

use reticle_agent_api::AgentCommand;
use reticle_gen::{FieldSchema, FieldType, ParamSchema, Registry};

use crate::tools::ToolSpec;

/// The name of the target-cell field the generator tools add on top of each
/// generator's own parameters.
const CELL_FIELD: &str = "cell";

/// Builds the generator-tool catalog: one [`ToolSpec`] per built-in generator.
///
/// Each tool's name is the generator id (a stable `&'static str`, which is also the
/// registry key and the `RunGenerator` `generator_id`), its description is the
/// generator's title and one-paragraph description, and its input schema is the
/// generator's [`ParamSchema`] converted by [`param_schema_to_json`] with a required
/// `cell` field prepended.
#[must_use]
pub fn generator_tools() -> Vec<ToolSpec> {
    Registry::with_builtins()
        .infos()
        .into_iter()
        .map(|info| ToolSpec {
            // `info.id` is `&'static str`, so it fits `ToolSpec::name` directly.
            name: info.id,
            description: format!("{}. {}", info.title, info.description),
            schema: param_schema_to_json(&info.schema),
        })
        .collect()
}

/// Converts a generator's [`ParamSchema`] into a model-facing JSON Schema object.
///
/// The result is an `object` schema whose properties are, in order: a required
/// `cell` string (the target cell, which is not a generator parameter), then one
/// property per [`FieldSchema`], mapped by its field type. Every generator field
/// is required, matching the generator's own `#[serde(default)]` structs (a form or a
/// model should supply them all rather than relying on silent defaults), and the
/// `cell` field is required too. `additionalProperties` is left permissive so an
/// extra key is ignored rather than rejected, matching the command tools.
#[must_use]
pub fn param_schema_to_json(schema: &ParamSchema) -> Value {
    let mut properties = Map::new();
    properties.insert(
        CELL_FIELD.to_owned(),
        json!({
            "type": "string",
            "description": "The target cell that gains the generated geometry. Must \
                            already exist in the document.",
        }),
    );
    let mut required: Vec<Value> = vec![json!(CELL_FIELD)];
    for field in &schema.fields {
        properties.insert(field.name.clone(), field_to_json(field));
        required.push(json!(field.name));
    }
    json!({
        "type": "object",
        "description": schema.description,
        "properties": Value::Object(properties),
        "required": Value::Array(required),
    })
}

/// Converts one [`FieldSchema`] to its JSON-Schema property.
///
/// * [`FieldType::Int`] becomes an `integer` with inclusive `minimum`/`maximum` from
///   the field range and a `default`; the unit, when present, is appended to the
///   description so a model sees "in DBU" or "per-mille".
/// * [`FieldType::Bool`] becomes a `boolean` with its default.
/// * [`FieldType::Enum`] becomes a `string` constrained by `enum` to its variants,
///   with its default.
fn field_to_json(field: &FieldSchema) -> Value {
    let description = match &field.unit {
        Some(unit) => format!("{} (in {unit})", field.doc),
        None => field.doc.clone(),
    };
    match &field.ty {
        FieldType::Int { min, max, step } => {
            let mut obj = json!({
                "type": "integer",
                "description": description,
                "minimum": min,
                "maximum": max,
                "default": field.default,
            });
            // Expose the form step as a hint only when it is a coarser stride than 1,
            // so a model knows the intended granularity without over-constraining.
            if *step > 1 {
                obj["multipleOf"] = json!(step);
            }
            obj
        }
        FieldType::Bool => json!({
            "type": "boolean",
            "description": description,
            "default": field.default,
        }),
        FieldType::Enum { variants } => json!({
            "type": "string",
            "description": description,
            "enum": variants,
            "default": field.default,
        }),
    }
}

/// Whether `name` is a built-in generator id (and therefore a generator tool).
#[must_use]
pub fn is_generator_tool(name: &str) -> bool {
    Registry::with_builtins().ids().contains(&name)
}

/// Maps a generator tool call to a [`AgentCommand::RunGenerator`].
///
/// Returns `None` when `name` is not a registered generator id (so the caller can
/// fall through to the command-tool path). Otherwise it splits the required `cell`
/// field out of `arguments` and folds every other field into the generator's
/// parameter object, yielding `RunGenerator { cell, generator_id: name, params }`.
/// A missing or non-string `cell` is a shape error returned as `Err`; the generator
/// itself validates the remaining parameters when the command is applied.
#[must_use]
pub fn to_generator_command(name: &str, arguments: &Value) -> Option<Result<AgentCommand, String>> {
    if !is_generator_tool(name) {
        return None;
    }
    let mut map = match arguments {
        Value::Object(m) => m.clone(),
        Value::Null => Map::new(),
        _ => return Some(Err("arguments must be a JSON object".to_owned())),
    };
    let cell = match map.remove(CELL_FIELD) {
        Some(Value::String(c)) => c,
        Some(_) => return Some(Err("`cell` must be a string".to_owned())),
        None => return Some(Err("a generator tool requires a `cell` argument".to_owned())),
    };
    Some(Ok(AgentCommand::RunGenerator {
        cell,
        generator_id: name.to_owned(),
        // Whatever fields remain are the generator's own parameters; the generator
        // validates them on apply.
        params: Value::Object(map),
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        field_to_json, generator_tools, is_generator_tool, param_schema_to_json,
        to_generator_command,
    };
    use reticle_agent_api::AgentCommand;
    use reticle_gen::{FieldSchema, Registry};
    use serde_json::json;

    /// Every built-in generator is advertised as exactly one tool, named for its id.
    #[test]
    fn one_tool_per_generator() {
        let tools = generator_tools();
        let ids = Registry::with_builtins().ids();
        assert_eq!(tools.len(), ids.len());
        for id in ids {
            assert!(
                tools.iter().any(|t| t.name == id),
                "generator {id} should have a tool"
            );
        }
        // Each tool has a non-trivial description and an object schema requiring cell.
        for t in &tools {
            assert!(t.description.len() > 10, "{} has a description", t.name);
            assert_eq!(t.schema["type"], "object");
            let required = t.schema["required"].as_array().unwrap();
            assert!(
                required.iter().any(|r| r == "cell"),
                "{} requires cell",
                t.name
            );
        }
    }

    /// The schema conversion maps each field type to the right JSON-Schema kind and
    /// carries the range and default.
    #[test]
    fn schema_conversion_maps_field_types() {
        let int = field_to_json(&FieldSchema::int("w", "width", 400, 300, 1000, "dbu"));
        assert_eq!(int["type"], "integer");
        assert_eq!(int["minimum"], 300);
        assert_eq!(int["maximum"], 1000);
        assert_eq!(int["default"], 400);
        assert!(
            int["description"].as_str().unwrap().contains("dbu"),
            "unit appears in the description: {int}"
        );

        let boolean = field_to_json(&FieldSchema::bool("taps", "place taps", true));
        assert_eq!(boolean["type"], "boolean");
        assert_eq!(boolean["default"], true);

        let enumerated = field_to_json(&FieldSchema::enumerated(
            "cut",
            "cut layer",
            &["mcon", "via"],
            "mcon",
        ));
        assert_eq!(enumerated["type"], "string");
        assert_eq!(enumerated["enum"], json!(["mcon", "via"]));
        assert_eq!(enumerated["default"], "mcon");
    }

    /// The via-farm schema round-trips through the converter with its known fields.
    #[test]
    fn via_farm_schema_has_its_fields() {
        let schema = Registry::with_builtins().schema("via_farm").unwrap();
        let json = param_schema_to_json(&schema);
        let props = &json["properties"];
        assert_eq!(props["cell"]["type"], "string");
        assert_eq!(props["cut"]["type"], "string");
        assert_eq!(props["rows"]["type"], "integer");
        assert_eq!(props["cols"]["type"], "integer");
        assert_eq!(props["rows"]["minimum"], 1);
    }

    /// A generator tool call maps to a `RunGenerator` with the cell split out and the
    /// rest folded into params.
    #[test]
    fn generator_call_maps_to_run_generator() {
        let cmd = to_generator_command(
            "via_farm",
            &json!({ "cell": "top", "cut": "mcon", "rows": 3, "cols": 3 }),
        )
        .expect("via_farm is a generator tool")
        .expect("valid arguments");
        match cmd {
            AgentCommand::RunGenerator {
                cell,
                generator_id,
                params,
            } => {
                assert_eq!(cell, "top");
                assert_eq!(generator_id, "via_farm");
                assert_eq!(params["cut"], "mcon");
                assert_eq!(params["rows"], 3);
                // The cell field is not leaked into the generator parameters.
                assert!(params.get("cell").is_none(), "cell is split out of params");
            }
            other => panic!("expected RunGenerator, got {other:?}"),
        }
    }

    /// A non-generator name is not a generator tool (the caller falls through).
    #[test]
    fn non_generator_name_is_none() {
        assert!(!is_generator_tool("add_rect"));
        assert!(to_generator_command("add_rect", &json!({})).is_none());
    }

    /// A generator call missing its cell is a shape error, not a fall-through.
    #[test]
    fn generator_call_without_cell_errors() {
        let r = to_generator_command("via_farm", &json!({ "cut": "mcon" }))
            .expect("via_farm is a generator tool");
        assert!(r.is_err(), "missing cell is an error");
    }
}
