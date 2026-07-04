//! The stdio JSON-RPC server: transport, dispatch, session, and budget.
//!
//! [`Server`] owns one [`Session`] and a [`Budget`]. [`Server::run`] reads
//! newline-delimited JSON-RPC 2.0 requests from a reader and writes responses to
//! a writer (stdin/stdout in the binary), matching the MCP stdio transport and
//! the existing `reticle-dev` server. [`Server::handle_line`] processes a single
//! request line and returns the response bytes, which the integration test drives
//! directly.
//!
//! # Methods
//!
//! * `initialize` advertises the protocol version and the `tools` capability.
//! * `tools/list` returns the full catalog ([`crate::tools::all_tools`]).
//! * `tools/call` dispatches a command tool (retagged into an
//!   [`AgentCommand`](reticle_agent_api::AgentCommand) and applied) or a context
//!   tool (handled in [`crate::context`]).
//! * `ping` is answered with an empty result.
//!
//! Any tool call that applies a command to the session first draws down the
//! [`Budget`]; once it is exhausted the call is rejected with a
//! `budget_exhausted` error and the session is left untouched.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use reticle_agent_api::{AgentResponse, ErrorCode, Session};

use crate::context;
use crate::tools;

/// The MCP protocol version this server implements (the stdio-transport revision
/// the sibling `reticle-dev` server also speaks).
const PROTOCOL_VERSION: &str = "2024-11-05";

/// JSON-RPC "method not found".
const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC "invalid params".
const INVALID_PARAMS: i64 = -32602;

/// A command budget: the number of commands a session may still apply.
///
/// Every tool call that applies a command to the session (all command tools, and
/// the `get_render_region` context tool, which renders through a command) draws
/// one unit. Pure read-only context tools do not. Once the remaining count hits
/// zero, further command tools are rejected with an
/// [`ErrorCode::BudgetExhausted`] error and the session is not touched.
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    remaining: u64,
}

impl Budget {
    /// A budget of `limit` commands.
    #[must_use]
    pub fn new(limit: u64) -> Self {
        Self { remaining: limit }
    }

    /// The commands still available.
    #[must_use]
    pub fn remaining(&self) -> u64 {
        self.remaining
    }

    /// Draws one command if any remain, returning whether it was granted.
    fn draw(&mut self) -> bool {
        if self.remaining == 0 {
            false
        } else {
            self.remaining -= 1;
            true
        }
    }
}

impl Default for Budget {
    /// A generous default (`10_000` commands) for interactive use; the binary
    /// lets the operator override it.
    fn default() -> Self {
        Self::new(10_000)
    }
}

/// An MCP server over one Reticle [`Session`] with a command [`Budget`].
#[derive(Debug)]
pub struct Server {
    session: Session,
    budget: Budget,
}

impl Default for Server {
    fn default() -> Self {
        Self::new(Budget::default())
    }
}

impl Server {
    /// A server with a fresh session and the given command budget.
    #[must_use]
    pub fn new(budget: Budget) -> Self {
        Self {
            session: Session::new(),
            budget,
        }
    }

    /// The remaining command budget.
    #[must_use]
    pub fn budget_remaining(&self) -> u64 {
        self.budget.remaining()
    }

