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
//! * [`snap`], geometry-aware snapping (vertices, edges, midpoints, centers),
//!   draggable ruler guides, and the snap-settings state.
//! * [`labels`], layout and formatting for the canvas text overlay (cell names,
//!   selection captions, live dimensions).
//! * [`history`], the [`reticle_model::EditableDocument`] undo/redo wrapper.
//! * [`command`], the command-palette catalog and fuzzy filter.
//! * [`keymap`], rebindable keyboard shortcuts: TOML load/save and conflicts.
//! * [`drc_panel`], running the DRC engine and formatting its violations.
//! * [`agent_panel`], the agent panel's run state machine, conversation-mode
//!   transcript, and narration feed over the `reticle-agent-api` transcript types.
//! * [`agent_history`], the session history browser: enumerating past run
//!   transcripts and loading one into the replay theater.
//! * [`replay`], the replay theater: transcript JSONL loading and the
//!   step/play/pause/speed playback machine over a live session.
//! * [`store`], the transcript storage seam: filesystem on native, a bundled
//!   transcript on wasm, so the theater opens on both.
//! * [`tech_editor`], the upgraded layer manager (reorder, recolor, fill style,
//!   solo) and the technology editor that validates and round-trips the tech file.
//! * [`netlight`], cached connectivity extraction for net highlighting.
//! * [`open`], the document-open seam: bytes plus a format hint to an opened
//!   document with structured, non-fatal warnings (native and wasm; the contract
//!   other file-open entry points route through).
//! * [`webopen`], the browser open path over that seam: drag-and-drop and `?gds=`
//!   remote-URL loading, the IndexedDB-persisted recent-files model, the big-file
//!   size-band decision and its measured ceiling, and the progressive-load progress
//!   state machine. Pure logic is unit-tested; the fetch/IndexedDB glue is wasm-only.
//! * [`productivity`], clipboard/duplicate/array/move-delta/via-stack editing logic
//!   behind the productivity side panel, every edit undo-integrated.
//! * [`inspector`], the read-only properties summary of the selection.
//! * [`fps`], the rolling frame-time meter behind the status-bar fps readout.
//! * [`session`], view/UI session save/restore (native file IO).
//! * [`tour`], the pure, egui-free first-run tour state machine (ordered steps,
//!   next/skip/finish, first-run-once, and the optional Wave 2 chapter).
//! * [`share`], the share-this-session relay room-link format.
//! * [`viewports`], the multi-pane split layout, hit-testing, and camera swaps.
//! * [`view3d`], the extruded 3D layer-stack window (orbit camera + wgpu glue).
//! * [`xsection`], cut-line cross-sections (interval math + elevation panel).
//! * [`demo`], the built-in hierarchical demo document.
//! * [`demoscript`], the scripted demo-capture mode that drives the real window and
//!   screenshots it, so the README media shows the actual editor (native only).
//! * [`usecases`], the four bundled worked use-case scenarios offered from the
//!   Start screen (inspect a SKY130 cell, find and fix a violation, watch the
//!   agent, build with the new tools), each preparing a starting document or the
//!   replay theater.
//! * [`startscreen`], the Start-screen model: the example-chip gallery of
//!   redistribution-cleared designs (compiled in, opened through the seam) and the
//!   recent-files display shape.
//! * [`notify`], the app-wide notification (toast) queue: the single human-readable
//!   surface every failure path reports through (pure, severity-tagged, bounded).
//! * [`app`], the [`eframe::App`] implementation that draws it all.
//!
//! The public [`App`] type is the frozen Wave 0 contract; it is now a real
//! `eframe::App` built on the modules above.

// The agent panel and replay theater are model-free and now build on wasm too.
// `reticle-agent-api` is wasm-buildable (its render command degrades to a clean
// error on wasm), and the theater sources its transcript through `store` (the
// filesystem on native, a bundled transcript on wasm), so the public web bundle
// can open straight into a playing theater (ADR 0026).
pub mod agent_history;
pub mod agent_panel;
pub mod app;
pub mod camera;
pub mod command;
pub mod culling;
pub mod demo;
pub mod demoscript;
pub mod draw;
pub mod drc_panel;
pub mod fps;
pub mod generate_panel;
pub mod grid;
pub mod history;
pub mod inspector;
pub mod keymap;
pub mod labels;
pub mod layers;
pub mod measure;
pub mod minimap;
pub mod netlight;
pub mod notify;
pub mod open;
pub mod ops;
pub mod outline;
pub mod productivity;
pub mod query;
pub mod replay;
pub mod selection;
pub mod session;
pub mod share;
pub mod snap;
pub mod startscreen;
pub mod store;
pub mod tech_editor;
pub mod tool;
pub mod tour;
pub mod usecases;
pub mod view3d;
pub mod viewer;
pub mod viewexport;
pub mod viewports;
pub mod webopen;
pub mod xsection;

pub use app::{App, StartView};
pub use notify::{Notification, Notifications, Severity};
pub use open::{DocFormat, OpenError, OpenOutcome, OpenWarning, open_document_bytes};
pub use startscreen::ExampleChip;
pub use webopen::{
    LoadPlan, LoadProgress, RecentFile, RecentFiles, WASM_OPEN_CEILING_BYTES,
    WASM_STREAMING_THRESHOLD_BYTES, WebOpenEvent, WebOpenInbox, classify_drop, gds_url_from_query,
    url_file_name,
};
