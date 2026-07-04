# 0049, MCP advertises each generator as its own tool

## Context

The MCP server ([`reticle-mcp`](../../crates/reticle-mcp)) advertises the frozen
`AgentCommand` surface to a model as one tool per command variant, plus three
read-only context tools. Wave 2D adds the `RunGenerator` command (ADR 0048), which
runs a generator by id with that generator's own parameter object. Exposing it as a
single generic `run_generator` tool would force the model to know each generator's id
and parameter shape from prose and hand-assemble an opaque `params` blob, which the
16k-window local models the benchmark targets do poorly.

A generator tool also differs from a command tool in a way the existing retag path
(`{op: name, ...arguments}` deserialized straight into an `AgentCommand`) cannot
express: the tool name is the generator id, not a command `op`, and the arguments are
the generator's parameters plus a target cell that is not one of those parameters.

## Decision

Advertise one MCP tool per built-in generator, named for the generator id
(`guard_ring`, `via_farm`, `pad_ring`, `seal_ring`, `fill`, `test_structure`). A new
[`generators`](../../crates/reticle-mcp/src/generators.rs) module iterates
`Registry::with_builtins().infos()` and, for each, converts the generator's
`ParamSchema` into a tight model-facing JSON Schema: an `Int` field becomes an
`integer` with inclusive `minimum`/`maximum` and a `default`, a `Bool` a `boolean`,
and an `Enum` a `string` constrained by `enum` to its variants, each with the field's
doc (and unit) in its description. The schema prepends a required `cell` string, and
every generator field is required, so a model is told the exact bounds and the target
up front.

A generator tool call maps to a `RunGenerator` command by a dedicated path
(`to_generator_command`): it splits the `cell` out of the arguments and folds the
remaining fields into the generator's `params` object. The server tries this path
first and falls through to the command-tool retag path for a name that is not a
generator id. The generator validates the parameters itself when the command is
applied, so a bad value returns an `invalid_argument` tool error rather than a schema
rejection. The generator tools sit between the command tools and the context tools in
the advertised catalog.

## Consequences

A model sees `via_farm` with typed `rows`/`cols` and a `cut` enum rather than a
generic blob, which is what the benchmark needs to pose generator tasks in natural
language. The subprocess integration test drives all six generator tools end to end
(and asserts the generated cell is DRC-clean and that an out-of-range parameter is a
well-formed tool error), so the tool-to-command mapping and the schema conversion are
covered against the real server. The conversion is hand-free: adding a seventh
generator to the registry adds its tool automatically, with no `reticle-mcp` change,
because the catalog is derived from `infos()`. The `cell`-splitting is the one place
the generator tools diverge from the one-to-one command tools, and it is contained in
the `generators` module.
