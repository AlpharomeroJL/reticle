//! The `eframe::App` implementation: panels, canvas, and interaction wiring.
//!
//! This is the only module that depends on `egui`. It owns the app state and, each
//! frame, draws the toolbar, layer manager, undo-history panel, status bar, command
//! palette window, and the layout canvas, routing pointer input through the active
//! tool. All the non-trivial logic (camera math, culling, selection, snapping,
//! measurement, history) lives in the sibling modules and is unit-tested there; the
//! code here is deliberately thin glue plus painting.

use eframe::egui;
use egui::{
    Align2, Color32, FontId, Pos2, Rect as EguiRect, Sense, Shape, Stroke, StrokeKind, Vec2,
};

use reticle_geometry::{Endcap, LayerId, Point, Rect, Shape as _};
use reticle_model::{DrawShape, LayerInfo, ShapeKind, Technology};
use reticle_render::{
    ExpandedScene, Palette, RetainedRenderer, RetainedScene, ViewUniform, WgpuRenderer,
};
use reticle_sync::SyncDocument;
use std::sync::Arc;

use crate::agent_panel::AgentPanelState;
use crate::camera::{ScreenRect, ViewCamera};
use crate::command::{self, Command};
use crate::culling::{self, DetailLevel, SceneIndex};
use crate::demo;
use crate::drc_panel::{self, DrcResults};
use crate::fps::FrameMeter;
use crate::grid::{self, GridSettings};
use crate::history::History;
use crate::inspector::{self, Inspection};
use crate::keymap::{self, Keymap};
use crate::labels;
use crate::layers::{self, LayerState};
use crate::minimap::MinimapLayout;
use crate::netlight::{Generation, Netlight};
use crate::outline::{self, OutlineTree, SavedSets};
use crate::productivity::{self, ProductivityState};
use crate::query::{LayerLookup, Query};
use crate::replay::ReplayTheater;
use crate::selection::{self, Selection};
use crate::snap::{self, Guide, SnapHint, SnapState};
use crate::tech_editor::TechEditorState;
use crate::tool::{Tool, ToolState};
use crate::tour::{Tour, TourTarget};
use crate::usecases::{Scenario, UseCase};
use crate::viewports::{self, Split, Viewports};
/// A transient status message shown in the bottom bar.
#[derive(Clone, Debug, Default)]
struct Status {
    /// The message text (empty means nothing to show).
    text: String,
}

impl Status {
    /// Replaces the status message.
    fn set(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }
}

/// A Start-screen click, collected inside the layout closure and applied after it
/// returns (see [`App::apply_start_action`]).
///
/// Recording the intent rather than acting inline lets each Start-screen section be
/// a plain closure over `ui` without also mutably borrowing `self`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StartAction {
    /// Enter one of the worked scenarios.
    EnterUseCase(UseCase),
    /// Load a bundled example chip through the open seam.
    LoadExample(crate::startscreen::ExampleChip),
    /// Dismiss the Start screen and keep the demo document.
    SkipToEditor,
}

/// The button a tour overlay press requested this frame.
///
/// Collected inside the overlay closure and applied after it returns so the borrow
/// of `self` inside the closure ends before the tour state is mutated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TourAction {
    /// Advance to the next step (or finish on the last one).
    Next,
    /// Skip the rest of the current chapter.
    Skip,
    /// Dismiss the tour entirely.
    Close,
}

/// The on-screen rectangle for each tour highlight target, measured this frame.
///
/// The `ui()` method fills this from the panel and canvas rectangles it already
/// lays out, so the tour highlights the *actual* control without hard-coding pixel
/// coordinates. A target left as `None` (not laid out this frame, e.g. a panel
/// scrolled off) simply draws no highlight box.
#[derive(Clone, Copy, Debug, Default)]
struct TourTargets {
    /// The central canvas rectangle.
    canvas: Option<EguiRect>,
    /// The left layer panel rectangle.
    layers: Option<EguiRect>,
    /// The top toolbar rectangle.
    toolbar: Option<EguiRect>,
    /// The right-hand column rectangle (DRC, net highlight, agent, ops,
    /// productivity, snap, search, tech, and view/export all live in it).
    right_column: Option<EguiRect>,
    /// The minimap rectangle inside the canvas, when the minimap is drawn.
    minimap: Option<EguiRect>,
}

impl TourTargets {
    /// The rectangle to highlight for `target`, or `None` if it was not measured.
    ///
    /// Several right-column panels share the column rectangle, because the tour
    /// points at "the panel on the right" rather than a sub-rectangle that depends
    /// on scroll position; that keeps the highlight robust.
    fn rect_for(&self, target: TourTarget) -> Option<EguiRect> {
        match target {
            // The open affordance and the drawing tools both live on the toolbar, so
            // they highlight the toolbar rectangle (the tour points at "the toolbar
            // control" rather than a sub-rect that shifts as the row wraps).
            TourTarget::OpenAffordance | TourTarget::Toolbar | TourTarget::DrawTools => {
                self.toolbar
            }
            TourTarget::Canvas => self.canvas,
            TourTarget::LayerPanel => self.layers,
            TourTarget::Minimap => self.minimap.or(self.canvas),
            // The Share section sits in the right-hand column alongside the panels
            // below, so it highlights the whole column like they do.
            TourTarget::ShareSection
            | TourTarget::DrcPanel
            | TourTarget::NetHighlight
            | TourTarget::AgentPanel
            | TourTarget::OpsPanel
            | TourTarget::ProductivityPanel
            | TourTarget::SnapPanel
            | TourTarget::SearchPanel
            | TourTarget::TechPanel
            | TourTarget::ViewExportPanel => self.right_column,
        }
    }
}

/// State for the search / selection-depth panel: the filter query bar, saved
/// selection sets, select-similar, and the cell/instance outline tree.
///
/// The heavy lifting (parsing, evaluation, saved-set bookkeeping, outline
/// building) lives in [`crate::query`] and [`crate::outline`]; this struct only
/// holds the panel's editable fields plus the cached outline. `pending_locate` is
/// a deferred camera target set when an outline row is clicked and consumed inside
/// [`App::canvas`] once the real screen rectangle is known, exactly like the DRC
/// panel's deferred zoom.
#[derive(Clone, Debug, Default)]
struct SearchState {
    /// The filter query text (e.g. `layer:METAL1 width<400`).
    query_text: String,
    /// The last query error message, shown under the bar (empty when none).
    error: String,
    /// The name field for saving/restoring a selection set.
    set_name: String,
    /// The cached outline tree, rebuilt when the document changes.
    outline: OutlineTree,
    /// The saved selection sets for this session.
    saved: SavedSets,
    /// A world rectangle the camera should frame on the next frame, set by an
    /// outline "locate" click and consumed in [`App::canvas`].
    pending_locate: Option<Rect>,
}

/// Which via-stack layer field a picker combo writes to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ViaLayerField {
    /// The lower enclosure layer.
    Lower,
    /// The upper enclosure layer.
    Upper,
    /// The cut/via layer.
    Cut,
}

/// The top-level application state: the collaborative document and the renderer.
///
/// The [`renderer`](App::renderer) and [`document`](App::document) accessors are the
/// frozen Wave 0 contract. Beyond them the struct now carries the full editor state:
/// the editable document with undo history, the view camera, the tool machine, the
/// layer/selection/grid models, and the command-palette UI state.
// The app root aggregates many independent one-bit UI facts (deferred-fit flag,
// window-open flags, overlay toggles); folding them into enums or sub-structs
// would only add indirection to the glue layer.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug)]
pub struct App {
    /// GPU renderer handle (used by the native PNG-export action).
    renderer: WgpuRenderer,
    /// Collaboration mirror of the document (Wave 0 contract; presence overlay).
    document: SyncDocument,

    /// The editable document with undo/redo, the layout the user edits.
    history: History,
    /// The world<->screen camera.
    camera: ViewCamera,
    /// Whether the camera should fit the design on the next frame (deferred so the
    /// real canvas size is known).
    fit_requested: bool,
    /// The tool state machine.
    tools: ToolState,
    /// The drawing / vertex-edit state: in-progress polygon and path builders, the
    /// path width and end cap, and any live vertex grab (see [`crate::draw`]).
    draw: crate::draw::DrawState,
    /// Layer table, visibility, and filter.
    layer_state: LayerState,
    /// The current shape selection (indices into the scene).
    pub(crate) selection: Selection,
    /// The boolean/transform operations panel state (numeric inputs, status).
    pub(crate) ops: crate::ops::OpsState,
    /// Grid, snapping, and ruler settings.
    grid: GridSettings,
    /// Geometry-snap settings and the user guides, plus the last frame's snap hint
    /// so the canvas can draw the snap indicator where the cursor caught geometry.
    snap: SnapState,
    /// The snap indicator to draw this frame, recomputed each hover from the cursor
    /// (see [`App::snap_world`]). `None` when the cursor caught nothing.
    snap_hint: Option<SnapHint>,
    /// An in-progress guide drag: the axis being pulled from a ruler. `Some` from
    /// the press inside a ruler bar until the pointer is released on the canvas.
    dragging_guide: Option<crate::snap::Axis>,
    /// Whether the canvas text-label overlay (cell names, selection captions, live
    /// dimensions) is drawn.
    labels_visible: bool,
    /// Whether the minimap overview panel is drawn (and steals clicks inside it).
    minimap_visible: bool,
    /// The canvas pane layout: split mode, focused pane, and per-pane cameras.
    viewports: Viewports,
    /// The name of the top cell being viewed.
    top_cell: String,

    /// The spatial index over the flattened scene, rebuilt when the document or
    /// viewed cell changes.
    scene: SceneIndex,
    /// A revision token bumped on every scene rebuild, used to invalidate the
    /// net-extraction cache after edits and undo/redo.
    doc_generation: Generation,

    /// The retained GPU scene cache (per-cell tessellation + expanded instances),
    /// rebuilt only when the document or layer visibility changes.
    retained: RetainedScene,
    /// The revision the retained scene reflects: the document revision combined with
    /// the layer-visibility signature. When it changes, the GPU renderer re-uploads.
    render_revision: u64,
    /// A hash of the current layer-visibility bits, recomputed each frame. A change
    /// (from a toggle, checkbox, or show/hide-all) triggers a retained rebuild since
    /// the tessellation bakes visibility in.
    visibility_sig: u64,
    /// The most recently expanded GPU geometry, shared into the paint callback. An
    /// `Arc` so handing it to the callback each frame is a refcount bump, not a copy;
    /// it is refreshed only when [`App::sync_retained`] rebuilds.
    expanded: Arc<ExpandedScene>,

    /// The agent panel: prompt, run state machine, narration, and cursor. The
    /// panel is model-free (it drives a scripted transcript) and builds on wasm.
    agent: AgentPanelState,
    /// The replay theater's playback state machine. Model-free, so it runs on
    /// both native and wasm; its transcript comes from [`store`](crate::store).
    replay: ReplayTheater,
    /// Whether the replay theater window is open.
    replay_open: bool,
    /// The transcript-path text in the theater's load row (native filesystem).
    replay_path: String,
    /// The last theater load error, shown under the load row (empty when none).
    replay_error: String,
    /// Where the theater reads transcripts from: the filesystem on native, a
    /// bundled transcript on wasm. Boxed so the field type is the same on both.
    store: Box<dyn crate::store::SessionStore>,
    /// The session history browser: past run transcripts the user can reopen,
    /// the directory the native scan reads, and its error line (empty when none).
    /// Refreshed on demand from [`crate::agent_history`], so no scan runs per frame.
    agent_history: crate::agent_history::HistoryBrowser,

    /// The DRC panel state: the last run's violations and the highlighted one.
    drc: DrcResults,
    /// Whether the camera should frame the selected violation on the next frame
    /// (deferred so the real canvas size is known, like [`fit_requested`](Self::fit_requested)).
    zoom_to_selected_violation: bool,
    /// The net-highlight state: cached connectivity plus the highlighted net.
    netlight: Netlight,
    /// The 3D layer-stack window's orbit-camera state.
    view3d: crate::view3d::View3d,

    /// The productivity panel state: the in-app clipboard plus the array,
    /// move-by-delta, and via-stack dialog fields.
    productivity: ProductivityState,

    /// The Generate panel state: the parameterized-generator catalog, the selected
    /// generator, and its typed form values. Drives the live preview overlay and the
    /// undo-integrated placement (see [`crate::generate_panel`]).
    generate: crate::generate_panel::GeneratePanelState,

    /// The search / selection-depth panel: filter query bar, saved selection
    /// sets, select-similar, and the cell/instance outline tree.
    search: SearchState,
    /// The technology-editor panel state: the draft technology being edited, its
    /// validation errors, and the tech-file round-trip text (see
    /// [`crate::tech_editor`]).
    tech_editor: TechEditorState,

    /// The rebindable shortcut map every key press resolves through.
    keymap: Keymap,
    /// Whether the shortcuts editor window is open.
    keymap_open: bool,
    /// The action awaiting a new chord, when the editor is capturing one.
    rebinding: Option<keymap::Action>,

    /// Whether the command palette window is open.
    palette_open: bool,
    /// The command-palette search query.
    palette_query: String,
    /// The query-bar text for "select by layer".
    layer_query: String,
    /// The relay host in the Share section (see [`crate::share`]).
    share_server: String,
    /// The room name in the Share section; sanitized into the link.
    share_room: String,
    /// The page origin the read-only viewer link points at (where the web bundle
    /// is served). Empty yields a relative viewer link (see [`crate::share`]).
    share_page: String,
    /// The most recent status-bar message.
    status: Status,
    /// Non-fatal warnings from the most recent document open (see [`crate::open`]).
    /// Populated by [`App::open_outcome`]; drives the small warnings window that
    /// [`App::open_warnings_window`] draws until the user dismisses it. Empty when
    /// the last open was clean or none has happened.
    open_warnings: Vec<crate::open::OpenWarning>,
    /// The app-wide notification (toast) queue: the single human-readable surface
    /// every failure path reports through (see [`crate::notify`]). Import errors,
    /// open-seam warnings, session-load and technology-parse failures, and the
    /// example gallery all route here via [`App::report_error`]/[`App::notify`], so
    /// nothing fails silently or only in the console. Drawn by
    /// [`App::notifications_area`]. Sibling lanes (browser open, share) route their
    /// failures through the same methods.
    notifications: crate::notify::Notifications,
    /// The recently opened files, most-recent first (see [`crate::webopen`]). Owned
    /// here so a Start screen can render reopen rows; the browser build persists it to
    /// `IndexedDB` across reloads. On native it is an in-session list, unused by the
    /// desktop UI, kept so the model is identical on both targets.
    recent_files: crate::webopen::RecentFiles,
    /// The progress of an in-flight progressive (big-file / remote) open in the
    /// browser build (see [`crate::webopen::LoadProgress`]). `Idle` when nothing is
    /// loading; drives the progress indicator and the load-failure message.
    load_progress: crate::webopen::LoadProgress,
    /// The mailbox async browser tasks (a `?gds=` fetch, an `IndexedDB` recent-load)
    /// post their results into for the synchronous `update` loop to apply (see
    /// [`crate::webopen::WebOpenInbox`]). Present on both targets so the field type is
    /// uniform; only ever fed on wasm.
    web_open: crate::webopen::WebOpenInbox,
    /// Whether the one-shot browser open path (recent-list load plus any `?gds=`
    /// fetch) has been kicked off. Guards it to the first wasm frame. Only read on
    /// wasm (native never spawns the path), so it is dead there by design.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    web_open_started: bool,
    /// The last world position under the cursor, for the status readout.
    cursor_world: Option<Point>,
    /// Rolling frame-time meter behind the status-bar fps readout.
    frame_meter: FrameMeter,
    /// Which view the app opened into (editor or the replay theater). The web mount
    /// selects this from the page URL so a public visitor lands on the theater
    /// (ADR 0026).
    start_view: StartView,
    /// Whether the Start screen (the worked-use-case chooser) is currently shown.
    /// Set on startup for the editor default so a first-time user is offered the
    /// worked scenarios; cleared once one is chosen or dismissed, and never shown for
    /// the replay-theater start view (that visitor lands straight in the theater).
    /// See [`crate::usecases`] and [`App::start_screen`].
    start_screen: bool,

    /// View and export polish: the egui theme, per-document camera bookmarks, the
    /// export scope/format, and the print-style monochrome toggle. The theme is
    /// applied to the egui `Visuals` each frame and the whole struct persists with
    /// the session view state (see [`crate::viewexport`]).
    view_export: crate::viewexport::ViewExport,

    // ---- Lane 4A: first-run tour --------------------------------------------
    /// The first-run tour state machine (pure; see [`crate::tour`]). Auto-starts on
    /// a fresh install and is relaunchable from the Help menu. Its "seen" bit
    /// persists with the session so the automatic tour shows only once.
    tour: Tour,

    /// Active one-shot screenshot smoke, set only by the native `--screenshot-smoke`
    /// launcher; `None` otherwise. Drives a single full-window egui screenshot to
    /// de-risk the capture path (see [`crate::demoscript`]); native only.
    #[cfg(not(target_arch = "wasm32"))]
    capture: Option<crate::demoscript::CaptureState>,

    /// Active scripted demo run, set only by the native `--demo-script` launcher;
    /// `None` in normal interactive and web use. It drives the editor through a timed
    /// step list and screenshots each capture frame for the README media harness (see
    /// [`crate::demoscript`]); native only, since a windowed screenshot is meaningless
    /// on wasm.
    #[cfg(not(target_arch = "wasm32"))]
    demo: Option<crate::demoscript::DemoRun>,

    /// Whether a demo capture wants the floating 3D-stack window shown. Off by
    /// default so non-3D tours are not cluttered by it; a `view3d on` step turns it on
    /// for the 3D tour. Ignored outside capture mode (the window shows normally).
    #[cfg(not(target_arch = "wasm32"))]
    demo_show_3d: bool,

    /// Whether a demo capture wants the right column scrolled to the search panel (the
    /// filter-query bar and outline tree), which otherwise sits below the fold. Set by
    /// a `filter`/`outline-locate` step so the query tour shows the bar it is driving.
    #[cfg(not(target_arch = "wasm32"))]
    demo_focus_search: bool,
}

/// The view the app opens into.
///
/// The native launcher and the desktop default use [`StartView::Editor`]. The web
/// mount reads a `?view=` query parameter and passes [`StartView::ReplayTheater`]
/// for a public visitor, so the deployed bundle opens to the replay theater rather
/// than the editor (ADR 0026).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum StartView {
    /// Open into the interactive editor. The desktop default.
    #[default]
    Editor,
    /// Open into the replay theater, playing the built-in scripted demo run. The
    /// default for a public web visitor.
    ReplayTheater,
}

impl StartView {
    /// Parses a `view` query value into a [`StartView`].
    ///
    /// `replay` (or `theater`) selects [`StartView::ReplayTheater`]; anything else,
    /// including an absent value, selects [`StartView::Editor`]. Case-insensitive.
    #[must_use]
    pub fn from_query_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "replay" | "theater" | "theatre" => StartView::ReplayTheater,
            _ => StartView::Editor,
        }
    }
}

