# Changelog

All notable changes to Reticle are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com), and the project uses
[conventional commits](https://www.conventionalcommits.org).
## [4.0.0] - 2026-07-02

### Performance

- Measure WASM cold load and collaboration echo; record real numbers

### Documentation

- Final task record and Section 16 self-audit
- Honest status audit; correct overstated claims
- Correct the same overstatements in the book chapters
- OASIS/streaming ADRs and the v4.0.0 measured numbers
- Record honest v4.0.0 progress in STATUS.md
- Mark lanes R1 and R4 in progress
- Record R1 and R4 checkpoint resume state
- Mark lane R4 done, parked for merge slot
- Mark lane R1 merged and done
- Mark lane R3 done, parked to merge last
- Mark lanes R2 and R4 merged
- Mark lane R3 merged, all four Wave R lanes integrated
- Refresh README and STATUS for the v4.0.0 rendering, UI, 3D, and measurement work
- Close-out measurements and doc refresh done, media in progress
- Add a v4.0.0 gallery of the captured engine media
- Mark Wave R media captured and merged

### Build and tooling

- Dev profile tuning, fail-fast ci order, v4/v5 run tracker
- Ignore quick-xml companion advisory RUSTSEC-2026-0194, allowlist iy identifier

### Chores

- Lock criterion dev-deps for the new model and drc benches

### Other

- Implement perf-check as a real regression gate
- Check the whole design by flattening the top cell
- Add convex decomposition by ear clipping
- Forbid em-dashes (voice rule), sweep the tree, add check-style gate
- Memoize cell_bbox with an edit-invalidated cache
- Extend the OASIS subset to paths, instances, and arrays
- Add an incremental re-check latency benchmark
- Make incremental re-check genuinely sublinear via a prepared context
- Memory-mapped out-of-core streaming with one unsafe block
- Add a headless fps benchmark; record 1M/10M offscreen fps
- DRC panel, net highlighting, and a properties inspector
- Ignore RUSTSEC-2026-0195 (quick-xml), unreachable upstream-pinned transitive advisory
- Add monotonic document revision counter
- Retained per-cell scene cache with instance expansion
- Chunked GPU buffer pages with a free-list allocator
- Windowed surface via egui-wgpu paint callback
- Status-bar fps and frame-time readout
- Multi-page retained rects, bench + PERF.md re-measure
- Fix intra-doc link to eframe::egui_wgpu::Callback
- Lane R1 windowed GPU surface and retained scene (10M at ~113 fps)
- 4x MSAA offscreen path with resolve and tolerance golden
- GPU stream compaction with exclusive scan and indirect args
- Indirect draw from compacted buffer with multi-draw and downlevel gates
- Per-chunk LOD selection reusing lod_for_zoom thresholds
- Flags-vs-compacted cull comparison in fps_bench; record numbers in PERF.md
- Lane R2 GPU-driven draw list (compaction, indirect, MSAA, LOD)
- Add optional physical stack directive to the technology format
- Extruded 3D layer-stack pipeline with orbit camera
- 3D stack window with orbit input via egui-wgpu callback
- Cut-line cross-section panel with a two-click cut tool
- Lane R4 3D layer-stack view and cut-line cross-section
- Add stack field to Technology literal exposed by the R4 merge
- Canvas text-label overlay for cell names and live dimensions
- Minimap overview panel with click-to-recenter navigation
- Multi-viewport split with per-pane cameras over the shared document
- Rebindable keyboard shortcuts with a TOML keymap and editor window
- Correct the crate doc to match where the bench targets live
- Unlink two private consts from public rustdoc
- Lane R3 UI (text labels, minimap, split viewports, keybindings)
- Render the 3D layer stack to assets/stack3d.png
- Render DRC violation markers to assets/drc.png
- Render the minimap overview still to assets/minimap.png
- Render maze-routed nets to assets/route.png
- Render two-user presence to assets/collab.png
- Sort routed shapes so route.png is byte-stable
- Capture DRC, route, collab, minimap, and 3D media
## [3.0.0] - 2026-07-01

### Features

- Robust polygon booleans and offsetting via i_overlay
- Generate Rust types from the schema with prost
- R-tree, uniform grid, LOD pyramid, and rkyv streaming
- GDSII and OASIS import/export and technology parsing
- Transactional editing, flattening, and recursive bboxes
- Offscreen wgpu renderer with GPU-driven culling
- Declarative, incremental design-rule checker
- Grid and maze router with rip-up and reroute
- Connectivity extraction and netlist compare
- Deterministic layout generator; add fuzz harness
- Yrs CRDT collaboration with presence and comments
- Axum WebSocket collaboration relay
- Rhai scripting API over the model
- Headless import, DRC, route, extract, export, render pipeline
- Trunk harness with WebGPU capability check and WebGL2 fallback
- Interactive egui editor, native and WASM
- Offscreen media capture for the hero image and browse GIF
- Mount the egui app in the browser via eframe

### Documentation

- Add the mdbook book, changelog config, and gate exclusions
- Hero media in the README, requirements table, and changelog
- Record measured performance results
- Document targets, corpora, and the Windows sanitizer caveat

### Chores

- Scaffold workspace, cross-crate contracts, and local CI gate
- V3.0.0
- Skip pre-commit lint when no justfile; add live-demo link
