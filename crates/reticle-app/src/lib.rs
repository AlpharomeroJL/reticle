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
//! * [`tool`], the Select/Pan/Measure tool state machine.
//! * [`measure`], distance measurement in DBU and microns.
//! * [`layers`], the layer table, visibility, and name filter.
//! * [`selection`], the shape-selection model and layer query.
//! * [`grid`], grid spacing, snapping, and ruler ticks.
//! * [`history`], the [`reticle_model::EditableDocument`] undo/redo wrapper.
//! * [`command`], the command-palette catalog and fuzzy filter.
//! * [`drc_panel`], running the DRC engine and formatting its violations.
//! * [`netlight`], cached connectivity extraction for net highlighting.
//! * [`inspector`], the read-only properties summary of the selection.
//! * [`fps`], the rolling frame-time meter behind the status-bar fps readout.
//! * [`session`], view/UI session save/restore (native file IO).
//! * [`demo`], the built-in hierarchical demo document.
//! * [`app`], the [`eframe::App`] implementation that draws it all.
//!
//! The public [`App`] type is the frozen Wave 0 contract; it is now a real
//! `eframe::App` built on the modules above.

pub mod app;
pub mod camera;
pub mod command;
pub mod culling;
pub mod demo;
pub mod drc_panel;
pub mod fps;
pub mod grid;
pub mod history;
pub mod inspector;
pub mod layers;
pub mod measure;
pub mod netlight;
pub mod selection;
pub mod session;
pub mod tool;

pub use app::App;