/// The thickness, in screen pixels, of the top and left ruler bars.
///
/// Shared by the ruler drawing and the guide-drag hit-test so a drag that begins
/// inside a bar lines up exactly with the painted ruler.
const RULER_BAR: f32 = 18.0;

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// Creates the app with the built-in demo document loaded, opening into the
    /// editor.
    ///
    /// This is cheap (it builds a small in-memory document and a spatial index) so
    /// it is safe to call from both the native launcher and the web mount point.
    #[must_use]
    pub fn new() -> Self {
        Self::with_start_view(StartView::Editor)
    }

    /// Creates the app opening into `start_view`.
    ///
    /// [`StartView::Editor`] is the desktop default. [`StartView::ReplayTheater`]
    /// opens the replay theater and loads the default transcript from the platform
    /// [`store`](crate::store) so a public web visitor sees the agent draw
    /// immediately (ADR 0026). The theater is model-free and runs on both native and
    /// wasm; on wasm it plays a bundled transcript, so the web bundle opens straight
    /// into a working theater.
    #[must_use]
    pub fn with_start_view(start_view: StartView) -> Self {
        let mut app = Self::build(start_view);
        app.apply_start_view();
        app
    }

    /// Applies the recorded [`StartView`] to the constructed app.
    ///
    /// For [`StartView::ReplayTheater`] it opens the theater window and loads the
    /// default transcript from the platform [`store`](crate::store) (the bundled
    /// transcript on wasm, the scripted run on native), so a public web visitor
    /// lands directly on a playing replay (ADR 0026). If the store cannot produce a
    /// transcript, the theater simply opens empty rather than failing.
    fn apply_start_view(&mut self) {
        if self.start_view == StartView::ReplayTheater {
            if let Ok((records, hash)) = self.store.default_transcript() {
                self.replay.load(records, hash);
            }
            self.replay_open = true;
        }
    }

    /// Builds the app state opening into `start_view` (without applying the view;
    /// [`with_start_view`](Self::with_start_view) applies it).
    #[must_use]
    fn build(start_view: StartView) -> Self {
        let doc = demo::demo_document();
        let layer_state = LayerState::from_technology(doc.technology());
        let scene = SceneIndex::build(&doc, demo::TOP_CELL);
        let document = SyncDocument::from_document("local", &doc);
        // Build the retained scene from the demo document with a visibility-aware
        // palette, so the GPU canvas has geometry from the first frame.
        let palette = palette_from_layers(&layer_state);
        let retained = RetainedScene::new(&doc, demo::TOP_CELL, &palette);
        let expanded = Arc::new(retained.expand());
        // Build the outline before `doc` is moved into the history below.
        let outline = OutlineTree::build(&doc);
        Self {
            renderer: WgpuRenderer::new(),
            document,
            history: History::new(doc),
            camera: ViewCamera::new(Point::ORIGIN, 0.05),
            fit_requested: true,
            tools: ToolState::new(),
            draw: crate::draw::DrawState::new(),
            layer_state,
            selection: Selection::new(),
            ops: crate::ops::OpsState::new(),
            grid: GridSettings::default(),
            snap: SnapState::default(),
            snap_hint: None,
            dragging_guide: None,
            labels_visible: true,
            minimap_visible: true,
            viewports: Viewports::new(),
            top_cell: demo::TOP_CELL.to_owned(),
            scene,
            doc_generation: 0,
            retained,
            render_revision: 0,
            visibility_sig: 0,
            expanded,
            agent: AgentPanelState::new(),
            replay: ReplayTheater::new(),
            replay_open: false,
            replay_path: String::new(),
            replay_error: String::new(),
            store: Box::new(crate::store::default_store()),
            agent_history: crate::agent_history::HistoryBrowser::new(),
            drc: DrcResults::new(),
            zoom_to_selected_violation: false,
            netlight: Netlight::new(),
            view3d: crate::view3d::View3d::new(),
            productivity: ProductivityState::new(),
            generate: crate::generate_panel::GeneratePanelState::new(),
            search: SearchState {
                outline,
                ..SearchState::default()
            },
            tech_editor: TechEditorState::new(),
            keymap: load_keymap(),
            keymap_open: false,
            rebinding: None,
            palette_open: false,
            palette_query: String::new(),
            layer_query: String::new(),
            share_server: crate::share::DEFAULT_SERVER.to_owned(),
            share_room: crate::share::room_id(demo::TOP_CELL),
            share_page: String::new(),
            status: Status::default(),
            open_warnings: Vec::new(),
            notifications: crate::notify::Notifications::new(),
            recent_files: crate::webopen::RecentFiles::new(),
            load_progress: crate::webopen::LoadProgress::Idle,
            web_open: crate::webopen::WebOpenInbox::new(),
            web_open_started: false,
            cursor_world: None,
            frame_meter: FrameMeter::default(),
            start_view,
            // Greet a first-time user with the worked-use-case chooser on the
            // editor default; the replay-theater start view drops the visitor
            // straight into the theater and never shows it.
            start_screen: start_view == StartView::Editor,
            view_export: crate::viewexport::ViewExport::new(),
            tour: Tour::from_seen(tour_already_seen()),
            #[cfg(not(target_arch = "wasm32"))]
            capture: None,
            #[cfg(not(target_arch = "wasm32"))]
            demo: None,
            #[cfg(not(target_arch = "wasm32"))]
            demo_show_3d: false,
            #[cfg(not(target_arch = "wasm32"))]
            demo_focus_search: false,
        }
    }

    /// The view the app opened into (editor or replay theater).
    #[must_use]
    pub fn start_view(&self) -> StartView {
        self.start_view
    }

    /// Whether the Start screen (the worked-use-case chooser) is currently shown.
    ///
    /// It greets a first-time user on the editor default and is cleared once a
    /// scenario is chosen or the chooser is dismissed. See [`crate::usecases`].
    #[must_use]
    pub fn start_screen(&self) -> bool {
        self.start_screen
    }

    /// Enters the chosen worked [`UseCase`], then dismisses the Start screen.
    ///
    /// A document-backed scenario installs its prepared document as the live layout
    /// (replacing the demo, framing its top cell); the agent scenario opens the
    /// replay theater on the bundled run. Centralizing this here keeps the Start
    /// screen's click wiring a single call and lets the whole flow be unit-tested
    /// without a window.
    pub fn enter_use_case(&mut self, use_case: UseCase) {
        match use_case.prepare() {
            Scenario::LoadDocument { document, top_cell } => {
                self.install_document(document, top_cell);
                self.status
                    .set(format!("Loaded scenario: {}", use_case.title()));
            }
            Scenario::OpenReplayTheater => {
                if let Ok((records, hash)) = self.store.default_transcript() {
                    self.replay.load(records, hash);
                }
                self.replay_open = true;
                self.replay.play();
                self.status
                    .set(format!("Opened scenario: {}", use_case.title()));
            }
        }
        self.start_screen = false;
    }

    /// Installs `document` as the live editing layout, framing `top_cell`.
    ///
    /// This replaces the demo (or a previously loaded scenario) wholesale: it
    /// rebuilds every piece of derived state the editor keeps in step with the
    /// document, exactly as [`build`](Self::build) does on startup, so the layer
    /// manager, canvas, spatial index, retained GPU scene, outline, collaboration
    /// mirror, and camera all reflect the new layout. The undo history starts fresh
    /// (a loaded scenario is a new editing session, not an edit of the old one), and
    /// any stale DRC results, selection, and net highlight are cleared.
    fn install_document(&mut self, document: reticle_model::Document, top_cell: String) {
        self.layer_state = LayerState::from_technology(document.technology());
        self.document = SyncDocument::from_document("local", &document);
        self.scene = SceneIndex::build(&document, &top_cell);
        let palette = palette_from_layers(&self.layer_state);
        self.retained = RetainedScene::new(&document, &top_cell, &palette);
        self.expanded = Arc::new(self.retained.expand());
        self.search.outline = OutlineTree::build(&document);
        self.top_cell = top_cell;
        self.history = History::new(document);
        // Force the retained GPU scene and net cache to rebuild against the new
        // document on the next frame, and reframe the camera on it.
        self.doc_generation = self.doc_generation.wrapping_add(1);
        self.render_revision = self.render_revision.wrapping_add(1);
        self.selection.clear();
        self.netlight.clear();
        self.drc.clear();
        self.fit_requested = true;
    }

    /// Opens a layout document from `bytes` and loads it into the editor.
    ///
    /// This is the app-facing wrapper over the document-open seam
    /// ([`crate::open::open_document_bytes`]): it imports through the hardened path
    /// (so no input can panic or hang), and on success installs the opened document
    /// as the live layout (replacing whatever was open), frames its top cell,
    /// dismisses the Start screen, and records any non-fatal warnings so the
    /// warnings window surfaces them. It takes bytes, not a path, so the same call
    /// works on native and on wasm.
    ///
    /// Other file-open entry points (a Start screen's open button, an example
    /// gallery, drag-and-drop) route through this, or call the seam directly and
    /// hand the [`OpenOutcome`](crate::open::OpenOutcome) to [`open_outcome`](Self::open_outcome).
    ///
    /// # Errors
    ///
    /// Returns the seam's [`OpenError`](crate::open::OpenError) unchanged when the
    /// bytes cannot be opened; the editor is left untouched in that case.
    pub fn open_document_bytes(
        &mut self,
        bytes: &[u8],
        format: crate::open::DocFormat,
    ) -> Result<(), crate::open::OpenError> {
        let outcome = crate::open::open_document_bytes(bytes, format)?;
        self.open_outcome(outcome);
        Ok(())
    }

    /// Loads an already-produced [`OpenOutcome`](crate::open::OpenOutcome) into the
    /// editor.
    ///
    /// Installs the document, frames its top cell, dismisses the Start screen, and
    /// stashes the outcome's warnings so the warnings window shows them. Split from
    /// [`open_document_bytes`](Self::open_document_bytes) so a caller that ran the
    /// seam itself (for example to inspect the warnings first) can still load the
    /// result through the same path.
    pub fn open_outcome(&mut self, outcome: crate::open::OpenOutcome) {
        let crate::open::OpenOutcome {
            document,
            top_cell,
            warnings,
        } = outcome;
        let cell_count = document.cell_count();
        self.install_document(document, top_cell);
        self.open_warnings = warnings;
        self.start_screen = false;
        if self.open_warnings.is_empty() {
            self.status
                .set(format!("Opened document ({cell_count} cells)"));
            self.notifications.info(
                format!("Opened document ({cell_count} cells)"),
                String::new(),
            );
        } else {
            self.status.set(format!(
                "Opened document ({cell_count} cells, {} warning(s))",
                self.open_warnings.len()
            ));
            // Every warning also rides the shared notification surface, so a headless
            // caller (or a user who dismissed the warnings window) still sees that
            // parts were skipped. The window remains for the itemized detail.
            for w in &self.open_warnings {
                self.notifications
                    .warning(w.summary.clone(), w.detail.clone());
            }
        }
    }

    /// The non-fatal warnings from the most recent document open (empty when the
    /// last open was clean or none has happened). Read by the warnings window and
    /// exposed so a richer error surface (owned by another lane) can render them.
    #[must_use]
    pub fn open_warnings(&self) -> &[crate::open::OpenWarning] {
        &self.open_warnings
    }

    /// Clears the stored open-warnings (dismisses the warnings window).
    pub fn clear_open_warnings(&mut self) {
        self.open_warnings.clear();
    }

    /// Reports a hard failure to the app's one human-readable error surface.
    ///
    /// This is the single sink every failure path routes through (see
    /// [`crate::notify`]): it queues an error notification with a one-line `summary`
    /// and a longer `detail`, which the toast area draws until the user dismisses it.
    /// No failure is silent, and none is only in the console.
    ///
    /// Concurrent lanes route their own failures here too (a browser file that will
    /// not open, a share link that cannot be formed), so all first-contact errors
    /// converge on one consistent surface.
    pub fn report_error(&mut self, summary: impl Into<String>, detail: impl Into<String>) {
        self.notifications.error(summary, detail);
    }

    /// Posts a neutral, informational notice to the notification surface.
    ///
    /// The non-error companion to [`report_error`](Self::report_error): the toast
    /// auto-dismisses after a few seconds. Use it for "opened", "copied", or similar
    /// confirmations that need to be visible without demanding a dismissal.
    pub fn notify(&mut self, summary: impl Into<String>, detail: impl Into<String>) {
        self.notifications.info(summary, detail);
    }

    /// The live notification queue (for the toast area and for tests).
    #[must_use]
    pub fn notifications(&self) -> &crate::notify::Notifications {
        &self.notifications
    }

    /// Opens a document from `bytes`, routing any failure to the error surface.
    ///
    /// The infallible companion to [`open_document_bytes`](Self::open_document_bytes):
    /// it runs the same hardened seam, but instead of returning the
    /// [`OpenError`](crate::open::OpenError) it reports it through
    /// [`report_error`](Self::report_error) and returns whether the open succeeded.
    /// This is what the Start screen's open button, the example gallery, and the
    /// drag-and-drop handler call, so every user-facing open surfaces its own error
    /// with no `Result` to thread through the egui glue. `source` names what was
    /// opened (a file name, or an example title) for the error's summary.
    pub fn open_bytes_reporting(
        &mut self,
        bytes: &[u8],
        format: crate::open::DocFormat,
        source: &str,
    ) -> bool {
        match crate::open::open_document_bytes(bytes, format) {
            Ok(outcome) => {
                self.open_outcome(outcome);
                true
            }
            Err(e) => {
                self.report_error(format!("Could not open {source}"), e.to_string());
                false
            }
        }
    }

    /// Opens one of the bundled [`ExampleChip`](crate::startscreen::ExampleChip)
    /// gallery designs, routing any failure to the error surface.
    ///
    /// Runs the chip's bytes through the same seam as every other open and installs
    /// the result, or reports a clean error if a committed design ever fails to
    /// import (a unit test guards against that). Returns whether it opened.
    pub fn open_example_chip(&mut self, chip: crate::startscreen::ExampleChip) -> bool {
        match chip.open() {
            Ok(outcome) => {
                self.open_outcome(outcome);
                self.notify(format!("Loaded example: {}", chip.title()), String::new());
                true
            }
            Err(e) => {
                self.report_error(
                    format!("Could not load example: {}", chip.title()),
                    e.to_string(),
                );
                false
            }
        }
    }

    /// The recently opened files, most-recent first.
    ///
    /// A Start screen (a sibling lane) renders these as reopen rows; the browser build
    /// persists the list to `IndexedDB` so it survives a reload. This is the read side
    /// of the loosely-coupled contract: this lane owns recording and persistence, the
    /// Start screen only reads. Empty until a file is opened through the browser open
    /// path.
    #[must_use]
    pub fn recent_files(&self) -> &[crate::webopen::RecentFile] {
        self.recent_files.entries()
    }

    /// Records `file` at the front of the recent list (deduping and capping per
    /// [`crate::webopen::RecentFiles`]).
    ///
    /// The browser open path calls this after a successful open (a drop, a `?gds=`
    /// fetch, or a reopen). It updates only the in-memory model; the wasm layer
    /// persists the list to `IndexedDB` separately via the wasm-only
    /// `webopen::store_recent_files` so the pure model stays testable.
    pub fn record_recent_file(&mut self, file: crate::webopen::RecentFile) {
        self.recent_files.record(file);
    }

    /// Replaces the whole recent list, e.g. with the one loaded from `IndexedDB` at
    /// startup on the browser build.
    pub fn set_recent_files(&mut self, recents: crate::webopen::RecentFiles) {
        self.recent_files = recents;
    }

    /// The current progressive-load progress (see [`crate::webopen::LoadProgress`]).
    #[must_use]
    pub fn load_progress(&self) -> &crate::webopen::LoadProgress {
        &self.load_progress
    }

    /// Sets the progressive-load progress, driving the progress indicator and the
    /// load-failure message. The wasm fetch/streaming path updates this as bytes
    /// arrive and the index builds.
    pub fn set_load_progress(&mut self, progress: crate::webopen::LoadProgress) {
        self.load_progress = progress;
    }

    /// Handles files dropped onto the page (browser) or window (native) this frame.
    ///
    /// egui surfaces dropped files with their bytes on web (`DroppedFile::bytes`) and
    /// their `name` on both targets. For each dropped file we classify the format from
    /// the name via [`crate::webopen::classify_drop`], apply the big-file
    /// size-band decision ([`crate::webopen::LoadPlan`]) so an oversized file is
    /// refused with an honest message instead of exhausting wasm memory, and open the
    /// bytes through the seam ([`open_document_bytes`](Self::open_document_bytes)),
    /// recording the file in the recent list on success. A drop of a non-layout file,
    /// or a failed import, sets a clear status message rather than doing nothing.
    ///
    /// Only the first successfully-classified dropped file is opened (opening several
    /// layouts at once has no meaning in a single-document editor); extra drops are
    /// ignored. On native, `bytes` may be absent (egui gives a path there instead); in
    /// that case we read the path so a desktop drag-and-drop also works.
    ///
    /// Returns `true` when a drop was consumed (opened, refused, or reported), so the
    /// caller can, for instance, dismiss the Start screen.
    fn handle_dropped_files(&mut self, ctx: &egui::Context) -> bool {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if dropped.is_empty() {
            return false;
        }
        for file in dropped {
            let Some(format) = crate::webopen::classify_drop(&file.name) else {
                continue;
            };
            let bytes = Self::dropped_file_bytes(&file);
            let Some(bytes) = bytes else {
                self.status
                    .set(format!("Could not read the dropped file {}", file.name));
                return true;
            };
            let plan = crate::webopen::LoadPlan::for_size(bytes.len() as u64);
            if let Some(message) = plan.refusal_message() {
                self.status.set(message.clone());
                self.load_progress = crate::webopen::LoadProgress::failed(message);
                return true;
            }
            match self.open_document_bytes(&bytes, format) {
                Ok(()) => {
                    self.record_recent_file(crate::webopen::RecentFile::local(
                        file.name.clone(),
                        bytes.len() as u64,
                    ));
                    self.load_progress = crate::webopen::LoadProgress::Idle;
                    self.persist_recent_files();
                }
                Err(e) => {
                    self.status.set(e.to_string());
                    self.load_progress = crate::webopen::LoadProgress::failed(e.to_string());
                }
            }
            return true;
        }
        // Every dropped file was a non-layout type: tell the user rather than
        // silently ignoring the gesture.
        self.status
            .set("Drop a .gds or .oas file to open it".to_owned());
        true
    }

    /// The bytes of a dropped file: from egui's in-memory `bytes` (always the source
    /// on web), or by reading the file path on native where egui provides a path
    /// instead. `None` when neither is available or the path read fails.
    fn dropped_file_bytes(file: &egui::DroppedFile) -> Option<Vec<u8>> {
        if let Some(bytes) = &file.bytes {
            return Some(bytes.to_vec());
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = &file.path {
            return std::fs::read(path).ok();
        }
        None
    }

    /// Whether a file is currently being dragged over the page, so the UI can show a
    /// "drop to open" affordance. Reads egui's per-frame `hovered_files`.
    fn is_file_hovering(ctx: &egui::Context) -> bool {
        ctx.input(|i| !i.raw.hovered_files.is_empty())
    }

    /// Draws the full-screen "drop a .gds or .oas file to open it" affordance shown
    /// while a file is dragged over the page, so the drop target is obvious.
    ///
    /// A dimming veil plus a dashed border and a centered prompt, painted on a
    /// foreground layer so it sits over the canvas and panels. Purely visual; the
    /// actual open happens in [`handle_dropped_files`](Self::handle_dropped_files) when
    /// the file is released.
    fn draw_drop_affordance(ctx: &egui::Context) {
        // The full page rectangle this frame. `raw.screen_rect` is what egui fills in
        // from the platform each frame; fall back to a unit rect if a headless frame
        // ever lacks it (the affordance is purely cosmetic, so a missing size just
        // draws nothing meaningful rather than panicking).
        let screen = ctx
            .input(|i| i.raw.screen_rect)
            .unwrap_or_else(|| EguiRect::from_min_size(Pos2::ZERO, Vec2::splat(1.0)));
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drop_affordance"),
        ));
        // Dim the page and draw an inset dashed frame to read as a drop zone.
        painter.rect_filled(screen, 0.0, Color32::from_rgba_unmultiplied(8, 10, 16, 180));
        let inset = screen.shrink(24.0);
        painter.rect_stroke(
            inset,
            12.0,
            Stroke::new(3.0, Color32::from_rgb(120, 170, 255)),
            StrokeKind::Inside,
        );
        painter.text(
            screen.center(),
            Align2::CENTER_CENTER,
            "Drop a .gds or .oas file to open it",
            FontId::proportional(22.0),
            Color32::from_rgb(220, 230, 245),
        );
    }

    /// Draws the progressive-load progress indicator, or the last load-failure
    /// message, over the page (browser open path).
    ///
    /// While a load is active ([`LoadProgress::is_active`](crate::webopen::LoadProgress::is_active))
    /// it shows a small centered card with a determinate bar (when the fetch total is
    /// known) or an indeterminate "working" note. A failed load shows a dismissible
    /// human-readable message (a CORS/network failure, an oversize refusal, a parse
    /// error), so a failure is never console-only and never a silent hang. `Idle` and
    /// `Done` draw nothing.
    fn draw_load_progress(&mut self, ctx: &egui::Context) {
        use crate::webopen::LoadProgress;
        match &self.load_progress {
            LoadProgress::Idle | LoadProgress::Done => {}
            LoadProgress::Fetching { .. } | LoadProgress::Indexing => {
                let fraction = self.load_progress.fraction();
                let label = match &self.load_progress {
                    LoadProgress::Fetching { received, total } if *total > 0 => {
                        format!(
                            "Downloading... {} / {} MiB",
                            received / (1024 * 1024),
                            total / (1024 * 1024)
                        )
                    }
                    LoadProgress::Fetching { received, .. } => {
                        format!("Downloading... {} MiB", received / (1024 * 1024))
                    }
                    _ => "Building the index...".to_owned(),
                };
                egui::Window::new("Opening")
                    .id(egui::Id::new("load_progress_window"))
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .show(ctx, |ui| {
                        ui.label(label);
                        let bar = match fraction {
                            Some(f) => egui::ProgressBar::new(f).show_percentage(),
                            // Indeterminate: an animated bar with no fixed fraction.
                            None => egui::ProgressBar::new(0.0).animate(true),
                        };
                        ui.add(bar);
                    });
                ctx.request_repaint();
            }
            LoadProgress::Failed { message } => {
                let message = message.clone();
                let mut open = true;
                egui::Window::new("Could not open the file")
                    .id(egui::Id::new("load_failed_window"))
                    .collapsible(false)
                    .resizable(false)
                    .open(&mut open)
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .show(ctx, |ui| {
                        ui.colored_label(Color32::from_rgb(240, 150, 150), &message);
                        ui.separator();
                        if ui.button("Dismiss").clicked() {
                            self.load_progress = LoadProgress::Idle;
                        }
                    });
                if !open {
                    self.load_progress = LoadProgress::Idle;
                }
            }
        }
    }

    /// Drives the browser open path: on the first wasm frame it kicks off the recent
    /// list load and any `?gds=` fetch; every frame it applies whatever those async
    /// tasks have posted to the [`web_open`](Self::web_open) inbox.
    ///
    /// This is the single bridge between the async fetch/`IndexedDB` tasks and the
    /// synchronous editor loop. The tasks post [`WebOpenEvent`](crate::webopen::WebOpenEvent)s;
    /// here, on the main thread, we install a fetched document through the seam, adopt
    /// a restored recent list, update the progress indicator, or record a failure, all
    /// via the same App methods a native open uses. On native the inbox is always empty
    /// and nothing is ever spawned, so this compiles to a cheap no-op.
    fn drive_web_open(&mut self, ctx: &egui::Context) {
        #[cfg(target_arch = "wasm32")]
        if !self.web_open_started {
            self.web_open_started = true;
            crate::webopen::start_web_open(&self.web_open, ctx.clone());
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = ctx;

        for event in self.web_open.drain() {
            self.apply_web_open_event(event);
        }
    }

    /// Applies one [`WebOpenEvent`](crate::webopen::WebOpenEvent) on the main thread.
    ///
    /// Split out so the mapping from an async result to an editor action is one small,
    /// readable place: progress updates the indicator, an `Opened` event imports the
    /// bytes through the seam and records the recent entry (persisting the updated list
    /// back to `IndexedDB` on wasm), a restored list is adopted wholesale, and a failure
    /// becomes both a status line and the load-failure card.
    fn apply_web_open_event(&mut self, event: crate::webopen::WebOpenEvent) {
        use crate::webopen::WebOpenEvent;
        match event {
            WebOpenEvent::Progress(progress) => {
                self.load_progress = progress;
            }
            WebOpenEvent::Opened {
                bytes,
                format,
                recent,
            } => match self.open_document_bytes(&bytes, format) {
                Ok(()) => {
                    self.record_recent_file(recent);
                    self.load_progress = crate::webopen::LoadProgress::Idle;
                    self.persist_recent_files();
                }
                Err(e) => {
                    self.status.set(e.to_string());
                    self.load_progress = crate::webopen::LoadProgress::failed(e.to_string());
                }
            },
            WebOpenEvent::Failed(message) => {
                self.status.set(message.clone());
                self.load_progress = crate::webopen::LoadProgress::failed(message);
            }
            WebOpenEvent::RecentsLoaded(recents) => {
                // Adopt the persisted list, but keep anything opened this session ahead
                // of it by re-recording current entries on top (most-recent-first is
                // preserved because `record` moves each to the front).
                let session = self.recent_files.entries().to_vec();
                let mut merged = recents;
                for file in session.into_iter().rev() {
                    merged.record(file);
                }
                self.recent_files = merged;
            }
        }
    }

    /// Persists the recent-files list to `IndexedDB` (wasm only; a no-op on native).
    ///
    /// Spawned fire-and-forget so a slow or blocked write never stalls a frame; losing
    /// persistence is tolerable (the in-memory list is still correct for the session).
    /// On native this compiles to an empty body, so `self` is unused there by design.
    #[cfg_attr(not(target_arch = "wasm32"), allow(clippy::unused_self))]
    fn persist_recent_files(&self) {
        #[cfg(target_arch = "wasm32")]
        {
            let recents = self.recent_files.clone();
            wasm_bindgen_futures::spawn_local(async move {
                crate::webopen::store_recent_files(&recents).await;
            });
        }
    }

    /// Arms a one-shot full-window screenshot smoke test (native launcher only).
    ///
    /// Loads the bundled SKY130 cell into the editor and runs DRC so the captured
    /// frame shows the real panels populated, then installs a
    /// [`CaptureState`](crate::demoscript::CaptureState) that the frame loop drives:
    /// it screenshots the window once, writes it to `out_path`, and closes. This
    /// exists to prove the egui viewport-screenshot round trip on this wgpu backend
    /// before the full demo-script harness is built on top of it.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_screenshot_smoke(&mut self, out_path: std::path::PathBuf) {
        self.enter_use_case(crate::usecases::UseCase::InspectCell);
        self.run_drc();
        self.capture = Some(crate::demoscript::CaptureState::smoke(out_path));
    }

    /// Arms a scripted demo run (native launcher only), writing captured frames under
    /// `out_dir`.
    ///
    /// Dismisses the Start screen so the editor renders from the first frame; the
    /// script's own `use-case` step then loads the scenario it wants. The first-run
    /// tour and the cross-section prompt are suppressed while a demo runs (see
    /// `in_demo_capture`) so captures show only the feature under test.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_demo_script(
        &mut self,
        script: crate::demoscript::Script,
        out_dir: std::path::PathBuf,
    ) {
        self.start_screen = false;
        self.demo = Some(crate::demoscript::DemoRun::new(script, out_dir));
    }

    /// Whether a demo capture (scripted run or one-shot smoke) is in progress.
    ///
    /// Capture mode hides transient chrome (the first-run tour overlay and the empty
    /// cross-section prompt) so the media shows the feature, not onboarding.
    #[cfg(not(target_arch = "wasm32"))]
    fn in_demo_capture(&self) -> bool {
        self.demo.is_some() || self.capture.is_some()
    }

    /// On wasm there is no capture mode, so nothing is ever suppressed.
    #[cfg(target_arch = "wasm32")]
    fn in_demo_capture(&self) -> bool {
        false
    }

    /// Applies one instantaneous demo step to the editor.
    ///
    /// Each arm invokes the exact same code path the interactive UI uses, so a demo
    /// capture shows the real feature rather than a staged mock. Scheduler-only steps
    /// (`Wait`/`Capture`/`Snap`/`Orbit`) never reach here.
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::too_many_lines)] // one arm per demo verb reads better flat
    fn apply_demo_step(&mut self, step: &crate::demoscript::Step) {
        use crate::demoscript::Step;
        match step {
            Step::UseCase(use_case) => self.enter_use_case(*use_case),
            Step::RunDrc => self.run_drc(),
            Step::SelectViolation(index) => {
                let _ = self.drc.select(*index);
            }
            Step::ZoomViolation => self.zoom_to_selected_violation = true,
            Step::Select(indices) => {
                if let Some((first, rest)) = indices.split_first() {
                    self.selection.select_one(*first);
                    self.selection.extend(rest.iter().copied());
                }
            }
            Step::HighlightNet(index) => {
                let _ = self.netlight.highlight_shape(
                    self.history.document(),
                    &self.top_cell,
                    self.doc_generation,
                    *index,
                );
            }
            Step::View3d(open) => {
                self.demo_show_3d = *open;
                if *open {
                    self.view3d.reset();
                }
            }
            Step::Filter(query) => {
                self.demo_focus_search = true;
                self.search.query_text.clone_from(query);
                self.run_filter_query();
            }
            Step::OutlineLocate(path) => {
                self.demo_focus_search = true;
                let target = self
                    .search
                    .outline
                    .nodes()
                    .iter()
                    .find(|n| n.locate.is_some() && n.label.contains(path.as_str()))
                    .and_then(|n| n.locate);
                match target {
                    Some(rect) => self.search.pending_locate = Some(rect),
                    None => eprintln!("demo: outline-locate found no node matching `{path}`"),
                }
            }
            Step::AddPoly(points) => {
                let verts: Vec<Point> = points
                    .iter()
                    .map(|(x, y)| {
                        Point::new(
                            i32::try_from(*x).unwrap_or(0),
                            i32::try_from(*y).unwrap_or(0),
                        )
                    })
                    .collect();
                // Drawn on met1 (68/20) so it can boolean-union with the starter met1
                // geometry the build scenario seeds.
                let shape = DrawShape::new(
                    LayerId::new(68, 20),
                    ShapeKind::Polygon(reticle_geometry::Polygon::new(verts)),
                );
                if self
                    .history
                    .apply(reticle_model::Edit::AddShape {
                        cell: self.top_cell.clone(),
                        shape,
                    })
                    .is_ok()
                {
                    self.rebuild_scene();
                }
            }
            Step::VertexMove {
                shape,
                vertex,
                delta,
            } => {
                if let Some(scene_shape) = self.scene.shapes().get(*shape) {
                    let kind = scene_shape.kind.clone();
                    let (verts, _closed) = crate::draw::editable_vertices(&kind);
                    if let Some(v) = verts.get(*vertex) {
                        let to = Point::new(
                            v.x + i32::try_from(delta.0).unwrap_or(0),
                            v.y + i32::try_from(delta.1).unwrap_or(0),
                        );
                        let moved = crate::draw::move_vertex(&verts, *vertex, to);
                        self.replace_shape_vertices(*shape, moved, "Moved vertex");
                    }
                }
            }
            Step::Union => {
                // Mirror the interactive boolean path: collect the selection, build the
                // edits, then apply them as one group and rebuild derived state.
                let selection: Vec<usize> = self.selection.iter().collect();
                let editable = self.editable_shape_count();
                let top = self.top_cell.clone();
                let edits = crate::ops::boolean_edits(
                    crate::ops::BoolKind::Union,
                    self.scene.shapes(),
                    &selection,
                    &top,
                    editable,
                );
                if !edits.is_empty() && self.history.apply_group(edits).is_ok() {
                    self.rebuild_scene();
                }
            }
            Step::Array { cols, rows, pitch } => {
                let direct = self.selected_direct_shapes();
                if !direct.is_empty() {
                    let shapes: Vec<DrawShape> = direct.iter().map(|(_, s)| s.clone()).collect();
                    let pitch = i32::try_from(*pitch).unwrap_or(0);
                    let arrayed =
                        crate::productivity::array_shapes(&shapes, *rows, *cols, pitch, pitch);
                    // Element (0,0) reproduces the originals, which already exist, so
                    // only the remaining copies are added.
                    let top = self.top_cell.clone();
                    let edits: Vec<reticle_model::Edit> = arrayed
                        .into_iter()
                        .skip(shapes.len())
                        .map(|shape| reticle_model::Edit::AddShape {
                            cell: top.clone(),
                            shape,
                        })
                        .collect();
                    if !edits.is_empty() && self.history.apply_group(edits).is_ok() {
                        self.rebuild_scene();
                        // Reframe so the new array copies are on screen.
                        self.fit_requested = true;
                    }
                }
            }
            // Free camera nudges are not used by any committed script yet.
            Step::Zoom(_) | Step::Pan(..) => {
                eprintln!("demo: step {step:?} not yet implemented");
            }
            // Handled by the scheduler, never dispatched as an action.
            Step::Wait(_) | Step::Capture { .. } | Step::Snap(_) | Step::Orbit(..) => {}
        }
    }

    /// Advances the active demo run by one frame: applies the scheduler's next
    /// instruction, requesting and saving full-window screenshots as it captures.
    ///
    /// The run is taken out of `self` for the duration so the step dispatch can borrow
    /// the app freely, then put back unless the run finished.
    #[cfg(not(target_arch = "wasm32"))]
    fn drive_demo(&mut self, ctx: &egui::Context) {
        use crate::demoscript::Tick;
        let Some(mut demo) = self.demo.take() else {
            return;
        };
        match demo.next_tick() {
            Tick::Idle => {}
            Tick::Apply(step) => self.apply_demo_step(&step),
            Tick::Capture { orbit } => {
                if orbit.0.abs() > f32::EPSILON || orbit.1.abs() > f32::EPSILON {
                    self.view3d.drag(orbit.0, orbit.1);
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            Tick::Save => {
                let shot = ctx.input(|i| {
                    i.raw.events.iter().find_map(|e| match e {
                        egui::Event::Screenshot { image, .. } => Some(image.clone()),
                        _ => None,
                    })
                });
                match shot {
                    Some(image) => {
                        let frame = crate::demoscript::frame_from_color_image(&image);
                        demo.store_frame(&frame);
                    }
                    None => demo.miss(),
                }
            }
            Tick::Done => {
                match demo.write_manifest() {
                    Ok(path) => eprintln!(
                        "demo: captured {} frames; manifest {}",
                        demo.frame_count(),
                        path.display()
                    ),
                    Err(e) => eprintln!("demo: manifest write failed: {e}"),
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }
        self.demo = Some(demo);
    }

    /// The renderer (frozen Wave 0 contract accessor).
    #[must_use]
    pub fn renderer(&self) -> &WgpuRenderer {
        &self.renderer
    }

    /// The collaborative document (frozen Wave 0 contract accessor).
    #[must_use]
    pub fn document(&self) -> &SyncDocument {
        &self.document
    }

    /// Rebuilds the scene spatial index from the current document and top cell.
    ///
    /// Called after any edit so culling and hit-testing see the new geometry. The
    /// selection is cleared because shape indices are no longer valid.
    fn rebuild_scene(&mut self) {
        self.scene = SceneIndex::build(self.history.document(), &self.top_cell);
        self.selection.clear();
        // Shape indices are no longer valid: drop the index-based net highlight and
        // bump the revision so the net-extraction cache re-extracts on the next pick.
        self.netlight.clear();
        self.doc_generation = self.doc_generation.wrapping_add(1);
        // The cell hierarchy may have changed, so refresh the outline tree.
        self.search.outline = OutlineTree::build(self.history.document());
    }

    /// The technology database-units-per-micron for the current document.
    fn dbu_per_micron(&self) -> i64 {
        self.history.document().technology().dbu_per_micron
    }

    /// A stable hash of the current per-layer visibility bits.
    fn compute_visibility_sig(&self) -> u64 {
        // FNV-1a over each row's (id bits, visible) so any toggle changes the hash.
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for row in self.layer_state.rows() {
            let bit = u64::from(u32::from(row.id.layer) << 8 | u32::from(row.id.datatype)) << 1
                | u64::from(row.visible);
            hash = (hash ^ bit).wrapping_mul(0x0000_0100_0000_01B3);
        }
        hash
    }

    /// The token the retained GPU scene is keyed on: the document revision folded
    /// with the layer-visibility signature. Changes on any edit, undo/redo, or layer
    /// toggle, and only then does the renderer retessellate and re-upload.
    fn current_render_revision(&self) -> u64 {
        self.history.revision().rotate_left(1) ^ self.visibility_sig
    }

    /// Rebuilds the retained scene from the current document and visibility if
    /// anything the GPU depends on changed since the last rebuild. Retessellates
    /// every cell with the visibility-aware palette (invisible layers dropped) and
    /// re-expands the instance list, then records the new revision.
    ///
    /// This runs at most once per real change; a plain camera move leaves the
    /// revision untouched, so it is a no-op and the GPU buffers are reused.
    fn sync_retained(&mut self) {
        self.visibility_sig = self.compute_visibility_sig();
        let revision = self.current_render_revision();
        if revision == self.render_revision && self.retained.top_cell() == self.top_cell {
            return; // nothing the GPU cares about changed
        }
        let palette = palette_from_layers(&self.layer_state);
        let names: Vec<String> = self
            .history
            .document()
            .cells()
            .map(|c| c.name.clone())
            .collect();
        self.retained.set_top_cell(&self.top_cell);
        for name in &names {
            self.retained.mark_dirty(name);
        }
        self.retained.rebuild(self.history.document(), &palette);
        self.expanded = Arc::new(self.retained.expand());
        self.render_revision = revision;
    }

    /// Runs a command-palette [`Command`], mutating the relevant app state.
    ///
    /// Centralizing execution here means the toolbar, keyboard shortcuts, and the
    /// palette all funnel through the same effects.
    fn run_command(&mut self, cmd: Command, screen: Option<ScreenRect>) {
        match cmd {
            Command::SetTool(tool) => {
                self.select_tool(tool);
                self.status.set(format!("Tool: {}", tool.label()));
            }
            Command::ToggleLayer(i) => {
                if let Some(row) = self.layer_state.rows().get(i) {
                    let id = row.id;
                    if let Some(now) = self.layer_state.toggle(id) {
                        self.status.set(format!(
                            "{} {}",
                            row_name(&self.layer_state, i),
                            on_off(now)
                        ));
                    }
                }
            }
            Command::Undo => {
                if self.history.undo() {
                    self.rebuild_scene();
                    self.status.set("Undo");
                } else {
                    self.status.set("Nothing to undo");
                }
            }
            Command::Redo => {
                if self.history.redo() {
                    self.rebuild_scene();
                    self.status.set("Redo");
                } else {
                    self.status.set("Nothing to redo");
                }
            }
            Command::ZoomToFit => {
                self.fit_requested = true;
                self.status.set("Zoom to fit");
            }
            Command::ToggleGrid => {
                self.grid.visible = !self.grid.visible;
                self.status
                    .set(format!("Grid {}", on_off(self.grid.visible)));
            }
            Command::ToggleSnap => {
                self.grid.snap_enabled = !self.grid.snap_enabled;
                self.status
                    .set(format!("Snap {}", on_off(self.grid.snap_enabled)));
            }
            Command::ClearSelection => {
                self.selection.clear();
                self.status.set("Selection cleared");
            }
            Command::SelectLayer(i) => {
                if let Some(row) = self.layer_state.rows().get(i) {
                    let id = row.id;
                    let hits = selection::shapes_on_layer(self.scene.shapes(), id);
                    let n = hits.len();
                    self.selection.set(hits);
                    self.status.set(format!("Selected {n} shape(s) on layer"));
                }
            }
            Command::ExportPng => self.export_png(screen),
        }
    }

    /// Exports the current view to a PNG next to the working directory (native).
    ///
    /// Uses the offscreen GPU renderer at the canvas resolution; if no GPU is
    /// available it sets a status message instead of failing. On the web this is a
    /// no-op (the palette does not offer it there).
    #[cfg(not(target_arch = "wasm32"))]
    fn export_png(&mut self, screen: Option<ScreenRect>) {
        let (w, h) = screen.map_or((1024, 768), |s| {
            (s.width.max(16.0) as u32, s.height.max(16.0) as u32)
        });
        let camera = screen.map_or_else(
            || reticle_model::Camera {
                center: self.camera.center(),
                pixels_per_dbu: self.camera.pixels_per_dbu() as f32,
                viewport: Rect::default(),
            },
            |s| self.camera.to_model_camera(&s),
        );
        match reticle_render::WgpuContext::new_blocking() {
            Some(ctx) => {
                let rgba = self.renderer.render_document_offscreen(
                    &ctx,
                    self.history.document(),
                    &self.top_cell,
                    &camera,
                    (w, h),
                );
                match write_png("reticle-export.png", w, h, &rgba) {
                    Ok(path) => self.status.set(format!("Exported {path}")),
                    Err(e) => self.status.set(format!("Export failed: {e}")),
                }
            }
            None => self.status.set("No GPU available; PNG export skipped"),
        }
    }

    /// PNG export is unavailable on the web; this stub keeps the call site uniform.
    #[cfg(target_arch = "wasm32")]
    #[allow(clippy::unused_self)]
    fn export_png(&mut self, _screen: Option<ScreenRect>) {
        self.status.set("PNG export is native-only");
    }

    /// Handles global keyboard shortcuts by resolving every key press through the
    /// rebindable [`Keymap`] (chords match modifiers exactly).
    ///
    /// Shortcuts are ignored while a text field has focus so typing in the palette
    /// or query bar does not trigger them. While the shortcuts editor is capturing
    /// a chord, the next key press rebinds the pending action instead of running
    /// anything (Escape cancels the capture); Escape otherwise closes the palette.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // Suppress shortcuts while a text field owns keyboard focus so typing in the
        // palette or query bar does not trigger tool changes.
        if ctx.memory(|m| m.focused().is_some()) {
            // Still allow Escape to close the palette even while its field has focus.
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.palette_open = false;
            }
            return;
        }
        let chords = pressed_chords(ctx);

        // Chord capture for the shortcuts editor: the next press rebinds.
        if let Some(action) = self.rebinding {
            if let Some(new_chord) = chords.into_iter().next() {
                self.rebinding = None;
                if new_chord.key == "Escape" {
                    self.status.set("Rebind canceled");
                } else {
                    let shown = new_chord.to_string();
                    let stolen = self.keymap.bind(action, Some(new_chord));
                    match stolen.first() {
                        Some(loser) => self.status.set(format!(
                            "{} bound to {shown}; {} is now unbound",
                            action.label(),
                            loser.label()
                        )),
                        None => self
                            .status
                            .set(format!("{} bound to {shown}", action.label())),
                    }
                }
            }
            return;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.palette_open = false;
        }
        for chord in chords {
            if let Some(action) = self.keymap.action_for(&chord) {
                self.run_action(action);
            }
        }
    }

    /// Runs a rebindable [`keymap::Action`], funneling through
    /// [`App::run_command`] wherever a palette command exists so shortcuts, the
    /// toolbar, and the palette share one set of effects.
    fn run_action(&mut self, action: keymap::Action) {
        match action {
            keymap::Action::OpenPalette => {
                self.palette_open = !self.palette_open;
                self.palette_query.clear();
            }
            keymap::Action::Undo => self.run_command(Command::Undo, None),
            keymap::Action::Redo => self.run_command(Command::Redo, None),
            keymap::Action::ZoomToFit => self.run_command(Command::ZoomToFit, None),
            keymap::Action::ToggleGrid => self.run_command(Command::ToggleGrid, None),
            keymap::Action::ToggleSnap => self.run_command(Command::ToggleSnap, None),
            keymap::Action::ToolSelect => self.run_command(Command::SetTool(Tool::Select), None),
            keymap::Action::ToolPan => self.run_command(Command::SetTool(Tool::Pan), None),
            keymap::Action::ToolMeasure => self.run_command(Command::SetTool(Tool::Measure), None),
            keymap::Action::ToggleLabels => {
                self.labels_visible = !self.labels_visible;
                self.status
                    .set(format!("Labels {}", on_off(self.labels_visible)));
            }
            keymap::Action::ToggleMinimap => {
                self.minimap_visible = !self.minimap_visible;
                self.status
                    .set(format!("Minimap {}", on_off(self.minimap_visible)));
            }
            keymap::Action::SplitSingle => self.set_split(Split::Single),
            keymap::Action::SplitHorizontal => self.set_split(Split::Horizontal),
            keymap::Action::SplitVertical => self.set_split(Split::Vertical),
        }
    }

    /// Applies a pane split and reports it in the status bar.
    fn set_split(&mut self, split: Split) {
        self.viewports.set_split(split, &self.camera);
        self.status.set(format!("View: {}", split.label()));
    }

    /// Draws the top toolbar: tool buttons, view actions, and the palette hint.
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Tool:");
            for tool in Tool::all() {
                let selected = self.tools.active() == tool;
                if ui.selectable_label(selected, tool.label()).clicked() {
                    self.select_tool(tool);
                }
            }
            // Path tool options: width and end cap, shown only while it is active.
            if self.tools.active() == Tool::DrawPath {
                ui.separator();
                ui.label("Width:");
                let mut w = self.draw.path.width();
                if ui
                    .add(egui::DragValue::new(&mut w).speed(5.0).range(1..=100_000))
                    .changed()
                {
                    self.draw.path.set_width(w);
                }
                let cap = self.draw.path.endcap();
                for (variant, name) in [
                    (Endcap::Flat, "Flat"),
                    (Endcap::Square, "Square"),
                    (Endcap::Round, "Round"),
                ] {
                    if ui.selectable_label(cap == variant, name).clicked() {
                        self.draw.path.set_endcap(variant);
                    }
                }
            }
            ui.separator();
            // The open affordance the first-run tour points at. It returns to the
            // Start screen, which holds the file-open guidance, the drag-and-drop
            // target, the example-chip gallery, and recent files, so opening a design
            // works the same on native and on the web (where there is no filesystem
            // dialog). Dragging a file onto the window opens it directly (see
            // `handle_dropped_files`).
            if ui
                .button("Open")
                .on_hover_text("Open a design, or drag a GDSII/OASIS file onto the window")
                .clicked()
            {
                self.start_screen = true;
            }
            if ui.button("Fit").clicked() {
                self.fit_requested = true;
            }
            let can_undo = self.history.can_undo();
            let can_redo = self.history.can_redo();
            if ui
                .add_enabled(can_undo, egui::Button::new("Undo"))
                .clicked()
            {
                self.run_command(Command::Undo, None);
            }
            if ui
                .add_enabled(can_redo, egui::Button::new("Redo"))
                .clicked()
            {
                self.run_command(Command::Redo, None);
            }
            ui.separator();
            ui.checkbox(&mut self.grid.visible, "Grid");
            ui.checkbox(&mut self.grid.snap_enabled, "Snap");
            ui.checkbox(&mut self.labels_visible, "Labels");
            ui.checkbox(&mut self.minimap_visible, "Minimap");
            ui.separator();
            for split in Split::all() {
                let selected = self.viewports.split() == split;
                if ui.selectable_label(selected, split.label()).clicked() {
                    self.set_split(split);
                }
            }
            ui.separator();
            let palette_label = self
                .keymap
                .chord_for(keymap::Action::OpenPalette)
                .map_or_else(|| "Palette".to_owned(), |c| format!("Palette ({c})"));
            if ui.button(palette_label).clicked() {
                self.palette_open = !self.palette_open;
                self.palette_query.clear();
            }
            if ui.button("Shortcuts").clicked() {
                self.keymap_open = !self.keymap_open;
            }
            self.help_menu(ui);
        });
    }

    /// The Help menu: relaunches the first-run tour.
    ///
    /// "Take the tour" replays the core walkthrough plus the optional Wave 2
    /// chapter; "Core tour only" replays just the core chapter. Either way the tour
    /// restarts from the first step (see [`crate::tour::Tour::relaunch`]).
    fn help_menu(&mut self, ui: &mut egui::Ui) {
        // egui closes the menu automatically once a button inside it is clicked.
        ui.menu_button("Help", |ui| {
            if ui.button("Take the tour").clicked() {
                self.tour.relaunch(true);
                self.status.set("Tour started");
            }
            if ui.button("Core tour only").clicked() {
                self.tour.relaunch(false);
                self.status.set("Core tour started");
            }
        });
    }

    /// Whether the tour counts as seen for persistence.
    ///
    /// It is seen unless it is an automatic first-run that has not finished yet, so
    /// a completed or dismissed first run, and any relaunch, persist `tour_seen =
    /// true`. That is what stops the automatic tour from ever showing twice.
    ///
    /// Native-only: the only caller is [`eframe::App::save`], which is itself gated
    /// off on wasm (there is no session file to write there).
    #[cfg(not(target_arch = "wasm32"))]
    fn tour_seen(&self) -> bool {
        // Seen unless it is an unfinished first run.
        !self.tour.is_first_run() || self.tour.is_finished()
    }

    /// Draws the tour overlay for the current step, if the tour is running.
    ///
    /// The overlay is a centered card with the step's title, body, a progress
    /// readout, and Next/Skip/Close buttons, plus a highlight box drawn around the
    /// step's target region. It is a no-op when the tour is idle or finished.
    ///
    /// `targets` maps each [`TourTarget`] to the on-screen rectangle the app
    /// measured this frame (a panel rect or the canvas rect); a target with no known
    /// rectangle simply draws no highlight box. Keeping the rectangles out here means
    /// the tour never depends on exact pixel coordinates.
    fn tour_overlay(&mut self, ctx: &egui::Context, targets: &TourTargets) {
        let Some(step) = self.tour.current().copied() else {
            return;
        };

        // Draw the highlight box around the step's target, if its rectangle is known
        // this frame. A bright stroke on the foreground layer, so it sits over the
        // panels without stealing input.
        if let Some(rect) = targets.rect_for(step.target) {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("tour_highlight"),
            ));
            painter.rect_stroke(
                rect,
                4.0,
                Stroke::new(3.0, Color32::from_rgb(255, 196, 0)),
                StrokeKind::Outside,
            );
        }

        // The instruction card. A fixed-width area near the bottom-center so it does
        // not cover the panel it points at.
        let (idx, total) = self.tour.progress().unwrap_or((0, 0));
        let chapter = step.chapter.label();
        let mut action: Option<TourAction> = None;
        egui::Area::new(egui::Id::new("tour_card"))
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -32.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_max_width(360.0);
                        ui.horizontal(|ui| {
                            ui.strong(step.title);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.weak(format!("{chapter} - {idx}/{total}"));
                                },
                            );
                        });
                        ui.add_space(4.0);
                        ui.label(step.body);
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            let last = idx == total;
                            let next_label = if last { "Done" } else { "Next" };
                            if ui.button(next_label).clicked() {
                                action = Some(TourAction::Next);
                            }
                            if ui.button("Skip").clicked() {
                                action = Some(TourAction::Skip);
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("Close").clicked() {
                                        action = Some(TourAction::Close);
                                    }
                                },
                            );
                        });
                    });
            });

        match action {
            Some(TourAction::Next) => self.tour.next(),
            Some(TourAction::Skip) => self.tour.skip(),
            Some(TourAction::Close) => self.tour.finish(),
            None => {}
        }
    }

    /// Draws the docked toolbar, status bar, side panels, and central canvas.
    ///
    /// Returns the measured canvas rectangle (for the palette/export next frame) and
    /// the [`TourTargets`] rectangles the running tour highlights. Each panel reports
    /// its rectangle straight from `Panel::show`, so a highlight tracks the real
    /// layout even after a resize; the minimap target is the actual minimap panel
    /// when it is drawn, falling back to the canvas otherwise.
    fn main_panels(
        &mut self,
        ui: &mut egui::Ui,
        gpu_format: Option<eframe::egui_wgpu::wgpu::TextureFormat>,
    ) -> (Option<ScreenRect>, TourTargets) {
        let toolbar = egui::Panel::top("toolbar")
            .show(ui, |ui| self.toolbar(ui))
            .response
            .rect;
        egui::Panel::bottom("status").show(ui, |ui| {
            self.status_bar(ui);
        });
        let layers = egui::Panel::left("layers")
            .resizable(true)
            .default_size(210.0)
            .show(ui, |ui| self.layer_panel(ui))
            .response
            .rect;
        let right_column = egui::Panel::right("history")
            .resizable(true)
            .default_size(240.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.history_panel(ui);
                        ui.separator();
                        self.inspector_panel(ui);
                        ui.separator();
                        self.drc_panel(ui);
                        ui.separator();
                        self.agent_section(ui);
                        ui.separator();
                        self.share_section(ui);
                        ui.separator();
                        self.ops_panel(ui);
                        self.productivity_panel(ui);
                        ui.separator();
                        self.generate_section(ui);
                        self.snap_panel(ui);
                        ui.separator();
                        self.view_export_panel(ui);
                        // During the query tour, scroll the column so the filter bar
                        // and outline tree (otherwise below the fold) are on screen.
                        #[cfg(not(target_arch = "wasm32"))]
                        if self.demo_focus_search {
                            ui.scroll_to_cursor(Some(egui::Align::TOP));
                        }
                        self.search_panel(ui);
                        ui.separator();
                        self.tech_editor_panel(ui);
                    });
            })
            .response
            .rect;
        let mut canvas_screen: Option<ScreenRect> = None;
        egui::CentralPanel::default().show(ui, |ui| {
            canvas_screen = Some(self.canvas(ui, gpu_format));
        });

        // The minimap rides in the canvas's top-right; highlight the real panel when
        // it is drawn and the scene has bounds, else fall back to the canvas.
        let minimap = canvas_screen.and_then(|screen| {
            if self.minimap_visible
                && let Some(bounds) = self.scene.bounds()
                && let Some(layout) = MinimapLayout::compute(&screen, bounds)
            {
                Some(egui_rect_of(&layout.panel))
            } else {
                None
            }
        });

        let targets = TourTargets {
            canvas: canvas_screen.map(|s| egui_rect_of(&s)),
            layers: Some(layers),
            toolbar: Some(toolbar),
            right_column: Some(right_column),
            minimap,
        };
        (canvas_screen, targets)
    }
    /// A click the Start screen recorded this frame, applied after the layout closure
    /// so the borrow of `self` inside the egui closure is released first.
    ///
    /// Collecting the intent and acting on it afterwards is the same pattern the tour
    /// overlay uses; it lets each Start-screen section be a plain closure over `ui`
    /// without also borrowing `self` mutably to run the action inline.
    fn start_screen_ui(&mut self, ui: &mut egui::Ui) {
        let mut action: Option<StartAction> = None;
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| {
                        ui.heading("Welcome to Reticle");
                        ui.label(
                            "Open your own design, load an example chip, or pick a \
                             worked scenario. You can skip to a blank editor anytime.",
                        );
                    });
                    ui.add_space(12.0);

                    Self::start_open_section(ui, &mut action);
                    ui.add_space(10.0);
                    Self::start_recent_section(ui, self.recent_files.entries(), &mut action);
                    ui.add_space(10.0);
                    Self::start_gallery_section(ui, &mut action);
                    ui.add_space(10.0);
                    Self::start_scenarios_section(ui, &mut action);

                    ui.add_space(10.0);
                    ui.vertical_centered(|ui| {
                        if ui.link("Skip to the editor").clicked() {
                            action = Some(StartAction::SkipToEditor);
                        }
                    });
                    ui.add_space(12.0);
                });
        });
        self.apply_start_action(action);
    }

    /// Draws the "open a file" section: a prominent drag-and-drop target hint and the
    /// supported formats. There is no synchronous file dialog on the web, so the
    /// honest primary affordance is drag-and-drop, which
    /// [`handle_dropped_files`](Self::handle_dropped_files) opens directly; this
    /// section makes that target and the accepted formats visible.
    fn start_open_section(ui: &mut egui::Ui, _action: &mut Option<StartAction>) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong("Open a file");
            ui.add_space(4.0);
            // The drag-drop target hint: a dashed-feeling framed area with the call to
            // action. Dropping anywhere on the window works; this names where and what.
            egui::Frame::group(ui.style())
                .fill(ui.visuals().faint_bg_color)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.vertical_centered(|ui| {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new("Drag a layout file here to open it").strong(),
                        );
                        ui.label(
                            egui::RichText::new("or drop it anywhere on the window")
                                .weak()
                                .small(),
                        );
                        ui.add_space(6.0);
                    });
                });
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("Supported: GDSII (.gds) and OASIS (.oas)")
                    .weak()
                    .small(),
            );
        });
    }

    /// Draws the recent-files section from the app's `recent_files` list.
    ///
    /// This app only renders the list; a persistence backend feeds it (see
    /// [`set_recent_files`](Self::set_recent_files)). When empty it shows a short
    /// placeholder rather than an empty box, so the section still reads as intentional
    /// on a first run.
    fn start_recent_section(
        ui: &mut egui::Ui,
        recent: &[crate::webopen::RecentFile],
        _action: &mut Option<StartAction>,
    ) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong("Recent files");
            ui.add_space(4.0);
            if recent.is_empty() {
                ui.label(
                    egui::RichText::new("Files you open will appear here.")
                        .weak()
                        .small(),
                );
            } else {
                for r in recent {
                    // Integer-only size formatting keeps clippy's precision-loss lint
                    // quiet; a sub-KiB file simply reads as "0 KiB".
                    let size = if r.size >= 1024 * 1024 {
                        format!("{} MiB", r.size / (1024 * 1024))
                    } else {
                        format!("{} KiB", r.size / 1024)
                    };
                    ui.horizontal(|ui| {
                        ui.monospace(&r.name);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.weak(size);
                        });
                    });
                }
            }
        });
    }

    /// Draws the example-chip gallery: one card per bundled
    /// [`ExampleChip`](crate::startscreen::ExampleChip), each with a Load button that
    /// opens the compiled-in design through the seam.
    fn start_gallery_section(ui: &mut egui::Ui, action: &mut Option<StartAction>) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong("Load an example chip");
            ui.label(
                egui::RichText::new("Redistribution-cleared real designs, built in.")
                    .weak()
                    .small(),
            );
            ui.add_space(4.0);
            for chip in crate::startscreen::ExampleChip::ALL {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.strong(chip.title());
                            ui.label(chip.description());
                            ui.label(egui::RichText::new(chip.attribution()).weak().small());
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Load").clicked() {
                                *action = Some(StartAction::LoadExample(chip));
                            }
                        });
                    });
                });
                ui.add_space(4.0);
            }
        });
    }

    /// Draws the worked-scenario cards (title, one-line description, Start
    /// button), one per [`UseCase`] the Start screen offers.
    fn start_scenarios_section(ui: &mut egui::Ui, action: &mut Option<StartAction>) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong("Worked scenarios");
            ui.label(
                egui::RichText::new("Drop straight into one capability with a prepared start.")
                    .weak()
                    .small(),
            );
            ui.add_space(4.0);
            for use_case in UseCase::ALL {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.strong(use_case.title());
                            ui.label(use_case.description());
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Start").clicked() {
                                *action = Some(StartAction::EnterUseCase(use_case));
                            }
                        });
                    });
                });
                ui.add_space(4.0);
            }
        });
    }

    /// Applies the [`StartAction`] a Start-screen section recorded this frame.
    ///
    /// Split from the layout closure so the mutable-`self` work (installing a
    /// document, opening the theater, dismissing the screen) runs after the immutable
    /// borrow inside the egui closure has ended.
    fn apply_start_action(&mut self, action: Option<StartAction>) {
        match action {
            Some(StartAction::EnterUseCase(use_case)) => self.enter_use_case(use_case),
            Some(StartAction::LoadExample(chip)) => {
                self.open_example_chip(chip);
            }
            Some(StartAction::SkipToEditor) => {
                // Dismiss the chooser and keep the demo document already loaded.
                self.start_screen = false;
                self.status.set("Editor");
            }
            None => {}
        }
    }

    /// Draws the left layer-manager panel: filter, per-layer swatch + visibility.
    fn layer_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Layers");
        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.text_edit_singleline(self.layer_state.filter_mut());
        });
        ui.horizontal(|ui| {
            if ui.small_button("Show all").clicked() {
                self.layer_state.show_all();
            }
            if ui.small_button("Hide all").clicked() {
                self.layer_state.hide_all();
            }
        });
        ui.separator();

        let indices = self.layer_state.filtered_indices();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let rows = self.layer_state.rows_mut();
                for i in indices {
                    let row = &mut rows[i];
                    ui.horizontal(|ui| {
                        let (r, g, b, _) = layers::rgba_components(row.color_rgba);
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(14.0, 14.0), Sense::hover());
                        ui.painter()
                            .rect_filled(rect, 2.0, Color32::from_rgb(r, g, b));
                        ui.checkbox(&mut row.visible, &row.name);
                    });
                }
            });

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Select layer:");
            ui.text_edit_singleline(&mut self.layer_query);
        });
        if ui.button("Select by layer name").clicked() {
            self.select_by_layer_name();
        }
    }

    /// Selects every shape whose layer name matches the query bar (case-insensitive
    /// substring); an empty query selects nothing.
    fn select_by_layer_name(&mut self) {
        let q = self.layer_query.trim().to_lowercase();
        if q.is_empty() {
            self.status.set("Enter a layer name to select");
            return;
        }
        let ids: Vec<LayerId> = self
            .layer_state
            .rows()
            .iter()
            .filter(|r| r.name.to_lowercase().contains(&q))
            .map(|r| r.id)
            .collect();
        let mut hits = Vec::new();
        for id in ids {
            hits.extend(selection::shapes_on_layer(self.scene.shapes(), id));
        }
        let n = hits.len();
        self.selection.set(hits);
        self.status.set(format!("Selected {n} shape(s)"));
    }

    /// Builds a name -> [`LayerId`] lookup from the layer table for query
    /// evaluation (resolving `layer:METAL1` to a concrete layer id).
    fn layer_lookup(&self) -> LayerLookup {
        LayerLookup::new(
            self.layer_state
                .rows()
                .iter()
                .map(|r| (r.name.clone(), r.id)),
        )
    }

    /// The search / selection-depth panel: the filter query bar, saved selection
    /// sets, select-similar, and the cell/instance outline tree.
    ///
    /// All the logic lives in [`crate::query`] and [`crate::outline`]; this method
    /// only draws the widgets and forwards the results into the live selection and
    /// the deferred camera locate.
    fn search_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Search");

        // --- Filter query bar ------------------------------------------------
        ui.label("Filter (e.g. layer:METAL1 width<400 area>1000):");
        let query_response = ui.text_edit_singleline(&mut self.search.query_text);
        let run = ui.button("Select matching").clicked()
            || (query_response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        if run {
            self.run_filter_query();
        }
        if !self.search.error.is_empty() {
            ui.colored_label(Color32::from_rgb(230, 120, 120), &self.search.error);
        }

        ui.separator();

        // --- Select similar --------------------------------------------------
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.selection.is_empty(),
                    egui::Button::new("Select similar"),
                )
                .clicked()
            {
                self.select_similar();
            }
            ui.label("same layer + size");
        });

        ui.separator();

        // --- Saved selection sets --------------------------------------------
        ui.label("Selection sets:");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.search.set_name);
            if ui
                .add_enabled(!self.selection.is_empty(), egui::Button::new("Save"))
                .clicked()
            {
                self.save_selection_set();
            }
        });
        let mut restore: Option<String> = None;
        let mut remove: Option<String> = None;
        for set in self.search.saved.sets() {
            ui.horizontal(|ui| {
                if ui
                    .button(format!("{} ({})", set.name, set.indices.len()))
                    .clicked()
                {
                    restore = Some(set.name.clone());
                }
                if ui.small_button("x").clicked() {
                    remove = Some(set.name.clone());
                }
            });
        }
        if let Some(name) = restore {
            self.restore_selection_set(&name);
        }
        if let Some(name) = remove {
            self.search.saved.remove(&name);
            self.status.set(format!("Removed set '{name}'"));
        }

        ui.separator();

        // --- Outline / hierarchy tree ----------------------------------------
        ui.label("Outline (click to locate):");
        let mut locate: Option<Rect> = None;
        egui::ScrollArea::vertical()
            .max_height(180.0)
            .auto_shrink([false, false])
            .id_salt("outline_tree")
            .show(ui, |ui| {
                for node in self.search.outline.nodes() {
                    let indent = "    ".repeat(node.depth);
                    let text = format!("{indent}{}", node.label);
                    let response =
                        ui.add_enabled(node.locate.is_some(), egui::Button::new(text).frame(false));
                    if response.clicked() {
                        locate = node.locate;
                    }
                }
            });
        if let Some(target) = locate {
            // Framed on the next canvas pass, once the screen rect is known.
            self.search.pending_locate = Some(target);
            self.status.set("Located");
        }
    }

    /// Parses the filter query bar and selects every matching shape, replacing the
    /// current selection. A parse error is shown under the bar and leaves the
    /// selection untouched.
    fn run_filter_query(&mut self) {
        match Query::parse(&self.search.query_text) {
            Ok(query) => {
                self.search.error.clear();
                if query.is_empty() {
                    self.status.set("Enter a filter to select");
                    return;
                }
                let lookup = self.layer_lookup();
                let hits = query.select(self.scene.shapes(), &lookup, &self.top_cell);
                let n = hits.len();
                self.selection.set(hits);
                self.status.set(format!("Filter selected {n} shape(s)"));
            }
            Err(e) => {
                self.search.error = e.to_string();
            }
        }
    }

    /// Grows the current selection by adding shapes on the same layer and of a
    /// similar size to any already-selected shape (see [`outline::select_similar`]).
    fn select_similar(&mut self) {
        if self.selection.is_empty() {
            self.status.set("Select a shape first");
            return;
        }
        let seed: std::collections::BTreeSet<usize> = self.selection.iter().collect();
        let before = seed.len();
        let grown = outline::select_similar(
            self.scene.shapes(),
            &seed,
            outline::DEFAULT_SIMILAR_TOLERANCE,
        );
        let added = grown.len().saturating_sub(before);
        self.selection.set(grown);
        self.status
            .set(format!("Select similar added {added} shape(s)"));
    }

    /// Saves the current selection under the name in the set-name field.
    fn save_selection_set(&mut self) {
        let name = self.search.set_name.clone();
        if self.search.saved.save(&name, self.selection.iter()) {
            self.status.set(format!(
                "Saved {} shape(s) as '{}'",
                self.selection.len(),
                name.trim()
            ));
        } else {
            self.status.set("Enter a name for the selection set");
        }
    }

    /// Restores a saved selection set into the live selection by name.
    fn restore_selection_set(&mut self, name: &str) {
        if let Some(indices) = self.search.saved.restore(name) {
            let indices = indices.to_vec();
            let n = indices.len();
            self.selection.set(indices);
            self.status.set(format!("Restored '{name}' ({n} shape(s))"));
        }
    }
    /// Draws the technology-editor panel and the upgraded layer manager at the end
    /// of the right panel. Delegates to [`crate::tech_editor::show`] with the three
    /// disjoint app fields it needs (its own state, the history, and the layer
    /// table); borrowing them as separate fields here keeps a single call site.
    fn tech_editor_panel(&mut self, ui: &mut egui::Ui) {
        crate::tech_editor::show(
            &mut self.tech_editor,
            &mut self.history,
            &mut self.layer_state,
            ui,
        );
    }

    /// Draws the right-hand undo-history panel: stack depths and step buttons.
    fn history_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("History");
        ui.label(format!("Undo stack: {}", self.history.undo_depth()));
        ui.label(format!("Redo stack: {}", self.history.redo_depth()));
        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.history.can_undo(), egui::Button::new("Step back"))
                .clicked()
            {
                self.run_command(Command::Undo, None);
            }
            if ui
                .add_enabled(self.history.can_redo(), egui::Button::new("Step fwd"))
                .clicked()
            {
                self.run_command(Command::Redo, None);
            }
        });
        ui.separator();
        ui.label(format!("Selected shapes: {}", self.selection.len()));
        ui.label(format!("Scene shapes: {}", self.scene.len()));
        if ui.button("Add demo rectangle").clicked() {
            self.add_demo_rectangle();
        }
    }

    /// Appends a rectangle to the top cell through the undo history, then rebuilds
    /// the scene, a concrete edit so undo/redo and the history panel are exercised
    /// from the UI.
    fn add_demo_rectangle(&mut self) {
        // Place it near the current view center so it is visible.
        let c = self.camera.center();
        let rect = Rect::new(
            c,
            Point::new(c.x.saturating_add(800), c.y.saturating_add(800)),
        );
        let layer = self
            .layer_state
            .rows()
            .first()
            .map_or(LayerId::new(1, 0), |r| r.id);
        let shape = DrawShape::new(layer, ShapeKind::Rect(rect));
        match self.history.apply(reticle_model::Edit::AddShape {
            cell: self.top_cell.clone(),
            shape,
        }) {
            Ok(()) => {
                self.rebuild_scene();
                self.status.set("Added rectangle");
            }
            Err(e) => self.status.set(format!("Edit failed: {e}")),
        }
    }

    /// The number of directly-editable shapes in the current top cell.
    ///
    /// Scene indices below this count map one-to-one onto the top cell's own shapes,
    /// which is what the operations builders need to turn a selection into edits.
    fn editable_shape_count(&self) -> usize {
        crate::ops::editable_shape_count(self.history.document(), &self.top_cell)
    }

    /// Runs one operations-panel action: `build` turns the current selection into a
    /// batch of edits, which are applied as a single undo step and the scene rebuilt.
    ///
    /// `build` receives the flattened scene shapes, the selected indices (ascending),
    /// the top-cell name, and the editable-shape count. When it returns no edits (the
    /// operation did not apply to this selection) the document is left untouched and a
    /// short note is shown; `label` names the operation in the status line.
    pub(crate) fn run_ops<F>(&mut self, label: &str, build: F)
    where
        F: FnOnce(&[DrawShape], &[usize], &str, usize) -> Vec<reticle_model::Edit>,
    {
        // Collect the edits first so every borrow of `self.scene`/`self.selection`
        // ends before we mutate through `self.history`.
        let selection: Vec<usize> = self.selection.iter().collect();
        let editable = self.editable_shape_count();
        let top = self.top_cell.clone();
        let edits = build(self.scene.shapes(), &selection, &top, editable);

        if edits.is_empty() {
            self.ops.status = format!("{label}: nothing to do for this selection");
            self.status.set(self.ops.status.clone());
            return;
        }
        let added = edits
            .iter()
            .filter(|e| matches!(e, reticle_model::Edit::AddShape { .. }))
            .count();
        match self.history.apply_group(edits) {
            Ok(()) => {
                self.rebuild_scene();
                self.ops.status = format!("{label}: {added} shape(s) produced");
                self.status.set(self.ops.status.clone());
            }
            Err(e) => {
                self.ops.status = format!("{label} failed: {e}");
                self.status.set(self.ops.status.clone());
            }
        }
    }
    /// The number of shapes drawn directly in the current top cell (not brought in
    /// by an instance or array).
    ///
    /// The flattened scene lists these direct shapes first, in cell order, so a
    /// selection index below this count maps one-to-one to the top cell's
    /// `shapes` vector. Instanced and arrayed geometry occupies the indices beyond
    /// it and cannot be edited in place through [`reticle_model::Edit::RemoveShape`],
    /// which addresses a cell's own shape list.
    fn top_cell_direct_shape_count(&self) -> usize {
        self.history
            .document()
            .cell(&self.top_cell)
            .map_or(0, |cell| cell.shapes.len())
    }

    /// The selected shapes that live directly in the top cell, as `(direct_index,
    /// shape)` pairs sorted by index.
    ///
    /// Selection indices are into the flattened scene; only those below
    /// [`top_cell_direct_shape_count`](Self::top_cell_direct_shape_count) name a
    /// directly-owned shape and so are the only ones a cut or move can act on. The
    /// returned shapes are cloned from the live document so callers can translate and
    /// re-add them.
    fn selected_direct_shapes(&self) -> Vec<(usize, DrawShape)> {
        let direct = self.top_cell_direct_shape_count();
        let Some(cell) = self.history.document().cell(&self.top_cell) else {
            return Vec::new();
        };
        let mut picked: Vec<(usize, DrawShape)> = self
            .selection
            .iter()
            .filter(|&i| i < direct)
            .map(|i| (i, cell.shapes[i].clone()))
            .collect();
        picked.sort_by_key(|(i, _)| *i);
        picked
    }

    /// The resolved [`DrawShape`]s currently selected, read from the flattened scene.
    ///
    /// Unlike [`selected_direct_shapes`](Self::selected_direct_shapes) this includes
    /// instanced and arrayed geometry, because copy only reads geometry and never
    /// needs to address the source cell's shape list.
    fn selected_scene_shapes(&self) -> Vec<DrawShape> {
        let shapes = self.scene.shapes();
        self.selection
            .iter()
            .filter_map(|i| shapes.get(i).cloned())
            .collect()
    }

    /// Adds every shape in `shapes` to the top cell through the undo history, then
    /// rebuilds the scene once. Returns the number added.
    ///
    /// Each shape is a separate [`reticle_model::Edit::AddShape`], so each is
    /// individually undoable; the scene and derived caches rebuild a single time at
    /// the end. On the first failing edit it stops, rebuilds, and reports the error.
    fn add_shapes_undoable(&mut self, shapes: Vec<DrawShape>) -> usize {
        let mut added = 0;
        for shape in shapes {
            match self.history.apply(reticle_model::Edit::AddShape {
                cell: self.top_cell.clone(),
                shape,
            }) {
                Ok(()) => added += 1,
                Err(e) => {
                    self.status.set(format!("Edit failed: {e}"));
                    break;
                }
            }
        }
        if added > 0 {
            self.rebuild_scene();
        }
        added
    }

    /// Copies the selected shapes onto the in-app clipboard.
    fn productivity_copy(&mut self) {
        let shapes = self.selected_scene_shapes();
        if shapes.is_empty() {
            self.status.set("Copy: nothing selected");
            return;
        }
        let n = shapes.len();
        self.productivity.clipboard.set(shapes);
        self.status.set(format!("Copied {n} shape(s)"));
    }

    /// Cuts the selected direct shapes: copies them to the clipboard, then removes
    /// them from the top cell through the undo history.
    ///
    /// Only shapes drawn directly in the top cell can be removed; any selected
    /// instanced or arrayed geometry is copied but left in place, and the status line
    /// notes how many were skipped.
    fn productivity_cut(&mut self) {
        // The clipboard captures the full selection (including instanced geometry).
        let all = self.selected_scene_shapes();
        if all.is_empty() {
            self.status.set("Cut: nothing selected");
            return;
        }
        let clip_count = all.len();
        self.productivity.clipboard.set(all);

        let direct = self.selected_direct_shapes();
        let removable = direct.len();
        let skipped = clip_count - removable;
        // Remove in descending index order so each removal leaves the lower indices
        // valid.
        let mut removed = 0;
        for (index, _) in direct.into_iter().rev() {
            match self.history.apply(reticle_model::Edit::RemoveShape {
                cell: self.top_cell.clone(),
                index,
            }) {
                Ok(()) => removed += 1,
                Err(e) => {
                    self.status.set(format!("Edit failed: {e}"));
                    break;
                }
            }
        }
        if removed > 0 {
            self.rebuild_scene();
        }
        if skipped > 0 {
            self.status.set(format!(
                "Cut {removed} shape(s); {skipped} instanced skipped"
            ));
        } else {
            self.status.set(format!("Cut {removed} shape(s)"));
        }
    }

    /// Pastes the clipboard into the top cell, offset by the panel's paste delta.
    fn productivity_paste(&mut self) {
        if self.productivity.clipboard.is_empty() {
            self.status.set("Paste: clipboard empty");
            return;
        }
        let shapes = productivity::translate_shapes(
            self.productivity.clipboard.shapes(),
            self.productivity.paste_dx,
            self.productivity.paste_dy,
        );
        let n = self.add_shapes_undoable(shapes);
        if n > 0 {
            self.status.set(format!("Pasted {n} shape(s)"));
        }
    }

    /// Duplicates the current selection in place, offset by the panel's paste delta.
    ///
    /// This is copy-plus-paste in one step over the resolved selection geometry, so
    /// it works on instanced shapes too (the duplicate is flat geometry in the top
    /// cell).
    fn productivity_duplicate(&mut self) {
        let selected = self.selected_scene_shapes();
        if selected.is_empty() {
            self.status.set("Duplicate: nothing selected");
            return;
        }
        let shapes = productivity::translate_shapes(
            &selected,
            self.productivity.paste_dx,
            self.productivity.paste_dy,
        );
        let n = self.add_shapes_undoable(shapes);
        if n > 0 {
            self.status.set(format!("Duplicated {n} shape(s)"));
        }
    }

    /// Arrays the current selection into a rows x columns grid at the panel's pitch,
    /// adding every element to the top cell through the undo history.
    ///
    /// The element count is capped by [`productivity::MAX_ARRAY_ELEMENTS`]; past the
    /// cap the commit is refused. Element `(0, 0)` reproduces the current selection,
    /// so the originals stay put and the array grows from them.
    fn productivity_array(&mut self) {
        let selected = self.selected_scene_shapes();
        if selected.is_empty() {
            self.status.set("Array: nothing selected");
            return;
        }
        if !self.productivity.array_is_committable() {
            self.status.set(format!(
                "Array: {} elements exceeds the {} cap",
                self.productivity.array_count(),
                productivity::MAX_ARRAY_ELEMENTS
            ));
            return;
        }
        let shapes = self.productivity.array_expand(&selected);
        let n = self.add_shapes_undoable(shapes);
        if n > 0 {
            self.status.set(format!(
                "Arrayed into {}x{} ({n} shape(s))",
                self.productivity.array_rows, self.productivity.array_cols
            ));
        }
    }

    /// Moves the selected direct shapes by the panel's move delta.
    ///
    /// A move is a remove of each original followed by an add of its translated copy,
    /// both through the undo history. Only directly-owned shapes can move; instanced
    /// geometry is left in place and reported as skipped.
    fn productivity_move_delta(&mut self) {
        let direct = self.selected_direct_shapes();
        if direct.is_empty() {
            self.status.set("Move: no movable selection");
            return;
        }
        let (dx, dy) = (self.productivity.move_dx, self.productivity.move_dy);
        // Remove originals in descending index order, keeping lower indices valid.
        let mut ok = true;
        for (index, _) in direct.iter().rev() {
            if let Err(e) = self.history.apply(reticle_model::Edit::RemoveShape {
                cell: self.top_cell.clone(),
                index: *index,
            }) {
                self.status.set(format!("Edit failed: {e}"));
                ok = false;
                break;
            }
        }
        if ok {
            // Re-add the translated copies (appended to the cell's shape list).
            for (_, shape) in &direct {
                let moved = productivity::translate_shape(shape, dx, dy);
                if let Err(e) = self.history.apply(reticle_model::Edit::AddShape {
                    cell: self.top_cell.clone(),
                    shape: moved,
                }) {
                    self.status.set(format!("Edit failed: {e}"));
                    break;
                }
            }
        }
        self.rebuild_scene();
        self.status.set(format!("Moved {} shape(s)", direct.len()));
    }

    /// Builds and commits a via stack at the panel's center through the undo history.
    ///
    /// The cut and its two layer enclosures are sized from the technology enclosure
    /// rules (see [`productivity::via_stack_shapes`]); each of the three rectangles is
    /// a separate undoable `AddShape`.
    fn productivity_via_stack(&mut self) {
        let tech = self.history.document().technology().clone();
        let Some(stack) = self.productivity.build_via_stack(&tech) else {
            self.status.set("Via stack: cut size must be positive");
            return;
        };
        let n = self.add_shapes_undoable(stack.into_shapes());
        if n > 0 {
            self.status.set(format!("Placed via stack ({n} shape(s))"));
        }
    }

    /// Draws the productivity side panel: clipboard copy/cut/paste and duplicate, the
    /// interactive array tool with a live preview, move-by-delta numeric entry, and
    /// the via-stack builder.
    ///
    /// The panel is thin glue: it binds egui widgets to [`ProductivityState`] fields
    /// and calls the `productivity_*` action methods, each of which routes its
    /// mutation through the undo history. The live array preview is drawn on the
    /// canvas by [`array_preview_shapes`](Self::array_preview_shapes), not here.
    fn productivity_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Productivity");

        // Clipboard: copy / cut / paste / duplicate.
        ui.label(format!(
            "Selection: {} | Clipboard: {}",
            self.selection.len(),
            self.productivity.clipboard.len()
        ));
        ui.horizontal(|ui| {
            if ui.button("Copy").clicked() {
                self.productivity_copy();
            }
            if ui.button("Cut").clicked() {
                self.productivity_cut();
            }
            if ui.button("Paste").clicked() {
                self.productivity_paste();
            }
            if ui.button("Duplicate").clicked() {
                self.productivity_duplicate();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Offset dx/dy:");
            ui.add(egui::DragValue::new(&mut self.productivity.paste_dx).speed(10));
            ui.add(egui::DragValue::new(&mut self.productivity.paste_dy).speed(10));
        });

        ui.separator();

        // Move-by-delta.
        ui.label("Move selection by delta");
        ui.horizontal(|ui| {
            ui.label("dx/dy:");
            ui.add(egui::DragValue::new(&mut self.productivity.move_dx).speed(10));
            ui.add(egui::DragValue::new(&mut self.productivity.move_dy).speed(10));
            if ui.button("Move").clicked() {
                self.productivity_move_delta();
            }
        });

        ui.separator();

        // Interactive array tool.
        ui.label("Array");
        ui.horizontal(|ui| {
            ui.label("rows/cols:");
            ui.add(
                egui::DragValue::new(&mut self.productivity.array_rows)
                    .speed(1)
                    .range(0..=1000),
            );
            ui.add(
                egui::DragValue::new(&mut self.productivity.array_cols)
                    .speed(1)
                    .range(0..=1000),
            );
        });
        ui.horizontal(|ui| {
            ui.label("row/col pitch:");
            ui.add(egui::DragValue::new(&mut self.productivity.array_row_pitch).speed(10));
            ui.add(egui::DragValue::new(&mut self.productivity.array_col_pitch).speed(10));
        });
        ui.checkbox(&mut self.productivity.array_preview, "Live preview");
        ui.horizontal(|ui| {
            let committable = self.productivity.array_is_committable();
            if ui
                .add_enabled(committable, egui::Button::new("Build array"))
                .clicked()
            {
                self.productivity_array();
            }
            ui.label(format!("{} elems", self.productivity.array_count()));
        });

        ui.separator();

        // Via-stack builder.
        ui.label("Via stack");
        self.via_layer_combo(ui, "lower", ViaLayerField::Lower);
        self.via_layer_combo(ui, "upper", ViaLayerField::Upper);
        self.via_layer_combo(ui, "cut", ViaLayerField::Cut);
        ui.horizontal(|ui| {
            ui.label("cut size:");
            ui.add(
                egui::DragValue::new(&mut self.productivity.via_cut_size)
                    .speed(10)
                    .range(1..=100_000),
            );
        });
        ui.horizontal(|ui| {
            ui.label("center x/y:");
            ui.add(egui::DragValue::new(&mut self.productivity.via_center_x).speed(10));
            ui.add(egui::DragValue::new(&mut self.productivity.via_center_y).speed(10));
        });
        ui.horizontal(|ui| {
            ui.label("default enc:");
            ui.add(
                egui::DragValue::new(&mut self.productivity.via_default_enclosure)
                    .speed(1)
                    .range(0..=100_000),
            );
        });
        if ui.button("Place via stack").clicked() {
            self.productivity_via_stack();
        }
    }

    /// Draws one labeled layer-picker combo box for the via-stack builder, writing
    /// the chosen [`LayerId`] into the named field.
    fn via_layer_combo(&mut self, ui: &mut egui::Ui, label: &str, field: ViaLayerField) {
        let rows: Vec<(LayerId, String)> = self
            .layer_state
            .rows()
            .iter()
            .map(|r| (r.id, r.name.clone()))
            .collect();
        let current = match field {
            ViaLayerField::Lower => self.productivity.via_lower,
            ViaLayerField::Upper => self.productivity.via_upper,
            ViaLayerField::Cut => self.productivity.via_cut,
        };
        let current_name = rows.iter().find(|(id, _)| *id == current).map_or_else(
            || format!("{}/{}", current.layer, current.datatype),
            |(_, n)| n.clone(),
        );
        ui.horizontal(|ui| {
            ui.label(format!("{label}:"));
            egui::ComboBox::from_id_salt((label, "via_layer"))
                .selected_text(current_name)
                .show_ui(ui, |ui| {
                    for (id, name) in &rows {
                        let target = match field {
                            ViaLayerField::Lower => &mut self.productivity.via_lower,
                            ViaLayerField::Upper => &mut self.productivity.via_upper,
                            ViaLayerField::Cut => &mut self.productivity.via_cut,
                        };
                        ui.selectable_value(target, *id, name);
                    }
                });
        });
    }

    /// Draws the Generate side panel: pick a parameterized layout generator, fill a
    /// form built from its schema, preview the geometry live on the canvas, and place
    /// it into the document as one undo step.
    ///
    /// The panel is thin glue over [`crate::generate_panel::GeneratePanelState`]: the
    /// combo box selects the generator, [`params_form`](crate::generate_panel::GeneratePanelState::params_form)
    /// renders the typed widgets from the schema, and the Generate button routes
    /// through [`generate_apply`](Self::generate_apply). The live preview is drawn on
    /// the canvas by [`generate_preview_shapes`](Self::generate_preview_shapes), not
    /// here.
    fn generate_section(&mut self, ui: &mut egui::Ui) {
        ui.heading("Generate");

        // Pick the generator by title; the catalog comes straight from the registry.
        let selected_title = self.generate.selected_info().title;
        egui::ComboBox::from_id_salt("generate_pick")
            .selected_text(selected_title)
            .show_ui(ui, |ui| {
                let count = self.generate.infos().len();
                for i in 0..count {
                    let title = self.generate.infos()[i].title;
                    let is_sel = self.generate.selected() == i;
                    if ui.selectable_label(is_sel, title).clicked() {
                        self.generate.select(i);
                    }
                }
            });

        // A one-line description of the selected generator.
        ui.label(self.generate.selected_info().description)
            .on_hover_text(self.generate.selected_id());

        ui.separator();

        // The typed parameter form (Int drag, Bool checkbox, Enum combo).
        self.generate.params_form(ui);

        ui.checkbox(&mut self.generate.preview, "Live preview");

        // Report the generated shape count, or the generator's validation error so the
        // user sees why a parameter is rejected.
        let tech = self.history.document().technology().clone();
        match self.generate.generate_into_scratch(&tech) {
            Ok(shapes) => {
                ui.label(format!("Generates {} shape(s)", shapes.len()));
                if ui.button("Generate").clicked() {
                    self.generate_apply();
                }
            }
            Err(err) => {
                ui.colored_label(egui::Color32::from_rgb(255, 140, 90), err);
                ui.add_enabled(false, egui::Button::new("Generate"));
            }
        }
        if ui.button("Reset to defaults").clicked() {
            self.generate.reset_selected_to_defaults();
        }
    }

    /// Places the selected generator's geometry into the top cell as one undo step.
    ///
    /// The whole generated structure is applied through
    /// [`History::apply_group`](crate::history::History::apply_group) as a single
    /// logical undo step, so one Undo removes all of it, matching the ADR 0048
    /// undo-integration requirement. On a validation error nothing is placed and the
    /// message is shown in the status bar.
    fn generate_apply(&mut self) {
        let tech = self.history.document().technology().clone();
        let edits = match self.generate.placement_edits(&self.top_cell, &tech) {
            Ok(edits) => edits,
            Err(err) => {
                self.status.set(format!("Generate failed: {err}"));
                return;
            }
        };
        if edits.is_empty() {
            self.status.set("Generate: produced no geometry");
            return;
        }
        let n = edits.len();
        let title = self.generate.selected_info().title;
        if self.history.apply_group(edits).is_ok() {
            self.rebuild_scene();
            self.status.set(format!("Generated {title} ({n} shape(s))"));
        } else {
            self.status.set("Generate: edit failed");
        }
    }

    /// The live-preview shapes for the Generate panel, or an empty list when the
    /// preview is off or the current parameters do not generate.
    ///
    /// Drawn as an overlay by the canvas (see [`draw_generate_preview`](Self::draw_generate_preview)),
    /// so the user sees the generated structure before committing it.
    fn generate_preview_shapes(&self) -> Vec<DrawShape> {
        let tech = self.history.document().technology().clone();
        self.generate.preview_shapes(&tech)
    }

    /// The live array-preview shapes for the current selection and array parameters,
    /// or an empty list when preview is off, nothing is selected, or the count is
    /// over the cap.
    ///
    /// These are the element `(1..)` copies only (the originals are already on the
    /// canvas), drawn as an overlay by the canvas so the user sees the array before
    /// committing.
    fn array_preview_shapes(&self) -> Vec<DrawShape> {
        if !self.productivity.array_preview || !self.productivity.array_is_committable() {
            return Vec::new();
        }
        let selected = self.selected_scene_shapes();
        if selected.is_empty() {
            return Vec::new();
        }
        // Skip element (0,0): it coincides with the existing selection.
        let full = self.productivity.array_expand(&selected);
        full.into_iter().skip(selected.len()).collect()
    }

    /// Highlights the net electrically connected to the shape at `idx`.
    ///
    /// Extraction (cached in [`Netlight`], keyed on the document generation) runs over
    /// the flattened top cell so the returned net indices line up with the scene's
    /// shape indices. Reports the net size in the status bar.
    fn highlight_net_of(&mut self, idx: usize) {
        let n = self.netlight.highlight_shape(
            self.history.document(),
            &self.top_cell,
            self.doc_generation,
            idx,
        );
        if n > 0 {
            self.status.set(format!("Net: {n} shape(s)"));
        }
    }

    /// Runs DRC over the flattened top cell and stores the violations.
    fn run_drc(&mut self) {
        let n = self.drc.run(self.history.document(), &self.top_cell);
        if n == 0 {
            self.status.set("DRC: no violations");
        } else {
            self.status.set(format!("DRC: {n} violation(s)"));
        }
    }

    /// Draws the DRC panel section: run/clear actions and the violation list.
    ///
    /// Clicking a violation records it as selected and zooms the camera to its
    /// location on the next frame (once the real canvas size is known).
    fn drc_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("DRC");
        ui.horizontal(|ui| {
            if ui.button("Run DRC").clicked() {
                self.run_drc();
            }
            if ui.button("Clear").clicked() {
                self.drc.clear();
                self.status.set("DRC cleared");
            }
        });
        if self.drc.has_run() {
            ui.label(format!("{} violation(s)", self.drc.len()));
        } else {
            ui.label("Not run");
        }
        ui.separator();

        let selected = self.drc.selected();
        let mut clicked: Option<usize> = None;
        egui::ScrollArea::vertical()
            .max_height(160.0)
            .auto_shrink([false, false])
            .id_salt("drc_list")
            .show(ui, |ui| {
                for (i, v) in self.drc.violations().iter().enumerate() {
                    let label = drc_panel::format_violation(v);
                    if ui.selectable_label(selected == Some(i), label).clicked() {
                        clicked = Some(i);
                    }
                }
            });
        if let Some(i) = clicked
            && self.drc.select(i).is_some()
        {
            // Frame the violation on the next canvas pass.
            self.zoom_to_selected_violation = true;
        }

        // "Ask agent to fix" the selected violation: assemble its region + rule
        // into a scoped context string and launch a scoped agent session on it.
        if let Some(i) = self.drc.selected()
            && let Some(v) = self.drc.violations().get(i)
        {
            let can_ask = !self.agent.is_running();
            if ui
                .add_enabled(can_ask, egui::Button::new("Ask agent to fix"))
                .on_hover_text(
                    "Launch a scoped agent run seeded with this violation's region and rule",
                )
                .clicked()
            {
                let context = drc_panel::fix_violation_prompt(v);
                self.ask_agent_to_fix(context);
            }
        }
    }

    /// Launches a scoped agent run seeded with the assembled violation `context`.
    ///
    /// The context string (region bounding box plus the broken rule; see
    /// [`drc_panel::fix_violation_prompt`]) becomes the agent panel's prompt and
    /// the run starts, so today the affordance hands the scoped instruction to the
    /// same panel plumbing a typed prompt drives.
    ///
    /// # Wave-3B seam
    ///
    /// The MINIMAL context pack and the real *scoped* harness (which would clip the
    /// session to `context`'s region and constrain edits to it) are Wave 3 Lane 3B.
    /// This method is the seam that harness consumes: it centralizes assembling the
    /// scoped instruction and handing it off. A Wave-3B harness replaces the body's
    /// `self.agent.start()` with a scoped-session launch that reads the same
    /// `context`, and everything upstream (the DRC button, the assembled string)
    /// stays unchanged.
    fn ask_agent_to_fix(&mut self, context: String) {
        self.agent.prompt = context;
        self.agent.start();
        self.status.set("Agent: scoped fix run started");
    }

    /// Draws the agent panel: prompt box, Run/Stop, live status, and narration.
    ///
    /// The state machine and the narration feed live in [`crate::agent_panel`];
    /// this is glue only. The panel drives a scripted transcript (no model or
    /// key), so Run always has something honest to narrate. Model-free, so it
    /// runs on wasm too.
    fn agent_section(&mut self, ui: &mut egui::Ui) {
        use crate::agent_panel::RunState;

        ui.heading("Agent");
        ui.horizontal(|ui| {
            ui.label("Prompt:");
            ui.text_edit_singleline(&mut self.agent.prompt);
        });
        ui.horizontal(|ui| {
            let running = self.agent.is_running();
            if ui.add_enabled(!running, egui::Button::new("Run")).clicked() {
                self.agent.start();
                self.status.set("Agent run started");
            }
            if ui.add_enabled(running, egui::Button::new("Stop")).clicked() {
                self.agent.stop();
                self.status.set("Agent run stopped");
            }
            if ui.button("Replay theater").clicked() {
                self.replay_open = !self.replay_open;
            }
            // Hand the finished (or stopped) run's transcript to the theater.
            let replayable = !running && self.agent.transcript().is_some();
            if ui
                .add_enabled(replayable, egui::Button::new("Replay this run"))
                .clicked()
                && let Some(transcript) = self.agent.transcript().cloned()
            {
                self.replay.load_transcript(transcript);
                self.replay_open = true;
                self.drc.clear();
            }
        });
        if let Some(status) = self.agent.latest_status() {
            let (done, total) = self.agent.progress();
            ui.label(format!(
                "iter {} | {} | {} violation(s) | step {done}/{total}",
                status.iteration, status.step, status.violations
            ));
        } else {
            ui.label(match self.agent.state() {
                RunState::Idle => "Idle: enter a prompt and press Run",
                RunState::Running => "Starting...",
                RunState::Stopped => "Stopped",
            });
        }
        ui.separator();
        egui::ScrollArea::vertical()
            .max_height(140.0)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .id_salt("agent_narration")
            .show(ui, |ui| {
                if self.agent.narration().is_empty() {
                    ui.label("No run yet");
                }
                for line in self.agent.narration() {
                    ui.monospace(line);
                }
            });
        self.agent_plan_section(ui);
        self.agent_conversation(ui);
        self.agent_history_section(ui);
    }

    /// Draws the agent's plan: the stated per-iteration intent (goal, intended
    /// tools, expected checks) derived by the harness before each proposal.
    ///
    /// This renders [`AgentPanelState::plan`](crate::agent_panel::AgentPanelState::plan),
    /// which rides on the run's transcript. It is transparency narration for the
    /// viewer and material for failure mining, not a binding contract: nothing here
    /// asserts the iteration used exactly these tools or that the checks passed.
    /// Empty for a run whose transcript carried no plan (for example one recorded
    /// before the plan log existed).
    fn agent_plan_section(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.strong("Plan");
        egui::ScrollArea::vertical()
            .max_height(120.0)
            .auto_shrink([false, false])
            .id_salt("agent_plan")
            .show(ui, |ui| {
                let plan = self.agent.plan();
                if plan.is_empty() {
                    ui.label("No plan yet (the harness emits one step per iteration).");
                }
                for (i, step) in plan.iter().enumerate() {
                    let tools = if step.intended_tools.is_empty() {
                        "(none)".to_owned()
                    } else {
                        step.intended_tools.join(", ")
                    };
                    let checks = if step.expected_checks.is_empty() {
                        "(none)".to_owned()
                    } else {
                        step.expected_checks.join(", ")
                    };
                    ui.monospace(format!("iter {i}: {}", step.goal));
                    ui.monospace(format!("    tools: {tools}"));
                    ui.monospace(format!("    checks: {checks}"));
                }
            });
    }

    /// Draws conversation mode: the running dialogue plus a follow-up input.
    ///
    /// Submitting a follow-up appends it to the running session as a new
    /// constraint (see [`AgentPanelState::submit_followup`](crate::agent_panel::AgentPanelState::submit_followup)):
    /// the message and an acknowledgement join the conversation transcript, and
    /// the instruction is recorded on the panel's follow-up list, the seam a
    /// Wave-3 scoped harness reads to steer the live model. The input is only
    /// enabled while a run is active, since a follow-up needs a session to attach
    /// to.
    fn agent_conversation(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Conversation");
            if ui.small_button("Clear").clicked() {
                self.agent.clear_conversation();
            }
        });
        egui::ScrollArea::vertical()
            .max_height(120.0)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .id_salt("agent_conversation")
            .show(ui, |ui| {
                use crate::agent_panel::Speaker;
                if self.agent.conversation().is_empty() {
                    ui.label("Run the agent, then send a follow-up to steer it.");
                }
                for entry in self.agent.conversation() {
                    let who = match entry.speaker {
                        Speaker::User => "you",
                        Speaker::Agent => "agent",
                    };
                    ui.label(format!("{who}: {}", entry.text));
                }
            });
        let running = self.agent.is_running();
        ui.horizontal(|ui| {
            ui.add_enabled(
                running,
                egui::TextEdit::singleline(&mut self.agent.followup)
                    .hint_text("Add a constraint or instruction..."),
            );
            if ui.add_enabled(running, egui::Button::new("Send")).clicked()
                && let Some(text) = self.agent.submit_followup()
            {
                self.status.set(format!("Follow-up sent: {text}"));
            }
        });
        if !running {
            ui.label("(Follow-ups apply to a running session.)");
        }
    }

    /// Draws the session history browser: a Refresh action, the path the native
    /// scan reads, and the list of past run transcripts. Clicking one loads it
    /// into the replay theater.
    ///
    /// The listing is on-demand (Refresh scans; drawing never touches the disk),
    /// and loading goes through the same [`store`](crate::store) seam the theater
    /// already loads through, so on wasm the browser lists the bundled demo and
    /// selecting it plays that.
    fn agent_history_section(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("History");
            if ui.button("Refresh").clicked() {
                self.agent_history.refresh();
            }
        });
        #[cfg(not(target_arch = "wasm32"))]
        ui.horizontal(|ui| {
            ui.label("Dir:");
            ui.text_edit_singleline(&mut self.agent_history.dir);
        });
        let mut chosen: Option<String> = None;
        egui::ScrollArea::vertical()
            .max_height(120.0)
            .auto_shrink([false, false])
            .id_salt("agent_history_list")
            .show(ui, |ui| {
                if self.agent_history.is_empty() {
                    ui.label("No past runs listed. Press Refresh.");
                }
                for entry in self.agent_history.entries() {
                    if ui.selectable_label(false, &entry.label).clicked() {
                        chosen = Some(entry.reference.clone());
                    }
                }
            });
        if let Some(reference) = chosen {
            self.load_history_entry(&reference);
        }
        if !self.agent_history.error.is_empty() {
            ui.colored_label(Color32::from_rgb(255, 120, 120), &self.agent_history.error);
        }
    }

    /// Loads the history transcript named by `reference` into the replay theater
    /// and opens it, through the platform [`store`](crate::store).
    ///
    /// On native this reads the JSONL file at `reference`. On wasm the store
    /// returns `Ok(None)` for an arbitrary reference, so the theater keeps its
    /// bundled transcript; either way the theater ends up loaded and open.
    fn load_history_entry(&mut self, reference: &str) {
        match self.store.load_reference(reference) {
            Ok(Some((records, hash))) => {
                self.replay.load(records, hash);
                self.replay_open = true;
                self.drc.clear();
                self.agent_history.error.clear();
                let (_, total) = self.replay.progress();
                self.status
                    .set(format!("History: loaded {total} record(s)"));
            }
            Ok(None) => {
                // wasm: no filesystem. Open the theater on its bundled default.
                if let Ok((records, hash)) = self.store.default_transcript() {
                    self.replay.load(records, hash);
                }
                self.replay_open = true;
                self.drc.clear();
                self.status.set("History: playing bundled demo");
            }
            Err(message) => self.agent_history.error = message,
        }
    }

    /// Installs a verify step's violation list into the DRC panel and overlay.
    ///
    /// Called whenever a running agent feed (or the replay theater) crosses a
    /// `run_drc` record: the list parsed from the recorded response replaces the
    /// panel's stored violations, so the markers on the canvas track the
    /// agent's propose-verify-correct loop in real time.
    fn apply_agent_drc_update(&mut self, violations: Vec<reticle_model::Violation>) {
        let n = violations.len();
        self.drc.set_violations(violations);
        if n == 0 {
            self.status.set("Agent verify: DRC clean");
        } else {
            self.status.set(format!("Agent verify: {n} violation(s)"));
        }
    }

    /// A verify step crossed by the *agent panel's own run*: install its overlay
    /// (as [`apply_agent_drc_update`](Self::apply_agent_drc_update)) and also note
    /// the result as an agent turn in the conversation, so the dialogue reflects
    /// the propose-verify-correct loop. The theater's own ticks use the plain
    /// overlay update, so replaying a transcript there does not write into this
    /// panel's conversation.
    fn apply_agent_run_verify(&mut self, violations: Vec<reticle_model::Violation>) {
        let n = violations.len();
        if n == 0 {
            self.agent.note_agent("verified: DRC clean");
        } else {
            self.agent
                .note_agent(format!("verified: {n} violation(s) remaining"));
        }
        self.apply_agent_drc_update(violations);
    }

    /// Applies a theater seek/step result to the DRC overlay: install the list
    /// the new position implies, or clear the markers when no verify has run
    /// yet at that point of the transcript.
    fn apply_replay_overlay(&mut self, update: Option<Vec<reticle_model::Violation>>) {
        match update {
            Some(v) => self.apply_agent_drc_update(v),
            None => self.drc.clear(),
        }
    }

    /// Loads the transcript named in the theater's path box through the platform
    /// [`store`](crate::store).
    ///
    /// On native this reads the JSONL file at that path. On wasm there is no
    /// filesystem, so the store returns `Ok(None)` and the theater keeps its
    /// bundled transcript, explaining that arbitrary paths are native-only.
    fn load_replay_from_path(&mut self) {
        let reference = self.replay_path.clone();
        match self.store.load_reference(&reference) {
            Ok(Some((records, hash))) => {
                self.replay.load(records, hash);
                self.replay_error.clear();
                self.drc.clear();
                let (_, total) = self.replay.progress();
                self.status.set(format!("Replay: loaded {total} record(s)"));
            }
            Ok(None) => {
                format!(
                    "Loading a transcript by path is native-only ({} build). Playing the bundled demo.",
                    self.store.origin_label()
                )
                .clone_into(&mut self.replay_error);
            }
            Err(message) => self.replay_error = message,
        }
    }

    /// Draws the document-open warnings window, when the last open produced any.
    ///
    /// A deliberately minimal, non-panicking surface: it lists each warning's
    /// summary with its detail on hover, and closing it clears the list. This is
    /// the small warnings surface the seam's contract promises; a richer,
    /// comprehensive error surface belongs to another lane, which routes its opens
    /// through the same seam and reads [`open_warnings`](Self::open_warnings).
    fn open_warnings_window(&mut self, ctx: &egui::Context) {
        if self.open_warnings.is_empty() {
            return;
        }
        let mut open = true;
        let title = format!("Import warnings ({})", self.open_warnings.len());
        egui::Window::new(title)
            .id(egui::Id::new("open_warnings_window"))
            .open(&mut open)
            .default_size([420.0, 220.0])
            .show(ctx, |ui| {
                ui.label("The document opened, but some parts were skipped or adjusted:");
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for w in &self.open_warnings {
                        ui.colored_label(Color32::from_rgb(230, 190, 90), &w.summary)
                            .on_hover_text(&w.detail);
                    }
                });
                ui.separator();
                if ui.button("Dismiss").clicked() {
                    // Emptying the list closes the window next frame; setting the
                    // local flag closes it this frame.
                    self.open_warnings.clear();
                }
            });
        if !open {
            self.open_warnings.clear();
        }
    }

    /// Draws the notification toast area: the app's one human-readable error and
    /// notice surface (see [`crate::notify`]).
    ///
    /// A stack of toasts anchored bottom-right, newest at the bottom, each colored by
    /// severity with its summary, an optional detail, and a close button. Errors stay
    /// until dismissed; informational and warning toasts auto-expire (aged by
    /// [`Notifications::advance`](crate::notify::Notifications::advance) in the frame
    /// loop). A dismissal is collected and applied after the layout closure so the
    /// borrow of `self` ends first.
    fn notifications_area(&mut self, ctx: &egui::Context) {
        if self.notifications.is_empty() {
            return;
        }
        let mut dismiss: Option<usize> = None;
        egui::Area::new(egui::Id::new("notifications_area"))
            .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-12.0, -12.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for (index, note) in self.notifications.iter().enumerate() {
                    let accent = severity_color(note.severity);
                    egui::Frame::popup(ui.style())
                        .stroke(Stroke::new(1.5, accent))
                        .inner_margin(egui::Margin::same(10))
                        .show(ui, |ui| {
                            ui.set_max_width(360.0);
                            ui.horizontal(|ui| {
                                ui.colored_label(accent, note.severity.label());
                                ui.strong(&note.summary);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.small_button("x").clicked() {
                                            dismiss = Some(index);
                                        }
                                    },
                                );
                            });
                            if note.has_detail() {
                                ui.label(egui::RichText::new(&note.detail).small().weak());
                            }
                        });
                    ui.add_space(6.0);
                }
            });
        if let Some(index) = dismiss {
            self.notifications.dismiss(index);
        }
    }

    /// Draws the replay theater window: load row, transport, readouts, and the
    /// replayed document painted through a [`crate::replay::FitView`].
    ///
    /// All playback logic lives in [`crate::replay`]; this is glue. The theater
    /// re-executes the transcript against a live engine session, so the canvas
    /// here shows real replayed geometry, and each `run_drc` record it crosses
    /// pushes its recorded violation list into the shared DRC overlay.
    fn replay_window(&mut self, ctx: &egui::Context) {
        if !self.replay_open {
            return;
        }
        let mut open = self.replay_open;
        egui::Window::new("Replay theater")
            .open(&mut open)
            .default_size([480.0, 460.0])
            .show(ctx, |ui| {
                self.replay_load_row(ui);
                ui.separator();
                self.replay_transport_row(ui);
                ui.separator();
                self.replay_readouts(ui);
                ui.separator();
                self.replay_canvas(ui);
            });
        self.replay_open = open;
    }

    /// The theater's load row: a JSONL path, or the built-in scripted demo run.
    fn replay_load_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Transcript:");
            ui.text_edit_singleline(&mut self.replay_path);
            if ui.button("Load").clicked() {
                self.load_replay_from_path();
            }
            if ui.button("Load demo run").clicked() {
                let (transcript, _) = crate::agent_panel::scripted_run("replay theater demo");
                self.replay.load_transcript(transcript);
                self.replay_error.clear();
                self.drc.clear();
            }
        });
        if !self.replay_error.is_empty() {
            ui.colored_label(Color32::from_rgb(255, 120, 120), &self.replay_error);
        }
    }

    /// The theater's transport row: restart, step back, play/pause, step
    /// forward, and the speed selector.
    fn replay_transport_row(&mut self, ui: &mut egui::Ui) {
        use crate::replay::SPEEDS;

        let loaded = self.replay.is_loaded();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(loaded, egui::Button::new("|<"))
                .on_hover_text("Restart")
                .clicked()
            {
                let update = self.replay.seek(0);
                self.apply_replay_overlay(update);
            }
            if ui
                .add_enabled(loaded, egui::Button::new("< Step"))
                .clicked()
            {
                let update = self.replay.step_back();
                self.apply_replay_overlay(update);
            }
            let play_label = if self.replay.is_playing() {
                "Pause"
            } else {
                "Play"
            };
            if ui
                .add_enabled(loaded, egui::Button::new(play_label))
                .clicked()
            {
                if self.replay.is_playing() {
                    self.replay.pause();
                } else {
                    self.replay.play();
                }
            }
            if ui
                .add_enabled(loaded, egui::Button::new("Step >"))
                .clicked()
                && let Some(update) = self.replay.step_forward()
            {
                self.apply_agent_drc_update(update);
            }
            let mut speed = self.replay.speed();
            egui::ComboBox::from_id_salt("replay_speed")
                .selected_text(format!("{speed}x"))
                .width(70.0)
                .show_ui(ui, |ui| {
                    for &s in &SPEEDS {
                        ui.selectable_value(&mut speed, s, format!("{s}x"));
                    }
                });
            self.replay.set_speed(speed);
        });
    }

    /// The theater's readouts: progress, shape count, hash verdict, violation
    /// count, and the "now playing" narration line.
    fn replay_readouts(&mut self, ui: &mut egui::Ui) {
        use crate::replay::HashCheck;

        let (done, total) = self.replay.progress();
        ui.horizontal(|ui| {
            ui.label(format!("Step {done}/{total}"));
            ui.separator();
            ui.label(format!("Shapes: {}", self.replay.shape_count()));
            ui.separator();
            ui.label(match self.replay.hash_check() {
                HashCheck::Pending => "hash: pending",
                HashCheck::Unverifiable => "hash: none recorded",
                HashCheck::Match => "hash: match",
                HashCheck::Mismatch => "hash: MISMATCH",
            });
            if self.replay.has_verified() {
                ui.separator();
                ui.label(format!(
                    "{} violation(s)",
                    self.replay.last_violations().len()
                ));
            }
        });
        if let Some(record) = self.replay.current_record() {
            ui.monospace(crate::agent_panel::narrate_record(record));
        } else {
            ui.label(if self.replay.is_loaded() {
                "At start: press Play or Step"
            } else {
                "No transcript loaded"
            });
        }
    }

    /// Paints the replayed document (and the last verify's violation markers)
    /// into the theater window, letterboxed by a [`crate::replay::FitView`].
    fn replay_canvas(&self, ui: &mut egui::Ui) {
        use crate::replay::{FitView, shapes_bbox};

        let size = Vec2::new(ui.available_width().max(160.0), 240.0);
        let (response, painter) = ui.allocate_painter(size, Sense::hover());
        let rect = response.rect;
        painter.rect_filled(rect, 4.0, Color32::from_rgb(12, 14, 18));
        let shapes = self.replay.flattened_shapes();
        let Some(bbox) = shapes_bbox(&shapes) else {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Nothing drawn yet",
                FontId::proportional(12.0),
                Color32::from_rgb(120, 126, 140),
            );
            return;
        };
        let view = FitView::fit(bbox, rect.width(), rect.height(), 14.0);
        let to_pos = |p: Point| {
            let (x, y) = view.to_screen(p);
            Pos2::new(rect.left() + x, rect.top() + y)
        };
        // Layer colors come from the replayed session's own technology (the
        // transcript installs it), with a neutral gray fallback.
        let doc = self.replay.document();
        let color_of = |layer: LayerId| -> Color32 {
            doc.technology()
                .layers
                .iter()
                .find(|l| l.id == layer)
                .map_or(Color32::from_rgb(150, 150, 150), |l| {
                    let (r, g, b, _) = layers::rgba_components(l.color_rgba);
                    Color32::from_rgb(r, g, b)
                })
        };
        for shape in &shapes {
            let base = color_of(shape.layer);
            let fill = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 170);
            let stroke = Stroke::new(1.0, base);
            match &shape.kind {
                ShapeKind::Rect(r) => {
                    let e = EguiRect::from_two_pos(to_pos(r.min), to_pos(r.max));
                    painter.rect_filled(e, 0.0, fill);
                    painter.rect_stroke(e, 0.0, stroke, StrokeKind::Middle);
                }
                ShapeKind::Polygon(poly) => {
                    let pts: Vec<Pos2> = poly.vertices().iter().map(|p| to_pos(*p)).collect();
                    if pts.len() >= 3 {
                        painter.add(Shape::convex_polygon(pts, fill, stroke));
                    }
                }
                ShapeKind::Path(path) => {
                    let pts: Vec<Pos2> = path.points().iter().map(|p| to_pos(*p)).collect();
                    if pts.len() >= 2 {
                        painter.add(Shape::line(pts, Stroke::new(2.0, base)));
                    }
                }
            }
        }
        // The last verify's markers, in the DRC overlay's alarm red.
        let marker = Stroke::new(2.0, Color32::from_rgb(255, 90, 90));
        for v in self.replay.last_violations() {
            let e =
                EguiRect::from_two_pos(to_pos(v.location.min), to_pos(v.location.max)).expand(2.0);
            painter.rect_stroke(e, 0.0, marker, StrokeKind::Middle);
        }
        if let Some(cell) = self.replay.render_cell() {
            painter.text(
                Pos2::new(rect.left() + 8.0, rect.top() + 6.0),
                Align2::LEFT_TOP,
                cell,
                FontId::monospace(10.0),
                Color32::from_rgb(150, 156, 170),
            );
        }
    }

    /// Draws the agent's cursor: a distinct ringed crosshair plus the agent's
    /// actor name, so it cannot be mistaken for a collaborator's presence dot.
    fn draw_agent_cursor(&self, painter: &egui::Painter, screen: &ScreenRect) {
        if self.agent.state() == crate::agent_panel::RunState::Idle {
            return;
        }
        let Some(world) = self.agent.cursor() else {
            return;
        };
        let p = self.world_pos_to_screen(screen, world);
        let color = Color32::from_rgb(235, 80, 220);
        let stroke = Stroke::new(2.0, color);
        painter.circle_stroke(p, 9.0, stroke);
        painter.circle_filled(p, 3.0, color);
        // Four crosshair ticks just outside the ring.
        for (dx, dy) in [(1.0f32, 0.0f32), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            painter.line_segment(
                [
                    Pos2::new(p.x + dx * 5.0, p.y + dy * 5.0),
                    Pos2::new(p.x + dx * 13.0, p.y + dy * 13.0),
                ],
                stroke,
            );
        }
        painter.text(
            Pos2::new(p.x + 15.0, p.y - 12.0),
            Align2::LEFT_CENTER,
            reticle_agent_api::AGENT_ACTOR,
            FontId::proportional(11.0),
            color,
        );
    }

    /// Draws the Share section: the relay host, the room, the collaborator join
    /// link, and a read-only viewer link, each with a copy button.
    ///
    /// The link formats live in [`crate::share`] (unit-tested there); this is glue.
    /// The collaborator link targets the relay's `GET /ws/{room}` route, so anyone
    /// who opens a client against it edits alongside this session. The **viewer**
    /// link is a page URL carrying `?view=viewer` (ADR 0038): whoever opens it in
    /// the web bundle joins the same room read-only, applying this session's live
    /// edits but never sending any back, and can toggle follow-mode to ride along
    /// with this session's viewport. Room-creation on the demo server is
    /// rate-limited and the room expires (see `reticle-demo`).
    fn share_section(&mut self, ui: &mut egui::Ui) {
        ui.heading("Share");
        ui.horizontal(|ui| {
            ui.label("Relay:");
            ui.text_edit_singleline(&mut self.share_server);
        });
        ui.horizontal(|ui| {
            ui.label("Room:");
            ui.text_edit_singleline(&mut self.share_room);
        });

        // The collaborator (read-write) join link.
        let link = crate::share::room_link(&self.share_server, &self.share_room);
        ui.monospace(&link);
        if ui.button("Copy link").clicked() {
            ui.ctx().copy_text(link);
            self.status.set("Share link copied");
        }

        ui.separator();

        // The read-only viewer link: a page URL that opens the bundle read-only,
        // joined to this room. The page origin is optional; empty yields a relative
        // link that resolves against wherever the bundle is loaded.
        ui.horizontal(|ui| {
            ui.label("Viewer page:");
            ui.text_edit_singleline(&mut self.share_page);
        });
        let viewer_link =
            crate::share::viewer_link(&self.share_page, &self.share_server, &self.share_room);
        ui.monospace(&viewer_link);
        if ui.button("Copy read-only viewer link").clicked() {
            ui.ctx().copy_text(viewer_link);
            self.status.set("Read-only viewer link copied");
        }
        ui.label("Viewers see live edits, pan and zoom independently, and can follow your view.");
    }

    /// Draws the view and export section: theme toggle, camera bookmarks, and the
    /// export controls (scope, format, monochrome, and the run button).
    ///
    /// The theme choice is applied to the egui visuals at the top of
    /// [`eframe::App::ui`]; here the button only flips the stored
    /// [`crate::viewexport::Theme`]. Bookmarks capture the live camera center and
    /// zoom and restore it in place. Export runs SVG or PNG over the whole view or
    /// the current selection, optionally in the print-style monochrome mode (see
    /// [`crate::viewexport`] and [`App::run_export`]).
    fn view_export_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("View and export");

        // Theme toggle.
        ui.horizontal(|ui| {
            ui.label(format!("Theme: {}", self.view_export.theme.label()));
            if ui.button("Toggle theme").clicked() {
                let now = self.view_export.toggle_theme();
                self.status.set(format!("Theme: {}", now.label()));
            }
        });

        ui.separator();

        // Camera bookmarks.
        ui.label("View bookmarks");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.view_export.name_input);
            if ui.button("Save view").clicked() {
                let center = self.camera.center();
                let ppd = self.camera.pixels_per_dbu();
                let name = self.view_export.name_input.clone();
                let stored = self.view_export.add_bookmark(&name, center, ppd);
                self.view_export.name_input.clear();
                self.status.set(format!("Saved view '{stored}'"));
            }
        });
        // Jump to or remove a saved view. Collect an action first so the immutable
        // borrow of the bookmark list ends before we mutate the camera or the list.
        let mut jump_to: Option<(Point, f64, String)> = None;
        let mut remove: Option<usize> = None;
        for (i, bm) in self.view_export.bookmarks.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.button(&bm.name).clicked() {
                    jump_to = Some((bm.center(), bm.pixels_per_dbu(), bm.name.clone()));
                }
                if ui.small_button("x").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some((center, ppd, name)) = jump_to {
            self.camera = ViewCamera::new(center, ppd);
            self.status.set(format!("Jumped to '{name}'"));
        }
        if let Some(i) = remove {
            self.view_export.remove_bookmark(i);
        }

        ui.separator();

        // Export controls.
        ui.label("Export");
        ui.horizontal(|ui| {
            ui.label("Scope:");
            ui.radio_value(
                &mut self.view_export.scope,
                crate::viewexport::ExportScope::View,
                crate::viewexport::ExportScope::View.label(),
            );
            ui.radio_value(
                &mut self.view_export.scope,
                crate::viewexport::ExportScope::Selection,
                crate::viewexport::ExportScope::Selection.label(),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Format:");
            ui.radio_value(
                &mut self.view_export.format,
                crate::viewexport::ExportFormat::Svg,
                crate::viewexport::ExportFormat::Svg.label(),
            );
            ui.radio_value(
                &mut self.view_export.format,
                crate::viewexport::ExportFormat::Png,
                crate::viewexport::ExportFormat::Png.label(),
            );
        });
        ui.checkbox(&mut self.view_export.monochrome, "Monochrome (print) mode");
        if ui.button("Export").clicked() {
            self.run_export();
        }
    }

    /// The shapes covered by the current export scope, cloned out of the scene.
    ///
    /// [`ExportScope::View`](crate::viewexport::ExportScope::View) is every shape in
    /// the flattened scene; [`ExportScope::Selection`](crate::viewexport::ExportScope::Selection)
    /// is only the current selection. Cloning ends the borrow of
    /// `self.scene`/`self.selection` before
    /// the export writes anything.
    fn export_shapes(&self) -> Vec<DrawShape> {
        match self.view_export.scope {
            crate::viewexport::ExportScope::View => self.scene.shapes().to_vec(),
            crate::viewexport::ExportScope::Selection => self.selected_scene_shapes(),
        }
    }

    /// Builds a layer to colour lookup for export from the live layer table.
    ///
    /// Snapshots the colour of every layer used by `shapes` into an owned map so the
    /// returned [`LayerPaint`](crate::viewexport::LayerPaint) closure does not borrow
    /// `self`, which lets the export helpers take the shapes and the paint at once.
    fn export_paint(&self, shapes: &[DrawShape]) -> impl Fn(LayerId) -> (u8, u8, u8, u8) + use<> {
        let mut map: std::collections::HashMap<LayerId, (u8, u8, u8, u8)> =
            std::collections::HashMap::new();
        for s in shapes {
            map.entry(s.layer)
                .or_insert_with(|| self.layer_color(s.layer));
        }
        move |layer: LayerId| map.get(&layer).copied().unwrap_or((160, 160, 160, 190))
    }

    /// Runs the configured export (scope, format, monochrome) and reports the result.
    ///
    /// SVG is generated purely from the shape list and written as text. PNG of the
    /// whole view reuses the native offscreen GPU renderer for a pixel-accurate,
    /// full-colour frame (skipped with a status note when no GPU is present, and a
    /// native-only path: on the web it reports that PNG export is native-only). PNG
    /// of a selection, or any monochrome PNG, uses the pure rasterizer so it works
    /// without a GPU. Output files land in the working directory (native); on the
    /// web only the SVG path produces bytes, offered as a status line.
    fn run_export(&mut self) {
        use crate::viewexport::{ExportFormat, ExportScope};
        let shapes = self.export_shapes();
        if matches!(self.view_export.scope, ExportScope::Selection) && shapes.is_empty() {
            self.status.set("Nothing selected to export");
            return;
        }
        let mono = self.view_export.monochrome;
        // Output pixel size: the current canvas, or a sensible default.
        let (w, h) = self.view_export.last_canvas.map_or((1024, 768), |s| {
            (s.width.max(16.0) as u32, s.height.max(16.0) as u32)
        });
        match self.view_export.format {
            ExportFormat::Svg => {
                let bounds = crate::viewexport::shapes_bounds(&shapes);
                let paint = self.export_paint(&shapes);
                let svg = crate::viewexport::shapes_to_svg(&shapes, bounds, w, h, &paint, mono);
                match self.write_export_text("reticle-export.svg", &svg) {
                    Ok(msg) => self.status.set(msg),
                    Err(e) => self.status.set(format!("SVG export failed: {e}")),
                }
            }
            ExportFormat::Png => self.export_png_scoped(&shapes, w, h, mono),
        }
    }

    /// Writes exported text (SVG) to `name` in the working directory (native).
    ///
    /// On the web there is no filesystem, so this reports the byte count instead of
    /// writing, keeping the call site uniform.
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::unused_self)]
    fn write_export_text(&self, name: &str, text: &str) -> std::io::Result<String> {
        let path = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(name);
        std::fs::write(&path, text)?;
        Ok(format!("Exported {}", path.display()))
    }

    /// Web stub: report the generated size rather than writing to a filesystem.
    #[cfg(target_arch = "wasm32")]
    #[allow(clippy::unused_self)]
    fn write_export_text(&self, _name: &str, text: &str) -> std::io::Result<String> {
        Ok(format!("Generated {} bytes of SVG", text.len()))
    }

    /// Exports `shapes` to PNG, choosing the GPU or the pure rasterizer path.
    ///
    /// A full-colour view export uses the native offscreen GPU renderer (pixel
    /// accurate, matching the canvas). A selection export or any monochrome export
    /// uses the pure [`rasterize`](crate::viewexport::rasterize) filler so it needs
    /// no GPU and honours the print mode. Both feed the crate's dependency-free PNG
    /// encoder.
    #[cfg(not(target_arch = "wasm32"))]
    fn export_png_scoped(&mut self, shapes: &[DrawShape], w: u32, h: u32, mono: bool) {
        use crate::viewexport::ExportScope;
        // The GPU path renders the whole document at the current camera in full
        // colour; it cannot restrict to a selection or recolour to monochrome, so
        // those fall through to the rasterizer.
        let use_gpu = matches!(self.view_export.scope, ExportScope::View) && !mono;
        if use_gpu {
            self.export_png(self.view_export.last_canvas);
            return;
        }
        let bounds = crate::viewexport::shapes_bounds(shapes);
        let paint = self.export_paint(shapes);
        let ras = crate::viewexport::rasterize(shapes, bounds, w, h, &paint, mono);
        let bytes = crate::app::png::encode_rgba(ras.width, ras.height, &ras.pixels);
        let path = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("reticle-export.png");
        match std::fs::write(&path, bytes) {
            Ok(()) => self.status.set(format!("Exported {}", path.display())),
            Err(e) => self.status.set(format!("PNG export failed: {e}")),
        }
    }

    /// Web stub: PNG export is native-only (no filesystem, no blocking GPU context).
    #[cfg(target_arch = "wasm32")]
    #[allow(clippy::unused_self)]
    fn export_png_scoped(&mut self, _shapes: &[DrawShape], _w: u32, _h: u32, _mono: bool) {
        self.status.set("PNG export is native-only");
    }

    /// Draws the snap and guides settings section.
    ///
    /// Surfaces the grid and snap toggles (grid on/off, snap-to-grid,
    /// snap-to-geometry, snap-to-guides), the grid spacing and snap radius, and the
    /// list of user guides with per-guide remove buttons plus add and clear
    /// actions. Grid facts live on [`crate::grid::GridSettings`] and the rest on
    /// [`crate::snap::SnapState`]; this panel edits both in place.
    fn snap_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Snap and guides");
        ui.checkbox(&mut self.grid.visible, "Show grid");
        ui.checkbox(&mut self.grid.snap_enabled, "Snap to grid");
        ui.checkbox(&mut self.snap.geometry_enabled, "Snap to geometry");
        ui.checkbox(&mut self.snap.guide_enabled, "Snap to guides");

        ui.horizontal(|ui| {
            ui.label("Grid spacing (DBU):");
            ui.add(egui::DragValue::new(&mut self.grid.base_step_dbu).range(1..=1_000_000));
        });
        ui.horizontal(|ui| {
            ui.label("Snap radius (px):");
            ui.add(
                egui::Slider::new(
                    &mut self.snap.radius_px,
                    snap::MIN_RADIUS_PX..=snap::MAX_RADIUS_PX,
                )
                .integer(),
            );
        });

        ui.separator();
        ui.horizontal(|ui| {
            ui.label(format!("Guides: {}", self.snap.guides.len()));
            if ui.button("Add H").clicked() {
                let y = self.camera.center().y;
                self.snap.add_guide(Guide::horizontal(y));
                self.status.set(format!("Guide y = {y}"));
            }
            if ui.button("Add V").clicked() {
                let x = self.camera.center().x;
                self.snap.add_guide(Guide::vertical(x));
                self.status.set(format!("Guide x = {x}"));
            }
            if ui.button("Clear").clicked() {
                self.snap.clear_guides();
            }
        });
        ui.label("Drag from a ruler to add a guide.");

        // The guide to delete after the loop, so the list is not mutated mid-walk.
        let mut remove: Option<usize> = None;
        for (i, g) in self.snap.guides.iter().enumerate() {
            ui.horizontal(|ui| {
                let text = match g.axis {
                    snap::Axis::Horizontal => format!("y = {}", g.coord),
                    snap::Axis::Vertical => format!("x = {}", g.coord),
                };
                ui.monospace(text);
                if ui.small_button("x").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.snap.remove_guide(i);
        }
    }

    /// Draws the properties inspector section for the current selection.
    fn inspector_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Properties");
        let indices: Vec<usize> = self.selection.iter().collect();
        let insp = inspector::inspect(self.scene.shapes(), &indices, &self.layer_state);
        match insp {
            Inspection::Empty => {
                ui.label("No selection");
            }
            Inspection::Single(info) => {
                ui.label(format!("Layer: {}", info.layer_label()));
                ui.label(inspector::format_bounds(&info.bounds));
                ui.label(format!("Width: {} DBU", info.width()));
                ui.label(format!("Height: {} DBU", info.height()));
                ui.label(format!("Area: {} DBU^2", info.area()));
            }
            Inspection::Multiple { count, bounds } => {
                ui.label(format!("Selected: {count} shapes"));
                ui.label(inspector::format_bounds(&bounds));
                ui.label(format!("Combined width: {} DBU", bounds.width()));
                ui.label(format!("Combined height: {} DBU", bounds.height()));
            }
        }
    }

    /// Draws the bottom status bar: tool, cursor coordinates, zoom, and messages.
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(format!("Tool: {}", self.tools.active().label()));
            ui.separator();
            if let Some(p) = self.cursor_world {
                ui.label(format!("Cursor: ({}, {}) DBU", p.x, p.y));
            } else {
                ui.label("Cursor: --");
            }
            ui.separator();
            ui.label(format!("Zoom: {:.4} px/DBU", self.camera.pixels_per_dbu()));
            ui.separator();
            ui.label(self.frame_meter.label());
            if let Some(m) = self.tools.measurement() {
                ui.separator();
                ui.label(format!(
                    "Measure: {:.1} DBU = {:.3} um  (dx {}, dy {})",
                    m.distance_dbu(),
                    m.distance_microns(),
                    m.dx(),
                    m.dy()
                ));
            }
            if !self.status.text.is_empty() {
                ui.separator();
                ui.label(&self.status.text);
            }
        });
    }

    /// Draws the command-palette window and runs the chosen command.
    fn palette_window(&mut self, ctx: &egui::Context, screen: Option<ScreenRect>) {
        if !self.palette_open {
            return;
        }
        let layer_names: Vec<String> = self
            .layer_state
            .rows()
            .iter()
            .map(|r| r.name.clone())
            .collect();
        let entries = command::catalog(&layer_names);

        let mut open = self.palette_open;
        let mut chosen: Option<Command> = None;
        egui::Window::new("Command palette")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_pos(Pos2::new(200.0, 120.0))
            .show(ctx, |ui| {
                ui.label("Type to filter, click to run:");
                let resp = ui.text_edit_singleline(&mut self.palette_query);
                resp.request_focus();
                ui.separator();
                let matches = command::filter(&entries, &self.palette_query);
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for entry in matches {
                            if ui.selectable_label(false, &entry.label).clicked() {
                                chosen = Some(entry.command);
                            }
                        }
                    });
            });
        self.palette_open = open;
        if let Some(cmd) = chosen {
            self.run_command(cmd, screen);
            self.palette_open = false;
        }
    }

    /// Draws the shortcuts editor window: every action with its current chord,
    /// rebind and clear controls, plus reset and (native) save.
    ///
    /// The chord capture itself happens in [`App::handle_shortcuts`]; this window
    /// only arms it, so what the editor shows and what the keyboard does can
    /// never disagree. Takeovers (binding a chord another action holds) are
    /// reported through the status bar by the capture path.
    fn keymap_window(&mut self, ctx: &egui::Context) {
        if !self.keymap_open {
            return;
        }
        let mut open = self.keymap_open;
        egui::Window::new("Keyboard shortcuts")
            .open(&mut open)
            .resizable(true)
            .default_pos(Pos2::new(260.0, 140.0))
            .show(ctx, |ui| {
                ui.label("Click Rebind, then press the new chord (Escape cancels).");
                ui.label("Binding a chord another action holds unbinds that action.");
                ui.separator();
                egui::Grid::new("keymap_grid")
                    .num_columns(3)
                    .striped(true)
                    .show(ui, |ui| {
                        for action in keymap::Action::all() {
                            ui.label(action.label());
                            let chord_text = self
                                .keymap
                                .chord_for(action)
                                .map_or_else(|| "(unbound)".to_owned(), ToString::to_string);
                            ui.monospace(chord_text);
                            ui.horizontal(|ui| {
                                if self.rebinding == Some(action) {
                                    ui.label("press keys...");
                                } else if ui.small_button("Rebind").clicked() {
                                    self.rebinding = Some(action);
                                }
                                if ui.small_button("Clear").clicked() {
                                    self.keymap.bind(action, None);
                                    if self.rebinding == Some(action) {
                                        self.rebinding = None;
                                    }
                                }
                            });
                            ui.end_row();
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Reset defaults").clicked() {
                        self.keymap = Keymap::defaults();
                        self.rebinding = None;
                        self.status.set("Keymap reset to defaults");
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    if ui.button("Save").clicked() {
                        match keymap::save(&self.keymap) {
                            Ok(()) => self.status.set("Keymap saved"),
                            Err(e) => self.status.set(format!("Keymap save failed: {e}")),
                        }
                    }
                });
            });
        self.keymap_open = open;
        if !self.keymap_open {
            self.rebinding = None;
        }
    }

    /// Draws the layout canvas and processes pointer interaction on it.
    ///
    /// Returns the canvas [`ScreenRect`] so the caller can hand it to actions (PNG
    /// export, deferred zoom-to-fit) that need the real pixel size.
    ///
    /// When `gpu_format` is `Some`, the layout geometry is drawn on the GPU through a
    /// retained paint callback (eframe's shared device); egui overlays still paint on
    /// top. When it is `None`, the geometry falls back to egui painting.
    fn canvas(
        &mut self,
        ui: &mut egui::Ui,
        gpu_format: Option<eframe::egui_wgpu::wgpu::TextureFormat>,
    ) -> ScreenRect {
        let size = ui.available_size();
        let (response, base_painter) = ui.allocate_painter(size, Sense::click_and_drag());
        let rect = response.rect;

        // Background (covers every pane and the divider).
        base_painter.rect_filled(rect, 0.0, Color32::from_rgb(16, 18, 22));

        // Pane layout over the shared document. The rest of this method edits and
        // draws the *focused* pane, so `screen` is that pane's rectangle (the whole
        // canvas when unsplit); unfocused panes render read-only previews.
        let full = ScreenRect::new(rect.min.x, rect.min.y, rect.width(), rect.height());
        let panes = self.viewports.rects(&full);
        let split = panes.len() > 1;
        let focus_changed = split && self.route_pane_focus(&response, &panes);
        let screen = panes.get(self.viewports.focused()).copied().unwrap_or(full);
        if split {
            self.draw_unfocused_panes(&base_painter, &panes);
        }
        // Clip focused-pane drawing so egui-painted geometry and overlays cannot
        // bleed across the divider; unsplit, the clip is the whole canvas.
        let painter = if split {
            base_painter.with_clip_rect(egui_rect_of(&screen))
        } else {
            base_painter.clone()
        };

        // Deferred fit now that we know the canvas size.
        if self.fit_requested {
            if let Some(bounds) = self.scene.bounds() {
                self.camera.zoom_to_fit(&screen, bounds);
            }
            self.fit_requested = false;
        }

        // Deferred zoom to the violation the DRC list just selected.
        if self.zoom_to_selected_violation {
            if let Some(loc) = self
                .drc
                .selected()
                .and_then(|i| self.drc.violations().get(i).map(|v| v.location))
            {
                // 25% border so the violation reads clearly with surrounding context.
                self.camera.zoom_to_rect(&screen, loc, 0.25);
            }
            self.zoom_to_selected_violation = false;
        }

        // Deferred locate to the outline row the search panel just clicked. Framed
        // with the same 25% border as the DRC locate so the target reads clearly.
        if let Some(target) = self.search.pending_locate.take() {
            self.camera.zoom_to_rect(&screen, target, 0.25);
        }

        // Input routes to the focused pane only; the click that switched focus is
        // consumed, and pointer positions over other panes never reach the tools.
        let pointer_in_pane = response
            .hover_pos()
            .is_none_or(|p| viewports::contains(&screen, p.x, p.y));
        if !focus_changed && pointer_in_pane {
            self.process_canvas_input(ui.ctx(), &response, &screen);
        } else {
            self.cursor_world = None;
            self.snap_hint = None;
        }

        // Draw grid + rulers under the geometry.
        if self.grid.visible {
            self.draw_grid(&painter, &screen);
        }

        // Draw the scene (shapes or, at low zoom, cell boxes).
        let viewport = self.camera.visible_world_rect(&screen);
        match culling::lod_for_zoom(self.camera.pixels_per_dbu()) {
            DetailLevel::Shapes => match gpu_format {
                // GPU path: render the retained scene through eframe's device. The
                // callback composites under the egui overlays queued below.
                Some(format) => {
                    self.draw_shapes_gpu(&painter, &screen, egui_rect_of(&screen), format);
                }
                // Fallback: paint the geometry with egui.
                None => self.draw_shapes(&painter, &screen, viewport),
            },
            DetailLevel::CellBoxes => self.draw_cell_boxes(&painter, &screen, viewport),
        }

        // Engine-driven overlays on top of the geometry.
        self.draw_net_highlight(&painter, &screen, viewport);
        self.draw_array_preview(&painter, &screen, viewport);
        self.draw_generate_preview(&painter, &screen, viewport);
        self.draw_drc_markers(&painter, &screen);

        // User guides under the ruler bars, then the rulers cover their ends.
        self.draw_guides(&painter, &screen);
        self.draw_rulers(&painter, &screen);
        self.draw_measure(&painter, &screen);
        self.draw_draw_overlay(&painter, &screen, &response, ui.ctx());
        // The snap indicator rides on top so it is never hidden by geometry.
        self.draw_snap_indicator(&painter, &screen);
        if self.labels_visible {
            self.draw_labels(&painter, &screen);
        }
        if self.minimap_visible {
            self.draw_minimap(&painter, &screen);
        }
        self.draw_presence(&painter, &screen);
        self.draw_agent_cursor(&painter, &screen);

        // Mark the focused pane when split (drawn unclipped so the full border
        // stroke shows).
        if split {
            base_painter.rect_stroke(
                egui_rect_of(&screen),
                0.0,
                Stroke::new(1.5, Color32::from_rgb(110, 160, 255)),
                StrokeKind::Middle,
            );
        }

        screen
    }

    /// Focuses the pane under a click or a fresh drag, swapping cameras through
    /// [`Viewports::focus`].
    ///
    /// Returns whether the event moved focus; such an event is consumed, so the
    /// same click can never also select or measure in the newly focused pane.
    fn route_pane_focus(&mut self, response: &egui::Response, panes: &[ScreenRect]) -> bool {
        if !(response.clicked() || response.drag_started()) {
            return false;
        }
        let Some(pos) = response.interact_pointer_pos() else {
            return false;
        };
        let Some(hit) = viewports::hit_pane(panes, pos.x, pos.y) else {
            return false;
        };
        if hit == self.viewports.focused() {
            return false;
        }
        self.viewports.focus(hit, &mut self.camera);
        self.status.set(format!("Pane {} focused", hit + 1));
        true
    }

    /// Draws read-only previews of the unfocused panes.
    ///
    /// Each pane renders the shared document through its own stored camera using
    /// the egui fallback path: the retained GPU callback binds a single camera per
    /// frame and its paint path is owned by the render lane, so secondary panes
    /// deliberately stay on the CPU painter. Tools, overlays, and edits apply only
    /// to the focused pane; a click here focuses the pane first.
    fn draw_unfocused_panes(&mut self, painter: &egui::Painter, panes: &[ScreenRect]) {
        let border = Stroke::new(1.0, Color32::from_rgb(70, 76, 90));
        for (i, pane) in panes.iter().enumerate() {
            if i == self.viewports.focused() {
                continue;
            }
            let Some(cam) = self.viewports.camera(i).copied() else {
                continue;
            };
            let pane_rect = egui_rect_of(pane);
            let clipped = painter.with_clip_rect(pane_rect);
            // Temporarily adopt the pane camera so the existing draw helpers
            // (which read `self.camera`) render this pane's view, then restore.
            let saved = self.camera;
            self.camera = cam;
            let viewport = self.camera.visible_world_rect(pane);
            match culling::lod_for_zoom(self.camera.pixels_per_dbu()) {
                DetailLevel::Shapes => self.draw_shapes(&clipped, pane, viewport),
                DetailLevel::CellBoxes => self.draw_cell_boxes(&clipped, pane, viewport),
            }
            self.camera = saved;
            painter.rect_stroke(pane_rect, 0.0, border, StrokeKind::Middle);
        }
    }

    /// Routes pointer input on the canvas through the active tool.
    fn process_canvas_input(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        screen: &ScreenRect,
    ) {
        // Track the cursor world position (snapped) for the status bar, and stash
        // the snap hint so the canvas can draw the snap indicator this frame.
        if let Some(pos) = response.hover_pos() {
            let raw = self.camera.screen_to_world(screen, pos.x, pos.y);
            let (snapped, hint) = self.snap_world(raw);
            self.cursor_world = Some(snapped);
            self.snap_hint = hint;
        } else {
            self.cursor_world = None;
            self.snap_hint = None;
        }

        // Guide drag: a drag that begins inside a ruler bar pulls out a guide line.
        // While such a drag is live it owns the pointer, so no tool acts on it; on
        // release the guide is dropped at the cursor (grid-snapped). Handled before
        // every tool so the ruler always wins the gesture.
        if self.handle_guide_drag(response, screen) {
            return;
        }

        // Minimap navigation: a click or drag inside the panel recenters the view
        // there and consumes the input so no tool acts on it.
        if self.minimap_visible
            && (response.clicked() || response.dragged())
            && let (Some(bounds), Some(pos)) =
                (self.scene.bounds(), response.interact_pointer_pos())
            && let Some(layout) = MinimapLayout::compute(screen, bounds)
            && layout.contains(pos.x, pos.y)
        {
            let center = layout.panel_to_world(pos.x, pos.y);
            self.camera = ViewCamera::new(center, self.camera.pixels_per_dbu());
            return;
        }

        // Zoom to cursor on scroll, regardless of tool.
        if response.hovered() {
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0
                && let Some(pos) = response.hover_pos()
            {
                let factor = (f64::from(scroll) * 0.0015).exp();
                self.camera.zoom_about(screen, factor, pos.x, pos.y);
            }
        }

        match self.tools.active() {
            Tool::Pan => {
                if response.dragged() {
                    let d = response.drag_delta();
                    self.camera.pan_pixels(d.x, d.y);
                }
            }
            Tool::Measure => {
                if response.clicked()
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    let raw = self.camera.screen_to_world(screen, pos.x, pos.y);
                    let (world, _) = self.snap_world(raw);
                    let dpm = self.dbu_per_micron();
                    if let Some(m) = self.tools.measure_click(world, dpm) {
                        self.status.set(format!(
                            "Distance {:.1} DBU ({:.3} um)",
                            m.distance_dbu(),
                            m.distance_microns()
                        ));
                    }
                }
            }
            Tool::Select => self.handle_select_input(ctx, response, screen),
            Tool::CutLine => {
                if response.clicked()
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    let raw = self.camera.screen_to_world(screen, pos.x, pos.y);
                    let (world, _) = self.snap_world(raw);
                    if let Some((a, b)) = self.tools.cutline_click(world) {
                        self.status
                            .set(format!("Cut ({}, {}) -> ({}, {})", a.x, a.y, b.x, b.y));
                    } else {
                        self.status.set("Cut line: pick the second point");
                    }
                }
            }
            Tool::DrawRect => self.handle_draw_rect_input(ctx, response, screen),
            Tool::DrawPolygon => self.handle_draw_polygon_input(ctx, response, screen),
            Tool::DrawPath => self.handle_draw_path_input(ctx, response, screen),
            Tool::EditVertex => self.handle_edit_vertex_input(ctx, response, screen),
        }
    }

    /// Switches to `tool`, resetting any half-drawn shape or vertex grab when the new
    /// tool is not a drawing tool (or when leaving one), so in-progress geometry never
    /// leaks between tools. The path width and end cap survive (see
    /// [`crate::draw::DrawState::reset`]).
    fn select_tool(&mut self, tool: Tool) {
        if self.tools.active() != tool && (self.tools.active().is_draw() || !tool.is_draw()) {
            self.draw.reset();
        }
        self.tools.set_active(tool);
    }

    /// Rectangle tool: drag to rubber-band a rectangle, with shift (square) and
    /// alt/ctrl (from-center) constraints; commit on release as an undo-integrated
    /// [`Edit::AddShape`](reticle_model::Edit).
    fn handle_draw_rect_input(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        screen: &ScreenRect,
    ) {
        if !response.drag_stopped() {
            return;
        }
        let (Some(origin), Some(current)) = (Self::drag_origin(response), response.hover_pos())
        else {
            return;
        };
        let anchor = self
            .grid
            .snap(self.camera.screen_to_world(screen, origin.x, origin.y));
        let cursor = self
            .grid
            .snap(self.camera.screen_to_world(screen, current.x, current.y));
        let mods = Self::rect_mods(ctx);
        let rect = crate::draw::rect_from_drag(anchor, cursor, mods);
        if rect.is_empty() {
            return;
        }
        self.commit_shape(ShapeKind::Rect(rect), "Drew rectangle");
    }

    /// Polygon tool: each click places a vertex; a double-click or Enter closes the
    /// ring into a polygon; Escape cancels the in-progress ring.
    fn handle_draw_polygon_input(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        screen: &ScreenRect,
    ) {
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.draw.poly.clear();
            self.status.set("Polygon cancelled");
            return;
        }
        let finish = response.double_clicked() || ctx.input(|i| i.key_pressed(egui::Key::Enter));
        if (response.clicked() || response.double_clicked())
            && let Some(pos) = response.interact_pointer_pos()
        {
            let world = self
                .grid
                .snap(self.camera.screen_to_world(screen, pos.x, pos.y));
            self.draw.poly.push(world);
        }
        if finish {
            if let Some(poly) = std::mem::take(&mut self.draw.poly).finish() {
                let n = poly.len();
                self.commit_shape(ShapeKind::Polygon(poly), &format!("Drew polygon ({n} pts)"));
            } else {
                self.draw.poly.clear();
                self.status.set("Polygon needs at least 3 vertices");
            }
        } else if !self.draw.poly.is_empty() {
            self.status.set(format!(
                "Polygon: {} vertices (double-click to close)",
                self.draw.poly.len()
            ));
        }
    }

    /// Path tool: each click places a point; a double-click or Enter finishes the
    /// wire with the toolbar's width and end cap; Escape cancels it.
    fn handle_draw_path_input(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        screen: &ScreenRect,
    ) {
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.draw.path.clear();
            self.status.set("Path cancelled");
            return;
        }
        let finish = response.double_clicked() || ctx.input(|i| i.key_pressed(egui::Key::Enter));
        if (response.clicked() || response.double_clicked())
            && let Some(pos) = response.interact_pointer_pos()
        {
            let world = self
                .grid
                .snap(self.camera.screen_to_world(screen, pos.x, pos.y));
            self.draw.path.push(world);
        }
        if finish {
            // Keep the width and end cap by rebuilding a fresh builder from the taken
            // one's settings after finishing.
            let width = self.draw.path.width();
            let endcap = self.draw.path.endcap();
            let builder = std::mem::take(&mut self.draw.path);
            if let Some(path) = builder.finish() {
                let n = path.points().len();
                self.commit_shape(ShapeKind::Path(path), &format!("Drew path ({n} pts)"));
            } else {
                self.status.set("Path needs at least 2 points");
            }
            self.draw.path.set_width(width);
            self.draw.path.set_endcap(endcap);
        } else if !self.draw.path.is_empty() {
            self.status.set(format!(
                "Path: {} points (double-click to finish)",
                self.draw.path.len()
            ));
        }
    }

    /// Vertex-edit tool over the selected shape: drag a vertex to move it, alt-click a
    /// vertex to delete it, or click on an edge to insert one. Only the top cell's own
    /// shapes (scene indices below its direct-shape count) are editable.
    fn handle_edit_vertex_input(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        screen: &ScreenRect,
    ) {
        let Some(shape_idx) = self.editable_selection() else {
            if response.clicked() {
                self.status
                    .set("Select a shape you drew to edit its vertices");
            }
            return;
        };
        let radius = self.vertex_pick_radius();
        let (verts, closed) = {
            let kind = &self.scene.shapes()[shape_idx].kind;
            crate::draw::editable_vertices(kind)
        };

        // Begin a drag: grab the nearest vertex under the press.
        if response.drag_started()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let world = self.camera.screen_to_world(screen, pos.x, pos.y);
            if let Some(v) = crate::draw::nearest_vertex(&verts, world, radius) {
                self.draw.grab = Some(crate::draw::VertexGrab {
                    shape: shape_idx,
                    vertex: v,
                });
            }
        }

        // Commit a vertex move on release.
        if response.drag_stopped() {
            if let (Some(grab), Some(pos)) = (self.draw.grab.take(), response.hover_pos()) {
                let to = self
                    .grid
                    .snap(self.camera.screen_to_world(screen, pos.x, pos.y));
                let moved = crate::draw::move_vertex(&verts, grab.vertex, to);
                self.replace_shape_vertices(shape_idx, moved, "Moved vertex");
            }
            return;
        }

        // A plain click either deletes (with a modifier) or inserts on an edge.
        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let world = self.camera.screen_to_world(screen, pos.x, pos.y);
            let delete_mod =
                ctx.input(|i| i.modifiers.alt || i.modifiers.command || i.modifiers.ctrl);
            if delete_mod {
                if let Some(v) = crate::draw::nearest_vertex(&verts, world, radius) {
                    let floor = if closed { 3 } else { 2 };
                    let (out, ok) = crate::draw::delete_vertex(&verts, v, floor);
                    if ok {
                        self.replace_shape_vertices(shape_idx, out, "Deleted vertex");
                    } else {
                        self.status
                            .set("Cannot delete: shape is at its minimum vertices");
                    }
                }
            } else if let Some(ins) =
                crate::draw::nearest_segment_insertion(&verts, world, radius, closed)
            {
                let out = crate::draw::insert_vertex_on_segment(&verts, ins.index, ins.point);
                self.replace_shape_vertices(shape_idx, out, "Inserted vertex");
            }
        }
    }

    /// The single selected shape's scene index if it is one of the top cell's own
    /// directly-editable shapes.
    ///
    /// The flattened scene lists the top cell's own shapes first (before any
    /// instances), so a scene index below the cell's direct-shape count maps exactly
    /// to `cell.shapes[index]`, which the vertex edit rewrites in place. A selection
    /// that is not exactly one such shape returns `None`.
    fn editable_selection(&self) -> Option<usize> {
        if self.selection.len() != 1 {
            return None;
        }
        let idx = self.selection.iter().next()?;
        let direct = self
            .history
            .document()
            .cell(&self.top_cell)
            .map_or(0, |c| c.shapes.len());
        (idx < direct).then_some(idx)
    }

    /// The vertex hit radius in DBU: a few screen pixels converted through the camera
    /// so picking feels the same at any zoom.
    fn vertex_pick_radius(&self) -> i64 {
        let ppd = self.camera.pixels_per_dbu().max(f64::MIN_POSITIVE);
        ((8.0 / ppd).round() as i64).max(1)
    }

    /// Reads the rectangle-drag modifiers (shift squares, alt/ctrl from-center) from
    /// the current egui input state.
    fn rect_mods(ctx: &egui::Context) -> crate::draw::RectMods {
        let m = ctx.input(|i| i.modifiers);
        crate::draw::RectMods::new(m.shift, m.alt, m.command || m.ctrl)
    }

    /// Commits a freshly drawn shape on the current layer as an undo-integrated edit.
    ///
    /// The layer is the first row in the layer table (falling back to layer 1/0), the
    /// same default the demo "add rectangle" action uses. On success the scene is
    /// rebuilt so the new shape is immediately pickable.
    fn commit_shape(&mut self, kind: ShapeKind, status: &str) {
        let layer = self
            .layer_state
            .rows()
            .first()
            .map_or(LayerId::new(1, 0), |r| r.id);
        let shape = DrawShape::new(layer, kind);
        match self.history.apply(reticle_model::Edit::AddShape {
            cell: self.top_cell.clone(),
            shape,
        }) {
            Ok(()) => {
                self.rebuild_scene();
                self.status.set(status.to_owned());
            }
            Err(e) => self.status.set(format!("Draw failed: {e}")),
        }
    }

    /// Replaces the top cell's shape at scene index `shape_idx` with a copy whose
    /// vertex ring is `vertices`, as a single undoable remove-then-add.
    ///
    /// The shape family is preserved through [`crate::draw::rebuild_kind`] (a
    /// rectangle promotes to a polygon once a corner leaves axis-alignment; a path
    /// keeps its width and cap). A ring that would be degenerate is declined. Because
    /// the scene lists direct shapes first, `shape_idx` is also the cell shape index
    /// the [`Edit::RemoveShape`](reticle_model::Edit) targets.
    fn replace_shape_vertices(&mut self, shape_idx: usize, vertices: Vec<Point>, status: &str) {
        let original = self.scene.shapes()[shape_idx].clone();
        let Some(kind) = crate::draw::rebuild_kind(&original.kind, vertices) else {
            self.status.set("Edit declined: too few vertices");
            return;
        };
        let replacement = DrawShape::new(original.layer, kind);
        if let Err(e) = self.history.apply(reticle_model::Edit::RemoveShape {
            cell: self.top_cell.clone(),
            index: shape_idx,
        }) {
            self.status.set(format!("Edit failed: {e}"));
            return;
        }
        match self.history.apply(reticle_model::Edit::AddShape {
            cell: self.top_cell.clone(),
            shape: replacement,
        }) {
            Ok(()) => {
                self.rebuild_scene();
                self.status.set(status.to_owned());
            }
            Err(e) => self.status.set(format!("Edit failed: {e}")),
        }
    }

    /// Select-tool input: click to pick the topmost shape, drag to rubber-band.
    fn handle_select_input(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        screen: &ScreenRect,
    ) {
        let additive = ctx.input(|i| i.modifiers.shift || i.modifiers.command || i.modifiers.ctrl);

        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let world = self.camera.screen_to_world(screen, pos.x, pos.y);
            match self.scene.pick(world) {
                Some(idx) => {
                    if additive {
                        self.selection.toggle(idx);
                    } else {
                        self.selection.select_one(idx);
                    }
                    self.highlight_net_of(idx);
                }
                None => {
                    if !additive {
                        self.selection.clear();
                        // Clicking empty space clears the connected-net highlight.
                        self.netlight.clear();
                    }
                }
            }
        }

        // Rubber-band: on drag release, select shapes fully inside the drag box.
        if response.drag_stopped()
            && let (Some(origin), Some(current)) =
                (Self::drag_origin(response), response.hover_pos())
        {
            let a = self.camera.screen_to_world(screen, origin.x, origin.y);
            let b = self.camera.screen_to_world(screen, current.x, current.y);
            let band = Rect::new(a, b);
            if band.width() > 0 && band.height() > 0 {
                let hits = selection::shapes_in_rect(self.scene.shapes(), band);
                if additive {
                    self.selection.extend(hits);
                } else {
                    self.selection.set(hits);
                }
            }
        }
    }

    /// The screen position where the current drag started, if any.
    ///
    /// egui exposes the press origin via `interact_pointer_pos` during a drag; when
    /// the drag has just stopped we reconstruct the origin from the current pointer
    /// and the accumulated drag delta.
    fn drag_origin(response: &egui::Response) -> Option<Pos2> {
        let current = response.hover_pos()?;
        let delta = response.drag_delta();
        Some(Pos2::new(current.x - delta.x, current.y - delta.y))
    }

    /// The guide axis a screen point sits over, if it is inside a ruler bar.
    ///
    /// A press inside the top bar (but past the top-left corner square) starts a
    /// horizontal guide; a press inside the left bar starts a vertical one. The
    /// shared corner square belongs to neither, so a drag from it starts no guide.
    fn ruler_axis_at(screen: &ScreenRect, x: f32, y: f32) -> Option<snap::Axis> {
        let in_top = y >= screen.top && y < screen.top + RULER_BAR;
        let in_left = x >= screen.left && x < screen.left + RULER_BAR;
        if in_left && y >= screen.top + RULER_BAR {
            Some(snap::Axis::Vertical)
        } else if in_top && x >= screen.left + RULER_BAR {
            Some(snap::Axis::Horizontal)
        } else {
            None
        }
    }

    /// Handles pulling a guide off a ruler; returns whether it consumed the input.
    ///
    /// Starting a drag inside a ruler bar arms a guide drag ([`App::dragging_guide`])
    /// and consumes every event until release, so no tool sees the gesture. On
    /// release the guide is committed at the grid-snapped release coordinate.
    fn handle_guide_drag(&mut self, response: &egui::Response, screen: &ScreenRect) -> bool {
        if response.drag_started()
            && let Some(pos) = response.interact_pointer_pos()
            && let Some(axis) = Self::ruler_axis_at(screen, pos.x, pos.y)
        {
            self.dragging_guide = Some(axis);
            return true;
        }
        let Some(axis) = self.dragging_guide else {
            return false;
        };
        if response.drag_stopped() {
            if let Some(pos) = response.hover_pos().or_else(|| Self::drag_origin(response)) {
                let world = self
                    .grid
                    .snap(self.camera.screen_to_world(screen, pos.x, pos.y));
                let guide = match axis {
                    snap::Axis::Horizontal => Guide::horizontal(world.y),
                    snap::Axis::Vertical => Guide::vertical(world.x),
                };
                self.snap.add_guide(guide);
                self.status.set(match axis {
                    snap::Axis::Horizontal => format!("Guide y = {}", world.y),
                    snap::Axis::Vertical => format!("Guide x = {}", world.x),
                });
            }
            self.dragging_guide = None;
        }
        // Owns the pointer for the whole drag (dragged, stopped, or the idle frame
        // between arming and the first motion).
        true
    }

    /// Draws the background grid lines within the canvas.
    fn draw_grid(&self, painter: &egui::Painter, screen: &ScreenRect) {
        let ppd = self.camera.pixels_per_dbu();
        let step = self.grid.display_step_dbu(ppd);
        let world = self.camera.visible_world_rect(screen);
        let color = Color32::from_rgb(34, 38, 46);
        let stroke = Stroke::new(1.0, color);

        for x in grid::grid_lines(world.min.x, world.max.x, step) {
            let (sx, _) = self
                .camera
                .world_to_screen(screen, Point::new(x, world.min.y));
            painter.line_segment(
                [
                    Pos2::new(sx, screen.top),
                    Pos2::new(sx, screen.top + screen.height),
                ],
                stroke,
            );
        }
        for y in grid::grid_lines(world.min.y, world.max.y, step) {
            let (_, sy) = self
                .camera
                .world_to_screen(screen, Point::new(world.min.x, y));
            painter.line_segment(
                [
                    Pos2::new(screen.left, sy),
                    Pos2::new(screen.left + screen.width, sy),
                ],
                stroke,
            );
        }

        // Emphasize the world axes.
        let axis = Stroke::new(1.5, Color32::from_rgb(60, 66, 78));
        let (ox, oy) = self.camera.world_to_screen(screen, Point::ORIGIN);
        painter.line_segment(
            [
                Pos2::new(ox, screen.top),
                Pos2::new(ox, screen.top + screen.height),
            ],
            axis,
        );
        painter.line_segment(
            [
                Pos2::new(screen.left, oy),
                Pos2::new(screen.left + screen.width, oy),
            ],
            axis,
        );
    }

    /// Draws the visible, non-hidden shapes with per-layer colors, highlighting the
    /// current selection.
    fn draw_shapes(&self, painter: &egui::Painter, screen: &ScreenRect, viewport: Rect) {
        let shapes = self.scene.shapes();
        for idx in self.scene.query(viewport) {
            let shape = &shapes[idx];
            if !self.layer_state.is_visible(shape.layer) {
                continue;
            }
            let (r, g, b, a) = self.layer_color(shape.layer);
            let fill = Color32::from_rgba_unmultiplied(r, g, b, a);
            let selected = self.selection.contains(idx);
            self.draw_one_shape(painter, screen, shape, fill, selected);
        }
    }

    /// Renders the layout geometry on the GPU through a retained paint callback.
    ///
    /// Refreshes the retained scene (a no-op unless the document or layer visibility
    /// changed), builds the camera projection for the canvas, and queues an
    /// [`eframe::egui_wgpu::Callback`] whose [`SceneCallback`] uploads and draws the scene on
    /// eframe's device. egui overlays queued after this composite on top.
    fn draw_shapes_gpu(
        &mut self,
        painter: &egui::Painter,
        screen: &ScreenRect,
        rect: EguiRect,
        format: eframe::egui_wgpu::wgpu::TextureFormat,
    ) {
        self.sync_retained();
        let camera = self.camera.to_model_camera(screen);
        // The projection uses the canvas size in points; egui sets the physical-pixel
        // viewport for the pass from the callback rect.
        let width = screen.width.max(1.0) as u32;
        let height = screen.height.max(1.0) as u32;
        let view = ViewUniform::from_camera(&camera, width, height);

        let callback = SceneCallback {
            view,
            revision: self.render_revision,
            expanded: Arc::clone(&self.expanded),
            format,
        };
        painter.add(eframe::egui_wgpu::Callback::new_paint_callback(
            rect, callback,
        ));
    }

    /// Draws a single [`DrawShape`] in the given fill color.
    fn draw_one_shape(
        &self,
        painter: &egui::Painter,
        screen: &ScreenRect,
        shape: &DrawShape,
        fill: Color32,
        selected: bool,
    ) {
        let outline = if selected {
            Stroke::new(2.0, Color32::from_rgb(255, 240, 120))
        } else {
            Stroke::new(1.0, fill.gamma_multiply(1.4))
        };
        match &shape.kind {
            ShapeKind::Rect(rect) => {
                let e = self.world_rect_to_screen(screen, *rect);
                painter.rect_filled(e, 0.0, fill);
                painter.rect_stroke(e, 0.0, outline, StrokeKind::Middle);
            }
            ShapeKind::Polygon(poly) => {
                let pts: Vec<Pos2> = poly
                    .vertices()
                    .iter()
                    .map(|p| self.world_pos_to_screen(screen, *p))
                    .collect();
                if pts.len() >= 3 {
                    painter.add(Shape::convex_polygon(pts, fill, outline));
                }
            }
            ShapeKind::Path(path) => {
                let pts: Vec<Pos2> = path
                    .points()
                    .iter()
                    .map(|p| self.world_pos_to_screen(screen, *p))
                    .collect();
                if pts.len() >= 2 {
                    // Width in screen pixels, at least 1px so thin wires stay visible.
                    let w =
                        (f64::from(path.width()) * self.camera.pixels_per_dbu()).max(1.0) as f32;
                    let stroke = if selected {
                        Stroke::new(w.max(2.0), Color32::from_rgb(255, 240, 120))
                    } else {
                        Stroke::new(w, fill)
                    };
                    painter.add(Shape::line(pts, stroke));
                }
            }
        }
    }

    /// Draws cell bounding boxes for the low-zoom level of detail.
    fn draw_cell_boxes(&self, painter: &egui::Painter, screen: &ScreenRect, viewport: Rect) {
        let stroke = Stroke::new(1.0, Color32::from_rgb(120, 140, 180));
        let fill = Color32::from_rgba_unmultiplied(60, 80, 120, 40);
        for cb in culling::visible_cell_boxes(self.history.document(), &self.top_cell, viewport) {
            let e = self.world_rect_to_screen(screen, cb.bbox);
            painter.rect_filled(e, 0.0, fill);
            painter.rect_stroke(e, 0.0, stroke, StrokeKind::Middle);
        }
    }

    /// Draws the highlighted net as a bright outline over its member shapes.
    ///
    /// Only members intersecting `viewport` are drawn, so the cost is bounded by what
    /// is on screen. The net indices come from [`Netlight`] and are indices into the
    /// same flattened scene shape list, so they map directly to `self.scene.shapes()`.
    fn draw_net_highlight(&self, painter: &egui::Painter, screen: &ScreenRect, viewport: Rect) {
        if self.netlight.is_empty() {
            return;
        }
        let shapes = self.scene.shapes();
        let color = Color32::from_rgb(120, 230, 255);
        let stroke = Stroke::new(2.5, color);
        let fill = Color32::from_rgba_unmultiplied(120, 230, 255, 60);
        for &idx in self.netlight.highlighted() {
            let Some(shape) = shapes.get(idx) else {
                continue;
            };
            if !shape.bounding_box().intersects(&viewport) {
                continue;
            }
            self.draw_shape_outline(painter, screen, shape, stroke, fill);
        }
    }

    /// Draws the interactive array tool's live preview: a faint outline of each
    /// pending array element before the user commits.
    ///
    /// The preview shapes come from [`array_preview_shapes`](Self::array_preview_shapes)
    /// (the element `1..` copies of the current selection at the panel's pitch), so
    /// this is empty unless preview is on, something is selected, and the count is
    /// within the cap. Only elements intersecting `viewport` are drawn.
    fn draw_array_preview(&self, painter: &egui::Painter, screen: &ScreenRect, viewport: Rect) {
        let preview = self.array_preview_shapes();
        if preview.is_empty() {
            return;
        }
        let color = Color32::from_rgb(180, 210, 120);
        let stroke = Stroke::new(1.5, color);
        let fill = Color32::from_rgba_unmultiplied(180, 210, 120, 40);
        for shape in &preview {
            if !shape.bounding_box().intersects(&viewport) {
                continue;
            }
            self.draw_shape_outline(painter, screen, shape, stroke, fill);
        }
    }

    /// Draws the Generate panel's live preview as a canvas overlay: the geometry the
    /// selected generator produces for the current form values, in a distinct accent.
    ///
    /// The shapes come from [`generate_preview_shapes`](Self::generate_preview_shapes),
    /// so this is empty unless the preview toggle is on and the parameters currently
    /// generate. Only shapes intersecting `viewport` are drawn. The structure is
    /// placed with its lower-left at the world origin (where the generators emit it),
    /// so the overlay shows exactly what a Generate places.
    fn draw_generate_preview(&self, painter: &egui::Painter, screen: &ScreenRect, viewport: Rect) {
        let preview = self.generate_preview_shapes();
        if preview.is_empty() {
            return;
        }
        let color = Color32::from_rgb(120, 190, 235);
        let stroke = Stroke::new(1.5, color);
        let fill = Color32::from_rgba_unmultiplied(120, 190, 235, 40);
        for shape in &preview {
            if !shape.bounding_box().intersects(&viewport) {
                continue;
            }
            self.draw_shape_outline(painter, screen, shape, stroke, fill);
        }
    }

    /// Draws a marker at every DRC violation location, emphasizing the selected one.
    ///
    /// Each violation is drawn as an outlined rectangle at its `location` (world to
    /// screen via the camera); the violation the user clicked in the list is drawn in
    /// a hotter color and slightly inflated so it stands out.
    fn draw_drc_markers(&self, painter: &egui::Painter, screen: &ScreenRect) {
        let selected = self.drc.selected();
        let normal = Stroke::new(2.0, Color32::from_rgb(255, 90, 90));
        let hot = Stroke::new(3.0, Color32::from_rgb(255, 200, 60));
        for (i, v) in self.drc.violations().iter().enumerate() {
            let is_sel = selected == Some(i);
            let e = self.world_rect_to_screen(screen, v.location);
            // Inflate a touch so a zero-area location still shows as a small box.
            let e = e.expand(if is_sel { 4.0 } else { 2.0 });
            painter.rect_stroke(
                e,
                0.0,
                if is_sel { hot } else { normal },
                StrokeKind::Middle,
            );
        }
    }

    /// Draws just the outline (and a faint fill) of a shape, for overlay emphasis.
    ///
    /// Unlike [`draw_one_shape`](Self::draw_one_shape) this never uses the shape's
    /// layer color; it is used by the net-highlight overlay to trace connected
    /// geometry in a single accent color.
    fn draw_shape_outline(
        &self,
        painter: &egui::Painter,
        screen: &ScreenRect,
        shape: &DrawShape,
        stroke: Stroke,
        fill: Color32,
    ) {
        match &shape.kind {
            ShapeKind::Rect(rect) => {
                let e = self.world_rect_to_screen(screen, *rect);
                painter.rect_filled(e, 0.0, fill);
                painter.rect_stroke(e, 0.0, stroke, StrokeKind::Middle);
            }
            ShapeKind::Polygon(poly) => {
                let pts: Vec<Pos2> = poly
                    .vertices()
                    .iter()
                    .map(|p| self.world_pos_to_screen(screen, *p))
                    .collect();
                if pts.len() >= 3 {
                    painter.add(Shape::convex_polygon(pts, fill, stroke));
                }
            }
            ShapeKind::Path(path) => {
                let pts: Vec<Pos2> = path
                    .points()
                    .iter()
                    .map(|p| self.world_pos_to_screen(screen, *p))
                    .collect();
                if pts.len() >= 2 {
                    painter.add(Shape::line(pts, stroke));
                }
            }
        }
    }

    /// Draws top/left rulers with tick marks and DBU labels.
    fn draw_rulers(&self, painter: &egui::Painter, screen: &ScreenRect) {
        let bar = RULER_BAR;
        let bg = Color32::from_rgb(24, 27, 33);
        let top_bar = EguiRect::from_min_size(
            Pos2::new(screen.left, screen.top),
            Vec2::new(screen.width, bar),
        );
        let left_bar = EguiRect::from_min_size(
            Pos2::new(screen.left, screen.top),
            Vec2::new(bar, screen.height),
        );
        painter.rect_filled(top_bar, 0.0, bg);
        painter.rect_filled(left_bar, 0.0, bg);

        let ppd = self.camera.pixels_per_dbu();
        let step = self.grid.display_step_dbu(ppd);
        let world = self.camera.visible_world_rect(screen);
        let tick = Stroke::new(1.0, Color32::from_rgb(90, 96, 110));
        let font = FontId::monospace(9.0);
        let label = Color32::from_rgb(170, 176, 190);

        for x in grid::grid_lines(world.min.x, world.max.x, step) {
            let (sx, _) = self.camera.world_to_screen(screen, Point::new(x, 0));
            if sx < screen.left + bar {
                continue;
            }
            painter.line_segment(
                [Pos2::new(sx, screen.top), Pos2::new(sx, screen.top + bar)],
                tick,
            );
            painter.text(
                Pos2::new(sx + 2.0, screen.top + 1.0),
                Align2::LEFT_TOP,
                x.to_string(),
                font.clone(),
                label,
            );
        }
        for y in grid::grid_lines(world.min.y, world.max.y, step) {
            let (_, sy) = self.camera.world_to_screen(screen, Point::new(0, y));
            if sy < screen.top + bar {
                continue;
            }
            painter.line_segment(
                [Pos2::new(screen.left, sy), Pos2::new(screen.left + bar, sy)],
                tick,
            );
            painter.text(
                Pos2::new(screen.left + 1.0, sy + 1.0),
                Align2::LEFT_TOP,
                y.to_string(),
                font.clone(),
                label,
            );
        }
    }

    /// Draws the user guide lines across the canvas.
    ///
    /// Horizontal guides span the full width at their world `y`; vertical guides
    /// span the full height at their world `x`. Guides off screen are skipped. A
    /// guide being actively dragged has no committed line yet, so nothing special is
    /// drawn for it here; it appears once released.
    fn draw_guides(&self, painter: &egui::Painter, screen: &ScreenRect) {
        let stroke = Stroke::new(1.0, Color32::from_rgb(80, 200, 220));
        for g in &self.snap.guides {
            match g.axis {
                snap::Axis::Horizontal => {
                    let (_, sy) = self.camera.world_to_screen(screen, Point::new(0, g.coord));
                    if sy >= screen.top && sy <= screen.top + screen.height {
                        painter.line_segment(
                            [
                                Pos2::new(screen.left, sy),
                                Pos2::new(screen.left + screen.width, sy),
                            ],
                            stroke,
                        );
                    }
                }
                snap::Axis::Vertical => {
                    let (sx, _) = self.camera.world_to_screen(screen, Point::new(g.coord, 0));
                    if sx >= screen.left && sx <= screen.left + screen.width {
                        painter.line_segment(
                            [
                                Pos2::new(sx, screen.top),
                                Pos2::new(sx, screen.top + screen.height),
                            ],
                            stroke,
                        );
                    }
                }
            }
        }
    }

    /// Draws the snap indicator at the point the cursor caught this frame.
    ///
    /// A small diamond marks the snapped point, colored by the kind of feature it
    /// hit, with a short caption naming the kind. Nothing is drawn when the cursor
    /// caught neither geometry nor a guide this frame.
    fn draw_snap_indicator(&self, painter: &egui::Painter, screen: &ScreenRect) {
        let Some(hint) = self.snap_hint else {
            return;
        };
        let color = match hint.kind {
            snap::SnapKind::Vertex => Color32::from_rgb(120, 230, 140),
            snap::SnapKind::Midpoint => Color32::from_rgb(230, 200, 110),
            snap::SnapKind::Center => Color32::from_rgb(220, 140, 220),
            snap::SnapKind::Edge => Color32::from_rgb(120, 190, 240),
            snap::SnapKind::Guide => Color32::from_rgb(80, 200, 220),
        };
        let c = self.world_pos_to_screen(screen, hint.point);
        // A diamond (a rotated square) drawn as four segments around the point.
        let r = 5.0;
        let pts = [
            Pos2::new(c.x, c.y - r),
            Pos2::new(c.x + r, c.y),
            Pos2::new(c.x, c.y + r),
            Pos2::new(c.x - r, c.y),
        ];
        let stroke = Stroke::new(1.5, color);
        for i in 0..pts.len() {
            painter.line_segment([pts[i], pts[(i + 1) % pts.len()]], stroke);
        }
        painter.text(
            Pos2::new(c.x + r + 3.0, c.y - r - 2.0),
            Align2::LEFT_BOTTOM,
            hint.kind.label(),
            FontId::monospace(10.0),
            color,
        );
    }

    /// Draws the in-progress or completed measurement overlay.
    fn draw_measure(&self, painter: &egui::Painter, screen: &ScreenRect) {
        let color = Color32::from_rgb(255, 210, 90);
        let stroke = Stroke::new(1.5, color);
        if let Some(m) = self.tools.measurement() {
            let a = self.world_pos_to_screen(screen, m.start);
            let b = self.world_pos_to_screen(screen, m.end);
            painter.line_segment([a, b], stroke);
            painter.circle_filled(a, 3.0, color);
            painter.circle_filled(b, 3.0, color);
            let mid = Pos2::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
            painter.text(
                mid,
                Align2::CENTER_CENTER,
                format!(
                    "{:.0} DBU / {:.2} um",
                    m.distance_dbu(),
                    m.distance_microns()
                ),
                FontId::monospace(11.0),
                color,
            );
        } else if let Some(start) = self.tools.measure_start() {
            // First point placed, awaiting the second.
            let a = self.world_pos_to_screen(screen, start);
            painter.circle_filled(a, 3.0, color);
        }
    }

    /// Draws the live preview for the active drawing or vertex-edit tool.
    ///
    /// The rectangle tool shows the rubber-band box under the current drag (with its
    /// modifier constraints applied); the polygon and path tools show the placed
    /// vertices, the edges between them, and a dashed segment out to the cursor; the
    /// vertex-edit tool ticks every vertex of the editable selection so the user sees
    /// what can be grabbed. Everything is derived from state each frame, so nothing is
    /// cached.
    fn draw_draw_overlay(
        &self,
        painter: &egui::Painter,
        screen: &ScreenRect,
        response: &egui::Response,
        ctx: &egui::Context,
    ) {
        let accent = Color32::from_rgb(120, 200, 255);
        let stroke = Stroke::new(1.5, accent);
        match self.tools.active() {
            Tool::DrawRect => {
                if response.dragged()
                    && let (Some(origin), Some(current)) =
                        (Self::drag_origin(response), response.hover_pos())
                {
                    let anchor = self
                        .grid
                        .snap(self.camera.screen_to_world(screen, origin.x, origin.y));
                    let cursor = self
                        .grid
                        .snap(self.camera.screen_to_world(screen, current.x, current.y));
                    let rect = crate::draw::rect_from_drag(anchor, cursor, Self::rect_mods(ctx));
                    if !rect.is_empty() {
                        let e = self.world_rect_to_screen(screen, rect);
                        painter.rect_stroke(e, 0.0, stroke, StrokeKind::Middle);
                    }
                }
            }
            Tool::DrawPolygon => {
                self.draw_vertex_chain(painter, screen, self.draw.poly.vertices(), true, accent);
            }
            Tool::DrawPath => {
                self.draw_vertex_chain(painter, screen, self.draw.path.points(), false, accent);
            }
            Tool::EditVertex => {
                if let Some(idx) = self.editable_selection() {
                    let (verts, _) = crate::draw::editable_vertices(&self.scene.shapes()[idx].kind);
                    for v in &verts {
                        let s = self.world_pos_to_screen(screen, *v);
                        painter.circle_filled(s, 3.5, accent);
                        painter.circle_stroke(s, 3.5, Stroke::new(1.0, Color32::BLACK));
                    }
                }
            }
            _ => {}
        }
    }

    /// Draws an in-progress vertex chain (polygon or path) plus a live segment to the
    /// cursor, used by the polygon and path preview.
    fn draw_vertex_chain(
        &self,
        painter: &egui::Painter,
        screen: &ScreenRect,
        verts: &[Point],
        close_hint: bool,
        color: Color32,
    ) {
        if verts.is_empty() {
            return;
        }
        let stroke = Stroke::new(1.5, color);
        let pts: Vec<Pos2> = verts
            .iter()
            .map(|v| self.world_pos_to_screen(screen, *v))
            .collect();
        for pair in pts.windows(2) {
            painter.line_segment([pair[0], pair[1]], stroke);
        }
        for pt in &pts {
            painter.circle_filled(*pt, 3.0, color);
        }
        // A faint segment from the last placed vertex to the live cursor.
        if let Some(pos) = self.cursor_world {
            let cursor = self.world_pos_to_screen(screen, pos);
            let last = *pts.last().expect("verts is non-empty");
            painter.line_segment([last, cursor], Stroke::new(1.0, color.gamma_multiply(0.6)));
            // For a polygon, also hint the closing edge back to the first vertex.
            if close_hint && pts.len() >= 2 {
                painter.line_segment(
                    [cursor, pts[0]],
                    Stroke::new(1.0, color.gamma_multiply(0.35)),
                );
            }
        }
    }

    /// Draws the canvas text-label overlay: cell names, the selection caption, and
    /// the live measurement readout.
    ///
    /// egui composites painter text after the GPU paint callback, so this text
    /// always reads on top of the geometry (no extra text-rendering dependency).
    /// Every layout and formatting decision lives in [`crate::labels`]; this method
    /// only converts world rectangles to screen space and issues the text calls.
    fn draw_labels(&self, painter: &egui::Painter, screen: &ScreenRect) {
        let font = FontId::monospace(labels::LABEL_FONT_PX);

        // Cell names, centered in each placement outline, at the cell-box LOD.
        if culling::lod_for_zoom(self.camera.pixels_per_dbu()) == DetailLevel::CellBoxes {
            let viewport = self.camera.visible_world_rect(screen);
            let boxes: Vec<labels::LabelBox> =
                culling::visible_cell_boxes(self.history.document(), &self.top_cell, viewport)
                    .into_iter()
                    .map(|cb| {
                        let e = self.world_rect_to_screen(screen, cb.bbox);
                        labels::LabelBox {
                            text: cb.cell,
                            left: e.min.x,
                            top: e.min.y,
                            width: e.width(),
                            height: e.height(),
                        }
                    })
                    .collect();
            let name_color = Color32::from_rgb(200, 210, 230);
            for label in labels::place_box_labels(&boxes, labels::LABEL_FONT_PX) {
                painter.text(
                    Pos2::new(label.x, label.y),
                    Align2::CENTER_CENTER,
                    label.text,
                    font.clone(),
                    name_color,
                );
            }
        }

        // The selection caption: layer text plus live dimensions at the bounds.
        let indices: Vec<usize> = self.selection.iter().collect();
        let dpm = self.dbu_per_micron();
        let caption = match inspector::inspect(self.scene.shapes(), &indices, &self.layer_state) {
            Inspection::Empty => None,
            Inspection::Single(info) => Some((
                info.bounds,
                labels::selection_caption(&info.layer_label(), &info.bounds, dpm),
            )),
            Inspection::Multiple { count, bounds } => {
                Some((bounds, labels::multi_selection_caption(count, &bounds, dpm)))
            }
        };
        if let Some((bounds, text)) = caption {
            let e = self.world_rect_to_screen(screen, bounds);
            let (x, y) = labels::caption_anchor(
                e.min.x,
                e.max.x,
                e.min.y,
                screen.top,
                labels::LABEL_FONT_PX,
            );
            painter.text(
                Pos2::new(x, y),
                Align2::CENTER_CENTER,
                text,
                font.clone(),
                Color32::from_rgb(255, 240, 120),
            );
        }

        // Live dimension readout at the cursor while the second measure point is
        // pending; the completed measurement is drawn by `draw_measure`.
        if let (Some(start), Some(cursor)) = (self.tools.measure_start(), self.cursor_world) {
            let text = labels::live_measure_caption(start, cursor, self.dbu_per_micron());
            let p = self.world_pos_to_screen(screen, cursor);
            painter.text(
                Pos2::new(p.x + 12.0, p.y - 12.0),
                Align2::LEFT_BOTTOM,
                text,
                font,
                Color32::from_rgb(255, 210, 90),
            );
        }
    }

    /// Draws the minimap overlay: document overview, placements, and the viewport.
    ///
    /// All geometry comes from [`MinimapLayout`]; this method only paints the
    /// rectangles it computes. The click/drag recentering lives at the top of
    /// [`App::process_canvas_input`] using the same layout, so what is drawn and
    /// what is clickable can never disagree.
    fn draw_minimap(&self, painter: &egui::Painter, screen: &ScreenRect) {
        let Some(bounds) = self.scene.bounds() else {
            return;
        };
        let Some(layout) = MinimapLayout::compute(screen, bounds) else {
            return;
        };
        let panel = EguiRect::from_min_size(
            Pos2::new(layout.panel.left, layout.panel.top),
            Vec2::new(layout.panel.width, layout.panel.height),
        );
        painter.rect_filled(panel, 3.0, Color32::from_rgba_unmultiplied(20, 23, 28, 230));
        painter.rect_stroke(
            panel,
            3.0,
            Stroke::new(1.0, Color32::from_rgb(70, 76, 90)),
            StrokeKind::Middle,
        );

        // Document bounds outline.
        let (bx, by, bw, bh) = layout.world_rect_to_panel(bounds);
        let doc_rect = EguiRect::from_min_size(Pos2::new(bx, by), Vec2::new(bw, bh));
        painter.rect_stroke(
            doc_rect,
            0.0,
            Stroke::new(1.0, Color32::from_rgb(90, 100, 120)),
            StrokeKind::Middle,
        );

        // Placement boxes give the overview its silhouette; cap the count so a
        // huge document cannot make the minimap the most expensive draw call.
        let fill = Color32::from_rgba_unmultiplied(90, 120, 170, 90);
        for cb in culling::visible_cell_boxes(self.history.document(), &self.top_cell, bounds)
            .into_iter()
            .take(256)
        {
            let (x, y, w, h) = layout.world_rect_to_panel(cb.bbox);
            painter.rect_filled(
                EguiRect::from_min_size(Pos2::new(x, y), Vec2::new(w, h)),
                0.0,
                fill,
            );
        }

        // The camera's visible world rectangle, clamped to the panel.
        let (vx, vy, vw, vh) = layout.world_rect_to_panel(self.camera.visible_world_rect(screen));
        painter.rect_stroke(
            EguiRect::from_min_size(Pos2::new(vx, vy), Vec2::new(vw, vh)),
            0.0,
            Stroke::new(1.5, Color32::from_rgb(255, 210, 90)),
            StrokeKind::Middle,
        );
    }

    /// Draws remote collaborators' cursors from the sync presence map (stretch:
    /// there are no live peers in this build, so this is normally empty).
    fn draw_presence(&self, painter: &egui::Painter, screen: &ScreenRect) {
        for (_, presence) in self.document.awareness().iter() {
            let (r, g, b, _) = layers::rgba_components(presence.color_rgba);
            let color = Color32::from_rgb(r, g, b);
            let p = self.world_pos_to_screen(screen, presence.cursor);
            painter.circle_filled(p, 4.0, color);
            if !presence.display_name.is_empty() {
                painter.text(
                    Pos2::new(p.x + 6.0, p.y),
                    Align2::LEFT_CENTER,
                    &presence.display_name,
                    FontId::proportional(11.0),
                    color,
                );
            }
        }
    }

    /// The `(r, g, b, a)` color for a layer, or a neutral gray if unknown.
    fn layer_color(&self, layer: LayerId) -> (u8, u8, u8, u8) {
        self.layer_state
            .rows()
            .iter()
            .find(|r| r.id == layer)
            .map_or((160, 160, 160, 190), |r| {
                let (rr, gg, bb, _) = layers::rgba_components(r.color_rgba);
                // Semi-transparent fill so overlapping layers read clearly.
                (rr, gg, bb, 170)
            })
    }

    /// Snaps a raw world point, trying geometry and guides first, then the grid.
    ///
    /// This is the single snap seam the canvas routes through. It gathers snap
    /// candidates from the visible shapes within the snap radius of `raw`, asks
    /// [`crate::snap::best_snap`] for the nearest vertex, edge, midpoint, center, or
    /// guide, and returns that point plus a [`SnapHint`] for drawing the indicator.
    /// When nothing is in range it falls back to [`crate::grid::GridSettings::snap`]
    /// (the on-grid point) and returns no hint. The returned point is what any tool
    /// should place; the hint drives only the on-canvas snap indicator.
    ///
    /// Lane 2A's drawing tools currently place at `self.grid.snap(raw)`; at
    /// integration they should place at the point this returns so drawn geometry
    /// snaps to existing geometry and guides too.
    fn snap_world(&self, raw: Point) -> (Point, Option<SnapHint>) {
        let radius_dbu = self.snap.radius_dbu(self.camera.pixels_per_dbu());
        if self.snap.geometry_enabled || self.snap.guide_enabled {
            let candidates = self.snap_candidates_near(raw, radius_dbu);
            if let Some(hint) = snap::best_snap(&self.snap, raw, radius_dbu, candidates) {
                return (hint.point, Some(hint));
            }
        }
        (self.grid.snap(raw), None)
    }

    /// Collects the snap candidates from visible shapes whose bounding box lies
    /// within `radius_dbu` of `raw`.
    ///
    /// The probe rectangle is `raw` expanded by the radius, so only geometry that
    /// could plausibly catch the cursor is walked. Shapes on hidden layers are
    /// skipped so an invisible layer never steals a snap.
    fn snap_candidates_near(&self, raw: Point, radius_dbu: i64) -> Vec<snap::SnapCandidate> {
        let r = i32::try_from(radius_dbu).unwrap_or(i32::MAX);
        let probe = Rect::new(raw.translate(-r, -r), raw.translate(r, r));
        let shapes = self.scene.shapes();
        let mut out = Vec::new();
        for idx in self.scene.query(probe) {
            let shape = &shapes[idx];
            if !self.layer_state.is_visible(shape.layer) {
                continue;
            }
            out.extend(snap::shape_candidates(shape));
        }
        out
    }

    /// Converts a world point to an egui screen position.
    fn world_pos_to_screen(&self, screen: &ScreenRect, p: Point) -> Pos2 {
        let (x, y) = self.camera.world_to_screen(screen, p);
        Pos2::new(x, y)
    }

    /// Converts a world rectangle to a (normalized) egui screen rectangle.
    fn world_rect_to_screen(&self, screen: &ScreenRect, r: Rect) -> EguiRect {
        let a = self.world_pos_to_screen(screen, r.min);
        let b = self.world_pos_to_screen(screen, r.max);
        EguiRect::from_two_pos(a, b)
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Apply the chosen theme to the egui visuals for this frame (dark by
        // default; the view/export panel toggles it). Setting it every frame keeps
        // the visuals in sync after a toggle or a restored session.
        ctx.set_visuals(match self.view_export.theme {
            crate::viewexport::Theme::Dark => egui::Visuals::dark(),
            crate::viewexport::Theme::Light => egui::Visuals::light(),
        });

        // Sample this frame's duration up front: both the Start screen and the editor
        // age notification toasts by it, so it is computed before either branch.
        let dt = ctx.input(|i| i.stable_dt).max(0.0);

        // Drive the browser open path: kick off the `?gds=` fetch and the IndexedDB
        // recent-list load on the first wasm frame, and apply whatever those async
        // tasks have posted since last frame. A no-op on native.
        self.drive_web_open(&ctx);

        // Open a file dropped onto the page (browser) or window (native) before
        // anything else this frame, so a drop works from any view including the Start
        // screen and the replay theater. The seam hardens the import, so a bad drop
        // cannot panic; an oversized or non-layout drop sets a clear status. While a
        // file is dragged over, draw a "drop to open" affordance.
        self.handle_dropped_files(&ctx);
        if Self::is_file_hovering(&ctx) {
            Self::draw_drop_affordance(&ctx);
        }
        // Surface an active or failed progressive/remote load (its progress bar and
        // any failure message) over every view.
        self.draw_load_progress(&ctx);
        // Age the toasts and drop any that have expired (errors persist).
        self.notifications.advance(dt);

        // The Start screen greets a first-time user with the worked use cases.
        // While it is showing it owns the whole frame: it draws the chooser and
        // returns, so the editor panels and canvas are not built underneath it. The
        // notification toasts are still drawn over it so a failed open (from a drop or
        // the gallery) is visible.
        if self.start_screen {
            self.start_screen_ui(ui);
            self.notifications_area(&ctx);
            ctx.request_repaint();
            return;
        }

        self.handle_shortcuts(&ctx);

        self.frame_meter
            .record(std::time::Duration::from_secs_f32(dt));

        // Advance the agent run by this frame's dt so narration and the agent
        // cursor animate while the panel is running. Each verify step the run
        // crosses hands back the violation list parsed from its `run_drc`
        // response, and installing it in the DRC results updates the panel list
        // and the canvas markers live, mid-run.
        if let Some(update) = self.agent.tick(dt) {
            self.apply_agent_run_verify(update);
        }

        // Advance replay-theater playback the same way; a playing transcript
        // updates the theater canvas and the DRC overlay as it crosses
        // verifies.
        if let Some(update) = self.replay.tick(dt) {
            self.apply_agent_drc_update(update);
        }

        // The surface color format when eframe is on its wgpu backend; drives the
        // retained GPU canvas. `None` (e.g. a glow build) falls back to egui painting.
        let gpu_format = frame.wgpu_render_state().map(|state| state.target_format);

        // Draw the docked panels and the canvas, collecting the canvas rect (for the
        // palette/export) and the tour highlight rectangles.
        let (canvas_screen, tour_targets) = self.main_panels(ui, gpu_format);
        // Cache the canvas rect so next frame's view/export panel can frame the
        // current view (the panel draws before the canvas is measured).
        self.view_export.last_canvas = canvas_screen;

        self.palette_window(&ctx, canvas_screen);
        // The floating 3D-stack window shows normally, but during a demo capture it is
        // shown only when a `view3d on` step asked for it, so non-3D tours stay clean.
        #[cfg(not(target_arch = "wasm32"))]
        let show_view3d = !self.in_demo_capture() || self.demo_show_3d;
        #[cfg(target_arch = "wasm32")]
        let show_view3d = true;
        if show_view3d {
            self.view3d.show(
                &ctx,
                frame,
                self.history.document(),
                &self.top_cell,
                &self.layer_state,
            );
        }
        if !self.in_demo_capture() {
            crate::xsection::window(
                &ctx,
                self.tools.cut_line(),
                self.scene.shapes(),
                self.history.document().technology(),
                &self.layer_state,
            );
        }
        self.keymap_window(&ctx);
        self.replay_window(&ctx);
        self.open_warnings_window(&ctx);
        // The app-wide notification toasts, over the panels and windows.
        self.notifications_area(&ctx);

        // Draw the first-run tour overlay last so its card and highlight sit over
        // everything else. Suppressed during a demo capture so onboarding chrome does
        // not cover the feature under test.
        if !self.in_demo_capture() {
            self.tour_overlay(&ctx, &tour_targets);
        }

        // Scripted-capture mode (native launcher only): advance the screenshot state
        // machine after everything has been drawn this frame, and close the window
        // once the capture is complete. The one-shot smoke and the scripted demo run
        // are independent; at most one is ever armed.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let smoke_done = self.capture.as_mut().is_some_and(|cap| cap.tick(&ctx));
            if smoke_done {
                self.capture = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            self.drive_demo(&ctx);
        }

        // Keep animating while dragging/measuring so interaction feels live.
        ctx.request_repaint();
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        // View/UI state is persisted to our own session file on native; egui's
        // storage is not used directly (no serde dependency in this crate).
        #[cfg(not(target_arch = "wasm32"))]
        {
            let hidden: Vec<LayerId> = self
                .layer_state
                .rows()
                .iter()
                .filter(|r| !r.visible)
                .map(|r| r.id)
                .collect();
            let state = crate::session::SessionState::capture(
                &self.camera,
                self.tools.active(),
                self.grid,
                self.view_export.theme,
                &hidden,
                self.tour_seen(),
            );
            let _ = crate::session::save(&state);
            // The keymap persists alongside the session so rebinds survive exit
            // even when the user never pressed Save in the editor.
            let _ = keymap::save(&self.keymap);
        }
    }
}

