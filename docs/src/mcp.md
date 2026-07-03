# MCP server

`reticle-mcp` exposes the frozen agent command surface to a language model over the
[Model Context Protocol](https://modelcontextprotocol.io), so a model host can drive
Reticle with the same operations the [agent harness](agent.md) uses, without any
custom glue.

## Tools from the frozen surface

Every [`AgentCommand`](agent.md) variant becomes one MCP *tool* with a JSON input
schema and a model-facing description, generated from the frozen types rather than
hand-maintained, so the tool set cannot drift from what the engine actually accepts.
That is 25 command tools (create a cell, add a rectangle, run DRC, check intent,
export, render, and so on) plus three read-only *context* tools the model uses to
observe state before it acts:

- `get_technology_rules` the active technology's layers and DRC rules;
- `get_document_summary` the current cells, shape counts, and top cells;
- `get_render_region` a PNG of a region, so the model can look at what it has drawn.

## Transport

The server speaks newline-delimited JSON-RPC 2.0 on stdin and stdout, matching the
MCP stdio transport, and is hand-rolled over `serde_json` rather than pulling in an
MCP framework, keeping the dependency surface small and the behavior explicit (ADR
0005). A per-session command budget bounds how many mutating tools a session may
apply; once exhausted, further command tools are rejected, so a host cannot drive an
unbounded number of edits.

## Running it

The `reticle-mcp` binary is a stdio server: a model host launches it and speaks
JSON-RPC over the pipe. It is registered alongside the project's `reticle-dev`
development server in `.mcp.json`. An integration test drives all 28 tools over a
real stdio subprocess and asserts each one, so the wire contract is covered end to
end.

See ADR [0005](https://github.com/AlpharomeroJL/reticle/blob/main/docs/decisions/0005-rust-mcp-servers.md)
and the [agent chapter](agent.md) for the command surface these tools mirror.
