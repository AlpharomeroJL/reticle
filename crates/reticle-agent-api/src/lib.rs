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

pub use command::AgentCommand;
pub use error::{AgentError, ErrorCode};
pub use ids::ElementId;
pub use response::{AgentResponse, Revision};

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
}
