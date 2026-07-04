//! The tool catalog: command tools plus read-only context tools.
//!
//! Two families of tool are advertised to an MCP client:
//!
//! * **Command tools** wrap the frozen
//!   [`AgentCommand`] surface one-to-one. Each is
//!   named for the command `op` (`snake_case`), carries a schema from the
//!   `schema` module, and a description stating what it does in DBU/layer terms
//!   and how it fails. A client's `arguments` object is re-tagged with the `op`
//!   and deserialized straight into an `AgentCommand` (see [`to_command`]).
//! * **Context tools** are read-only reporting helpers that do not correspond to a
//!   single command: [`GET_TECHNOLOGY_RULES`], [`GET_DOCUMENT_SUMMARY`], and
//!   [`GET_RENDER_REGION`]. They are handled directly in the `context` module.

use serde_json::{Value, json};

use reticle_agent_api::AgentCommand;

use crate::schema;

/// The `get_technology_rules` context tool name.
pub const GET_TECHNOLOGY_RULES: &str = "get_technology_rules";
/// The `get_document_summary` context tool name.
pub const GET_DOCUMENT_SUMMARY: &str = "get_document_summary";
/// The `get_render_region` context tool name.
pub const GET_RENDER_REGION: &str = "get_render_region";

/// A single advertised tool: its name, model-facing description, and input schema.
#[derive(Clone, Debug)]
pub struct ToolSpec {
    /// The tool name (`snake_case`; matches the command `op` for command tools).
    pub name: &'static str,
    /// A model-facing description: what the tool does, in what units, how it fails.
    pub description: String,
    /// The JSON Schema for the tool's arguments object.
    pub schema: Value,
}

impl ToolSpec {
    /// This tool as the JSON object MCP `tools/list` expects.
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": self.schema,
        })
    }
}

/// A shared reminder appended to every command-tool description so a model that
/// only sees one tool still has the state units and error model in context.
const CONVENTIONS: &str = " Coordinates are database units (DBU); layers are a \
    GDSII (layer, datatype) pair. On failure the tool returns an error object with \
    a machine-readable `code` (no_such_cell, no_such_element, no_such_layer, \
    invalid_argument, engine_error, budget_exhausted) and a message.";

