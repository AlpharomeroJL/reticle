//! Server-side transcript capture.
//!
//! Proves that a [`reticle_mcp::Server`] built with [`reticle_mcp::Server::with_transcript`]
//! streams every command it applies, whatever the client, as a session-transcript
//! JSONL (one [`reticle_agent_api::CommandRecord`] per line), that failures are
//! captured with a failure outcome, and that the captured transcript is
//! replay-verifiable (it reproduces the same document a direct application does). This
//! is what lets a client the harness does not control (a raw MCP client, for example
//! Claude Code driving the server) leave a mineable, replayable transcript.

use std::sync::{Arc, Mutex};

use reticle_agent_api::{AgentCommand, CommandRecord, Outcome, Session, Transcript, replay};
use reticle_mcp::{Budget, Server};
use serde_json::{Value, json};

/// A `Write` sink that appends to a shared buffer, so an in-process test can read
/// back exactly what the server streamed.
#[derive(Clone)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("sink mutex").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// One `tools/call` JSON-RPC request line for `name` with `arguments`.
fn call_line(id: u64, name: &str, arguments: &Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments.clone() },
    })
    .to_string()
}

/// The serde `op` tag of a command, read from its JSON form.
fn command_op(command: &AgentCommand) -> String {
    serde_json::to_value(command)
        .ok()
        .and_then(|v| v.get("op").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default()
}

#[test]
fn server_captures_every_command_including_failures_and_replays() {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let mut server =
        Server::with_transcript(Budget::default(), Box::new(SharedBuf(buffer.clone())));

    // Two guaranteed successes then two guaranteed failures: a delete of a missing
    // cell (rejected by require_cell) and a create with an empty name. All four use
    // create_cell/delete_cell, which need no technology, so this is self-contained.
    let _ = server.handle_line(&call_line(1, "create_cell", &json!({ "name": "a" })));
    let _ = server.handle_line(&call_line(2, "create_cell", &json!({ "name": "b" })));
    let _ = server.handle_line(&call_line(3, "delete_cell", &json!({ "name": "ghost" })));
    let _ = server.handle_line(&call_line(4, "create_cell", &json!({ "name": "" })));

    // Read back exactly what the server streamed: one CommandRecord JSON per line.
    let text = String::from_utf8(buffer.lock().expect("sink mutex").clone()).expect("utf8");
    let records: Vec<CommandRecord> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("each line is a CommandRecord"))
        .collect();

    // Every applied command is captured in order, with the right ops and outcomes,
    // including the two failures.
    assert_eq!(records.len(), 4, "one record per applied command");
    let ops: Vec<String> = records.iter().map(|r| command_op(&r.command)).collect();
    assert_eq!(
        ops,
        ["create_cell", "create_cell", "delete_cell", "create_cell"]
    );
    assert!(matches!(records[0].outcome, Outcome::Ok(_)));
    assert!(matches!(records[1].outcome, Outcome::Ok(_)));
    assert!(
        matches!(records[2].outcome, Outcome::Err(_)),
        "the missing-cell delete is captured as a failure"
    );
    assert!(
        matches!(records[3].outcome, Outcome::Err(_)),
        "the empty-name create is captured as a failure"
    );
    // The sequence numbers are the session positions, in order.
    assert_eq!(
        records.iter().map(|r| r.seq).collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );

    // The captured transcript is replay-verifiable: replaying reproduces a stable
    // document hash, and it matches applying the same commands directly to a fresh
    // session, so the server captured the run faithfully.
    let captured = Transcript {
        records: records.clone(),
        final_hash: 0,
        plan: Vec::new(),
    };
    let captured_hash = replay(&captured).expect("captured transcript replays");
    assert_eq!(
        captured_hash,
        replay(&captured).expect("replay is deterministic"),
        "replay of the same transcript is deterministic"
    );

    let mut direct = Session::new();
    let _ = direct.apply(AgentCommand::CreateCell {
        name: "a".to_owned(),
    });
    let _ = direct.apply(AgentCommand::CreateCell {
        name: "b".to_owned(),
    });
    let _ = direct.apply(AgentCommand::DeleteCell {
        name: "ghost".to_owned(),
    });
    let _ = direct.apply(AgentCommand::CreateCell {
        name: String::new(),
    });
    let direct_transcript = Transcript {
        records: direct.transcript().to_vec(),
        final_hash: 0,
        plan: Vec::new(),
    };
    assert_eq!(
        captured_hash,
        replay(&direct_transcript).expect("direct transcript replays"),
        "the server-captured transcript replays to the same document as direct application"
    );
}

#[test]
fn capture_is_off_by_default() {
    // Server::new is the no-capture path; driving a command must work and record
    // nothing to any sink. The Debug summary reports the capture state.
    let mut server = Server::new(Budget::default());
    let response = server.handle_line(&call_line(1, "create_cell", &json!({ "name": "a" })));
    assert!(
        response.is_some(),
        "the command still runs with capture off"
    );
    assert!(
        format!("{server:?}").contains("capturing: false"),
        "Server::new does not capture"
    );
}
