# Architecture Decision Records

Short records of the non-obvious choices made during the build, in the format
context → decision → consequences. New records are appended, never rewritten; a
superseded decision is marked and linked to its replacement.

| # | Title |
|---|---|
| [0001](0001-keep-name-reticle.md) | Keep the working name "Reticle" |
| [0002](0002-integer-database-units.md) | Integer database units; `Dbu = i32`, widened math |
| [0003](0003-i-overlay-for-booleans.md) | `i_overlay` for polygon booleans and offsetting |
| [0004](0004-gds21-and-inhouse-oasis.md) | `gds21` for GDSII; in-house OASIS subset |
| [0005](0005-rust-mcp-servers.md) | MCP servers in Rust, outside the workspace |
| [0006](0006-contract-first-skeleton.md) | Contract-first compiling skeleton |
| [0007](0007-yrs-for-crdt.md) | `yrs` for the collaborative CRDT |
| [0008](0008-vendored-protoc.md) | `protoc-bin-vendored`, no system protoc |
| [0009](0009-webgpu-with-webgl2-fallback.md) | WebGPU primary with WebGL2 fallback |
| [0010](0010-profiling-stack.md) | `puffin` + `criterion` for profiling and benchmarks |
| [0011](0011-incremental-dependency-resolution.md) | Incremental per-wave dependency resolution |
| [0012](0012-rust-edition-2024.md) | Rust edition 2024 |
| [0013](0013-out-of-core-streaming-scope.md) | Out-of-core streaming: zero-copy primitive now, mmap paging deferred |
| [0014](0014-no-em-dash-voice-rule.md) | Voice rule: no em-dashes, enforced by check-style |
| [0015](0015-oasis-subset-extended.md) | OASIS subset extended to paths, instances, and arrays |
| [0016](0016-memmap2-out-of-core-streaming.md) | Out-of-core streaming via memmap2: the one unsafe block |
| [0017](0017-quick-xml-transitive-advisory.md) | Ignore quick-xml 0.39 advisories (RUSTSEC-2026-0194/0195): unreachable, upstream-pinned |
| [0018](0018-agent-api-layering-and-element-ids.md) | Agent API layering: serde-friendly types and session-owned element ids |
| [0019](0019-structured-violation.md) | Structured DRC violations, enriched in place |
| [0020](0020-product-crates-in-workspace.md) | Agent, MCP, benchmark, and demo crates live in the workspace |
| [0021](0021-intent-types-in-extract.md) | Intent types live in reticle-extract; serde on geometry value types |
| [0022](0022-agent-crdt-collaborator-bridge.md) | The agent as a live CRDT collaborator: a step-atomic bridge |
| [0023](0023-resume-authoritative-plan.md) | Resume orientation: docs/TASKS.md is the authoritative plan |
