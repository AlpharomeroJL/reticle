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
| [0024](0024-demo-server-binary-crate.md) | The demo server binary lives in its own composition crate |
| [0025](0025-real-demo-harness-streams-steps.md) | The real demo harness streams atomic steps to the relay |
| [0026](0026-pages-opens-to-replay-theater.md) | The public Pages bundle opens to the replay theater |
| [0027](0027-e2e-webgl2-gate-webgpu-attempted.md) | Playwright e2e: WebGL2 hard gate, WebGPU attempted and skipped honestly headless |
| [0028](0028-v6-subagent-worktree-orchestration.md) | v6.0.0 run: subagent worktree lanes, a thin integration agent |
| [0029](0029-result-record-backend-label.md) | ResultRecord gains a backend and quantization label |
| [0030](0030-orchestrator-creates-lane-worktrees.md) | Lane worktrees are created by the orchestrator before spawning |
| [0031](0031-wave3-agent-command-expansion.md) | Wave 3 expands the AgentCommand surface for the Wave 2 tools |
| [0032](0032-transcript-plan-log.md) | Transcript gains an additive per-iteration plan log |
| [0033](0033-v7-housekeeping-media-and-changelog.md) | v7 housekeeping: prune offscreen media, regroup the changelog |
| [0034](0034-import-hardening-and-warnings.md) | GDSII import hardening: contain panics, degrade to structured warnings |
| [0035](0035-document-open-seam.md) | The document-open seam: bytes plus a format hint, warnings alongside |
| [0036](0036-browser-open-path-drop-url-indexeddb.md) | The browser open path: drop, `?gds=` URL, and an IndexedDB recent list |
| [0037](0037-browser-big-file-bands-and-measured-ceiling.md) | Browser big-file bands: an in-memory/streaming split and a measured ceiling |
| [0038](0038-read-only-viewer-sync.md) | Read-only session viewers: live sync, an independent camera, and follow-mode |
| [0039](0039-share-rooms-rate-limit-and-ttl.md) | Share rooms on the demo server: rate-limited creation and a TTL |
| [0040](0040-app-notification-error-surface.md) | One app-level notification surface every failure path reports through |
| [0041](0041-start-screen-first-contact.md) | Product-grade first contact: gallery, drag-drop, and a tour that covers open |
| [0042](0042-generator-trait-typed-and-erased.md) | Generator framework: a typed trait plus a type-erased registry path |
| [0043](0043-generators-drc-clean-by-construction.md) | Generators are DRC-clean by construction, proven by the real DRC engine |
| [0044](0044-pad-ring-generator-on-the-subset.md) | Pad-ring generator: die-aware I/O ring on the subset, power pads as via staples |
| [0045](0045-seal-ring-generator-on-the-subset.md) | Seal-ring generator: a stacked-metal-plus-cut barrier on the subset |
