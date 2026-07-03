//! JSON input schemas for the command tools.
//!
//! Each [`AgentCommand`](reticle_agent_api::AgentCommand) variant is exposed as a
//! tool whose `inputSchema` is a JSON Schema (draft 2020-12 vocabulary, the
//! subset MCP clients understand) built here by hand from the command's argument
//! shape. Hand-writing rather than deriving lets each field carry a description
//! in the units and conventions a model needs: coordinates are database units
//! (DBU), layers are a GDSII `(layer, datatype)` pair, and magnification is an
//! integer ratio.
//!
//! The builders return [`serde_json::Value`] objects that are assembled into the
//! tool catalog in [`crate::tools`]. Field names match the serde field names on
//! the command variants exactly, so the arguments object a client sends can be
//! re-tagged with the command `op` and deserialized straight into an
//! `AgentCommand`.

use serde_json::{Value, json};

/// An `object` schema with the given properties and required keys.
///
/// `properties` is a list of `(name, schema)` pairs; `required` names the subset
/// that must be present. `additionalProperties` is left permissive so a client
/// may pass extra fields (they are ignored on deserialization).
fn object(properties: &[(&str, Value)], required: &[&str]) -> Value {
    let props: serde_json::Map<String, Value> = properties
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect();
    json!({
        "type": "object",
        "properties": props,
        "required": required,
    })
}

/// The empty-argument schema, for commands that take no fields.
pub fn no_args() -> Value {
    json!({ "type": "object", "properties": {} })
}

/// A signed integer field in database units.
fn dbu(description: &str) -> Value {
    json!({ "type": "integer", "description": description })
}

/// A `{x, y}` point in database units.
fn point(description: &str) -> Value {
    json!({
        "type": "object",
        "description": description,
        "properties": {
            "x": dbu("X coordinate in database units (DBU)."),
            "y": dbu("Y coordinate in database units (DBU)."),
        },
        "required": ["x", "y"],
    })
}

/// A `{min, max}` axis-aligned rectangle in database units.
fn rect(description: &str) -> Value {
    json!({
        "type": "object",
        "description": description,
        "properties": {
            "min": point("Lower-left corner, in DBU."),
            "max": point("Upper-right corner, in DBU."),
        },
        "required": ["min", "max"],
    })
}

/// A GDSII `{layer, datatype}` pair.
fn layer() -> Value {
    json!({
        "type": "object",
        "description": "A GDSII layer and datatype. `layer` selects the mask layer \
                        (for example 68 = met1 in SKY130); `datatype` selects the \
                        purpose on that layer (for example 20 = drawing, 16 = pin, \
                        5 = label). Both are unsigned 16-bit numbers. The layer must \
                        exist in the active technology or the command fails with \
                        `no_such_layer`.",
        "properties": {
            "layer": { "type": "integer", "minimum": 0, "maximum": 65535,
                       "description": "GDSII layer number (0-65535)." },
            "datatype": { "type": "integer", "minimum": 0, "maximum": 65535,
                          "description": "GDSII datatype/purpose number (0-65535)." },
        },
        "required": ["layer", "datatype"],
    })
}

/// The D4 orientation enum, as a string.
fn orientation() -> Value {
    json!({
        "type": "string",
        "description": "One of the eight D4 orientations. `r0`/`r90`/`r180`/`r270` \
                        rotate counter-clockwise; the `mirror_x*` variants mirror \
                        across the x axis first, then rotate.",
        "enum": ["r0", "r90", "r180", "r270",
                 "mirror_x", "mirror_x90", "mirror_x180", "mirror_x270"],
    })
}

/// A placement transform: orientation, integer magnification ratio, translation.
fn transform() -> Value {
    json!({
        "type": "object",
        "description": "A placement transform applied to a child cell or a set of \
                        shapes: an orientation, an integer magnification ratio \
                        `mag_num / mag_den` (both positive), and a translation \
                        `(dx, dy)` in DBU.",
        "properties": {
            "orientation": orientation(),
            "mag_num": { "type": "integer", "minimum": 1,
                         "description": "Magnification numerator (positive)." },
            "mag_den": { "type": "integer", "minimum": 1,
                         "description": "Magnification denominator (positive, non-zero)." },
            "dx": dbu("Translation along x, in DBU."),
            "dy": dbu("Translation along y, in DBU."),
        },
        "required": ["orientation", "mag_num", "mag_den", "dx", "dy"],
    })
}

/// An array of stable element ids (unsigned integers).
fn element_ids(description: &str) -> Value {
    json!({
        "type": "array",
        "description": description,
        "items": { "type": "integer", "minimum": 1,
                   "description": "A stable element id returned by an earlier \
                                   mutating command." },
    })
}