/// Builds the full command-tool catalog.
///
/// One [`ToolSpec`] per [`AgentCommand`] variant. Each description is suffixed
/// with the shared units-and-error conventions at build time so they ride along
/// with each tool.
#[allow(clippy::too_many_lines)]
pub fn command_tools() -> Vec<ToolSpec> {
    // Each command description is the tool-specific body plus the shared units and
    // error conventions, so a model seeing one tool still has them in context.
    fn desc(body: &str) -> String {
        format!("{body}{CONVENTIONS}")
    }

    vec![
        ToolSpec {
            name: "create_cell",
            description: desc("Create a new, empty cell in the document."),
            schema: schema::create_cell(),
        },
        ToolSpec {
            name: "delete_cell",
            description: desc("Delete a cell and all of its contents by name."),
            schema: schema::delete_cell(),
        },
        ToolSpec {
            name: "add_rect",
            description: desc(
                "Add an axis-aligned rectangle to a cell on a layer. \
                               Returns the new shape's stable element id.",
            ),
            schema: schema::add_rect(),
        },
        ToolSpec {
            name: "add_polygon",
            description: desc(
                "Add a polygon (three or more vertices) to a cell on a \
                               layer. Returns the new shape's stable element id.",
            ),
            schema: schema::add_polygon(),
        },
        ToolSpec {
            name: "add_path",
            description: desc(
                "Add a path (a centerline with a width and end-cap) to \
                               a cell on a layer. Returns the new shape's id.",
            ),
            schema: schema::add_path(),
        },
        ToolSpec {
            name: "place_instance",
            description: desc(
                "Place one instance of a child cell inside a parent \
                               cell under a transform. Returns the instance's id.",
            ),
            schema: schema::place_instance(),
        },
        ToolSpec {
            name: "place_array",
            description: desc(
                "Place a regular columns-by-rows array of a child cell \
                               inside a parent cell. Returns the array's id.",
            ),
            schema: schema::place_array(),
        },
        ToolSpec {
            name: "transform_shapes",
            description: desc(
                "Apply a transform in place to a set of existing shapes \
                               addressed by id.",
            ),
            schema: schema::transform_shapes(),
        },
        ToolSpec {
            name: "delete_shapes",
            description: desc("Delete a set of existing shapes addressed by id."),
            schema: schema::delete_shapes(),
        },
        ToolSpec {
            name: "query_shapes",
            description: desc(
                "List the shapes in a cell (id, slot, layer, geometry, \
                               bounding box), optionally filtered by layer and \
                               region. Read-only.",
            ),
            schema: schema::query_shapes(),
        },
        ToolSpec {
            name: "get_cell_info",
            description: desc(
                "Summarize a cell: shape/instance/array/label/pin \
                               counts and its bounding box in DBU. Read-only.",
            ),
            schema: schema::get_cell_info(),
        },
        ToolSpec {
            name: "list_layers",
            description: desc(
                "List the layers in the active technology (number, \
                               datatype, name, color, visibility) plus the database \
                               resolution. Read-only.",
            ),
            schema: schema::no_args(),
        },
        ToolSpec {
            name: "set_technology",
            description: desc(
                "Replace the active technology from technology-file \
                               text. Cell contents are preserved.",
            ),
            schema: schema::set_technology(),
        },
        ToolSpec {
            name: "run_drc",
            description: desc(
                "Run design-rule checking over a cell (optionally a \
                               region) and return the violations found. Read-only.",
            ),
            schema: schema::run_drc(),
        },
        ToolSpec {
            name: "get_violations",
            description: desc(
                "Return the standing DRC violation set. Violations are \
                               not cached between runs, so this returns an empty set \
                               with a note; call run_drc for results. Read-only.",
            ),
            schema: schema::no_args(),
        },
        ToolSpec {
            name: "route_net",
            description: desc(
                "Route a net between two or more terminals on a layer, \
                               adding wire shapes to the cell. Returns routed/failed \
                               counts and total wire length in DBU.",
            ),
            schema: schema::route_net(),
        },
        ToolSpec {
            name: "run_extract",
            description: desc(
                "Extract connectivity (a netlist of nets and their \
                               member shapes) from a cell. Read-only.",
            ),
            schema: schema::run_extract(),
        },
        ToolSpec {
            name: "check_intent",
            description: desc(
                "Check a cell against a connectivity intent spec and \
                               report opens and shorts. Read-only.",
            ),
            schema: schema::check_intent(),
        },
        ToolSpec {
            name: "netlist_compare",
            description: desc(
                "Compare a cell's extracted netlist against an expected \
                               netlist and report whether they are equivalent. \
                               Read-only.",
            ),
            schema: schema::netlist_compare(),
        },
        ToolSpec {
            name: "export_gds",
            description: desc(
                "Export the whole document as GDSII. Returns the bytes \
                               as an array of byte values. Read-only.",
            ),
            schema: schema::no_args(),
        },
        ToolSpec {
            name: "export_oasis",
            description: desc(
                "Export the whole document as OASIS. Returns the bytes \
                               as an array of byte values. Read-only.",
            ),
            schema: schema::no_args(),
        },
        ToolSpec {
            name: "import_gds",
            description: desc(
                "Import a GDSII document, replacing the session \
                               document. All prior element ids become invalid.",
            ),
            schema: schema::import_gds(),
        },
        ToolSpec {
            name: "render_png",
            description: desc(
                "Render a region of the document to a PNG (returned as \
                               a byte array). Requires a GPU adapter; without one the \
                               tool fails with engine_error. For an inline image use \
                               the get_render_region context tool instead.",
            ),
            schema: schema::render_png(),
        },
        ToolSpec {
            name: "save_session",
            description: desc(
                "Serialize the session (document plus command \
                               transcript) to a snapshot, returned as a byte array. \
                               Read-only.",
            ),
            schema: schema::no_args(),
        },
        ToolSpec {
            name: "load_session",
            description: desc(
                "Load a session from a snapshot string, rebuilding the \
                               document by replaying its recorded commands.",
            ),
            schema: schema::load_session(),
        },
        ToolSpec {
            name: "boolean_combine",
            description: desc(
                "Combine two or more shapes with a planar boolean \
                               (union, intersection, difference, xor), writing the \
                               result to a target layer and deleting the inputs. \
                               Rectangles and polygons only; paths are skipped. \
                               Returns the new shape ids.",
            ),
            schema: schema::boolean_combine(),
        },
        ToolSpec {
            name: "align_shapes",
            description: desc(
                "Align a set of shapes within their combined bounding \
                               box (left, right, top, bottom, center_x, center_y). \
                               Each shape keeps its id.",
            ),
            schema: schema::align_shapes(),
        },
        ToolSpec {
            name: "distribute_shapes",
            description: desc(
                "Distribute three or more shapes so the gaps between \
                               adjacent shapes are equal along an axis. The extremes \
                               stay put; inner shapes move. Each keeps its id.",
            ),
            schema: schema::distribute_shapes(),
        },
        ToolSpec {
            name: "offset_shapes",
            description: desc(
                "Grow (positive delta) or shrink (negative delta) shapes \
                               by an offset in DBU, replacing each on its own layer. \
                               Rectangles and polygons only; paths are skipped.",
            ),
            schema: schema::offset_shapes(),
        },
        ToolSpec {
            name: "build_via_stack",
            description: desc(
                "Build a via stack at a point: a square cut on the cut \
                               layer plus an enclosure on a lower and an upper layer, \
                               sized from the technology's enclosure rules (or a \
                               default). Returns the three new shape ids.",
            ),
            schema: schema::build_via_stack(),
        },
    ]
}

