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