/// A `cell` name field with a shared description.
fn cell_name(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

// ===== per-command schemas ==================================================

/// `create_cell`
pub fn create_cell() -> Value {
    object(
        &[(
            "name",
            cell_name(
                "Name for the new, empty cell. Must be unique; a duplicate \
                       name fails with `invalid_argument`.",
            ),
        )],
        &["name"],
    )
}

/// `delete_cell`
pub fn delete_cell() -> Value {
    object(
        &[(
            "name",
            cell_name(
                "Name of the cell to remove. Fails with `no_such_cell` if it \
                       is absent.",
            ),
        )],
        &["name"],
    )
}

/// `add_rect`
pub fn add_rect() -> Value {
    object(
        &[
            ("cell", cell_name("Target cell that gains the rectangle.")),
            ("layer", layer()),
            (
                "rect",
                rect(
                    "The rectangle to add, in DBU. Corners are normalized, so \
                           `min`/`max` need not be pre-sorted.",
                ),
            ),
        ],
        &["cell", "layer", "rect"],
    )
}

/// `add_polygon`
pub fn add_polygon() -> Value {
    object(
        &[
            ("cell", cell_name("Target cell that gains the polygon.")),
            ("layer", layer()),
            (
                "points",
                json!({
                    "type": "array",
                    "description": "The polygon vertices in order, in DBU. At least \
                                    three are required; the ring is closed implicitly \
                                    (do not repeat the first point).",
                    "items": point("A polygon vertex in DBU."),
                    "minItems": 3,
                }),
            ),
        ],
        &["cell", "layer", "points"],
    )
}

/// `add_path`
pub fn add_path() -> Value {
    object(
        &[
            ("cell", cell_name("Target cell that gains the path.")),
            ("layer", layer()),
            (
                "width",
                json!({ "type": "integer", "minimum": 0,
                        "description": "Path width in DBU (non-negative)." }),
            ),
            (
                "points",
                json!({
                    "type": "array",
                    "description": "The path spine vertices in order, in DBU. At \
                                    least two are required.",
                    "items": point("A spine vertex in DBU."),
                    "minItems": 2,
                }),
            ),
            (
                "endcap",
                json!({
                    "type": "string",
                    "description": "End-cap style. `flat` (the default) ends at the \
                                    terminal vertex; `square` extends by half the \
                                    width; `round` approximates a semicircular cap.",
                    "enum": ["flat", "square", "round"],
                }),
            ),
        ],
        &["cell", "layer", "width", "points"],
    )
}

/// `place_instance`
pub fn place_instance() -> Value {
    object(
        &[
            ("cell", cell_name("Parent cell that gains the placement.")),
            (
                "child",
                cell_name("The child cell to place. Must already exist."),
            ),
            ("transform", transform()),
        ],
        &["cell", "child", "transform"],
    )
}

/// `place_array`
pub fn place_array() -> Value {
    object(
        &[
            ("cell", cell_name("Parent cell that gains the array.")),
            (
                "child",
                cell_name("The child cell to array. Must already exist."),
            ),
            ("transform", transform()),
            (
                "columns",
                json!({ "type": "integer", "minimum": 1,
                        "description": "Number of columns (positive)." }),
            ),
            (
                "rows",
                json!({ "type": "integer", "minimum": 1,
                        "description": "Number of rows (positive)." }),
            ),
            (
                "column_pitch",
                dbu("Column pitch (center-to-center) in DBU."),
            ),
            ("row_pitch", dbu("Row pitch (center-to-center) in DBU.")),
        ],
        &[
            "cell",
            "child",
            "transform",
            "columns",
            "rows",
            "column_pitch",
            "row_pitch",
        ],
    )
}

/// `transform_shapes`
pub fn transform_shapes() -> Value {
    object(
        &[
            (
                "ids",
                element_ids(
                    "The shapes to transform. Only shape ids are accepted; \
                             an instance or array id fails with `invalid_argument`. \
                             Each id keeps addressing the same shape after the \
                             transform.",
                ),
            ),
            ("transform", transform()),
        ],
        &["ids", "transform"],
    )
}

/// `delete_shapes`
pub fn delete_shapes() -> Value {
    object(
        &[(
            "ids",
            element_ids(
                "The shapes to delete. Only shape ids are accepted; an \
                         instance or array id fails with `invalid_argument`. \
                         Surviving ids keep addressing the same shapes.",
            ),
        )],
        &["ids"],
    )
}

/// `query_shapes`
pub fn query_shapes() -> Value {
    object(
        &[
            ("cell", cell_name("The cell to query.")),
            ("layer", {
                let mut l = layer();
                l["description"] = json!(
                    "Optional layer filter. When present, \
                        only shapes on this exact `(layer, datatype)` are returned."
                );
                l
            }),
            (
                "region",
                rect(
                    "Optional region filter, in DBU. When present, only \
                             shapes whose bounding box overlaps or touches this \
                             rectangle are returned.",
                ),
            ),
        ],
        &["cell"],
    )
}

/// `get_cell_info`
pub fn get_cell_info() -> Value {
    object(
        &[(
            "cell",
            cell_name(
                "The cell to summarize (shape/instance/array/label/pin \
                              counts and bounding box).",
            ),
        )],
        &["cell"],
    )
}

/// `set_technology`
pub fn set_technology() -> Value {
    object(
        &[(
            "source",
            json!({ "type": "string",
                    "description": "The full text of a Reticle technology file \
                                    (`.tech`): `dbu_per_micron`, `layer` lines, DRC \
                                    `rule` lines, and optional `stack` lines. \
                                    Replaces the active technology; cell contents are \
                                    kept. A parse error fails with `invalid_argument`." }),
        )],
        &["source"],
    )
}

/// `run_drc`
pub fn run_drc() -> Value {
    object(
        &[
            ("cell", cell_name("The cell to design-rule-check.")),
            (
                "region",
                rect(
                    "Optional region to scope the check to, in DBU. Omit to \
                             check the whole cell.",
                ),
            ),
        ],
        &["cell"],
    )
}

/// `route_net`
pub fn route_net() -> Value {
    object(
        &[
            ("cell", cell_name("The cell to route in.")),
            (
                "net",
                json!({ "type": "string", "description": "The net name to assign to \
                                                          the routed wires." }),
            ),
            ("layer", layer()),
            (
                "terminals",
                json!({
                    "type": "array",
                    "description": "The terminal points to connect, in DBU. At least \
                                    two are required.",
                    "items": point("A terminal point in DBU."),
                    "minItems": 2,
                }),
            ),
        ],
        &["cell", "net", "layer", "terminals"],
    )
}

/// `run_extract`
pub fn run_extract() -> Value {
    object(
        &[("cell", cell_name("The cell to extract a netlist from."))],
        &["cell"],
    )
}

/// `check_intent`
pub fn check_intent() -> Value {
    object(
        &[
            ("cell", cell_name("The cell to check against the intent.")),
            (
                "intent",
                json!({ "type": "string",
                        "description": "A connectivity intent spec as a JSON string \
                                        (an `IntentSpec`: named nets, each with \
                                        terminals of `{name, layer, region}`, plus \
                                        optional forbidden layer pairs). Malformed \
                                        JSON fails with `invalid_argument`. The result \
                                        reports opens and shorts." }),
            ),
        ],
        &["cell", "intent"],
    )
}

/// `netlist_compare`
pub fn netlist_compare() -> Value {
    object(
        &[
            (
                "cell",
                cell_name("The cell whose extracted netlist is compared."),
            ),
            (
                "expected",
                json!({ "type": "string",
                        "description": "The expected netlist as a JSON string: \
                                        `{\"nets\":[{\"name\":..,\"shapes\":[..]}]}` \
                                        (or a bare array of such nets), where `shapes` \
                                        are shape indices. The result reports whether \
                                        the netlists are equivalent." }),
            ),
        ],
        &["cell", "expected"],
    )
}

/// `import_gds`
pub fn import_gds() -> Value {
    object(
        &[(
            "bytes",
            json!({
                "type": "array",
                "description": "The GDSII file contents as an array of byte values \
                                (0-255). Replaces the entire session document; all \
                                prior element ids become invalid.",
                "items": { "type": "integer", "minimum": 0, "maximum": 255 },
            }),
        )],
        &["bytes"],
    )
}

/// `render_png`
pub fn render_png() -> Value {
    object(
        &[
            (
                "region",
                rect("The region of the document to render, in DBU."),
            ),
            (
                "width",
                json!({ "type": "integer", "minimum": 1,
                        "description": "Output width in pixels." }),
            ),
            (
                "height",
                json!({ "type": "integer", "minimum": 1,
                        "description": "Output height in pixels." }),
            ),
        ],
        &["region", "width", "height"],
    )
}

/// `get_render_region` (context tool): same region/size arguments as
/// `render_png`, but the result is an inline base64 PNG.
pub fn render_region() -> Value {
    object(
        &[
            (
                "region",
                rect("The region of the document to render, in DBU."),
            ),
            (
                "width",
                json!({ "type": "integer", "minimum": 1,
                        "description": "Output width in pixels." }),
            ),
            (
                "height",
                json!({ "type": "integer", "minimum": 1,
                        "description": "Output height in pixels." }),
            ),
        ],
        &["region", "width", "height"],
    )
}

/// `load_session`
pub fn load_session() -> Value {
    object(
        &[(
            "snapshot",
            json!({ "type": "string",
                    "description": "A serialized session snapshot (the JSON produced \
                                    by `save_session`). Rebuilds the document by \
                                    replaying its recorded commands." }),
        )],
        &["snapshot"],
    )
}

// ===== Wave 2 editor-op schemas =============================================

/// The planar boolean operation enum, as a string.
fn boolean_op() -> Value {
    json!({
        "type": "string",
        "description": "The planar boolean to apply. `union` merges the shapes; \
                        `intersection` keeps only their common region; `difference` \
                        subtracts the later shapes from the first; `xor` keeps the \
                        region covered an odd number of times.",
        "enum": ["union", "intersection", "difference", "xor"],
    })
}

/// The align-kind enum, as a string.
fn align_kind() -> Value {
    json!({
        "type": "string",
        "description": "How to align the shapes within their combined bounding box. \
                        `left`/`right`/`top`/`bottom` line the corresponding edges up \
                        to the extreme edge; `center_x`/`center_y` center on the \
                        selection's midline.",
        "enum": ["left", "right", "center_x", "top", "bottom", "center_y"],
    })
}

/// The distribute-axis enum, as a string.
fn axis() -> Value {
    json!({
        "type": "string",
        "description": "The axis whose gaps are equalized. `horizontal` respaces \
                        left-to-right; `vertical` respaces top-to-bottom.",
        "enum": ["horizontal", "vertical"],
    })
}

/// `boolean_combine`
pub fn boolean_combine() -> Value {
    object(
        &[
            (
                "cell",
                cell_name("The cell holding the input shapes and receiving the result."),
            ),
            ("bool_op", boolean_op()),
            (
                "ids",
                element_ids(
                    "The input shapes, addressed by id. At least two are required. \
                     Rectangles and polygons participate; paths are skipped. The \
                     inputs are deleted and replaced by the result.",
                ),
            ),
            ("layer", {
                let mut l = layer();
                l["description"] = json!(
                    "The layer and datatype the result polygons are written to. May \
                     differ from the inputs' layers."
                );
                l
            }),
        ],
        &["cell", "bool_op", "ids", "layer"],
    )
}

/// `align_shapes`
pub fn align_shapes() -> Value {
    object(
        &[
            (
                "ids",
                element_ids(
                    "The shapes to align, addressed by id. At least two are \
                     required, and all must be in the same cell. Each keeps its id.",
                ),
            ),
            ("align", align_kind()),
        ],
        &["ids", "align"],
    )
}

/// `distribute_shapes`
pub fn distribute_shapes() -> Value {
    object(
        &[
            (
                "ids",
                element_ids(
                    "The shapes to distribute, addressed by id. At least three are \
                     required, and all must be in the same cell. The two extreme \
                     shapes stay put; the inner shapes move to equalize the gaps. \
                     Each keeps its id.",
                ),
            ),
            ("axis", axis()),
        ],
        &["ids", "axis"],
    )
}

/// `offset_shapes`
pub fn offset_shapes() -> Value {
    object(
        &[
            (
                "ids",
                element_ids(
                    "The shapes to offset, addressed by id. Rectangles and polygons \
                     participate; paths are skipped. Each keeps its id (a shrink \
                     that erases a shape retires its id).",
                ),
            ),
            (
                "delta",
                dbu(
                    "The offset amount in DBU: positive grows the shapes outward, \
                     negative shrinks them inward.",
                ),
            ),
        ],
        &["ids", "delta"],
    )
}

/// `build_via_stack`
pub fn build_via_stack() -> Value {
    object(
        &[
            ("cell", cell_name("The cell that gains the via stack.")),
            ("lower_layer", {
                let mut l = layer();
                l["description"] = json!("The lower routing layer to enclose the cut on.");
                l
            }),
            ("upper_layer", {
                let mut l = layer();
                l["description"] = json!("The upper routing layer to enclose the cut on.");
                l
            }),
            ("cut_layer", {
                let mut l = layer();
                l["description"] = json!("The cut/via layer the square cut is drawn on.");
                l
            }),
            ("center", point("The center of the via stack, in DBU.")),
            (
                "cut_size",
                json!({ "type": "integer", "minimum": 1,
                        "description": "The side length of the square cut in DBU (positive)." }),
            ),
            (
                "default_enclosure",
                dbu("The enclosure margin (DBU) to use for a layer that has no \
                     enclosure rule in the active technology. When a rule exists it \
                     wins over this default."),
            ),
        ],
        &[
            "cell",
            "lower_layer",
            "upper_layer",
            "cut_layer",
            "center",
            "cut_size",
            "default_enclosure",
        ],
    )
}