/// Builds the read-only context-tool catalog (the three tools beyond the command
/// surface).
pub fn context_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: GET_TECHNOLOGY_RULES,
            description: "Return the active technology's DRC rules as structured \
                          data: for each rule its name, kind (width, spacing, \
                          enclosure, extension, notch, area, density, angle), the \
                          layer(s) it applies to, and its threshold value (DBU for \
                          length rules, DBU squared for area, milli-degrees for \
                          angle). Also reports the technology name and database \
                          resolution. Read-only."
                .to_owned(),
            schema: schema::no_args(),
        },
        ToolSpec {
            name: GET_DOCUMENT_SUMMARY,
            description: "Summarize the whole document: the number of cells, which \
                          cells are tops, per-cell shape/instance/array counts, the \
                          overall bounding box in DBU, and the current revision. A \
                          cheap way to orient before editing. Read-only."
                .to_owned(),
            schema: schema::no_args(),
        },
        ToolSpec {
            name: GET_RENDER_REGION,
            description: "Render a region of the document to a PNG and return it as \
                          a base64 data URI plus its pixel dimensions, for visual \
                          inspection. The region is in DBU. Requires a GPU adapter; \
                          without one the tool reports that rendering is \
                          unavailable rather than failing the session. Read-only."
                .to_owned(),
            schema: schema::render_region(),
        },
    ]
}

/// The full advertised catalog: command tools, then generator tools, then context
/// tools.
///
/// The generator tools ([`crate::generators::generator_tools`]) sit between the
/// command tools and the context tools; each is named for a built-in generator id
/// and maps to a `RunGenerator` command rather than a one-to-one command tool.
pub fn all_tools() -> Vec<ToolSpec> {
    let mut tools = command_tools();
    tools.extend(crate::generators::generator_tools());
    tools.extend(context_tools());
    tools
}

