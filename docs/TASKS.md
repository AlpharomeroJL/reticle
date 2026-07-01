# Reticle task checklist

Tracks the build against `docs/PLAN.md`. `[x]` done, `[~]` in progress, `[ ]` todo.
Checked off by the workstream that completes the item and audited at each wave merge.

## Wave 0 — contracts & scaffolding (serial)

- [x] Relocated cargo caches verified; toolchains, components, targets, and CLI tools installed.
- [x] Workspace, lints, profiles, dual licenses, `.gitignore`, `.gitattributes`, `rust-toolchain.toml`.
- [x] `justfile` gate (`just ci`), `deny.toml`, `typos.toml`, `rustfmt.toml`, `.mcp.json`.
- [x] `docs/PLAN.md`, `docs/TASKS.md`, ADRs 0001–0012.
- [x] Protobuf schema `proto/reticle.proto` (geometry, hierarchy, CRDT/presence/comment, versioning).
- [x] Shared Rust contracts in `reticle-geometry` and `reticle-model`.
- [x] Compiling skeleton for all workspace members; `just ci` green baseline.
- [x] Skills authored; MCP servers under `tools/`.
- [x] `git init`; first commit as Josef Long, no AI trailer.

## Wave 1 — foundations (parallel) — DONE

- [x] `reticle-geometry`: integer primitives, `i_overlay` booleans/offset, winding; property test vs winding oracle; criterion bench.
- [x] `reticle-proto`: prost generation from the schema via vendored protoc; encode/decode; round-trip tests.
- [x] `reticle-index`: `rstar` R-tree, uniform grid, tile/LOD pyramid, `rkyv` streaming; property tests vs linear oracle; bench (build ~239 ms/1M, nearest ~970 ns — provisional, re-measured in the perf pass).
- [x] `reticle-io`: GDSII (`gds21`) + OASIS subset + technology parser; round-trip, corpus, and robustness proptests.

## Wave 2 — core subsystems (parallel) — DONE

- [x] `reticle-model`: cells/instances/arrays, transactional undo/redo, flatten, recursive bbox; unit + undo/redo property tests.
- [x] `reticle-render`: wgpu offscreen renderer (instanced rects, lyon tessellation, palette), compute-shader cell culling; golden-image tests; wasm build.
- [x] `reticle-drc`: all eight rule kinds, R-tree acceleration, incremental `check_region`; property tests vs naive checker.
- [x] `reticle-route`: grid + A* maze, multi-layer vias, rip-up/reroute, congestion; Manhattan-optimality property test.
- [x] `reticle-extract`: union-find connectivity, cross-layer vias, netlist compare; property test vs naive oracle.

## Wave 3 — collaboration, server, scripting, CLI (parallel) — IN PROGRESS

- [~] `reticle-sync`: `yrs` CRDT over the model, presence, comments, offline reconcile; convergence tests.
- [~] `reticle-server`: `axum` + `tokio` WebSocket relay, rooms, broadcast, initial-state, persistence hook.
- [~] `reticle-script`: `rhai` API (create/query/transform/DRC/export), plugin folder, example scripts.
- [~] `reticle-cli`: headless import/DRC/route/extract/export/render pipeline with clap.

## Wave 4 — application, web, xtask (parallel)

- [~] `xtask`: deterministic layout generator (done); media capture (Wave 5).
- [ ] `reticle-app`: egui UI, tools, command palette, layer manager, measurement, session/autosave, undo panel; native + WASM.
- [~] `web`: Trunk harness, `index.html`, WebGPU capability check + WebGL2 fallback (harness done; app mount pending).

## Wave 5 — docs, fuzz, benches, media, release

- [x] mdbook book scaffolded (overview, architecture, per-subsystem chapters) with mermaid.
- [~] Fuzz targets authored (GDSII/OASIS parsers, geometry booleans); seed corpora + runs pending.
- [ ] Benchmark history committed; `PERF.md` with measured numbers.
- [ ] `assets/hero.png` + `browse/drc/route/collab` GIFs; README finalized.
- [ ] Repo `AlpharomeroJL/reticle` created; `main` pushed.
- [ ] Book + WASM demo deployed to `gh-pages`; Pages enabled.
- [ ] `CHANGELOG.md` via `git-cliff`; tag `v3.0.0`; `gh release create`.
- [ ] Requirements-mapping table current; Section 16 self-audit passed.