/// Formats a boolean as `on`/`off` for status messages.
fn on_off(v: bool) -> &'static str {
    if v { "on" } else { "off" }
}

/// Builds a render [`Palette`] that reflects the app's current layer visibility.
///
/// The retained tessellation skips invisible layers via the palette, so folding
/// `LayerState`'s per-row visibility into a synthetic [`Technology`] here is what
/// makes a layer toggle hide geometry on the GPU canvas.
fn palette_from_layers(layers: &LayerState) -> Palette {
    let tech = Technology {
        name: String::new(),
        dbu_per_micron: 1,
        layers: layers
            .rows()
            .iter()
            .map(|r| LayerInfo {
                id: r.id,
                name: r.name.clone(),
                color_rgba: r.color_rgba,
                visible: r.visible,
            })
            .collect(),
        rules: Vec::new(),
        stack: Vec::new(),
    };
    Palette::from_technology(&tech)
}

/// The egui-wgpu paint callback that renders the retained scene on eframe's device.
///
/// It carries the camera projection, the current render revision, the expanded GPU
/// geometry (shared by `Arc`), and the surface color format. The heavy GPU state
/// (pipelines, buffers) lives in egui-wgpu's `callback_resources`, created lazily on
/// the first paint and reused afterwards, so a plain camera move only rewrites the
/// view uniform.
struct SceneCallback {
    /// The world -> clip projection for this frame.
    view: ViewUniform,
    /// The revision the `expanded` geometry reflects; the renderer re-uploads only
    /// when this changes.
    revision: u64,
    /// The expanded GPU geometry (rects with transforms + baked mesh), shared in.
    expanded: Arc<ExpandedScene>,
    /// The surface color format the renderer must target.
    format: eframe::egui_wgpu::wgpu::TextureFormat,
}