/// Re-tags a command tool's `arguments` object with its `op` and deserializes it
/// into an [`AgentCommand`].
///
/// The tool name is the command `op`, and every schema field name matches the
/// command's serde field name, so `{op: name, ...arguments}` is exactly the
/// serialized form of the command. Returns `None` for a name that is not a
/// command tool (a context tool, or an unknown name); returns the serde error
/// message on a shape mismatch.
pub fn to_command(name: &str, arguments: &Value) -> Option<Result<AgentCommand, String>> {
    // Context tools are not commands.
    if matches!(
        name,
        GET_TECHNOLOGY_RULES | GET_DOCUMENT_SUMMARY | GET_RENDER_REGION
    ) {
        return None;
    }
    // Build `{ "op": name, ...arguments }`. A missing/empty arguments object is
    // treated as no fields, which suffices for the no-argument commands.
    let mut map = match arguments {
        Value::Object(m) => m.clone(),
        Value::Null => serde_json::Map::new(),
        _ => return Some(Err("arguments must be a JSON object".to_owned())),
    };
    map.insert("op".to_owned(), Value::String(name.to_owned()));
    match serde_json::from_value::<AgentCommand>(Value::Object(map)) {
        Ok(cmd) => Some(Ok(cmd)),
        Err(e) => Some(Err(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::{all_tools, command_tools, context_tools, to_command};
    use reticle_agent_api::AgentCommand;
    use serde_json::json;

    /// Every command tool's name round-trips into a command through `to_command`
    /// with a minimal-but-valid arguments object, proving the schema field names
    /// match the command serde fields.
    #[test]
    fn command_names_map_to_commands() {
        // A representative valid arguments object per command name.
        let cases: &[(&str, serde_json::Value)] = &[
            ("create_cell", json!({ "name": "top" })),
            ("delete_cell", json!({ "name": "top" })),
            (
                "add_rect",
                json!({ "cell": "top", "layer": { "layer": 68, "datatype": 20 },
                                 "rect": { "min": { "x": 0, "y": 0 }, "max": { "x": 10, "y": 10 } } }),
            ),
            (
                "add_polygon",
                json!({ "cell": "top", "layer": { "layer": 67, "datatype": 20 },
                                    "points": [ { "x": 0, "y": 0 }, { "x": 10, "y": 0 }, { "x": 0, "y": 10 } ] }),
            ),
            (
                "add_path",
                json!({ "cell": "top", "layer": { "layer": 68, "datatype": 20 },
                                 "width": 20, "points": [ { "x": 0, "y": 0 }, { "x": 100, "y": 0 } ] }),
            ),
            (
                "place_instance",
                json!({ "cell": "top", "child": "sub",
                                       "transform": { "orientation": "r0", "mag_num": 1, "mag_den": 1, "dx": 0, "dy": 0 } }),
            ),
            (
                "place_array",
                json!({ "cell": "top", "child": "sub",
                                    "transform": { "orientation": "r0", "mag_num": 1, "mag_den": 1, "dx": 0, "dy": 0 },
                                    "columns": 2, "rows": 2, "column_pitch": 100, "row_pitch": 100 }),
            ),
            (
                "transform_shapes",
                json!({ "ids": [1],
                                         "transform": { "orientation": "r90", "mag_num": 1, "mag_den": 1, "dx": 0, "dy": 0 } }),
            ),
            ("delete_shapes", json!({ "ids": [1] })),
            ("query_shapes", json!({ "cell": "top" })),
            ("get_cell_info", json!({ "cell": "top" })),
            ("list_layers", json!({})),
            (
                "set_technology",
                json!({ "source": "technology t\ndbu_per_micron 1000\n" }),
            ),
            ("run_drc", json!({ "cell": "top" })),
            ("get_violations", json!({})),
            (
                "route_net",
                json!({ "cell": "top", "net": "n", "layer": { "layer": 68, "datatype": 20 },
                                  "terminals": [ { "x": 0, "y": 0 }, { "x": 100, "y": 0 } ] }),
            ),
            ("run_extract", json!({ "cell": "top" })),
            ("check_intent", json!({ "cell": "top", "intent": "{}" })),
            (
                "netlist_compare",
                json!({ "cell": "top", "expected": "{\"nets\":[]}" }),
            ),
            ("export_gds", json!({})),
            ("export_oasis", json!({})),
            ("import_gds", json!({ "bytes": [0, 1, 2] })),
            (
                "render_png",
                json!({ "region": { "min": { "x": 0, "y": 0 }, "max": { "x": 10, "y": 10 } },
                                   "width": 64, "height": 64 }),
            ),
            ("save_session", json!({})),
            ("load_session", json!({ "snapshot": "{}" })),
            (
                "boolean_combine",
                json!({ "cell": "top", "bool_op": "union", "ids": [1, 2],
                                     "layer": { "layer": 68, "datatype": 20 } }),
            ),
            ("align_shapes", json!({ "ids": [1, 2], "align": "left" })),
            (
                "distribute_shapes",
                json!({ "ids": [1, 2, 3], "axis": "horizontal" }),
            ),
            ("offset_shapes", json!({ "ids": [1], "delta": 10 })),
            (
                "build_via_stack",
                json!({ "cell": "top",
                                    "lower_layer": { "layer": 68, "datatype": 20 },
                                    "upper_layer": { "layer": 69, "datatype": 20 },
                                    "cut_layer": { "layer": 66, "datatype": 44 },
                                    "center": { "x": 0, "y": 0 }, "cut_size": 40,
                                    "default_enclosure": 5 }),
            ),
        ];
        for (name, args) in cases {
            let got =
                to_command(name, args).unwrap_or_else(|| panic!("{name} should be a command tool"));
            assert!(got.is_ok(), "{name} args should deserialize: {got:?}");
        }
        // The case table must cover every advertised command tool.
        assert_eq!(
            cases.len(),
            command_tools().len(),
            "case table must cover all commands"
        );
    }

    /// The three context tools are not treated as commands.
    #[test]
    fn context_tools_are_not_commands() {
        for t in context_tools() {
            assert!(
                to_command(t.name, &json!({})).is_none(),
                "{} is not a command",
                t.name
            );
        }
    }

    /// A tag round-trips: retagging then reserializing yields the same command.
    #[test]
    fn retag_matches_direct_command() {
        let cmd = to_command("create_cell", &json!({ "name": "x" }))
            .unwrap()
            .unwrap();
        assert_eq!(cmd, AgentCommand::CreateCell { name: "x".into() });
    }

    /// The advertised catalog is command tools, the generator tools, and the three
    /// context tools, with unique names.
    #[test]
    fn catalog_is_complete_and_unique() {
        let all = all_tools();
        let generators = crate::generators::generator_tools().len();
        assert_eq!(all.len(), command_tools().len() + generators + 3);
        let mut names: Vec<_> = all.iter().map(|t| t.name).collect();
        names.sort_unstable();
        let mut deduped = names.clone();
        deduped.dedup();
        assert_eq!(names, deduped, "tool names must be unique");
    }
}
