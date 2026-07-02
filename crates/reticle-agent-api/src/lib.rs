//! Serializable command API over the Reticle engine.
//!
//! This crate is the frozen Wave 0 contract for programmatic and agent-driven
//! editing. It provides a serde command and response vocabulary over the
//! existing engine crates (`reticle-model`, `reticle-io`, `reticle-drc`,
//! `reticle-route`, `reticle-extract`), addressed by stable [`ElementId`]s, plus
//! a structured [`AgentError`] so a command never panics. A session owns an
//! editable document and a monotonic revision.
//!
//! The command and response enums and the session are frozen here and
//! implemented in a later wave; this module establishes the identifier and error
//! contracts the rest of the surface builds on.

pub mod args;
mod command;
mod error;
mod ids;
mod response;
mod status;
mod transcript;

pub use command::AgentCommand;
pub use error::{AgentError, ErrorCode};
pub use ids::ElementId;
pub use response::{AgentResponse, Revision};
pub use status::{AGENT_ACTOR, AgentStatus};
pub use transcript::{CommandRecord, Outcome, Transcript};

// The connectivity intent types live in `reticle-extract`, next to the extraction
// the checker uses, and are re-exported here for callers of the command surface
// (ADR 0021). `reticle-agent-api` depends on `reticle-extract`, so this avoids a
// dependency cycle that placing them here would create.
pub use reticle_extract::{
    ForbiddenPair, IntentNet, IntentReport, IntentSpec, Open, Short, Terminal,
};

/// The result of applying a command: a response or a structured error.
pub type CommandResult = Result<AgentResponse, AgentError>;

#[cfg(test)]
mod tests {
    use super::args::{LayerArg, PointArg, RectArg};
    use super::{AgentCommand, AgentResponse, ElementId};

    /// Every command round-trips through JSON unchanged.
    #[test]
    fn command_json_round_trip() {
        let cmds = vec![
            AgentCommand::CreateCell { name: "top".into() },
            AgentCommand::AddRect {
                cell: "top".into(),
                layer: LayerArg {
                    layer: 68,
                    datatype: 20,
                },
                rect: RectArg {
                    min: PointArg { x: 0, y: 0 },
                    max: PointArg { x: 100, y: 100 },
                },
            },
            AgentCommand::AddPolygon {
                cell: "top".into(),
                layer: LayerArg {
                    layer: 67,
                    datatype: 20,
                },
                points: vec![
                    PointArg { x: 0, y: 0 },
                    PointArg { x: 10, y: 0 },
                    PointArg { x: 0, y: 10 },
                ],
            },
            AgentCommand::RunDrc {
                cell: "top".into(),
                region: None,
            },
            AgentCommand::ExportGds,
        ];
        for cmd in cmds {
            let json = serde_json::to_string(&cmd).expect("serialize");
            let back: AgentCommand = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(cmd, back, "command must round-trip: {json}");
        }
    }

    /// The `op` tag is present and in `snake_case`.
    #[test]
    fn command_tag_is_op() {
        let json = serde_json::to_string(&AgentCommand::ListLayers).expect("serialize");
        assert_eq!(json, r#"{"op":"list_layers"}"#);
    }

    /// Responses round-trip, including the revision and affected ids.
    #[test]
    fn response_json_round_trip() {
        let r = AgentResponse::Ok {
            revision: 7,
            affected: vec![ElementId(1), ElementId(2)],
        };
        let json = serde_json::to_string(&r).expect("serialize");
        let back: AgentResponse = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(r, back);
    }

    /// The transcript record, intent spec, and status all round-trip through JSON.
    #[test]
    fn frozen_types_round_trip() {
        use super::{
            AgentStatus, CommandRecord, IntentNet, IntentReport, IntentSpec, Outcome, Terminal,
            Transcript,
        };

        let record = CommandRecord {
            seq: 0,
            command: AgentCommand::ListLayers,
            revision_before: 3,
            revision_after: 3,
            outcome: Outcome::Ok(AgentResponse::Data {
                revision: 3,
                value: serde_json::json!({"layers": []}),
            }),
            ts_start_ms: 10,
            ts_end_ms: 12,
            tokens_in: Some(40),
            tokens_out: None,
        };
        let transcript = Transcript {
            records: vec![record],
            final_hash: 0xDEAD_BEEF,
        };
        let json = serde_json::to_string(&transcript).expect("serialize");
        let back: Transcript = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(transcript, back);

        let intent = IntentSpec {
            nets: vec![IntentNet {
                name: "vdd".into(),
                terminals: vec![Terminal {
                    name: "vdd".into(),
                    layer: reticle_geometry::LayerId::new(68, 20),
                    region: reticle_geometry::Rect::new(
                        reticle_geometry::Point::new(0, 0),
                        reticle_geometry::Point::new(10, 10),
                    ),
                }],
            }],
            forbidden: vec![],
        };
        let back: IntentSpec =
            serde_json::from_str(&serde_json::to_string(&intent).unwrap()).unwrap();
        assert_eq!(intent, back);
        assert!(IntentReport::default().is_satisfied());

        let status = AgentStatus {
            iteration: 2,
            step: "verifying".into(),
            violations: 1,
            running: true,
        };
        let back: AgentStatus =
            serde_json::from_str(&serde_json::to_string(&status).unwrap()).unwrap();
        assert_eq!(status, back);
    }
}