impl eframe::egui_wgpu::CallbackTrait for SceneCallback {
    fn prepare(
        &self,
        device: &eframe::egui_wgpu::wgpu::Device,
        queue: &eframe::egui_wgpu::wgpu::Queue,
        _screen_descriptor: &eframe::egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut eframe::egui_wgpu::wgpu::CommandEncoder,
        resources: &mut eframe::egui_wgpu::CallbackResources,
    ) -> Vec<eframe::egui_wgpu::wgpu::CommandBuffer> {
        // Lazily create (or recreate on a format change) the GPU renderer stored in
        // egui-wgpu's per-renderer resource map.
        let needs_new = resources
            .get::<RetainedRenderer>()
            .is_none_or(|r| r.format() != self.format);
        if needs_new {
            resources.insert(RetainedRenderer::new(device, self.format));
        }
        if let Some(renderer) = resources.get_mut::<RetainedRenderer>() {
            renderer.sync_expanded(device, queue, &self.expanded, self.revision);
            renderer.set_camera(queue, &self.view);
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut eframe::egui_wgpu::wgpu::RenderPass<'static>,
        resources: &eframe::egui_wgpu::CallbackResources,
    ) {
        if let Some(renderer) = resources.get::<RetainedRenderer>() {
            // Constrain the draw to the canvas viewport so world geometry does not
            // spill over the side panels.
            let vp = info.viewport_in_pixels();
            render_pass.set_viewport(
                vp.left_px as f32,
                vp.top_px as f32,
                vp.width_px.max(1) as f32,
                vp.height_px.max(1) as f32,
                0.0,
                1.0,
            );
            renderer.paint(render_pass);
        }
    }
}

/// The chords pressed this frame, in event order, using egui's canonical key
/// names so each one can be looked up in the [`Keymap`] with a string compare.
fn pressed_chords(ctx: &egui::Context) -> Vec<keymap::Chord> {
    ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => Some(keymap::Chord {
                    ctrl: modifiers.command || modifiers.ctrl,
                    shift: modifiers.shift,
                    alt: modifiers.alt,
                    key: key.name().to_owned(),
                }),
                _ => None,
            })
            .collect()
    })
}

