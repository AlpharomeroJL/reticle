//! Stdio integration test: drive the built `reticle-mcp` binary as a subprocess.
//!
//! This launches the server executable, speaks newline-delimited JSON-RPC 2.0
//! over its stdin/stdout, and asserts on the responses. It exercises the full
//! transport (`initialize`, `tools/list`, `tools/call`) end to end and drives a
//! representative editing session: create a cell, add geometry, run DRC, extract,
//! check intent, export, render, and the budget limit. A coverage assertion at
//! the end proves every advertised tool was called at least once across the
//! test.
//!
//! The binary path comes from `CARGO_BIN_EXE_reticle-mcp`, which Cargo sets for
//! integration tests, so no path wrangling is needed.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

/// A live server subprocess and its piped stdio, plus the set of tool names
/// called (for the coverage assertion).
struct Harness {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    called: BTreeSet<String>,
    next_id: i64,
}

impl Harness {
    /// Launches the server with a generous budget and performs the `initialize`
    /// handshake.
    fn start() -> Self {
        Self::start_with_budget("10000")
    }

    /// Launches the server with an explicit `RETICLE_MCP_BUDGET`.
    fn start_with_budget(budget: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_reticle-mcp"))
            .env("RETICLE_MCP_BUDGET", budget)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn reticle-mcp");
        let stdin = child.stdin.take().expect("child stdin");
        let stdout = BufReader::new(child.stdout.take().expect("child stdout"));
        Self {
            child,
            stdin,
            stdout,
            called: BTreeSet::new(),
            next_id: 0,
        }
    }

    /// Sends a request and reads exactly one response line back.
    fn request(&mut self, mut msg: Value) -> Value {
        self.next_id += 1;
        msg["jsonrpc"] = json!("2.0");
        msg["id"] = json!(self.next_id);
        let line = msg.to_string();
        writeln!(self.stdin, "{line}").expect("write request");
        self.stdin.flush().expect("flush request");

        let mut response = String::new();
        let read = self.stdout.read_line(&mut response).expect("read response");
        assert!(read > 0, "server closed the stream unexpectedly");
        serde_json::from_str(&response).expect("parse response JSON")
    }

    /// Calls a tool, recording its name for coverage, and returns the response.
    /// Takes `arguments` by value so call sites pass a `json!(...)` literal.
    #[allow(clippy::needless_pass_by_value)]
    fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.called.insert(name.to_owned());
        self.request(json!({
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }))
    }

    /// The parsed structured payload of a tool result.
    fn payload(response: &Value) -> &Value {
        &response["result"]["structuredContent"]
    }

    /// Asserts a tool call succeeded (not an error) and returns its payload. A
    /// method (not an associated function) so tests read as `h.ok_payload(&r)`.
    #[allow(clippy::unused_self)]
    fn ok_payload<'a>(&self, response: &'a Value) -> &'a Value {
        assert_eq!(
            response["result"]["isError"], false,
            "tool call should succeed: {response}"
        );
        Self::payload(response)
    }

    /// Cleanly shuts the server down by closing stdin and reaping the process.
    fn shutdown(mut self) {
        drop(self.stdin);
        let _ = self.child.wait();
    }
}

/// A small technology file: met1 drawing plus a met1 spacing rule, enough to
/// drive DRC, extraction, intent, and rendering. Rule syntax is
/// `rule <kind> <layer> <datatype> <value>`; the parser names the rule
/// `<kind>_<layer>_<datatype>` (here `spacing_68_20`).
const TECH: &str = "technology demo\n\
                    dbu_per_micron 1000\n\
                    layer 68 20 met1 3A6FD490\n\
                    layer 68 16 met1_pin 3A6FD4C0\n\
                    rule spacing 68 20 140\n";

