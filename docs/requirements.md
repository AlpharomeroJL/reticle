# Requirements mapping

Every subsystem exists to satisfy a concrete engineering requirement. This table is
kept honest as scope evolves; it is audited before each release.

| Requirement | Reticle feature | Status |
|---|---|---|
| Production-grade Rust, native and WebAssembly | One workspace, a `wgpu` renderer, a native app plus a WASM bundle and a browser demo. | Built |
| Deeply interactive, low-latency CAD/Figma-like editing | `egui` editing suite: canvas with pan/zoom, tools, command palette, layer manager, measurement, undo history. | Built |
| Efficient geometry querying, graphs, routing | R-tree and uniform-grid indices, hierarchical bbox culling, connectivity extraction, and a maze router with rip-up and reroute. | Built |
| High-performance rendering, spatial indexing, streaming millions of polygons | GPU-driven cell culling on a compute shader, a tile/LOD pyramid, `rkyv` out-of-core streaming, instanced draws; interactive on hierarchical designs with billions of effective leaf shapes. | Built |
| Continuous profiling and optimization | `criterion` benchmark suite with committed history, an offscreen frame-timing harness, `tracing`/`puffin` profiling, and `PERF.md`. | Built |
| Real-time collaboration: sync, conflict resolution | Hierarchical CRDT via `yrs`, a WebSocket relay (`axum`), presence and threaded comments, offline-then-reconcile. | Built |
| Schema evolution, migrations, Protobuf serialization | `prost` Protobuf schema with explicit `schema_version` and a migration path for the document and wire formats. | Built |
| WebGPU, Vulkan, OpenGL | `wgpu` targeting WebGPU and Vulkan/Metal/DX12 natively, with a WebGL2 fallback via the `eframe` glow backend for broad reach. | Built |
| CRDTs, operational transforms, WebSockets | `yrs` CRDT over WebSockets with awareness. | Built |
| GDSII, KiCad, CAD tooling | GDSII (`gds21`) and an in-house OASIS subset, a technology-file parser, and a DRC engine. | Built |
| Novel UI, 2D and 3D visualization | Layout canvas, minimap, DRC and net overlays, layer-stack views, and a 3D layer-stack view with a cut-line cross-section. | Built |
| Agent-driven editing with objective verification | A serializable command API (`reticle-agent-api`), a propose-verify-correct harness graded by the SKY130 DRC subset and connectivity intent (`reticle-agent`), an MCP server (`reticle-mcp`), a 63-task graded benchmark (`reticle-bench`), and a rate-limited demo server that runs the loop live (`reticle-demo-server`). | Built (mock-graded in this environment; the live model runs the same harness when a key is present) |

Notes on honest scope: the OASIS reader/writer is a focused subset (no mature Rust
crate exists); the interactive canvas renders with `egui` while the `wgpu` renderer
with compute culling serves the offscreen and batch paths; browser collaboration
reuses the native CRDT and relay. See `docs/decisions/` for the reasoning.