/// The keymap to start with: the saved file on native (defaults if absent).
#[cfg(not(target_arch = "wasm32"))]
fn load_keymap() -> Keymap {
    keymap::load().map_or_else(Keymap::default, |(map, _warnings)| map)
}

/// The keymap to start with: always the defaults on the web (no filesystem).
#[cfg(target_arch = "wasm32")]
fn load_keymap() -> Keymap {
    Keymap::default()
}

/// Whether the first-run tour has already been shown, from the persisted session.
///
/// On native this reads the `tour_seen` bit out of the saved session file, so the
/// automatic tour runs only on the very first launch. A missing or unreadable
/// session counts as not-seen, so a brand-new install still gets the tour.
#[cfg(not(target_arch = "wasm32"))]
fn tour_already_seen() -> bool {
    crate::session::load().is_some_and(|s| s.tour_seen)
}

/// Whether the first-run tour has already been shown, on the web.
///
/// There is no filesystem on `wasm32`, so the flag cannot persist between page
/// loads. Treat the tour as already seen so it does not reopen on every visit to
/// the public bundle (which lands on the replay theater anyway); the Help menu can
/// still relaunch it within a session.
#[cfg(target_arch = "wasm32")]
fn tour_already_seen() -> bool {
    true
}

/// Converts a canvas [`ScreenRect`] to an egui rectangle.
fn egui_rect_of(screen: &ScreenRect) -> EguiRect {
    EguiRect::from_min_size(
        Pos2::new(screen.left, screen.top),
        Vec2::new(screen.width, screen.height),
    )
}

