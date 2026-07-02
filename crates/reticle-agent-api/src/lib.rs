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

mod error;
mod ids;

pub use error::{AgentError, ErrorCode};
pub use ids::ElementId;
