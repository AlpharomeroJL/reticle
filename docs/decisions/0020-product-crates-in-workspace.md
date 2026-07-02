# 0020, Agent, MCP, benchmark, and demo crates live in the workspace

## Context

ADR 0005 put the developer-tool MCP servers (`tools/reticle-dev-mcp`,
`tools/crate-docs-mcp`) outside the Cargo workspace, so they never slow the `just ci`
gate. The v5 run adds five new crates: `reticle-agent-api`, `reticle-mcp`,
`reticle-agent`, `reticle-bench`, and `reticle-demo`. One of them, `reticle-mcp`, is
also an MCP server, which raised the question of whether it too belongs outside the
workspace by the same reasoning as ADR 0005.

## Decision

All five crates are workspace members. The distinction is product versus tooling, not
protocol. `tools/reticle-dev-mcp` wraps `just` recipes to automate the developer's own
build; it has nothing to do with the shipped engine, so keeping it out of the gate costs
nothing. `reticle-mcp` is a product deliverable: it exposes the engine's command API as
MCP tools for end users and is a thin shim over `reticle-agent-api`, so it must be built,
linted, and tested by the same gate as the rest of the engine. The same holds for the
agent harness, the benchmark infrastructure, and the demo server.

## Consequences

- `just ci` builds and tests the whole v5 surface, so a change to the frozen command
  types that breaks a downstream product crate fails the gate immediately.
- The workspace grows from 17 to 22 members. Build time rises modestly; the shared
  dependency graph (serde, and the engine crates these all reuse) keeps it bounded, and
  the dev-profile dependency optimization already in place absorbs most of it.
- The exclusion list in the root manifest still names only the two developer-tool MCP
  servers and the fuzz crate, so the product-versus-tooling line stays legible.