/// The accent color for a notification severity in the toast area.
///
/// Blue for a neutral notice, amber for a recoverable warning, red for a hard
/// failure. Kept beside the toast drawing so the color mapping lives with the egui
/// glue rather than in the pure [`crate::notify`] model.
fn severity_color(severity: crate::notify::Severity) -> Color32 {
    match severity {
        crate::notify::Severity::Info => Color32::from_rgb(120, 170, 235),
        crate::notify::Severity::Warning => Color32::from_rgb(230, 190, 90),
        crate::notify::Severity::Error => Color32::from_rgb(235, 110, 100),
    }
}

/// The display name of the layer row at `index`, or an empty string.
fn row_name(state: &LayerState, index: usize) -> String {
    state
        .rows()
        .get(index)
        .map(|r| r.name.clone())
        .unwrap_or_default()
}

/// Writes RGBA8 pixels to a PNG file, returning the absolute path written.
///
/// Uses a minimal hand-rolled PNG encoder (single IDAT, zlib stored blocks) so the
/// crate needs no image dependency. Native only.
#[cfg(not(target_arch = "wasm32"))]
fn write_png(name: &str, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<String> {
    let path = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(name);
    let bytes = crate::app::png::encode_rgba(width, height, rgba);
    std::fs::write(&path, bytes)?;
    Ok(path.display().to_string())
}

/// A dependency-free PNG encoder for the export action.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod png {
    /// Encodes tightly-packed RGBA8 pixels (row 0 at the top) into PNG file bytes.
    ///
    /// The image data is stored uncompressed inside a single zlib stream using
    /// "stored" (type 0) deflate blocks, so no compression library is required. The
    /// result is a valid, if unoptimized, PNG.
    #[must_use]
    pub fn encode_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);

        // IHDR.
        let mut ihdr = Vec::with_capacity(13);
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.push(8); // bit depth
        ihdr.push(6); // color type RGBA
        ihdr.push(0); // compression
        ihdr.push(0); // filter
        ihdr.push(0); // interlace
        write_chunk(&mut out, *b"IHDR", &ihdr);

        // Raw image with a 0 filter byte per row.
        let stride = (width as usize) * 4;
        let mut raw = Vec::with_capacity((stride + 1) * height as usize);
        for y in 0..height as usize {
            raw.push(0);
            let start = y * stride;
            let end = start + stride;
            if end <= rgba.len() {
                raw.extend_from_slice(&rgba[start..end]);
            } else {
                raw.extend(std::iter::repeat_n(0u8, stride));
            }
        }

        let idat = zlib_store(&raw);
        write_chunk(&mut out, *b"IDAT", &idat);
        write_chunk(&mut out, *b"IEND", &[]);
        out
    }

    /// Wraps `data` in a PNG chunk (length, type, data, CRC).
    fn write_chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(&kind);
        out.extend_from_slice(data);
        let mut crc = Crc::new();
        crc.update(&kind);
        crc.update(data);
        out.extend_from_slice(&crc.finish().to_be_bytes());
    }

    /// Wraps `data` in a zlib stream using uncompressed (stored) deflate blocks.
    fn zlib_store(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(0x78); // CMF: deflate, 32K window
        out.push(0x01); // FLG: no dict, fastest
        // Stored blocks of at most 65535 bytes.
        let mut i = 0;
        while i < data.len() {
            let chunk = (data.len() - i).min(0xFFFF);
            let last = i + chunk >= data.len();
            out.push(u8::from(last)); // BFINAL, BTYPE=00
            let len = chunk as u16;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&(!len).to_le_bytes());
            out.extend_from_slice(&data[i..i + chunk]);
            i += chunk;
        }
        // Adler-32 of the uncompressed data.
        out.extend_from_slice(&adler32(data).to_be_bytes());
        out
    }

    /// Computes an Adler-32 checksum.
    fn adler32(data: &[u8]) -> u32 {
        let mut a = 1u32;
        let mut b = 0u32;
        for &byte in data {
            a = (a + u32::from(byte)) % 65521;
            b = (b + a) % 65521;
        }
        (b << 16) | a
    }

    /// A minimal CRC-32 (as used by PNG) accumulator.
    struct Crc {
        value: u32,
    }

    impl Crc {
        /// Starts a new CRC accumulator.
        fn new() -> Self {
            Self { value: 0xFFFF_FFFF }
        }

        /// Feeds bytes into the CRC.
        fn update(&mut self, data: &[u8]) {
            for &byte in data {
                let mut c = (self.value ^ u32::from(byte)) & 0xFF;
                for _ in 0..8 {
                    c = if c & 1 != 0 {
                        0xEDB8_8320 ^ (c >> 1)
                    } else {
                        c >> 1
                    };
                }
                self.value = c ^ (self.value >> 8);
            }
        }

        /// Finalizes and returns the CRC value.
        fn finish(self) -> u32 {
            self.value ^ 0xFFFF_FFFF
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn png_has_signature_and_chunks() {
            let px = [255u8, 0, 0, 255, 0, 255, 0, 255];
            let png = encode_rgba(2, 1, &px);
            assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
            // Contains IHDR, IDAT, IEND markers.
            let has = |needle: &[u8; 4]| png.windows(4).any(|w| w == needle);
            assert!(has(b"IHDR"));
            assert!(has(b"IDAT"));
            assert!(has(b"IEND"));
        }

        #[test]
        fn adler_and_crc_known_values() {
            // Adler-32 of "abc" is 0x024D0127.
            assert_eq!(adler32(b"abc"), 0x024D_0127);
            let mut crc = Crc::new();
            crc.update(b"abc");
            // CRC-32 of "abc" is 0x352441C2.
            assert_eq!(crc.finish(), 0x3524_41C2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An agent verify step's parsed violations must land in the DRC panel (and
    /// therefore the canvas markers), mid-run, exactly as a local run would.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn agent_verify_updates_the_drc_overlay_live() {
        let mut app = App::new();
        assert!(!app.drc.has_run());
        app.agent.prompt = "overlay wiring".to_owned();
        app.agent.start();
        // Drain the run one generous tick at a time, applying each verify
        // update the way the frame loop does.
        let mut updates = 0;
        for _ in 0..1_000 {
            if let Some(update) = app.agent.tick(10.0) {
                updates += 1;
                app.apply_agent_drc_update(update);
            }
            if !app.agent.is_running() {
                break;
            }
        }
        assert!(updates >= 1, "at least one verify step fired");
        // The script's final verify is clean, so the overlay ends empty but run.
        assert!(app.drc.has_run());
        assert!(app.drc.is_empty());
        assert_eq!(app.status.text, "Agent verify: DRC clean");
        // A mid-run flagged list replaces the panel content the same way.
        let (transcript, _) = crate::agent_panel::scripted_run("flagged");
        let flagged = transcript
            .records
            .iter()
            .find_map(|r| {
                if let reticle_agent_api::Outcome::Ok(reticle_agent_api::AgentResponse::Data {
                    value,
                    ..
                }) = &r.outcome
                {
                    let v = crate::agent_panel::violations_from_json(value);
                    if v.is_empty() { None } else { Some(v) }
                } else {
                    None
                }
            })
            .expect("the script's first verify flags the thin wire");
        app.apply_agent_drc_update(flagged);
        assert!(!app.drc.is_empty());
        assert!(app.status.text.contains("violation(s)"));
    }

    /// The replay theater re-executes a transcript against a live session, and
    /// its verify records drive the shared DRC overlay through the same path
    /// the agent run uses; rewinding clears the overlay again.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn replay_theater_replays_and_drives_the_overlay() {
        let mut app = App::new();
        let (transcript, _) = crate::agent_panel::scripted_run("theater glue");
        let total = transcript.records.len();
        app.replay.load_transcript(transcript);
        assert!(app.replay.is_loaded());
        // Step to just past the first verify, applying overlay updates the way
        // the transport buttons do.
        let mut first_flagged = None;
        while first_flagged.is_none() && !app.replay.at_end() {
            if let Some(update) = app.replay.step_forward() {
                first_flagged = Some(update.clone());
                app.apply_agent_drc_update(update);
            }
        }
        let flagged = first_flagged.expect("the script verifies");
        assert!(!flagged.is_empty(), "first verify flags the thin wire");
        assert_eq!(app.drc.len(), flagged.len());
        assert!(app.replay.shape_count() >= 1);
        // Restarting clears the overlay: no verify crossed at position 0.
        let update = app.replay.seek(0);
        app.apply_replay_overlay(update);
        assert!(!app.drc.has_run());
        assert_eq!(app.replay.progress(), (0, total));
    }

    /// "Ask agent to fix" a selected violation seeds the agent panel with the
    /// assembled region + rule context and starts a run on it (the Wave-3B seam).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn ask_agent_to_fix_seeds_and_starts_a_scoped_run() {
        let mut app = App::new();
        // Install a violation the way an agent verify or a local DRC run would.
        let v = reticle_model::Violation {
            rule: "min_width_met1".to_owned(),
            kind: reticle_model::RuleKind::Width,
            layer: reticle_geometry::LayerId::new(4, 0),
            other_layer: None,
            measured: 60,
            required: 100,
            location: reticle_geometry::Rect::new(
                reticle_geometry::Point::new(23_000, 0),
                reticle_geometry::Point::new(23_060, 2_000),
            ),
            message: "feature 60 < min width 100".to_owned(),
        };
        app.drc.set_violations(vec![v.clone()]);
        app.drc.select(0).expect("index 0 exists");

        let context = crate::drc_panel::fix_violation_prompt(&v);
        app.ask_agent_to_fix(context.clone());
        // The prompt is the scoped context and a run is now active.
        assert_eq!(app.agent.prompt, context);
        assert!(app.agent.is_running());
        assert!(app.agent.prompt.contains("(23000, 0)-(23060, 2000)"));
        assert!(app.agent.prompt.contains("min_width_met1"));
        assert_eq!(app.status.text, "Agent: scoped fix run started");
    }

    /// Loading a history entry through the store drives the replay theater: on
    /// native a real transcript path loads and opens the theater.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn load_history_entry_loads_the_theater_from_a_path() {
        use std::io::Write as _;
        // Write a real scripted transcript to a temp JSONL file.
        let (transcript, _) = crate::agent_panel::scripted_run("history load");
        let count = transcript.records.len();
        let mut path = std::env::temp_dir();
        path.push(format!(
            "reticle-hist-load-{}.transcript.jsonl",
            std::process::id()
        ));
        {
            let mut f = std::fs::File::create(&path).expect("create transcript");
            for record in &transcript.records {
                writeln!(f, "{}", serde_json::to_string(record).expect("serialize"))
                    .expect("write record");
            }
            writeln!(f, "{{\"final_hash\":{}}}", transcript.final_hash).expect("write trailer");
        }

        let mut app = App::new();
        assert!(!app.replay_open);
        app.load_history_entry(path.to_str().expect("utf-8 path"));
        assert!(app.replay_open, "loading opens the theater");
        assert_eq!(app.replay.progress(), (0, count));
        assert!(app.agent_history.error.is_empty());
        assert!(app.status.text.contains("loaded"));

        let _ = std::fs::remove_file(&path);
    }

    /// A follow-up submitted through the panel while a run is active lands in the
    /// conversation and on the follow-up seam.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn conversation_followup_records_through_the_panel() {
        let mut app = App::new();
        app.agent.prompt = "route a net".to_owned();
        app.agent.start();
        app.agent.followup = "avoid the keepout".to_owned();
        let sent = app.agent.submit_followup().expect("running");
        assert_eq!(sent, "avoid the keepout");
        assert_eq!(app.agent.followups(), ["avoid the keepout"]);
        assert!(
            app.agent
                .conversation()
                .iter()
                .any(|e| e.text == "avoid the keepout")
        );
    }

    /// The Share section's defaults compose a joinable relay link for the
    /// demo document's room out of the box.
    #[test]
    fn share_defaults_compose_the_demo_room_link() {
        let app = App::new();
        let link = crate::share::room_link(&app.share_server, &app.share_room);
        assert_eq!(link, "ws://127.0.0.1:3030/ws/chip_top");
        // A user-typed https relay and a messy room name still compose.
        let link = crate::share::room_link("https://relay.example/", "My Layout!");
        assert_eq!(link, "wss://relay.example/ws/my-layout");
    }

    #[test]
    fn app_new_loads_demo_scene() {
        let app = App::new();
        assert!(!app.scene.is_empty());
        assert_eq!(app.top_cell, demo::TOP_CELL);
        assert!(app.history.document().cell(demo::TOP_CELL).is_some());
    }

    #[test]
    fn start_view_query_parsing() {
        use super::StartView;
        assert_eq!(
            StartView::from_query_value("replay"),
            StartView::ReplayTheater
        );
        assert_eq!(
            StartView::from_query_value("Theater"),
            StartView::ReplayTheater
        );
        assert_eq!(StartView::from_query_value("editor"), StartView::Editor);
        // Anything unrecognized (or empty) falls back to the editor.
        assert_eq!(StartView::from_query_value(""), StartView::Editor);
        assert_eq!(StartView::from_query_value("nonsense"), StartView::Editor);
        // The desktop default is the editor.
        assert_eq!(StartView::default(), StartView::Editor);
    }

    #[test]
    fn new_opens_into_the_editor() {
        let app = App::new();
        assert_eq!(app.start_view(), super::StartView::Editor);
        assert!(
            !app.replay_open,
            "the editor default does not open the theater"
        );
    }

    #[test]
    fn new_greets_with_the_start_screen() {
        // The editor default shows the worked-use-case chooser at startup.
        let app = App::new();
        assert!(
            app.start_screen(),
            "the editor default greets with the chooser"
        );
        // The replay-theater start view drops the visitor straight into the theater.
        let web = App::with_start_view(super::StartView::ReplayTheater);
        assert!(
            !web.start_screen(),
            "the replay start view skips the chooser"
        );
    }

    #[test]
    fn entering_the_inspect_scenario_loads_the_sky130_cell() {
        let mut app = App::new();
        app.enter_use_case(super::UseCase::InspectCell);
        // The chooser is dismissed and the live document is now the inverter, not
        // the demo.
        assert!(!app.start_screen());
        assert_eq!(app.top_cell, "sky130_fd_sc_hd__inv_1");
        assert!(
            app.history
                .document()
                .cell("sky130_fd_sc_hd__inv_1")
                .is_some(),
            "the inverter cell is loaded"
        );
        // Derived state was rebuilt against the new document.
        assert!(
            !app.scene.is_empty(),
            "scene index rebuilt for the new cell"
        );
        assert!(
            !app.history.can_undo(),
            "a loaded scenario is a fresh session"
        );
    }

    #[test]
    fn entering_the_violation_scenario_loads_a_checkable_doc() {
        let mut app = App::new();
        app.enter_use_case(super::UseCase::FindAndFixViolation);
        assert!(!app.start_screen());
        // Running DRC on the loaded document reports the seeded violation.
        let n = app.drc.run(app.history.document(), &app.top_cell);
        assert!(n >= 1, "the seeded violation is reported after loading");
    }

    #[test]
    fn entering_the_agent_scenario_opens_the_theater() {
        let mut app = App::new();
        assert!(!app.replay_open);
        app.enter_use_case(super::UseCase::WatchTheAgent);
        assert!(!app.start_screen());
        assert!(app.replay_open, "the agent scenario opens the theater");
        let (_, total) = app.replay.progress();
        assert!(total > 0, "the bundled transcript is loaded and playing");
    }

    #[test]
    fn entering_the_build_scenario_loads_the_starter_doc() {
        let mut app = App::new();
        app.enter_use_case(super::UseCase::BuildWithTools);
        assert!(!app.start_screen());
        assert_eq!(app.top_cell, "SANDBOX");
        assert!(app.history.document().cell("SANDBOX").is_some());
    }

    #[test]
    fn an_import_error_is_surfaced_as_a_queued_error_notification() {
        // The error-sink contract: a failed open reports through the notification
        // surface instead of failing silently. Feed bytes that are not GDSII and
        // assert an Error notification naming the source is queued.
        let mut app = App::new();
        assert!(app.notifications().is_empty());
        let opened = app.open_bytes_reporting(
            b"definitely not a gds",
            crate::open::DocFormat::Gds,
            "junk.gds",
        );
        assert!(!opened, "bad bytes do not open");
        assert_eq!(app.notifications().len(), 1);
        let note = app.notifications().iter().next().expect("a notification");
        assert_eq!(note.severity, crate::notify::Severity::Error);
        assert!(
            note.summary.contains("junk.gds"),
            "the error names what failed to open"
        );
        assert!(!note.detail.is_empty(), "the error carries a reason");
        // The editor was left untouched (still the demo top cell).
        assert_eq!(app.top_cell, crate::demo::TOP_CELL);
    }

    #[test]
    fn loading_an_example_chip_installs_it_and_leaves_the_start_screen() {
        // The gallery load path: bytes to an opened, installed document via the seam,
        // dismissing the Start screen, with a confirming notice and no error.
        let mut app = App::new();
        assert!(app.start_screen());
        let opened = app.open_example_chip(crate::startscreen::ExampleChip::TinyTapeoutMin);
        assert!(opened, "the bundled sample opens");
        assert!(!app.start_screen(), "loading dismisses the Start screen");
        assert_eq!(app.top_cell, "TT_MIN_TOP");
        assert!(
            app.history.document().cell("TT_MIN_TOP").is_some(),
            "the sample's top cell is installed"
        );
        // A clean import posts an info notice, never an error.
        assert!(
            app.notifications()
                .iter()
                .all(|n| n.severity != crate::notify::Severity::Error),
            "a clean example load queues no error"
        );
    }

    #[test]
    fn dropping_a_corpus_gds_opens_and_renders_it() {
        // The Wave 1 merge-gate drop path, end to end at the app level: a real corpus
        // file arrives as an egui dropped file (the shape eframe hands the app from a
        // browser or a native drop), and `handle_dropped_files` classifies it, opens it
        // through the hardened seam, installs it, dismisses the Start screen, and records
        // it in the recent list, with no error surfaced. The browser DOM-to-egui event
        // translation is eframe's own, exercised by the boot e2e; this proves the app's
        // half deterministically and headlessly.
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("repo root is two levels above the crate")
            .join("corpus")
            .join("tinytapeout")
            .join("real_tinytapeout_min.gds");
        let bytes = std::fs::read(&path).expect("the committed corpus sample is present");

        let mut app = App::new();
        assert!(app.start_screen(), "starts on the Start screen");
        assert!(app.recent_files().is_empty(), "no recent files yet");

        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        raw.dropped_files.push(egui::DroppedFile {
            name: "real_tinytapeout_min.gds".to_owned(),
            bytes: Some(bytes.into()),
            ..Default::default()
        });
        ctx.begin_pass(raw);
        app.handle_dropped_files(&ctx);
        let _ = ctx.end_pass();

        assert!(
            !app.start_screen(),
            "a successful drop dismisses the Start screen"
        );
        assert_eq!(
            app.top_cell, "TT_MIN_TOP",
            "the dropped design's top cell is framed"
        );
        assert!(
            app.history.document().cell("TT_MIN_TOP").is_some(),
            "the dropped design is installed"
        );
        assert_eq!(
            app.recent_files().len(),
            1,
            "the drop is recorded in the recent list"
        );
        assert_eq!(app.recent_files()[0].name, "real_tinytapeout_min.gds");
        assert!(
            app.notifications()
                .iter()
                .all(|n| n.severity != crate::notify::Severity::Error),
            "a clean drop surfaces no error"
        );
    }

    #[test]
    fn a_clean_open_notifies_info_and_a_warning_open_queues_a_warning() {
        // A clean open posts an info notice and no warnings window; an open that
        // yields import warnings routes each warning through the same surface.
        use reticle_model::{Cell, Document, DrawShape, Exporter, ShapeKind};
        let mut clean = Document::new();
        let mut cell = Cell::new("CLEAN");
        cell.shapes.push(DrawShape::new(
            reticle_geometry::LayerId::new(3, 0),
            ShapeKind::Rect(reticle_geometry::Rect::new(
                reticle_geometry::Point::new(0, 0),
                reticle_geometry::Point::new(100, 100),
            )),
        ));
        clean.insert_cell(cell);
        clean.set_top_cells(vec!["CLEAN".to_owned()]);
        let bytes = reticle_io::Gds.export(&clean).expect("export");

        let mut app = App::new();
        assert!(app.open_bytes_reporting(&bytes, crate::open::DocFormat::Gds, "clean.gds"));
        assert!(
            app.open_warnings().is_empty(),
            "a clean file has no warnings"
        );
        assert_eq!(app.notifications().len(), 1);
        assert_eq!(
            app.notifications().iter().next().unwrap().severity,
            crate::notify::Severity::Info
        );
    }

    #[test]
    fn report_error_and_notify_are_the_public_error_surface() {
        // The methods sibling lanes route through: report_error queues an error that
        // persists, notify queues an auto-expiring info notice.
        let mut app = App::new();
        app.report_error("share failed", "the room name was empty");
        app.notify("copied", "");
        assert_eq!(app.notifications().len(), 2);
        assert_eq!(
            app.notifications().max_severity(),
            Some(crate::notify::Severity::Error)
        );
    }

    #[test]
    fn recent_files_accessor_round_trips_what_a_backend_feeds() {
        // The shape Lane 1B feeds: set a recent list, read it back for the Start
        // screen to render.
        let mut app = App::new();
        assert!(
            app.recent_files().is_empty(),
            "empty until a backend feeds it"
        );
        let mut recents = crate::webopen::RecentFiles::new();
        recents.record(crate::webopen::RecentFile::local("adder.gds", 1_024));
        recents.record(crate::webopen::RecentFile::local("ring.oas", 2_048));
        app.set_recent_files(recents);
        assert_eq!(app.recent_files().len(), 2);
        // `record` moves each new entry to the front, so the most-recently recorded
        // ("ring.oas") is first and the earlier one ("adder.gds") follows.
        assert_eq!(app.recent_files()[0].name, "ring.oas");
        assert_eq!(app.recent_files()[1].name, "adder.gds");
        assert_eq!(app.recent_files()[1].size, 1_024);
    }

    #[test]
    fn with_replay_theater_opens_the_theater_with_a_loaded_run() {
        let app = App::with_start_view(super::StartView::ReplayTheater);
        assert_eq!(app.start_view(), super::StartView::ReplayTheater);
        // The theater window opens on construction and the built-in scripted run is
        // loaded, so a visitor lands on a playable replay.
        assert!(app.replay_open, "the replay start view opens the theater");
        let (_, total) = app.replay.progress();
        assert!(total > 0, "the built-in scripted transcript is loaded");
    }

    #[test]
    fn toggling_layer_via_command_hides_it() {
        let mut app = App::new();
        let id = app.layer_state.rows()[0].id;
        assert!(app.layer_state.is_visible(id));
        app.run_command(Command::ToggleLayer(0), None);
        assert!(!app.layer_state.is_visible(id));
    }

    #[test]
    fn hidden_layer_is_excluded_from_draw_list() {
        // Build the set of visible shape indices for a viewport the way `draw_shapes`
        // does, and confirm hiding a layer removes its shapes.
        let mut app = App::new();
        let bounds = app.scene.bounds().unwrap();
        let visible_layer = app.scene.shapes()[app.scene.query(bounds)[0]].layer;

        let count_visible = |app: &App| -> usize {
            app.scene
                .query(bounds)
                .into_iter()
                .filter(|&i| app.layer_state.is_visible(app.scene.shapes()[i].layer))
                .count()
        };
        let before = count_visible(&app);
        app.layer_state.set_visible(visible_layer, false);
        let after = count_visible(&app);
        assert!(after < before, "hiding a layer should shrink the draw list");
    }

    #[test]
    fn undo_redo_command_restores_scene() {
        let mut app = App::new();
        let before = app.scene.len();
        app.add_demo_rectangle();
        assert_eq!(app.scene.len(), before + 1);
        app.run_command(Command::Undo, None);
        assert_eq!(app.scene.len(), before);
        app.run_command(Command::Redo, None);
        assert_eq!(app.scene.len(), before + 1);
    }

    #[test]
    fn set_tool_command_switches_tool() {
        let mut app = App::new();
        app.run_command(Command::SetTool(Tool::Measure), None);
        assert_eq!(app.tools.active(), Tool::Measure);
    }

    #[test]
    fn select_layer_command_populates_selection() {
        let mut app = App::new();
        // Pick a layer index that actually has shapes.
        let target_layer = app.scene.shapes()[0].layer;
        let idx = app
            .layer_state
            .rows()
            .iter()
            .position(|r| r.id == target_layer)
            .unwrap();
        app.run_command(Command::SelectLayer(idx), None);
        assert!(!app.selection.is_empty());
        for i in app.selection.iter() {
            assert_eq!(app.scene.shapes()[i].layer, target_layer);
        }
    }

    #[test]
    fn toggle_grid_and_snap_commands() {
        let mut app = App::new();
        let g0 = app.grid.visible;
        app.run_command(Command::ToggleGrid, None);
        assert_ne!(app.grid.visible, g0);
        let s0 = app.grid.snap_enabled;
        app.run_command(Command::ToggleSnap, None);
        assert_ne!(app.grid.snap_enabled, s0);
    }

    #[test]
    fn clear_selection_command_empties_it() {
        let mut app = App::new();
        app.selection.set([0, 1, 2]);
        app.run_command(Command::ClearSelection, None);
        assert!(app.selection.is_empty());
    }

    #[test]
    fn run_drc_populates_violations_from_demo() {
        let mut app = App::new();
        assert!(!app.drc.has_run());
        app.run_drc();
        assert!(app.drc.has_run());
        // The demo has thin poly gates (200 DBU) under a 100-DBU default rule plus
        // other geometry; the run either flags something or cleanly finds nothing,
        // and either way marks itself as having run.
        for v in app.drc.violations() {
            assert!(!v.rule.is_empty());
        }
    }

    #[test]
    fn highlight_net_of_marks_connected_shapes() {
        let mut app = App::new();
        // Pick any real shape and highlight its net; the clicked shape must be part
        // of the highlighted set the overlay draws.
        let idx = app.scene.query(app.scene.bounds().unwrap())[0];
        app.highlight_net_of(idx);
        assert!(!app.netlight.is_empty());
        assert!(app.netlight.contains(idx));
    }

    #[test]
    fn editing_clears_net_highlight_and_bumps_generation() {
        let mut app = App::new();
        let idx = app.scene.query(app.scene.bounds().unwrap())[0];
        app.highlight_net_of(idx);
        assert!(!app.netlight.is_empty());
        let gen_before = app.doc_generation;
        app.add_demo_rectangle();
        assert!(app.netlight.is_empty(), "edit must clear the highlight");
        assert_ne!(app.doc_generation, gen_before, "generation must advance");
    }

    #[test]
    fn label_overlay_defaults_on() {
        let app = App::new();
        assert!(app.labels_visible, "labels should be on out of the box");
    }

    #[test]
    fn minimap_defaults_on_and_maps_the_demo_bounds() {
        let app = App::new();
        assert!(app.minimap_visible, "minimap should be on out of the box");
        // The demo scene must produce a usable layout on a typical canvas, and a
        // click at the mapped center must recenter to (nearly) that world point.
        let screen = ScreenRect::new(0.0, 0.0, 800.0, 600.0);
        let bounds = app.scene.bounds().expect("demo has bounds");
        let layout = MinimapLayout::compute(&screen, bounds).expect("layout fits");
        let world_center = Point::new(
            i32::midpoint(bounds.min.x, bounds.max.x),
            i32::midpoint(bounds.min.y, bounds.max.y),
        );
        let (px, py) = layout.world_to_panel(world_center);
        assert!(layout.contains(px, py));
        let back = layout.panel_to_world(px, py);
        assert!((i64::from(back.x) - i64::from(world_center.x)).abs() < 100);
        assert!((i64::from(back.y) - i64::from(world_center.y)).abs() < 100);
    }

    #[test]
    fn run_action_routes_keymap_actions_to_their_effects() {
        let mut app = App::new();
        app.run_action(keymap::Action::ToolMeasure);
        assert_eq!(app.tools.active(), Tool::Measure);
        let labels_before = app.labels_visible;
        app.run_action(keymap::Action::ToggleLabels);
        assert_ne!(app.labels_visible, labels_before);
        let minimap_before = app.minimap_visible;
        app.run_action(keymap::Action::ToggleMinimap);
        assert_ne!(app.minimap_visible, minimap_before);
        app.run_action(keymap::Action::SplitHorizontal);
        assert_eq!(app.viewports.pane_count(), 2);
        app.run_action(keymap::Action::SplitSingle);
        assert_eq!(app.viewports.pane_count(), 1);
        app.run_action(keymap::Action::OpenPalette);
        assert!(app.palette_open);
    }

    #[test]
    fn rebinding_through_the_map_redirects_the_action() {
        let mut app = App::new();
        // Force a known map so the test does not depend on any user keymap file.
        app.keymap = Keymap::defaults();
        let chord = keymap::Chord::parse("Ctrl+Shift+Q").expect("valid chord");
        assert_eq!(app.keymap.action_for(&chord), None);
        let stolen = app.keymap.bind(keymap::Action::Redo, Some(chord.clone()));
        assert!(stolen.is_empty());
        assert_eq!(app.keymap.action_for(&chord), Some(keymap::Action::Redo));
        // The old default no longer fires.
        let old = keymap::Chord::parse("Ctrl+Y").expect("valid chord");
        assert_eq!(app.keymap.action_for(&old), None);
    }

    #[test]
    fn split_view_shares_the_document_across_pane_cameras() {
        let mut app = App::new();
        assert_eq!(app.viewports.pane_count(), 1);
        app.viewports.set_split(Split::Horizontal, &app.camera);
        assert_eq!(app.viewports.pane_count(), 2);
        // The new pane starts on the live view.
        assert_eq!(app.viewports.camera(1), Some(&app.camera));
        // Focus pane 1, move its view, and confirm pane 0's camera was banked.
        let pane0_before = app.camera;
        app.viewports.focus(1, &mut app.camera);
        app.camera = ViewCamera::new(Point::new(7777, -3333), 0.5);
        assert_eq!(app.viewports.camera(0), Some(&pane0_before));
        // Both panes look at the same document: there is exactly one scene.
        assert!(!app.scene.is_empty());
        // Collapsing keeps the view the user is on.
        app.viewports.set_split(Split::Single, &app.camera);
        assert_eq!(app.viewports.focused(), 0);
        assert_eq!(app.camera, ViewCamera::new(Point::new(7777, -3333), 0.5));
    }

    #[test]
    fn selecting_violation_arms_deferred_zoom() {
        let mut app = App::new();
        app.run_drc();
        if app.drc.is_empty() {
            return; // Nothing to zoom to on this build.
        }
        assert!(!app.zoom_to_selected_violation);
        assert!(app.drc.select(0).is_some());
        app.zoom_to_selected_violation = true;
        assert_eq!(app.drc.selected(), Some(0));
    }

    /// Selects the first `n` direct (non-instanced) shapes of the top cell, which are
    /// the first `n` entries of the flattened scene.
    fn select_first_direct(app: &mut App, n: usize) {
        let direct = app.top_cell_direct_shape_count();
        app.selection.set(0..n.min(direct));
    }

    #[test]
    fn copy_then_paste_adds_shapes_and_is_undoable() {
        let mut app = App::new();
        // Copy the first direct top-cell shape.
        select_first_direct(&mut app, 1);
        app.productivity_copy();
        assert_eq!(app.productivity.clipboard.len(), 1);

        let before = app.scene.len();
        app.productivity_paste();
        assert_eq!(app.scene.len(), before + 1, "paste adds one shape");

        // The paste landed on the undo stack.
        assert!(app.history.can_undo());
        app.run_command(Command::Undo, None);
        assert_eq!(app.scene.len(), before, "undo removes the pasted shape");
    }

    #[test]
    fn duplicate_offsets_selection_and_is_undoable() {
        let mut app = App::new();
        select_first_direct(&mut app, 2);
        let selected = app.selection.len();
        assert!(selected >= 1);
        let before = app.scene.len();
        app.productivity_duplicate();
        assert_eq!(app.scene.len(), before + selected);
        app.run_command(Command::Undo, None);
        // Undo peels the duplicates back off one at a time; one undo removes one.
        assert_eq!(app.scene.len(), before + selected - 1);
    }

    #[test]
    fn build_array_commits_every_element_undoably() {
        let mut app = App::new();
        select_first_direct(&mut app, 1);
        app.productivity.array_rows = 2;
        app.productivity.array_cols = 3;
        app.productivity.array_row_pitch = 1000;
        app.productivity.array_col_pitch = 1000;
        let before = app.scene.len();
        app.productivity_array();
        // One source shape into a 2x3 grid adds six shapes.
        assert_eq!(app.scene.len(), before + 6);
        // Each element is its own undo entry.
        assert!(app.history.undo_depth() >= 6);
    }

    #[test]
    fn array_over_the_cap_is_refused() {
        let mut app = App::new();
        select_first_direct(&mut app, 1);
        app.productivity.array_rows = 500;
        app.productivity.array_cols = 500; // 250_000 > MAX_ARRAY_ELEMENTS
        let before = app.scene.len();
        app.productivity_array();
        assert_eq!(app.scene.len(), before, "an over-cap array commits nothing");
    }

    #[test]
    fn move_delta_shifts_a_direct_shape_and_is_undoable() {
        // Start from a document whose top cell owns a direct rect, so the assertion
        // on the shifted geometry is exercised regardless of the demo's shape mix.
        let mut app = App::new();
        let rect = Rect::new(Point::new(0, 0), Point::new(300, 300));
        app.history
            .apply(reticle_model::Edit::AddShape {
                cell: app.top_cell.clone(),
                shape: DrawShape::new(LayerId::new(4, 0), ShapeKind::Rect(rect)),
            })
            .unwrap();
        app.rebuild_scene();

        // The rect we just added is the last direct shape of the top cell.
        let direct = app.top_cell_direct_shape_count();
        let idx = direct - 1;
        assert!(matches!(app.scene.shapes()[idx].kind, ShapeKind::Rect(_)));

        let before_len = app.scene.len();
        app.selection.set([idx]);
        app.productivity.move_dx = 1234;
        app.productivity.move_dy = -567;
        app.productivity_move_delta();
        // A move is remove + add, so the total count is unchanged.
        assert_eq!(app.scene.len(), before_len);
        // The moved copy exists at the shifted position somewhere in the scene.
        let want = Rect::new(Point::new(1234, -567), Point::new(1534, -267));
        let found = app
            .scene
            .shapes()
            .iter()
            .any(|s| matches!(&s.kind, ShapeKind::Rect(r) if *r == want));
        assert!(found, "the moved shape appears at the new position");
        assert!(app.history.can_undo());
    }

    #[test]
    fn via_stack_places_three_shapes_undoably() {
        let mut app = App::new();
        app.productivity.via_lower = LayerId::new(4, 0);
        app.productivity.via_upper = LayerId::new(5, 0);
        app.productivity.via_cut = LayerId::new(7, 0);
        app.productivity.via_cut_size = 200;
        app.productivity.via_center_x = 5000;
        app.productivity.via_center_y = 5000;
        let before = app.scene.len();
        app.productivity_via_stack();
        assert_eq!(app.scene.len(), before + 3, "cut plus two enclosures");
        assert!(app.history.undo_depth() >= 3);
    }

    #[test]
    fn array_preview_is_empty_without_a_selection() {
        let mut app = App::new();
        app.selection.clear();
        assert!(app.array_preview_shapes().is_empty());
        // With a selection and preview on, it yields the non-origin elements.
        select_first_direct(&mut app, 1);
        app.productivity.array_rows = 2;
        app.productivity.array_cols = 2;
        app.productivity.array_preview = true;
        assert_eq!(
            app.array_preview_shapes().len(),
            3,
            "4 elements minus origin"
        );
        app.productivity.array_preview = false;
        assert!(app.array_preview_shapes().is_empty());
    }

    /// The Generate section renders headlessly without panicking: the schema-driven
    /// form (drag values, checkbox, combo box) and the live-preview readout all build
    /// inside a real egui pass. This exercises the UI path the unit tests in
    /// `generate_panel` cannot (they cover the pure logic only).
    #[test]
    fn generate_section_renders_without_panic() {
        let mut app = App::new();
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        egui::Window::new("generate test").show(&ctx, |ui| {
            app.generate_section(ui);
        });
        let _ = ctx.end_pass();
        // The default selection generates a non-empty preview.
        assert!(!app.generate_preview_shapes().is_empty());
    }

    /// Generating through the app places the whole structure as one undo step: the
    /// scene grows by the generated shape count, and a single Undo removes all of it.
    #[test]
    fn generate_apply_places_structure_as_one_undo_step() {
        let mut app = App::new();
        // Select the via farm and set a known 3x3 mcon array (9 cuts + 2 plates).
        let farm = app
            .generate
            .infos()
            .iter()
            .position(|i| i.id == "via_farm")
            .expect("via_farm registered");
        app.generate.select(farm);
        app.generate.selected_params_mut()["rows"] = serde_json::Value::from(3);
        app.generate.selected_params_mut()["cols"] = serde_json::Value::from(3);

        let before = app.scene.len();
        app.generate_apply();
        assert_eq!(
            app.scene.len(),
            before + 11,
            "the generated structure (9 cuts + 2 plates) is placed"
        );
        // One Undo removes the entire generated structure (apply_group is one step).
        app.run_command(Command::Undo, None);
        assert_eq!(
            app.scene.len(),
            before,
            "a single undo removes the whole generated structure"
        );
    }

    #[test]
    fn cut_removes_direct_shapes_and_fills_clipboard() {
        let mut app = App::new();
        select_first_direct(&mut app, 1);
        assert_eq!(app.selection.len(), 1);
        let before = app.scene.len();
        app.productivity_cut();
        assert_eq!(
            app.productivity.clipboard.len(),
            1,
            "cut fills the clipboard"
        );
        assert_eq!(app.scene.len(), before - 1, "cut removes the direct shape");
        app.run_command(Command::Undo, None);
        assert_eq!(app.scene.len(), before, "undo restores the cut shape");
    }
}