    /// Runs the stdio loop: read one JSON-RPC request per line from `input`, write
    /// each response as a line to `output`, until end of input.
    ///
    /// Notifications (requests without an `id`) are consumed and not answered, per
    /// JSON-RPC. Malformed or non-JSON lines are skipped so a stray blank line
    /// does not tear down the session.
    pub fn run<R: BufRead, W: Write>(&mut self, input: R, mut output: W) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Some(response) = self.handle_line(&line) {
                writeln!(output, "{response}")?;
                output.flush()?;
            }
        }
        Ok(())
    }

    /// Handles one request line, returning the response JSON to write, or `None`
    /// for a notification (no `id`) or an unparsable line.
    #[must_use]
    pub fn handle_line(&mut self, line: &str) -> Option<Value> {
        let msg: Value = serde_json::from_str(line).ok()?;
        // Notifications carry no `id` and get no response.
        let id = msg.get("id").cloned()?;
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let response = match method {
            "initialize" => ok(&id, &initialize_result()),
            "ping" => ok(&id, &json!({})),
            "tools/list" => ok(&id, &json!({ "tools": tool_list() })),
            "tools/call" => self.handle_call(&id, &msg),
            other => rpc_error(&id, METHOD_NOT_FOUND, &format!("method not found: {other}")),
        };
        Some(response)
    }

    /// Dispatches a `tools/call` request.
    fn handle_call(&mut self, id: &Value, msg: &Value) -> Value {
        let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return rpc_error(id, INVALID_PARAMS, "tools/call requires a tool name");
        };
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        // Context tools first: two are pure reads; `get_render_region` applies a
        // render command and so draws budget.
        match name {
            tools::GET_TECHNOLOGY_RULES => {
                return tool_ok(id, &context::get_technology_rules(&self.session));
            }
            tools::GET_DOCUMENT_SUMMARY => {
                return tool_ok(id, &context::get_document_summary(&self.session));
            }
            tools::GET_RENDER_REGION => {
                if !self.budget.draw() {
                    return tool_err(id, &budget_error_json());
                }
                return match context::get_render_region(&mut self.session, &arguments) {
                    Ok(v) => tool_ok(id, &v),
                    Err(e) => tool_err(id, &json!({ "code": "invalid_argument", "message": e })),
                };
            }
            _ => {}
        }

        // Otherwise it is a generator tool (mapped to a RunGenerator command) or a
        // one-to-one command tool. A generator id resolves first; if the name is not
        // a generator, fall through to the command-tool retag path.
        let Some(parsed) = crate::generators::to_generator_command(name, &arguments)
            .or_else(|| tools::to_command(name, &arguments))
        else {
            return tool_err(
                id,
                &json!({ "code": "invalid_argument", "message": format!("unknown tool: {name}") }),
            );
        };
        let command = match parsed {
            Ok(cmd) => cmd,
            Err(e) => {
                return tool_err(
                    id,
                    &json!({ "code": "invalid_argument",
                             "message": format!("invalid arguments for {name}: {e}") }),
                );
            }
        };

        if !self.budget.draw() {
            return tool_err(id, &budget_error_json());
        }

        match self.session.apply(command) {
            Ok(response) => tool_ok(id, &response_json(&response)),
            Err(err) => tool_err(
                id,
                &json!({ "code": error_code_str(err.code), "message": err.message }),
            ),
        }
    }
}

/// The advertised tool catalog as a JSON array.
fn tool_list() -> Vec<Value> {
    tools::all_tools()
        .iter()
        .map(tools::ToolSpec::to_json)
        .collect()
}

/// The `initialize` result: protocol version, capabilities, and server info.
fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "reticle-mcp",
            "version": env!("CARGO_PKG_VERSION"),
        },
    })
}

/// A successful JSON-RPC response wrapping `result`.
fn ok(id: &Value, result: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// A JSON-RPC protocol error (not a tool error).
fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// A `tools/call` success result carrying `payload` as pretty-printed JSON text.
///
/// The MCP tool-result shape is `{content: [{type:"text", text}], isError}`. The
/// payload (a command response or a context payload) is serialized as indented
/// JSON so a model reads structured fields, and echoed under `structuredContent`
/// for clients that consume it directly.
fn tool_ok(id: &Value, payload: &Value) -> Value {
    let text = serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string());
    ok(
        id,
        &json!({
            "content": [{ "type": "text", "text": text }],
            "structuredContent": payload,
            "isError": false,
        }),
    )
}

/// A `tools/call` error result: the structured error as text, with `isError`.
fn tool_err(id: &Value, error: &Value) -> Value {
    let text = serde_json::to_string_pretty(error).unwrap_or_else(|_| error.to_string());
    ok(
        id,
        &json!({
            "content": [{ "type": "text", "text": text }],
            "structuredContent": error,
            "isError": true,
        }),
    )
}

/// The budget-exhausted tool error payload.
fn budget_error_json() -> Value {
    json!({
        "code": error_code_str(ErrorCode::BudgetExhausted),
        "message": "session command budget exhausted; no further commands will be applied",
    })
}

