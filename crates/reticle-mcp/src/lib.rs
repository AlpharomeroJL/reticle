//! Model Context Protocol server for Reticle.
//!
//! Exposes every `reticle-agent-api` command as an MCP tool with a JSON schema
//! and a model-facing description (units, database-unit conventions, error
//! semantics), plus read-only context tools. One document per session with a
//! configurable command budget. Frozen Wave 0 skeleton; the tool implementations
//! and the stdio transport land in a later wave.
