# Reticle task checklist

Tracks the build against `docs/PLAN.md`. `[x]` done, `[~]` in progress, `[ ]` todo.
Checked off by the workstream that completes the item and audited at each wave merge.

## Wave 0 — contracts & scaffolding (serial)

- [~] Relocated cargo caches verified (`CARGO_HOME`, `CARGO_TARGET_DIR`); PATH has `D:\dev\cargo\bin`.
- [~] Toolchains: stable + nightly; components `rust-src`, `llvm-tools-preview`, `miri`; target `wasm32-unknown-unknown`.
- [~] CLI tools installed (`just`, `cargo-nextest`, `cargo-deny`, `typos-cli`, `cargo-machete`, `cargo-fuzz`, `cargo-criterion`, `cargo-llvm-cov`, `git-cliff`, `mdbook`, `mdbook-mermaid`, `trunk`, `wasm-bindgen-cli`, `wasm-opt`, `gifski`).
- [x] Workspace `Cargo.toml`, lints, profiles, dual licenses, `.gitignore`, `rust-toolchain.toml`.
- [x] `justfile` gate (`just ci`), `deny.toml`, `typos.toml`, `rustfmt.toml`, `.mcp.json`.
- [x] `docs/PLAN.md`, `docs/TASKS.md`.
- [ ] ADRs 0001–0012 under `docs/decisions/`.
- [ ] Protobuf schema `reticle-proto/proto/reticle.proto` (geometry, hierarchy, CRDT/presence/comment, schema_version + migration hook).
- [ ] Shared Rust contracts: `reticle-geometry` primitives + `Shape`/`SpatialIndex` traits; `reticle-model` `Cell`/`Instance`/`DocumentStore`/`RuleSet`/`Router`/`Importer`/`Exporter`/`Renderer` traits.
- [ ] Compiling skeleton for all 17 workspace members.
- [ ] Skills authored under `.claude/skills/`.
- [ ] MCP servers under `tools/` (`reticle-dev`, `crate-docs`).
- [ ] `git init`; first commit (compiling skeleton) as Josef Long, no AI trailer.
- [ ] `cargo fetch` + `cargo build --workspace` green; `just ci` green.

## Wave 1 — foundations (parallel)

### reticle-geometry
- [ ] Point/Rect/Polygon/Path/Transform/Orientation on integer DBU.
- [ ] Boolean ops (union/intersection/difference/xor) + offset/sizing via `i_overlay`.
- [ ] Winding/orientation, convex decomposition, clipping.
- [ ] proptest vs brute-force oracle; fuzz targets; criterion benches; rustdoc; book chapter.

### reticle-proto
- [ ] prost-build + `protoc-bin-vendored` generation from `.proto`.
- [ ] Versioning + migration path; conversions to/from model types.
- [ ] Round-trip tests; rustdoc; book chapter.

### reticle-index
- [ ] `rstar` R-tree (bulk load), uniform grid, tile/LOD pyramid.
- [ ] point / rect / nearest-edge / k-NN queries.
- [ ] `rkyv` zero-copy streaming for out-of-core.
- [ ] proptest vs linear-scan oracle; benches (build < 500 ms/1M, pick < 1 ms/1M); rustdoc; book chapter.

### reticle-io
- [ ] GDSII read/write via `gds21`; technology-file parser.
- [ ] OASIS reader/writer (in-house subset); VLSIR import/export.
- [ ] Round-trip fidelity corpus; parser fuzz targets; rustdoc; book chapter.

## Wave 2 — core subsystems (parallel)

### reticle-model
- [ ] Cells, instances, arrays; nested transforms; per-cell bbox cache; flatten/unflatten.
- [ ] Transactional edit history (undo/redo); CRDT-friendly layout.
- [ ] proptest (edit/undo invariants); benches; rustdoc; book chapter.

### reticle-render
- [ ] wgpu instanced polygon/path pipelines (`lyon` tessellation, `glam`/`bytemuck`).
- [ ] Compute-shader GPU-driven culling; tile + LOD; AA edges.
- [ ] `glyphon` glyph atlas; layer styling/themes; minimap; DRC/net overlays; 3D cross-section.
- [ ] Offscreen render harness; golden-image tests; benches (1M @ 60 fps); rustdoc; book chapter.

### reticle-drc
- [ ] Declarative rules: width/spacing/enclosure/extension/notch/area/density/angle.
- [ ] Incremental re-check (< 100 ms local edit); zoom-to marker model.
- [ ] proptest vs naive checker; benches; rustdoc; book chapter.

### reticle-route
- [ ] Grid + maze router (Lee / A* via `pathfinding`), multi-net, rip-up & reroute.
- [ ] Obstacle avoidance from geometry + DRC spacing; cross-layer vias.
- [ ] Congestion + length reporting; tests; benches; rustdoc; book chapter.

### reticle-extract
- [ ] Per-net connectivity across contacts/vias; net highlighting; netlist compare.
- [ ] proptest vs union-find oracle; tests; rustdoc; book chapter.

## Wave 3 — collaboration, server, scripting, CLI (parallel)

### reticle-sync
- [ ] `yrs` document over the model; update encode/decode.
- [ ] Presence (cursor/selection/viewport) + threaded comments; offline reconcile.
- [ ] CRDT convergence tests (order-independent); rustdoc; book chapter.

### reticle-server
- [ ] `axum` + `tokio` relay; rooms; awareness; broadcast; initial-state sync; persistence hook.
- [ ] Integration tests; rustdoc; book chapter.

### reticle-script
- [ ] `rhai` API: create/query/transform/DRC/route/export; plugin folder; examples.
- [ ] Tests; rustdoc; book chapter.

### reticle-cli
- [ ] Headless pipeline: import → DRC → route → extract → export → render-to-image.
- [ ] Integration tests on corpus; rustdoc; book chapter.

## Wave 4 — application, web, xtask (parallel)

### reticle-app
- [ ] `egui` UI; tool state machine; command palette; rebindable keys; config.
- [ ] Multi-viewport; layer manager; selection filters + query bar; rulers/grid/snap/guides.
- [ ] Measurement suite; session save/restore; autosave; crash recovery; undo-history panel.
- [ ] Native + WASM; tests; rustdoc; book chapter.

### web
- [ ] Trunk harness, `index.html`, `Trunk.toml`; WebGPU capability check + WebGL2 fallback.
- [ ] Cold-load-to-interactive < 3 s measurement.

### xtask
- [ ] Deterministic parameterized layout generator (shapes × layers × hierarchy depth).
- [ ] Offscreen media-capture command; `perf-check`.

## Wave 5 — docs, fuzz, benches, media, release (parallel)

- [ ] mdbook book complete (overview, architecture, per-subsystem, format specs, scripting API, PERF methodology, user + contributor guides) with mermaid diagrams.
- [ ] Fuzz seed corpora committed; long fuzz smoke runs recorded.
- [ ] Benchmark history committed; `PERF.md` with measured numbers.
- [ ] `assets/hero.png` + `browse/drc/route/collab` GIFs; README complete.
- [ ] Repo `AlpharomeroJL/reticle` created; `main` pushed.
- [ ] Book + WASM demo deployed to `gh-pages`; Pages enabled.
- [ ] `CHANGELOG.md` via `git-cliff`; tag `v3.0.0`; `gh release create` with binaries + notes.
- [ ] Requirements-mapping table current; Section 16 self-audit passed.
