# Changelog

All notable changes to Reticle are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com), and the project uses
[conventional commits](https://www.conventionalcommits.org).
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