/// Shapes an [`AgentResponse`] into the JSON a tool result carries.
///
/// The three response kinds are surfaced so a model reads the useful fields at
/// the top level, tagged by `result`:
///
/// * `Ok` keeps `revision` and `affected` (the created/changed element ids).
/// * `Data` is flattened: the structured `value` object's own fields are lifted
///   to the top level alongside `result` and `revision`, so a query like
///   `list_layers` exposes `technology`/`layers` directly rather than nested
///   under `value`. (A non-object `value`, which the command surface does not
///   currently produce, is kept under a `value` key.)
/// * `Blob` keeps `revision` and the raw `bytes` array (GDSII/OASIS/PNG).
fn response_json(response: &AgentResponse) -> Value {
    match response {
        AgentResponse::Ok { revision, affected } => json!({
            "result": "ok",
            "revision": revision,
            "affected": affected,
        }),
        AgentResponse::Data { revision, value } => {
            let mut obj = serde_json::Map::new();
            obj.insert("result".to_owned(), json!("data"));
            obj.insert("revision".to_owned(), json!(revision));
            match value {
                Value::Object(fields) => {
                    for (k, v) in fields {
                        // Never let a payload field shadow the envelope keys.
                        if k != "result" && k != "revision" {
                            obj.insert(k.clone(), v.clone());
                        }
                    }
                }
                other => {
                    obj.insert("value".to_owned(), other.clone());
                }
            }
            Value::Object(obj)
        }
        AgentResponse::Blob { revision, bytes } => json!({
            "result": "blob",
            "revision": revision,
            "bytes": bytes,
        }),
        // `AgentResponse` is `#[non_exhaustive]`; a future variant falls back to
        // its natural tagged serialization rather than failing to compile.
        other => serde_json::to_value(other).unwrap_or_else(
            |e| json!({ "code": "engine_error", "message": format!("serialize response: {e}") }),
        ),
    }
}

