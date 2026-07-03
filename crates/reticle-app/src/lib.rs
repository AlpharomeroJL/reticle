//! The interactive Reticle application.
//!
//! This crate is the `egui`/`eframe` editor for large hierarchical 2D IC-layout
//! scenes. It runs both natively (via [`eframe::run_native`] in the binary) and in
//! the browser (WASM), where the separate `web` crate mounts [`App`].
//!
//! # Architecture
//!
//! The app is split into small, window-free logic modules plus a thin egui glue
//! layer, so the interesting behavior is unit-tested without a GPU or a window:
//!
//! * [`camera`], the world<->screen pan/zoom transform (zoom-to-cursor, fit).
//! * [`culling`], viewport culling over a spatial index, plus level-of-detail.
//! * [`draw`], the drawing tools and vertex-level editing geometry.
//! * [`tool`], the Select/Pan/Measure tool state machine.
//! * [`measure`], distance measurement in DBU and microns.
//! * [`minimap`], the overview panel's world-to-panel mapping and viewport rect.
//! * [`layers`], the layer table, visibility, and name filter.
//! * [`selection`], the shape-selection model and layer query.
//! * [`grid`], grid spacing, snapping, and ruler ticks.
//! * [`labels`], layout and formatting for the canvas text overlay (cell names,
//!   selection captions, live dimensions).
//! * [`history`], the [`reticle_model::EditableDocument`] undo/redo wrapper.
//! * [`command`], the command-palette catalog and fuzzy filter.
//! * [`keymap`], rebindable keyboard shortcuts: TOML load/save and conflicts.
//! * [`drc_panel`], running the DRC engine and formatting its violations.
//! * [`agent_panel`], the agent panel's run state machine and narration feed over
//!   the `reticle-agent-api` transcript types.
//! * [`replay`], the replay theater: transcript JSONL loading and the
//!   step/play/pause/speed playback machine over a live session.
//! * [`store`], the transcript storage seam: filesystem on native, a bundled
//!   transcript on wasm, so the theater opens on both.
//! * [`netlight`], cached connectivity extraction for net highlighting.
//! * [`inspector`], the read-only properties summary of the selection.
//! * [`fps`], the rolling frame-time meter behind the status-bar fps readout.
//! * [`session`], view/UI session save/restore (native file IO).
//! * [`share`], the share-this-session relay room-link format.
//! * [`viewports`], the multi-pane split layout, hit-testing, and camera swaps.
//! * [`view3d`], the extruded 3D layer-stack window (orbit camera + wgpu glue).
//! * [`xsection`], cut-line cross-sections (interval math + elevation panel).
//! * [`demo`], the built-in hierarchical demo document.
//! * [`app`], the [`eframe::App`] implementation that draws it all.
//!
//! The public [`App`] type is the frozen Wave 0 contract; it is now a real
//! `eframe::App` built on the modules above.

// The agent panel and replay theater are model-free and now build on wasm too.
// `reticle-agent-api` is wasm-buildable (its render command degrades to a clean
// error on wasm), and the theater sources its transcript through `store` (the
// filesystem on native, a bundled transcript on wasm), so the public web bundle
// can open straight into a playing theater (ADR 0026).
pub mod agent_panel;
pub mod app;
pub mod camera;
pub mod command;
pub mod culling;
pub mod demo;
pub mod draw;
pub mod drc_panel;
pub mod fps;
pub mod grid;
pub mod history;
pub mod inspector;
pub mod keymap;
pub mod labels;
pub mod layers;
pub mod measure;
pub mod minimap;
pub mod netlight;
pub mod ops;
pub mod replay;
pub mod selection;
pub mod session;
pub mod share;
pub mod store;
pub mod tool;
pub mod view3d;
pub mod viewports;
pub mod xsection;

pub use app::{App, StartView};
