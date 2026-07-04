//! Model Context Protocol server for Reticle.
//!
//! This crate exposes the frozen [`reticle_agent_api`] command surface to a
//! language model over the [Model Context Protocol]. Every
//! [`AgentCommand`](reticle_agent_api::AgentCommand) variant becomes an MCP
//! *tool* with a JSON input schema and a model-facing description, and three
//! read-only *context* tools sit alongside them
//! ([`get_technology_rules`](tools), [`get_document_summary`](tools), and
//! [`get_render_region`](tools)). One further family, the *generator* tools
//! ([`generators`]), advertises each built-in [`reticle_gen`] layout generator as
//! its own tool (`guard_ring`, `via_farm`, and the rest), schema'd from the
//! generator's parameters and mapped to a
//! [`RunGenerator`](reticle_agent_api::AgentCommand::RunGenerator) command.
//!
//! # Transport
//!
//! The server speaks newline-delimited JSON-RPC 2.0 on stdin/stdout, matching
//! the MCP stdio transport and the existing `reticle-dev` server. It is
//! hand-rolled over [`serde_json`] rather than pulling in an MCP framework, so
//! the dependency surface stays small and the wire format stays inspectable.
//! Drive it with [`Server::run`], or step it message-by-message with
//! [`Server::handle_line`] (used by the integration test).
//!
//! # Session model
//!
//! One [`Server`] owns exactly one [`Session`](reticle_agent_api::Session), so a
//! server process edits a single document. A [`Budget`] caps how many tool calls
//! a session may apply; once exhausted, further command tools are rejected with
//! an [`ErrorCode::BudgetExhausted`](reticle_agent_api::ErrorCode) payload rather
//! than mutating the document.
//!
//! # Units and conventions
//!
//! All coordinates are **database units** (DBU), the technology's integer
//! coordinate resolution (`dbu_per_micron` DBU to the micron). Layers are a
//! GDSII `(layer, datatype)` pair. These conventions are repeated in each tool
//! description so a model calling a single tool has them in context.
//!
//! [Model Context Protocol]: https://modelcontextprotocol.io

mod base64;
mod context;
pub mod generators;
mod schema;
mod server;
pub mod tools;

pub use server::{Budget, Server};