#[test]
#[allow(clippy::too_many_lines)]
fn stdio_server_drives_a_full_session() {
    let mut h = Harness::start();

    // ----- handshake --------------------------------------------------------
    let init = h.request(json!({ "method": "initialize" }));
    assert_eq!(init["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(init["result"]["serverInfo"]["name"], "reticle-mcp");

    let ping = h.request(json!({ "method": "ping" }));
    assert_eq!(ping["result"], json!({}));

    // ----- tools/list -------------------------------------------------------
    let listed = h.request(json!({ "method": "tools/list" }));
    let advertised: BTreeSet<String> = listed["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect();
    assert!(advertised.contains("create_cell"));
    assert!(advertised.contains("get_render_region"));

    // ----- technology + cells ----------------------------------------------
    let tech = h.call("set_technology", json!({ "source": TECH }));
    assert_eq!(h.ok_payload(&tech)["result"], "ok");

    let layers = h.call("list_layers", json!({}));
    assert_eq!(h.ok_payload(&layers)["technology"], "demo");

    let rules = h.call("get_technology_rules", json!({}));
    let rules_payload = h.ok_payload(&rules);
    assert_eq!(rules_payload["rule_count"], 1);
    assert_eq!(rules_payload["rules"][0]["name"], "spacing_68_20");
    assert_eq!(rules_payload["rules"][0]["kind"], "spacing");
    assert_eq!(rules_payload["rules"][0]["value_units"], "dbu");

    // A sub cell to instance and array.
    let sub = h.call("create_cell", json!({ "name": "sub" }));
    assert_eq!(h.ok_payload(&sub)["result"], "ok");
    h.call(
        "add_rect",
        json!({ "cell": "sub", "layer": { "layer": 68, "datatype": 20 },
                "rect": { "min": { "x": 0, "y": 0 }, "max": { "x": 50, "y": 50 } } }),
    );

    let top = h.call("create_cell", json!({ "name": "top" }));
    assert_eq!(h.ok_payload(&top)["result"], "ok");

    // ----- geometry: rect, polygon, path -----------------------------------
    let rect = h.call(
        "add_rect",
        json!({ "cell": "top", "layer": { "layer": 68, "datatype": 20 },
                "rect": { "min": { "x": 0, "y": 0 }, "max": { "x": 100, "y": 100 } } }),
    );
    let rect_payload = h.ok_payload(&rect);
    assert_eq!(rect_payload["revision"], 5);
    let rect_id = rect_payload["affected"][0].as_u64().unwrap();

    h.call(
        "add_polygon",
        json!({ "cell": "top", "layer": { "layer": 68, "datatype": 20 },
                "points": [ { "x": 200, "y": 0 }, { "x": 300, "y": 0 }, { "x": 300, "y": 100 }, { "x": 200, "y": 100 } ] }),
    );
    let path = h.call(
        "add_path",
        json!({ "cell": "top", "layer": { "layer": 68, "datatype": 20 },
                "width": 20, "endcap": "square",
                "points": [ { "x": 0, "y": 400 }, { "x": 400, "y": 400 } ] }),
    );
    assert_eq!(h.ok_payload(&path)["result"], "ok");

    // ----- placements -------------------------------------------------------
    h.call(
        "place_instance",
        json!({ "cell": "top", "child": "sub",
                "transform": { "orientation": "r0", "mag_num": 1, "mag_den": 1, "dx": 1000, "dy": 0 } }),
    );
    h.call(
        "place_array",
        json!({ "cell": "top", "child": "sub",
                "transform": { "orientation": "r0", "mag_num": 1, "mag_den": 1, "dx": 0, "dy": 1000 },
                "columns": 2, "rows": 2, "column_pitch": 100, "row_pitch": 100 }),
    );

    // ----- transform + query + delete on the rect --------------------------
    let moved = h.call(
        "transform_shapes",
        json!({ "ids": [rect_id],
                "transform": { "orientation": "r90", "mag_num": 1, "mag_den": 1, "dx": 0, "dy": 0 } }),
    );
    assert_eq!(h.ok_payload(&moved)["affected"], json!([rect_id]));

    let queried = h.call(
        "query_shapes",
        json!({ "cell": "top", "layer": { "layer": 68, "datatype": 20 } }),
    );
    let shapes = h.ok_payload(&queried)["shapes"].as_array().unwrap().len();
    assert!(
        shapes >= 3,
        "expected the rect, polygon and path, got {shapes}"
    );

    let info = h.call("get_cell_info", json!({ "cell": "top" }));
    assert_eq!(h.ok_payload(&info)["instances"], 1);

    // ----- DRC --------------------------------------------------------------
    let drc = h.call("run_drc", json!({ "cell": "top" }));
    let drc_payload = h.ok_payload(&drc);
    assert!(drc_payload["count"].is_number(), "DRC reports a count");

    let violations = h.call("get_violations", json!({}));
    assert!(h.ok_payload(&violations)["note"].is_string());

    // ----- routing on a second net-friendly cell ---------------------------
    let route = h.call(
        "route_net",
        json!({ "cell": "top", "net": "n1", "layer": { "layer": 68, "datatype": 20 },
                "terminals": [ { "x": 0, "y": 0 }, { "x": 500, "y": 0 } ] }),
    );
    assert!(h.ok_payload(&route)["routed"].is_number());

    // ----- extraction / intent / netlist compare ---------------------------
    let extract = h.call("run_extract", json!({ "cell": "top" }));
    assert!(h.ok_payload(&extract)["net_count"].is_number());

    let intent = h.call(
        "check_intent",
        json!({ "cell": "top", "intent": "{\"nets\":[],\"forbidden\":[]}" }),
    );
    // An empty intent is trivially satisfied; the payload is the report object.
    assert!(h.ok_payload(&intent).is_object());

    let compare = h.call(
        "netlist_compare",
        json!({ "cell": "top", "expected": "{\"nets\":[]}" }),
    );
    assert!(h.ok_payload(&compare)["equivalent"].is_boolean());

    // ----- Wave 2 editor ops (boolean, align, distribute, offset, via) -----
    // A dedicated cell keeps these ids clean and independent of the export path.
    h.call("create_cell", json!({ "name": "ops" }));
    let add_ops_rect = |h: &mut Harness, x0: i64, y0: i64, x1: i64, y1: i64| -> u64 {
        let r = h.call(
            "add_rect",
            json!({ "cell": "ops", "layer": { "layer": 68, "datatype": 20 },
                    "rect": { "min": { "x": x0, "y": y0 }, "max": { "x": x1, "y": y1 } } }),
        );
        h.ok_payload(&r)["affected"][0].as_u64().unwrap()
    };

    // boolean_combine: union of two overlapping rects into one shape on layer 68.
    let u1 = add_ops_rect(&mut h, 0, 0, 100, 100);
    let u2 = add_ops_rect(&mut h, 50, 0, 150, 100);
    let combined = h.call(
        "boolean_combine",
        json!({ "cell": "ops", "bool_op": "union", "ids": [u1, u2],
                "layer": { "layer": 68, "datatype": 20 } }),
    );
    assert_eq!(
        h.ok_payload(&combined)["affected"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "union yields one shape"
    );

    // offset_shapes: grow the union result by 10 DBU (it keeps its id).
    let union_id = h.ok_payload(&combined)["affected"][0].as_u64().unwrap();
    let grown = h.call("offset_shapes", json!({ "ids": [union_id], "delta": 10 }));
    assert_eq!(h.ok_payload(&grown)["affected"], json!([union_id]));

    // align_shapes + distribute_shapes over three fresh rects.
    let a1 = add_ops_rect(&mut h, 0, 500, 10, 510);
    let a2 = add_ops_rect(&mut h, 200, 600, 210, 610);
    let a3 = add_ops_rect(&mut h, 400, 700, 410, 710);
    let aligned = h.call(
        "align_shapes",
        json!({ "ids": [a1, a2, a3], "align": "bottom" }),
    );
    assert_eq!(
        h.ok_payload(&aligned)["affected"],
        json!([a1, a2, a3]),
        "align keeps the ids"
    );
    let distributed = h.call(
        "distribute_shapes",
        json!({ "ids": [a1, a2, a3], "axis": "horizontal" }),
    );
    assert_eq!(h.ok_payload(&distributed)["affected"], json!([a1, a2, a3]));

    // build_via_stack: cut plus two enclosures (default margins, no via rule).
    let via = h.call(
        "build_via_stack",
        json!({ "cell": "ops",
                "lower_layer": { "layer": 68, "datatype": 20 },
                "upper_layer": { "layer": 68, "datatype": 16 },
                "cut_layer": { "layer": 66, "datatype": 44 },
                "center": { "x": 1000, "y": 1000 }, "cut_size": 40,
                "default_enclosure": 10 }),
    );
    assert_eq!(
        h.ok_payload(&via)["affected"].as_array().unwrap().len(),
        3,
        "via stack is cut plus two enclosures"
    );

    // ----- export GDS / OASIS, round-trip import ---------------------------
    let gds = h.call("export_gds", json!({}));
    let gds_payload = h.ok_payload(&gds);
    assert_eq!(gds_payload["result"], "blob");
    let gds_bytes: Vec<u8> = gds_payload["bytes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_u64().unwrap() as u8)
        .collect();
    assert!(gds_bytes.len() > 4, "GDS export should have content");

    let oasis = h.call("export_oasis", json!({}));
    assert_eq!(h.ok_payload(&oasis)["result"], "blob");

    let imported = h.call("import_gds", json!({ "bytes": gds_bytes }));
    assert_eq!(h.ok_payload(&imported)["result"], "ok");
    // After import the document still summarizes; top/sub survive the round trip.
    let summary = h.call("get_document_summary", json!({}));
    let summary_payload = h.ok_payload(&summary);
    assert!(summary_payload["cell_count"].as_u64().unwrap() >= 2);

    // ----- render (command + context tool), graceful without a GPU ---------
    let render = h.call(
        "render_png",
        json!({ "region": { "min": { "x": 0, "y": 0 }, "max": { "x": 400, "y": 400 } },
                "width": 64, "height": 64 }),
    );
    // With a GPU it is a blob; headless it is an engine_error. Either is a
    // well-formed response and must not crash the server.
    let render_ok = render["result"]["isError"] == json!(false);
    if render_ok {
        assert_eq!(Harness::payload(&render)["result"], "blob");
    } else {
        assert_eq!(Harness::payload(&render)["code"], "engine_error");
    }

    let region = h.call(
        "get_render_region",
        json!({ "region": { "min": { "x": 0, "y": 0 }, "max": { "x": 400, "y": 400 } },
                "width": 48, "height": 48 }),
    );
    let region_payload = h.ok_payload(&region);
    assert!(region_payload["available"].is_boolean());
    if region_payload["available"] == json!(true) {
        assert!(
            region_payload["image_data_uri"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
    }

    // ----- session save / load ---------------------------------------------
    let saved = h.call("save_session", json!({}));
    let saved_payload = h.ok_payload(&saved);
    assert_eq!(saved_payload["result"], "blob");
    let snapshot: String = saved_payload["bytes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_u64().unwrap() as u8 as char)
        .collect();
    let loaded = h.call("load_session", json!({ "snapshot": snapshot }));
    assert_eq!(h.ok_payload(&loaded)["result"], "ok");

    // ----- delete a shape and a cell ---------------------------------------
    // Query the imported top cell to get a live shape id, then delete it.
    let requery = h.call("query_shapes", json!({ "cell": "top" }));
    let some_id = h.ok_payload(&requery)["shapes"]
        .as_array()
        .and_then(|shapes| shapes.iter().find_map(|s| s["id"].as_u64()));
    if let Some(id) = some_id {
        let deleted = h.call("delete_shapes", json!({ "ids": [id] }));
        assert_eq!(h.ok_payload(&deleted)["affected"], json!([id]));
    } else {
        // No id-addressable shape survived the import; exercise the tool with an
        // absent id, which is a well-formed no_such_element error.
        let deleted = h.call("delete_shapes", json!({ "ids": [999_999] }));
        assert_eq!(deleted["result"]["isError"], true);
    }

    let del_cell = h.call("delete_cell", json!({ "name": "sub" }));
    // `sub` may or may not exist after the GDS round trip; either a clean delete
    // or a no_such_cell error exercises the tool.
    assert!(del_cell["result"]["isError"].is_boolean());

    // ----- generator tools (one per built-in generator) --------------------
    // Each generator tool takes a target `cell` plus the generator's own params and
    // maps to a RunGenerator command. A fresh cell isolates the generated geometry.
    // The generators emit on the SKY130 subset layers regardless of the session's
    // demo technology, so these run without setting SKY130. Driving all six keeps
    // the coverage assertion satisfied and proves the generator surface end to end.
    h.call("create_cell", json!({ "name": "gens" }));

    // A default guard ring (li1 with taps): its footprint is well-formed geometry.
    let guard = h.call(
        "guard_ring",
        json!({ "cell": "gens", "layer": "li1", "region_width": 2000,
                "region_height": 2000, "ring_width": 400, "taps": true }),
    );
    let guard_ids = h.ok_payload(&guard)["affected"].as_array().unwrap().len();
    assert!(guard_ids >= 4, "a ring is at least four strips: {guard}");

    // A 3x3 mcon via farm: nine cuts plus a lower and upper plate.
    let farm = h.call(
        "via_farm",
        json!({ "cell": "gens", "cut": "mcon", "rows": 3, "cols": 3 }),
    );
    assert_eq!(
        h.ok_payload(&farm)["affected"].as_array().unwrap().len(),
        11,
        "9 cuts plus two plates: {farm}"
    );

    // The remaining four generators, driven with schema-default-shaped params.
    let pad = h.call(
        "pad_ring",
        json!({ "cell": "gens", "die_width": 200_000, "die_height": 200_000,
                "pad_pitch": 100_000, "pad_size": 60_000, "power_pads": 4 }),
    );
    assert_eq!(h.ok_payload(&pad)["result"], "ok", "pad_ring: {pad}");

    let seal = h.call(
        "seal_ring",
        json!({ "cell": "gens", "stack": "up_to_met3", "die_width": 100_000,
                "die_height": 100_000, "ring_width": 900 }),
    );
    assert_eq!(h.ok_payload(&seal)["result"], "ok", "seal_ring: {seal}");

    let fill = h.call(
        "fill",
        json!({ "cell": "gens", "layer": "li1", "region_width": 10_000,
                "region_height": 10_000, "tile": 400, "target_density_permille": 400 }),
    );
    assert_eq!(h.ok_payload(&fill)["result"], "ok", "fill: {fill}");

    let test_struct = h.call(
        "test_structure",
        json!({ "cell": "gens", "kind": "van_der_pauw", "layer": "li1",
                "feature_width": 400, "feature_length": 2000, "count": 8 }),
    );
    assert_eq!(
        h.ok_payload(&test_struct)["result"],
        "ok",
        "test_structure: {test_struct}"
    );

    // The generated geometry is DRC-clean by construction; run DRC over the cell to
    // prove the generator surface lands clean layout end to end. (The demo tech
    // carries only a met1 spacing rule, but the generators satisfy the full subset,
    // so the check is trivially clean here; the strong check is in reticle-gen.)
    let gen_drc = h.call("run_drc", json!({ "cell": "gens" }));
    assert_eq!(
        h.ok_payload(&gen_drc)["count"],
        0,
        "generated cell is clean"
    );

    // A generator tool with an out-of-range parameter is a well-formed tool error
    // (the generator's own validation), never a crash.
    let bad_gen = h.call(
        "via_farm",
        json!({ "cell": "gens", "cut": "mcon", "rows": 0, "cols": 3 }),
    );
    assert_eq!(bad_gen["result"]["isError"], true);
    assert_eq!(Harness::payload(&bad_gen)["code"], "invalid_argument");

    // ----- coverage assertion: every advertised tool was called ------------
    let missing: Vec<&String> = advertised.difference(&h.called).collect();
    assert!(
        missing.is_empty(),
        "these advertised tools were never called: {missing:?}"
    );

    h.shutdown();
}

#[test]
fn stdio_server_enforces_budget() {
    // A budget of two commands: the first two apply, the third is rejected.
    let mut h = Harness::start_with_budget("2");

    let a = h.call("create_cell", json!({ "name": "a" }));
    assert_eq!(a["result"]["isError"], false);
    let b = h.call("create_cell", json!({ "name": "b" }));
    assert_eq!(b["result"]["isError"], false);

    let c = h.call("create_cell", json!({ "name": "c" }));
    assert_eq!(c["result"]["isError"], true);
    assert_eq!(
        Harness::payload(&c)["code"],
        "budget_exhausted",
        "third command must be rejected: {c}"
    );

    // A read-only context tool still answers after the budget is spent.
    let summary = h.call("get_document_summary", json!({}));
    assert_eq!(summary["result"]["isError"], false);
    assert_eq!(Harness::payload(&summary)["cell_count"], 2);

    h.shutdown();
}