/// The stable snake-case string for an [`ErrorCode`], matching its serde name.
fn error_code_str(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::NoSuchCell => "no_such_cell",
        ErrorCode::NoSuchElement => "no_such_element",
        ErrorCode::InvalidArgument => "invalid_argument",
        ErrorCode::NoSuchLayer => "no_such_layer",
        ErrorCode::EngineError => "engine_error",
        ErrorCode::BudgetExhausted => "budget_exhausted",
        // `ErrorCode` is `#[non_exhaustive]`; a future code maps to a generic
        // label rather than failing to compile.
        _ => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::{Budget, Server};
    use serde_json::{Value, json};

    /// Sends one request line and returns the parsed response.
    fn call(server: &mut Server, request: &Value) -> Value {
        server
            .handle_line(&request.to_string())
            .expect("request expects a response")
    }

    /// A `tools/call` request for `name` with `arguments`. Takes `arguments` by
    /// value so call sites can pass a `json!(...)` literal directly.
    #[allow(clippy::needless_pass_by_value)]
    fn call_tool(id: i64, name: &str, arguments: Value) -> Value {
        json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        })
    }

    /// Extracts the parsed `structuredContent` of a tool result.
    fn structured(response: &Value) -> &Value {
        &response["result"]["structuredContent"]
    }

    /// `initialize` reports the protocol version and server name.
    #[test]
    fn initialize_reports_server_info() {
        let mut s = Server::default();
        let r = call(
            &mut s,
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }),
        );
        assert_eq!(r["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(r["result"]["serverInfo"]["name"], "reticle-mcp");
    }

    /// `tools/list` advertises every command tool and the three context tools.
    #[test]
    fn tools_list_covers_catalog() {
        let mut s = Server::default();
        let r = call(
            &mut s,
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        );
        let listed = r["result"]["tools"].as_array().unwrap();
        assert_eq!(listed.len(), super::tools::all_tools().len());
        let names: Vec<&str> = listed.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"create_cell"));
        assert!(names.contains(&"get_technology_rules"));
        assert!(names.contains(&"get_render_region"));
        // Each tool has a description and an object input schema.
        for t in listed {
            assert!(t["description"].as_str().unwrap().len() > 10);
            assert_eq!(t["inputSchema"]["type"], "object");
        }
    }

    /// A notification (no id) yields no response.
    #[test]
    fn notification_has_no_response() {
        let mut s = Server::default();
        assert!(
            s.handle_line(&json!({ "jsonrpc": "2.0", "method": "initialized" }).to_string())
                .is_none()
        );
    }

    /// An unknown method is a JSON-RPC method-not-found error.
    #[test]
    fn unknown_method_errors() {
        let mut s = Server::default();
        let r = call(
            &mut s,
            &json!({ "jsonrpc": "2.0", "id": 9, "method": "no/such" }),
        );
        assert_eq!(r["error"]["code"], super::METHOD_NOT_FOUND);
    }

    /// A create/add sequence applies and reports the new element id and revision.
    #[test]
    fn command_tools_apply_and_report() {
        let mut s = Server::default();
        let created = call(
            &mut s,
            &call_tool(1, "create_cell", json!({ "name": "top" })),
        );
        assert_eq!(created["result"]["isError"], false);
        assert_eq!(structured(&created)["result"], "ok");

        let added = call(
            &mut s,
            &call_tool(
                2,
                "add_rect",
                json!({ "cell": "top", "layer": { "layer": 68, "datatype": 20 },
                        "rect": { "min": { "x": 0, "y": 0 }, "max": { "x": 100, "y": 100 } } }),
            ),
        );
        let payload = structured(&added);
        assert_eq!(payload["result"], "ok");
        assert_eq!(payload["revision"], 2);
        assert_eq!(payload["affected"], json!([1]));
    }

    /// An engine error (missing cell) becomes a tool error with the right code.
    #[test]
    fn missing_cell_is_tool_error() {
        let mut s = Server::default();
        let r = call(
            &mut s,
            &call_tool(
                1,
                "add_rect",
                json!({ "cell": "ghost", "layer": { "layer": 68, "datatype": 20 },
                    "rect": { "min": { "x": 0, "y": 0 }, "max": { "x": 1, "y": 1 } } }),
            ),
        );
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(structured(&r)["code"], "no_such_cell");
    }

    /// Bad arguments (wrong shape) are rejected before the session is touched.
    #[test]
    fn bad_arguments_are_rejected() {
        let mut s = Server::default();
        let r = call(&mut s, &call_tool(1, "add_rect", json!({ "cell": "top" })));
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(structured(&r)["code"], "invalid_argument");
    }

    /// The budget rejects further commands once exhausted, and read-only context
    /// tools keep working.
    #[test]
    fn budget_is_enforced() {
        let mut s = Server::new(Budget::new(1));
        // First command consumes the only unit.
        let first = call(&mut s, &call_tool(1, "create_cell", json!({ "name": "a" })));
        assert_eq!(first["result"]["isError"], false);
        assert_eq!(s.budget_remaining(), 0);
        // Second command is rejected with budget_exhausted; the session is intact.
        let second = call(&mut s, &call_tool(2, "create_cell", json!({ "name": "b" })));
        assert_eq!(second["result"]["isError"], true);
        assert_eq!(structured(&second)["code"], "budget_exhausted");
        // A read-only context tool still answers (no budget draw).
        let summary = call(&mut s, &call_tool(3, "get_document_summary", json!({})));
        assert_eq!(summary["result"]["isError"], false);
        assert_eq!(structured(&summary)["cell_count"], 1);
    }

    /// The `get_technology_rules` context tool returns the rule table. The rule
    /// syntax is `rule <kind> <layer> <datatype> <value>`; the parser derives the
    /// name `<kind>_<layer>_<datatype>`.
    #[test]
    fn context_technology_rules_tool() {
        let mut s = Server::default();
        let set = call(
            &mut s,
            &call_tool(
                1,
                "set_technology",
                json!({
                    "source": "technology demo\ndbu_per_micron 1000\nlayer 68 20 met1 3A6FD490\nrule width 68 20 140\n"
                }),
            ),
        );
        assert_eq!(set["result"]["isError"], false, "set_technology: {set}");
        let r = call(&mut s, &call_tool(2, "get_technology_rules", json!({})));
        assert_eq!(r["result"]["isError"], false);
        assert_eq!(structured(&r)["rules"][0]["name"], "width_68_20");
        assert_eq!(structured(&r)["rules"][0]["kind"], "width");
    }

    /// A generator tool (`via_farm`) applies through the server: the cell is split
    /// out of the arguments, the run maps to a `RunGenerator`, and the geometry lands.
    #[test]
    fn generator_tool_applies_and_reports() {
        let mut s = Server::default();
        let created = call(
            &mut s,
            &call_tool(1, "create_cell", json!({ "name": "top" })),
        );
        assert_eq!(created["result"]["isError"], false);

        // A default 3x3 mcon farm: 9 cuts plus two plates = 11 shapes.
        let farm = call(
            &mut s,
            &call_tool(
                2,
                "via_farm",
                json!({ "cell": "top", "cut": "mcon", "rows": 3, "cols": 3 }),
            ),
        );
        let payload = structured(&farm);
        assert_eq!(payload["result"], "ok", "via_farm applied: {farm}");
        assert_eq!(
            payload["affected"].as_array().unwrap().len(),
            11,
            "9 cuts plus two plates"
        );
    }

    /// A generator tool with out-of-range parameters is a tool error (the generator's
    /// own validation), not a panic, and the code is `invalid_argument`.
    #[test]
    fn generator_tool_rejects_bad_params() {
        let mut s = Server::default();
        call(
            &mut s,
            &call_tool(1, "create_cell", json!({ "name": "top" })),
        );
        let bad = call(
            &mut s,
            &call_tool(
                2,
                "via_farm",
                json!({ "cell": "top", "cut": "mcon", "rows": 0, "cols": 3 }),
            ),
        );
        assert_eq!(bad["result"]["isError"], true);
        assert_eq!(structured(&bad)["code"], "invalid_argument");
    }
}
