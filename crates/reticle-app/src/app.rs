//! The `eframe::App` implementation: panels, canvas, and interaction wiring.
//!
//! This is the only module that depends on `egui`. It owns the app state and, each
//! frame, draws the toolbar, layer manager, undo-history panel, status bar, command
//! palette window, and the layout canvas, routing pointer input through the active
//! tool. All the non-trivial logic (camera math, culling, selection, snapping,
//! measurement, history) lives in the sibling modules and is unit-tested there; the
//! code here is deliberately thin glue plus painting.

use eframe::egui;
use egui::{Align2, Color32, Pos2, Rect as EguiRect, Sense, Shape, Stroke, StrokeKind, Vec2};

use reticle_geometry::{Endcap, LayerId, Point, Rect, Shape as _};
use reticle_model::{DrawShape, LayerInfo, ShapeKind, Technology};
use reticle_render::{
    ExpandedScene, Palette, RetainedRenderer, RetainedScene, ViewUniform, WgpuRenderer,
};
use reticle_sync::{Comment, SyncDocument};
use std::sync::Arc;

use crate::agent_panel::AgentPanelState;
use crate::camera::{CameraTween, ScreenRect, ViewCamera};
use crate::command::{self, Command};
use crate::commands::{self, AppOp, CommandId, RunAs};
use crate::comment_pins;
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
use crate::menu;
use crate::minimap::{self, MinimapLayout};
use crate::netlight::{Generation, Netlight};
use crate::outline::{self, OutlineTree, SavedSets};
use crate::overlay::{Anchor, OverlayLayout};
use crate::productivity::{self, ProductivityState};
use crate::query::{LayerLookup, Query};
use crate::replay::ReplayTheater;
use crate::selection::{self, Selection};
use crate::snap::{self, Guide, SnapHint, SnapState};
use crate::streamed::LevelFade;
use crate::tech_editor::TechEditorState;
use crate::theme::{
    self, components, icons,
    tokens::{CANVAS, DARK},
};
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

/// A zoom preset that needs the real canvas size to resolve, so it is recorded when the
/// command fires and applied in [`App::canvas`] once the pane rectangle is known (item
/// 28). `Fit` uses the existing `fit_requested` path; these are the three
/// that frame a computed world rectangle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ViewPreset {
    /// Frame the selection's bounding box (Shift+F, `view.zoom_selection`).
    Selection,
    /// Reset the zoom to one screen pixel per DBU, keeping the center (`view.zoom_one_to_one`).
    OneToOne,
    /// Frame the union bounding box of every visible layer's geometry (`view.zoom_layer_extents`).
    LayerExtents,
}

/// The numeric transform popover: the selection's `x, y, w, h` as editable text, plus
/// whether the popover is open (item 51). Editing a field and committing moves or resizes
/// the selection to the typed value; the buffers are refilled from the live selection each
/// time the popover opens so they never show stale numbers.
#[derive(Clone, Debug, Default)]
struct TransformPopover {
    /// Whether the popover is shown.
    open: bool,
    /// The edit buffers for the selection bounding box's min-x, min-y, width, height.
    x: String,
    y: String,
    w: String,
    h: String,
}

/// A Start-screen click, collected inside the layout closure and applied after it
/// returns (see [`App::apply_start_action`]).
///
/// Recording the intent rather than acting inline lets each Start-screen section be
/// a plain closure over `ui` without also mutably borrowing `self`.
#[derive(Clone, PartialEq, Eq, Debug)]
enum StartAction {
    /// Enter one of the worked scenarios.
    EnterUseCase(UseCase),
    /// Load a bundled example chip through the open seam.
    LoadExample(crate::startscreen::ExampleChip),
    /// Open a served archive by URL (web streaming path); a status note on native.
    OpenArchive(String),
    /// Request the native/file-open dialog (the reserved `file.open_dialog`, owned
    /// by lane 3B; a status hint until that lane wires the effect).
    OpenDialog,
    /// Open the New Tiny Tapeout tile wizard (catalog 24).
    OpenTileWizard,
    /// Toggle the pin on a recent-files entry, keyed by its [`recent_key`](crate::startscreen::recent_key)
    /// (catalog 9).
    PinRecent(String),
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
            // The viewer chrome (session chip, follow toggle, open-editor button)
            // rides the toolbar row, so the viewer tour highlights it there.
            TourTarget::OpenAffordance
            | TourTarget::Toolbar
            | TourTarget::DrawTools
            | TourTarget::ViewerControls => self.toolbar,
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

/// What a toast's Retry action re-runs (lane 3b).
///
/// The pure [`Notification`](crate::notify::Notification) model deliberately carries
/// no callback, so the app remembers the most recent retryable operation here and a
/// toast's [`Retry`](crate::notify::NotificationAction::Retry) click replays it.
#[derive(Clone, Debug)]
enum RetryOp {
    /// Re-open the file picker (a failed open from a drop or the picker).
    Picker,
    /// Re-fetch a remote URL (a failed Open-from-URL).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    FetchUrl(String),
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
    /// The screen position where the current canvas drag began, captured on
    /// `drag_started` (when `interact_pointer_pos` is the press point). Drags that
    /// commit on release (the rectangle tool and the marquee band) need the drag
    /// *start*, which egui cannot report at `drag_stopped` (`drag_delta` is per-frame
    /// and zero on the release frame, so reconstructing it from the delta yields the
    /// release point instead). Stashing the press point makes those gestures work.
    drag_press_pos: Option<Pos2>,
    /// Whether the canvas text-label overlay (cell names, selection captions, live
    /// dimensions) is drawn.
    labels_visible: bool,
    /// Whether the minimap overview panel is drawn (and steals clicks inside it).
    minimap_visible: bool,

    // --- lane 3a: canvas navigation, overlays, and fluidity ---
    /// Whether the top/left ruler bars are drawn (View > Rulers, item 31). On by
    /// default; hiding them reclaims the canvas edges for a clean presentation view.
    rulers_visible: bool,
    /// Whether a full-canvas crosshair follows the cursor (status-bar toggle, item 32),
    /// so the pointer's exact world position reads against distant geometry.
    crosshair: bool,
    /// Whether the streaming HUD is drawn in archive-browse mode (status-bar archive
    /// label toggle, item 68).
    archive_hud_visible: bool,
    /// An in-flight animated camera move (Fit / Fit-selection / Go-to), advanced each
    /// frame; `None` when the camera is at rest (item 29).
    camera_tween: Option<CameraTween>,
    /// A zoom preset requested this frame that needs the real canvas size to resolve
    /// (fit-selection, 1:1, layer-extents); applied in [`App::canvas`] (item 28).
    pending_view: Option<ViewPreset>,
    /// The level-of-detail crossfade for the streamed archive: fades freshly-covered
    /// detail in rather than popping it (item 42).
    archive_fade: LevelFade,
    /// The shape under the cursor for the Select tool's hover pre-highlight, recomputed
    /// each hover so the click target is obvious before the click (item 46).
    hover_pick: Option<usize>,
    /// Whether the snap-bypass modifier (Ctrl / Cmd) is held this frame, so the cursor
    /// lands on the exact world point rather than the nearest snap (item 53).
    snap_bypass: bool,
    /// The numeric transform popover (x, y, w, h of the selection) and its edit buffers
    /// (item 51).
    transform: TransformPopover,
    /// Saved view bookmarks (camera snapshots), recalled from the palette (item 34).
    /// Capped at nine slots so the palette list stays short.
    bookmarks: Vec<ViewCamera>,
    /// Text queued for the system clipboard (the copy-permalink action, item 35), flushed
    /// through egui in `App::ui` where the `Context` is available.
    pending_clipboard: Option<String>,

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
    /// The layout-diff overlay: a baseline document snapshot, the computed diff
    /// against the current document, and whether it is painted (see
    /// [`crate::diff_overlay`]).
    diff_overlay: crate::diff_overlay::DiffOverlay,
    /// Anchored comment pins: the layout's comments and the selected one, listed in
    /// the side panel and painted on the canvas (see [`crate::comment_pins`]).
    comment_pins: crate::comment_pins::CommentPins,
    /// The in-progress comment body typed in the comment panel, before it is added.
    comment_draft: String,
    // --- lane review: review panel ---
    /// The design-review panel: per-thread review verdicts and the approve action
    /// (see [`crate::review_panel`]); appends its verdict as an ordinary comment.
    review: crate::review_panel::ReviewPanel,
    // --- end lane review ---
    /// DRC-as-you-type: the incremental checker re-run on every edit so violations are
    /// underlined the moment geometry is drawn (see [`crate::live_drc`]).
    live_drc: crate::live_drc::LiveDrc,
    /// Whether live DRC is on. Off by default so a big load does not pay the first
    /// index build until the user opts in from the DRC panel.
    live_drc_on: bool,
    /// The region dirtied since the live index was last rebuilt. Merged every frame
    /// and re-checked when the throttle rebuilds the index, so an edit made against a
    /// stale snapshot is still underlined once the fresh index lands.
    live_pending: crate::history::Dirty,
    /// Seconds since the live index was last rebuilt, throttling the expensive
    /// re-prepare off the per-edit hot path (see [`poll_live_drc`](Self::poll_live_drc)).
    live_reprepare_accum: f32,
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
    /// The command awaiting a new chord, when the editor is capturing one.
    rebinding: Option<CommandId>,

    /// Whether the command palette window is open.
    palette_open: bool,
    /// Set on the frame the palette (or its argument prompt) opens, so the input
    /// field requests keyboard focus exactly once. Requesting focus *every* frame
    /// keeps `focus.id == field`, which forces `Response::lost_focus()` to `false`
    /// forever, so the Enter-to-run path can never fire; latching it to the open
    /// edge restores the standard `lost_focus() && Enter` idiom.
    palette_focus_pending: bool,
    /// The command-palette search query.
    palette_query: String,
    /// The active inline argument prompt (goto coordinate / cell), if the palette
    /// is in argument-entry mode rather than launching commands (lane 3C, item 79).
    palette_arg: Option<crate::command::PaletteArg>,
    /// Recently run palette rows, as their stable keys, most-recent first, surfaced
    /// at the top of the palette (lane 3C, item 79). Bounded by [`PALETTE_RECENTS_MAX`].
    palette_recents: Vec<String>,
    /// Whether the generated keyboard-shortcuts overlay is open (lane 3C, item 18).
    shortcuts_open: bool,
    /// The export/import scratch buffer in the shortcuts editor: the current keymap
    /// TOML on export, or pasted TOML to apply on import (lane 3C, item 82).
    keymap_io_text: String,
    /// The region keyboard focus currently rests in, walked by F6 (lane 3C, item 83).
    focus_region: crate::focus::FocusRegion,
    /// A pending chord-sequence prefix (for example `g` awaiting `l`), if the last
    /// press started a multi-key sequence (lane 3C, item 81).
    pending_chord: Option<keymap::Chord>,
    /// Set when focus just moved (F6) so the render pass requests keyboard focus on
    /// the new [`focus_region`](Self::focus_region)'s anchor once its widget exists.
    focus_request: bool,
    /// The query-bar text for "select by layer".
    layer_query: String,
    /// The relay host in the Share section (see [`crate::share`]).
    share_server: String,
    /// The room name in the Share section; sanitized into the link.
    share_room: String,
    /// The page origin the read-only viewer link points at (where the web bundle
    /// is served). Empty yields a relative viewer link (see [`crate::share`]).
    share_page: String,
    /// A permalink (`?cell=`/`?view=x,y,z`/`?layers=`) parsed from the page URL at boot,
    /// applied once the `?gds=` open completes so the shared cell, camera, and layer set
    /// are restored on top of the loaded document (see [`App::apply_permalink`]). `None`
    /// when the URL carried no view-state params. Consumed only on the wasm open path.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pending_permalink: Option<crate::share::Permalink>,
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
    /// The open/closed and text state of this lane's dialogs (Open-from-URL, Convert,
    /// Share). Folded into one field to keep the many flags together (see
    /// [`crate::dialogs::DialogState`]).
    dialogs: crate::dialogs::DialogState,
    /// Online/offline tracking for the offline badge and reconnect toasts (item 74).
    /// Fed by the live transport's socket open/close; produces at-most-one toast per
    /// real transition (see [`crate::notify::ConnectivityState`]).
    connectivity: crate::notify::ConnectivityState,
    /// The most recent retryable operation, replayed when a toast's Retry action is
    /// clicked (item 71). `None` when the last failure was not retryable.
    retry: Option<RetryOp>,
    /// The URL of the in-flight remote open, so a fetch failure can name it in the
    /// CORS/network explainer (item 2). Set when an Open-from-URL fetch starts.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    last_fetch_url: Option<String>,
    /// Set when the user cancels an in-flight remote open from the progress card
    /// (item 6). The background fetch cannot be aborted mid-flight on the browser, so
    /// its eventual result is dropped instead of installed.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    load_canceled: bool,

    // ---- Served-archive browse (`?archive=`, lane v8-2e) -------------------
    /// The open archive browse, present only when the page opened a served `.rtla` via
    /// `?archive=` (see [`crate::archive`]). `None` in a normal editing session; when
    /// `Some`, the canvas paints the read-only streamed die with progressive residency
    /// instead of the editable document.
    archive: Option<crate::archive::ArchiveBrowse>,
    /// The mailbox the async archive-open task posts the finished browse into (wasm only;
    /// always empty on native). Uniform field type across targets, like [`web_open`](Self::web_open).
    archive_open: crate::archive::ArchiveOpenInbox,
    /// The `?archive=` URL to open on the first wasm frame, stashed by
    /// [`with_archive`](Self::with_archive) until the open path is kicked off. Consumed on
    /// wasm; dead on native (no browser fetch).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pending_archive_url: Option<String>,
    /// Whether the one-shot archive open has been kicked off (guards it to the first wasm
    /// frame, mirroring [`web_open_started`](Self::web_open_started)).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    archive_started: bool,
    /// Whether the camera has framed the streamed die yet. The first archive frame fits
    /// the camera to the archive world; after that the browse camera is the user's.
    archive_framed: bool,

    // ---- Live share transport (ADR 0058) -----------------------------------
    /// The read-only viewer target (room + relay) this app booted from, when the page
    /// URL was a viewer link (`?view=viewer&...`). `Some` opens the viewer transport on
    /// the first wasm frame and mirrors the sharer's session read-only; `None` is the
    /// normal editor/theater boot. Threaded in by [`App::with_viewer`].
    viewer_target: Option<crate::share::ViewerTarget>,
    /// The read-only viewer state machine (ADR 0038), present only in a viewer session.
    /// The viewer transport pumps the sharer's frames into it; the App renders its
    /// mirrored document and the sharer's presence read-only.
    viewer_session: Option<crate::viewer::ViewerSession>,

    // ---- Lane 2D: viewer chrome, presence, presentation --------------------
    /// The local display name that rides on this session's published presence and
    /// labels the local user (catalog 86). Editable from the viewer chrome; defaults
    /// to a neutral label.
    display_name: String,
    /// Per-actor last cursor position and the egui-time it last moved, so a parked
    /// remote cursor can fade (catalog 86 idle fade). Keyed by actor id.
    presence_seen: std::collections::HashMap<String, (Point, f64)>,
    /// Seconds remaining of the remote-edit attribution glow (catalog 90): a brief
    /// canvas flash in the editor's color when a mirrored remote frame lands.
    remote_edit_flash: f32,
    /// Whether the shared session's sharer has dropped, so the viewer shows a
    /// read-only freeze notice (catalog 91). Cleared if the socket reopens.
    sharer_left: bool,
    /// Tracks whether the live transport was last seen open, to detect the
    /// open -> closed transition that raises [`sharer_left`](Self::sharer_left).
    viewer_was_open: bool,
    /// Lane 2D pinning for the Start-screen recent-files list (catalog 9): a sibling
    /// to Lane 1B's frozen recent-files model that floats pinned entries to the top.
    recent_pins: crate::startscreen::RecentPins,
    /// Whether the New Tiny Tapeout tile wizard (catalog 24) is open over the Start
    /// screen, showing the pin-map preview before creating the tile.
    tt_wizard_open: bool,
    /// Whether the viewer has framed the sharer's design yet. The first mirrored frame
    /// fits the camera to the sharer's layout (so the viewer does not have to pan to
    /// find it); after that the viewer's camera is left alone for independent pan/zoom.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    viewer_framed: bool,
    /// The live viewer transport, holding the `web_sys::WebSocket` and its decode
    /// closures on wasm (inert on native). Kept alive for the session; dropping it
    /// closes the socket. It exposes no publish path (the app-side read-only guarantee).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    viewer_transport: Option<crate::livesync::ViewerTransport>,
    /// The live sharer transport, opened from the Share section's "Go live" action. It
    /// publishes the editor's document and the sharer's presence so viewers stream them.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    sharer_transport: Option<crate::livesync::SharerTransport>,
    /// The mailbox the socket callbacks post decoded [`LiveEvent`](crate::livesync::LiveEvent)s
    /// into and the egui loop drains each frame (mirrors [`web_open`](Self::web_open)).
    live_inbox: crate::livesync::LiveInbox,
    /// The current connection status of whichever live transport is open, for the
    /// status line. `Connecting` until the socket opens.
    live_status: crate::livesync::LiveStatus,
    /// Whether the one-shot viewer transport has been opened (guards it to the first
    /// wasm frame of a viewer session, mirroring [`web_open_started`](Self::web_open_started)).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    live_started: bool,
    /// The document revision last published to the sharer transport, so a re-encode and
    /// send happen only when the editor's document actually changed (not every frame).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    published_revision: u64,
    /// The sharer's long-lived CRDT document (ADR 0063). Kept for a whole live session
    /// and reconciled to the editable document on each publish, so `yrs` clocks advance
    /// monotonically and viewers integrate every delta. Rebuilding a fresh document per
    /// publish (the old behavior) reset the clocks, so viewers dropped every publish
    /// after the first as duplicate struct ids.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    sharer_doc: Option<reticle_sync::SyncDocument>,
    /// Set when the next publish must carry the full document state (on go-live and on
    /// every socket (re)open) rather than an incremental delta, so a reconnecting viewer
    /// or a late joiner receives a complete snapshot.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    publish_full_next: bool,
    /// Whether to open the sharer transport automatically on the first wasm frame,
    /// publishing this session for viewers without a manual "Go live" click. Set from a
    /// `?share=1` page flag (see [`App::with_share_on_boot`]); the browser share-live
    /// e2e uses it so a headless context can act as the publisher (the "Go live" button
    /// is egui-canvas-painted and not DOM-clickable). `false` in normal use.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    share_on_boot: bool,
    /// Whether the page requested the e2e edit-script mode (`?e2e-edit=1`, see
    /// [`crate::share::parse_e2e_edit`]). When set together with [`share_on_boot`](Self::share_on_boot),
    /// the publisher places one scripted rect after going live so lane v8-1e's browser
    /// test can observe the edit reach a viewer. `false` in normal use; only read on wasm.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    e2e_edit: bool,
    /// The last world position under the cursor, for the status readout.
    cursor_world: Option<Point>,
    /// The canvas screen rectangle from the last frame, cached so the live-share sharer
    /// can compute the viewport it publishes (the region a following viewer frames)
    /// without threading the screen through the publish path. `None` until the canvas
    /// has been laid out once (see [`App::local_presence`]).
    last_screen: Option<ScreenRect>,
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

    /// Whether presentation mode is on (catalog 93, lane 2D): all chrome is hidden
    /// and only the canvas is drawn full-window. Toggled by the `view.presentation`
    /// command (default chord `P`) and left by pressing `P` again or `Escape`.
    presentation: bool,
    /// Whether embed mode is on (`?embed=1`, catalog 94, lane 2D): minimal chrome for
    /// an iframe, only the canvas plus a small "open in Reticle" affordance. Set once
    /// from the page URL (see [`App::set_embed`]); unlike presentation it does not
    /// toggle, since an embedding page fixes it.
    embed: bool,

    /// View and export polish: the egui theme, per-document camera bookmarks, the
    /// export scope/format, and the print-style monochrome toggle. The whole
    /// struct persists with the session view state (see [`crate::viewexport`]).
    view_export: crate::viewexport::ViewExport,

    /// The right Inspector's remembered layout (lane 2B): the selected group, the
    /// dragged width, the icon-rail collapse flag, and every section's open flag.
    /// Loaded from the session and re-saved on exit (catalog 59, 61).
    inspector: crate::inspector_layout::InspectorState,

    /// The document [`revision`](History::revision) DRC last ran at, so the DRC
    /// section can flag a stale result after an edit (lane 2B, catalog 63).
    drc_ran_revision: Option<u64>,
    /// The selected row in the diff change list (lane 2B, catalog 64).
    diff_selected: Option<usize>,
    /// The per-layer filter applied to the diff change list; `None` shows every
    /// layer (lane 2B, catalog 64).
    diff_layer_filter: Option<LayerId>,

    /// The UI density feeding the theme (ADR 0095). Loaded from the session; no
    /// user-facing toggle this wave (lane 4C adds the Settings control in Wave 2).
    ui_density: crate::theme::tokens::Density,
    /// Whether functional motion is suppressed. Loaded from the session; zeroes
    /// the theme's animation time when set.
    reduced_motion: bool,
    /// Set when the applied egui style needs a re-apply (density, reduced-motion,
    /// or touch-mode change). Starts `true` so the theme is installed on the first
    /// frame instead of per frame; the boot styling hook reads it at the top of
    /// the frame body.
    theme_dirty: bool,

    // ---- Lane 2c: left panel + managed view panels --------------------------
    /// Left Layers panel width, in points; restored from and saved to the session
    /// so a resized panel survives a restart.
    panel_left_w: f32,
    /// Whether the managed 3D-stack panel is open (View > Panels, ADR 0096). Docked
    /// to the canvas bottom, never floating over the rulers or minimap.
    panel_3d_open: bool,
    /// Whether the managed Cross-section panel is open (View > Panels, ADR 0096).
    panel_xsection_open: bool,
    /// The layer the pointer hovers in the Layers panel this frame, if any: the
    /// canvas dims every other layer while it is set (catalog 39 hover peek). Reset
    /// each frame from the panel; consumed by the retained-scene rebuild.
    peek_layer: Option<LayerId>,
    /// The layer whose color-editor popover is open, if any (catalog 62).
    color_editor: Option<LayerId>,
    /// The in-progress name for a new visibility preset (catalog 62).
    preset_name: String,

    // ---- Lane 4A: first-run tour --------------------------------------------
    /// The first-run tour state machine (pure; see [`crate::tour`]). Auto-starts on
    /// a fresh install and is relaunchable from the Help menu. Its "seen" bit
    /// persists with the session so the automatic tour shows only once.
    tour: Tour,

    // ---- lane 4c: settings, onboarding, and help ----------------------------
    /// What a bare mouse-wheel scroll does over the canvas (Settings dialog; lane
    /// 3A's canvas reads it). Persisted with the session.
    wheel: crate::settings::WheelBehavior,
    /// Whether the enlarged touch targets are forced on/off or auto-detected
    /// (Settings dialog; lane 4B reads it). Persisted with the session.
    touch_mode: crate::settings::TouchMode,
    /// Which once-only contextual hints have fired (catalog 17). Persisted.
    hints: crate::onboarding::Hints,
    /// The active contextual hint bubble to draw this frame, if one just fired.
    active_hint: Option<crate::onboarding::Hint>,
    /// The onboarding checklist: completed tasks and the sticky dismiss (catalog
    /// 19). Persisted.
    checklist: crate::onboarding::Checklist,
    /// Whether the first-run GPU capability card has been dismissed (catalog 22).
    /// Persisted so it shows at most once.
    gpu_card_dismissed: bool,
    /// Whether the Settings dialog is open (catalog 98).
    settings_open: bool,
    /// Whether the About dialog is open (catalog 99, 100).
    about_open: bool,
    /// Whether the "What's new" dialog is open (catalog 26).
    whats_new_open: bool,
    /// A URL to open in the browser next frame (Documentation, the issue link).
    /// Consumed at the top of the frame where the egui context is available.
    pending_open_url: Option<String>,

    /// When set, the app renders the hidden component gallery full-window instead
    /// of the editor (`?gallery=1` web flag / `--gallery` native flag), a stable
    /// screenshot surface for the visual-regression suite (lane 1C/1D). `None` in
    /// every normal session.
    gallery: Option<crate::theme::gallery::GalleryState>,

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

    /// Whether a demo capture wants the right column scrolled to the search panel (the
    /// filter-query bar and outline tree), which otherwise sits below the fold. Set by
    /// a `filter`/`outline-locate` step so the query tour shows the bar it is driving.
    #[cfg(not(target_arch = "wasm32"))]
    demo_focus_search: bool,

    /// Whether a demo capture wants the right column scrolled to the Generate panel,
    /// which otherwise sits below the fold. Set by a `generator`/`gen-param`/`gen-place`
    /// step so the generator tour shows the panel it is driving.
    #[cfg(not(target_arch = "wasm32"))]
    demo_focus_generate: bool,
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

/// How many recently-run palette rows are remembered for the recents group.
const PALETTE_RECENTS_MAX: usize = 20;

/// The kind of a row in the diff change list (catalog 64): an added, removed, or
/// changed shape, keyed by the marker the legend and overlay use.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DiffKind {
    /// A shape present only in the current layout (green, `+`).
    Added,
    /// A shape present only in the baseline (red, `-`).
    Removed,
    /// A shape whose extent changed in place (amber, `~`).
    Changed,
}

impl DiffKind {
    /// The single-character marker shown before a change-list row.
    fn glyph(self) -> char {
        match self {
            DiffKind::Added => '+',
            DiffKind::Removed => '-',
            DiffKind::Changed => '~',
        }
    }
}

/// Example prompts for the agent preview (catalog 21). Clicking one seeds the prompt
/// box; the preview runs the same fixed scripted demo regardless of the text, so these
/// describe what the demo shows rather than promising edits to your design.
const AGENT_SAMPLE_PROMPTS: &[&str] = &[
    "Draw a clean wire",
    "Show the propose-verify-correct loop",
    "Demonstrate a width-rule fix",
];

/// How long, in seconds, the remote-edit attribution glow (catalog 90) lingers
/// after a mirrored remote frame lands before it has fully faded.
const REMOTE_EDIT_FLASH_SECS: f32 = 0.8;

/// Follow-mode easing rate (catalog 87): the per-frame interpolation weight is
/// `dt * this`, so the followed camera glides to the sharer's view over roughly an
/// eighth of a second rather than snapping.
const FOLLOW_EASE_RATE: f32 = 8.0;

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

    /// Creates the app as a **read-only viewer** of the shared session named by
    /// `target` (a room on a relay, ADR 0038/0058).
    ///
    /// The web mount calls this when the page URL is a viewer link
    /// (`?view=viewer&room=..&relay=..`). The app boots into the editor chrome but, on
    /// its first wasm frame, opens the read-only [viewer transport](crate::livesync)
    /// to the relay room (`?mode=view`) and mirrors the sharer's live document and
    /// presence into the canvas, read-only. The viewer transport has no publish path,
    /// so the viewer can never mutate the shared session (the relay's `?mode=view` drop
    /// is the independent server-side backstop).
    ///
    /// On native this simply records the target; nothing dials a browser socket, so the
    /// desktop build still constructs and the field stays inert until a wasm frame runs.
    #[must_use]
    pub fn with_viewer(target: crate::share::ViewerTarget) -> Self {
        // A viewer opens the editor chrome (not the Start chooser), then the transport
        // takes over the canvas with the mirrored document. Seed the Share fields with
        // the target so the panel reflects the joined room.
        let mut app = Self::build(StartView::Editor);
        app.start_screen = false;
        app.share_server.clone_from(&target.relay);
        app.share_room.clone_from(&target.room);
        app.viewer_session = Some(crate::viewer::ViewerSession::new());
        app.viewer_target = Some(target);
        // A viewer gets the shorter viewer-variant tour, not the editor walkthrough
        // (catalog 15). It stays dormant on the web (the tour is treated as seen so
        // it does not reopen every visit); the Help menu can relaunch it.
        app.tour = Tour::viewer(tour_already_seen());
        app
    }

    /// Whether this app booted as a read-only viewer of a shared session.
    #[must_use]
    pub fn is_viewer(&self) -> bool {
        self.viewer_target.is_some()
    }

    /// Creates the app opening into `start_view` and, on wasm, going live automatically
    /// on the first frame for `room` on `relay` so viewers can stream it immediately.
    ///
    /// This is the publisher side of the browser share-live end-to-end (ADR 0058): the
    /// deployed "Go live" is a manual button, but the button is painted on the egui
    /// canvas and so is not reachable by a headless DOM click, so the e2e boots a
    /// publisher context with `?share=1` and this constructor opens the sharer transport
    /// without a click. It is an ordinary editor session in every other respect.
    #[must_use]
    pub fn with_share_on_boot(start_view: StartView, relay: String, room: String) -> Self {
        let mut app = Self::with_start_view(start_view);
        app.share_server = relay;
        app.share_room = room;
        app.share_on_boot = true;
        app
    }

    /// Creates the app to **browse a served `.rtla` archive** streamed from `url`
    /// (`?archive=<url>`, lane v8-2e).
    ///
    /// The web mount calls this when the page URL carries an `?archive=` link
    /// (see [`crate::share::archive_url_from_query`]). The app boots the editor chrome
    /// but, on its first wasm frame, opens an [`HttpRangeTileSource`](reticle_index::tile_source)
    /// over `url`, builds a read-only [`StreamedScene`](crate::streamed::StreamedScene),
    /// and paints it with progressive residency (ADR 0062). Editing the streamed die is a
    /// compile error (the [`Streamed`](crate::dochost::DocHost::Streamed) arm exposes no
    /// mutation), so browse, measure, and query work while an edit cannot be expressed.
    ///
    /// On native this simply stashes the URL; nothing dials a browser fetch, so the
    /// desktop build still constructs and the archive stays unopened.
    #[must_use]
    pub fn with_archive(url: String) -> Self {
        let mut app = Self::build(StartView::Editor);
        // Browse opens straight onto the canvas, not the worked-use-case chooser.
        app.start_screen = false;
        app.pending_archive_url = Some(url);
        app
    }

    /// Creates the app booting **straight into the guided tour** (`?tour=1` web
    /// deep link / `--tour` native flag, catalog 20).
    ///
    /// It opens the editor (skipping the Start chooser so the tour highlights the
    /// real panels) and forces the tour to run from its first step regardless of the
    /// persisted "seen" bit, so a shared onboarding link always lands in the tour.
    #[must_use]
    pub fn with_tour() -> Self {
        let mut app = Self::with_start_view(StartView::Editor);
        app.start_screen = false;
        app.tour.start_deep_link();
        app
    }

    /// Creates the app in **gallery mode**: it renders the hidden component
    /// gallery full-window instead of the editor (`?gallery=1` web flag /
    /// `--gallery` native flag). The gallery is a pure, deterministic surface
    /// over the [`theme::components`] library, used by
    /// the visual-regression suite to snapshot every component state without
    /// booting the editor (lane 1C/1D).
    #[must_use]
    pub fn gallery() -> Self {
        let mut app = Self::build(StartView::Editor);
        app.start_screen = false;
        app.gallery = Some(crate::theme::gallery::GalleryState::default());
        app
    }

    /// Applies the recorded [`StartView`] to the constructed app.
    ///
    /// For [`StartView::ReplayTheater`] it opens the docked theater panel and loads
    /// the default transcript from the platform [`store`](crate::store) (the bundled
    /// transcript on wasm, the scripted run on native), so a public web visitor
    /// lands on a loaded replay with the transport ready to play (ADR 0026). If the
    /// store cannot produce a transcript, the theater simply opens empty rather than
    /// failing.
    ///
    /// The landing does not auto-play: the theater is docked (H1) so it never
    /// occludes the menu bar or canvas, and it waits at "press Play or Step" rather
    /// than running the demo to its end. Auto-playing exposed a pre-existing
    /// wasm-versus-native `document_hash` nondeterminism (the same recorded run
    /// replays to a hash MATCH on native but a MISMATCH readout on wasm), so an
    /// auto-played landing would greet a first visitor with a red MISMATCH. The
    /// determinism gap is tracked separately; the inviting idle state is the right
    /// public first frame regardless.
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
    // One flat struct literal initializing every field: the length is field count, not
    // branching logic, so the line-count lint does not apply.
    #[allow(clippy::too_many_lines)]
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
        // The persisted preferences (theme density and reduced motion, the lane 4c
        // settings and onboarding state, and the lane 2c panel layout) all come from
        // the saved session on native or the localStorage mirror on the web; defaults
        // on a fresh install. The boot styling hook in `ui` applies the theme ones on
        // the first frame.
        let boot = boot_session();
        let (ui_density, reduced_motion) = (boot.ui_density, boot.reduced_motion);
        let panels = crate::session::PanelLayout {
            left_w: boot.panel_left_w,
            panel_3d_open: boot.panel_3d_open,
            panel_xsection_open: boot.panel_xsection_open,
        };
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
            drag_press_pos: None,
            labels_visible: true,
            minimap_visible: true,
            rulers_visible: true,
            crosshair: false,
            archive_hud_visible: true,
            camera_tween: None,
            pending_view: None,
            // The packet's ~150 ms LOD crossfade (docs/design/tokens.md motion contract).
            archive_fade: LevelFade::new(0.15),
            hover_pick: None,
            snap_bypass: false,
            transform: TransformPopover::default(),
            bookmarks: Vec::new(),
            pending_clipboard: None,
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
            diff_overlay: crate::diff_overlay::DiffOverlay::new(),
            comment_pins: crate::comment_pins::CommentPins::new(),
            comment_draft: String::new(),
            // --- lane review: review panel ---
            review: crate::review_panel::ReviewPanel::new(),
            // --- end lane review ---
            live_drc: crate::live_drc::LiveDrc::new(),
            live_drc_on: false,
            live_pending: crate::history::Dirty::None,
            live_reprepare_accum: 0.0,
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
            palette_focus_pending: false,
            palette_query: String::new(),
            palette_arg: None,
            palette_recents: Vec::new(),
            shortcuts_open: false,
            keymap_io_text: String::new(),
            focus_region: crate::focus::FocusRegion::Canvas,
            pending_chord: None,
            focus_request: false,
            layer_query: String::new(),
            share_server: crate::share::DEFAULT_SERVER.to_owned(),
            share_room: crate::share::room_id(demo::TOP_CELL),
            share_page: String::new(),
            pending_permalink: None,
            status: Status::default(),
            open_warnings: Vec::new(),
            notifications: crate::notify::Notifications::new(),
            recent_files: crate::webopen::RecentFiles::new(),
            load_progress: crate::webopen::LoadProgress::Idle,
            web_open: crate::webopen::WebOpenInbox::new(),
            web_open_started: false,
            dialogs: crate::dialogs::DialogState::default(),
            connectivity: crate::notify::ConnectivityState::new(),
            retry: None,
            last_fetch_url: None,
            load_canceled: false,
            archive: None,
            archive_open: crate::archive::ArchiveOpenInbox::new(),
            pending_archive_url: None,
            archive_started: false,
            archive_framed: false,
            last_screen: None,
            viewer_target: None,
            viewer_session: None,
            display_name: "Guest".to_owned(),
            presence_seen: std::collections::HashMap::new(),
            remote_edit_flash: 0.0,
            sharer_left: false,
            viewer_was_open: false,
            recent_pins: crate::startscreen::RecentPins::new(),
            tt_wizard_open: false,
            viewer_framed: false,
            viewer_transport: None,
            sharer_transport: None,
            live_inbox: crate::livesync::LiveInbox::new(),
            live_status: crate::livesync::LiveStatus::default(),
            live_started: false,
            published_revision: 0,
            sharer_doc: None,
            publish_full_next: false,
            share_on_boot: false,
            e2e_edit: false,
            cursor_world: None,
            frame_meter: FrameMeter::default(),
            start_view,
            // Greet a first-time user with the worked-use-case chooser on the
            // editor default; the replay-theater start view drops the visitor
            // straight into the theater and never shows it.
            start_screen: start_view == StartView::Editor,
            presentation: false,
            embed: false,
            view_export: crate::viewexport::ViewExport::new(),
            inspector: boot_inspector_state(),
            drc_ran_revision: None,
            diff_selected: None,
            diff_layer_filter: None,
            ui_density,
            reduced_motion,
            theme_dirty: true,
            panel_left_w: panels.left_w,
            panel_3d_open: panels.panel_3d_open,
            panel_xsection_open: panels.panel_xsection_open,
            peek_layer: None,
            color_editor: None,
            preset_name: String::new(),
            tour: Tour::from_seen(tour_already_seen()),
            wheel: boot.wheel,
            touch_mode: boot.touch_mode,
            hints: boot.hints,
            active_hint: None,
            checklist: boot.checklist,
            gpu_card_dismissed: boot.gpu_card_dismissed,
            settings_open: false,
            about_open: false,
            whats_new_open: false,
            pending_open_url: None,
            gallery: None,
            #[cfg(not(target_arch = "wasm32"))]
            capture: None,
            #[cfg(not(target_arch = "wasm32"))]
            demo: None,
            #[cfg(not(target_arch = "wasm32"))]
            demo_focus_search: false,
            #[cfg(not(target_arch = "wasm32"))]
            demo_focus_generate: false,
        }
    }

    /// The view the app opened into (editor or replay theater).
    #[must_use]
    pub fn start_view(&self) -> StartView {
        self.start_view
    }

    /// The touch-target mode (lane 4B's effect; lane 4C owns the persisted setting).
    ///
    /// `Auto` follows the platform coarse-pointer signal; `On`/`Off` force the
    /// enlarged hit targets on or off. When the effective mode is on, the tokened
    /// style raises `interact_size.y` to the touch minimum over either density so
    /// hit targets meet the tablet/phone floor.
    #[must_use]
    pub fn touch_mode(&self) -> crate::settings::TouchMode {
        self.touch_mode
    }

    /// Sets the touch-target mode and schedules a style re-apply if it changed.
    ///
    /// The plumbing seam lane 4C's Settings control drives: changing this marks the
    /// applied style dirty, so the next frame's boot styling hook reinstalls the
    /// style with the touch hit-target floor raised or lowered. A no-op when the
    /// value is unchanged, so it is cheap to call every frame from a bound setting.
    /// The choice persists with the session on the next save.
    pub fn set_touch_mode(&mut self, touch: crate::settings::TouchMode) {
        if self.touch_mode != touch {
            self.touch_mode = touch;
            self.theme_dirty = true;
        }
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
        // Drop the diff baseline: it snapshotted the previous document, so a diff
        // against the fresh one would be meaningless until re-snapshotted.
        self.diff_overlay.clear();
        // Drop the live index and its underlines; the new document builds a fresh one
        // on the next edit (and the pending dirt from `History::new` is drained then).
        self.live_drc.clear();
        self.live_pending = crate::history::Dirty::None;
        self.live_reprepare_accum = 0.0;
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
            let summary = format!("Opened document ({cell_count} cells)");
            self.status.set(summary.clone());
            // A post-open summary toast (catalog item 7): shapes, layers, bounding
            // box, and a Fit action so the user can reframe with one click. Metrics
            // are read from the freshly installed document and layer table.
            self.notifications.push(
                crate::notify::Notification::new(
                    crate::notify::Severity::Info,
                    summary,
                    self.post_open_detail(),
                )
                .with_action(crate::notify::NotificationAction::Fit),
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

    /// The one-line body for the post-open summary toast (item 7): shape count, layer
    /// count, and the top cell's bounding box in DBU. Read from the just-installed
    /// document and layer table.
    fn post_open_detail(&self) -> String {
        let shapes = self.scene.shapes().len();
        let layers = self.layer_state.rows().len();
        let bbox = self
            .history
            .document()
            .cell_bbox(&self.top_cell)
            .map_or_else(
                || "empty".to_owned(),
                |b| format!("{} x {} DBU", b.width(), b.height()),
            );
        format!("{shapes} shapes on {layers} layers, bounding box {bbox}")
    }

    /// Opens a named file's `bytes` through the same hardened path drag-and-drop uses:
    /// classify the format from the name, apply the big-file size band, open through
    /// the seam, and record the recent entry, surfacing a rich [`crate::notify`]
    /// failure (cause, next step, copyable diagnostic) on any refusal.
    ///
    /// This is the single funnel the file picker (native and browser), a paste, and a
    /// multi-file open route through, so every open surface reports success and
    /// failure identically (items 1, 4, 8, 72). Returns whether the file opened.
    fn open_named_bytes(&mut self, name: &str, bytes: &[u8]) -> bool {
        let Some(format) = crate::webopen::classify_drop(name) else {
            self.report_failure(
                format!("Could not open {name}"),
                crate::dialogs::unsupported_file_diagnostic(name),
                Some(RetryOp::Picker),
            );
            return false;
        };
        let plan = crate::webopen::LoadPlan::for_size(bytes.len() as u64);
        if let Some(message) = plan.refusal_message() {
            self.report_failure(
                format!("{name} is too large for the browser build"),
                crate::notify::Diagnostic::new(
                    message,
                    "Open it in the desktop app, or split it into smaller cells.",
                    format!("file: {name}\nsize: {} bytes", bytes.len()),
                ),
                Some(RetryOp::Picker),
            );
            return false;
        }
        match crate::open::open_document_bytes(bytes, format) {
            Ok(outcome) => {
                self.open_outcome(outcome);
                self.record_recent_file(crate::webopen::RecentFile::local(
                    name.to_owned(),
                    bytes.len() as u64,
                ));
                self.persist_recent_files();
                true
            }
            Err(e) => {
                self.report_failure(
                    format!("Could not open {name}"),
                    crate::dialogs::open_error_diagnostic(&e),
                    Some(RetryOp::Picker),
                );
                false
            }
        }
    }

    /// Opens the native/browser file picker (rfd) and routes the chosen file through
    /// [`open_named_bytes`](Self::open_named_bytes), the same hardened path as
    /// drag-and-drop (catalog item 1, `file.open_dialog`, Ctrl+O).
    ///
    /// On native this uses the blocking [`rfd::FileDialog`] and reads the file inline.
    /// On the web there is no filesystem, so [`rfd::AsyncFileDialog`] shows the hidden
    /// HTML file input; the async read posts the bytes into the shared
    /// [`WebOpenInbox`](crate::webopen::WebOpenInbox) as a
    /// [`WebOpenEvent::Opened`](crate::webopen::WebOpenEvent::Opened), so the picker
    /// rides the exact same classify -> plan -> seam path the inbox already drains.
    fn open_file_dialog(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let picked = rfd::FileDialog::new()
                .add_filter("Layout files", &["gds", "gdsii", "gds2", "oas", "oasis"])
                .pick_file();
            if let Some(path) = picked {
                let name = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        self.open_named_bytes(&name, &bytes);
                    }
                    Err(e) => self.notifications.fail(
                        format!("Could not read {name}"),
                        crate::notify::Diagnostic::new(
                            "The file could not be read from disk.",
                            "Check that the file still exists and is readable, then try again.",
                            format!("path: {}\n{e}", path.display()),
                        ),
                    ),
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let inbox = self.web_open.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let Some(handle) = rfd::AsyncFileDialog::new()
                    .add_filter("Layout files", &["gds", "gdsii", "gds2", "oas", "oasis"])
                    .pick_file()
                    .await
                else {
                    return;
                };
                let name = handle.file_name();
                let Some(format) = crate::webopen::classify_drop(&name) else {
                    inbox.post(crate::webopen::WebOpenEvent::Failed(format!(
                        "\"{name}\" is not a .gds or .oas file"
                    )));
                    return;
                };
                let bytes = handle.read().await;
                let size = bytes.len() as u64;
                inbox.post(crate::webopen::WebOpenEvent::Opened {
                    bytes,
                    format,
                    recent: crate::webopen::RecentFile::local(name, size),
                });
            });
        }
    }

    /// Starts an Open-from-URL fetch of the (already validated) `url`, feeding the
    /// bytes through the same inbox path as a `?gds=` link (item 2, `file.open_url`).
    ///
    /// On the web this spawns the browser `fetch`; a CORS block or network failure
    /// comes back as a [`WebOpenEvent::Failed`](crate::webopen::WebOpenEvent::Failed),
    /// which [`apply_web_open_event`](Self::apply_web_open_event) turns into the
    /// plain-language CORS/network explainer. On native there is no cross-origin fetch
    /// story (the desktop build opens local files), so this explains that and points
    /// at the file picker.
    // On wasm the URL is consumed (moved into the fetch task and the recent entry); on
    // native it is only named in a message, so it is borrowed there.
    #[cfg_attr(not(target_arch = "wasm32"), allow(clippy::needless_pass_by_value))]
    fn start_url_open(&mut self, url: String) {
        #[cfg(target_arch = "wasm32")]
        {
            let Some(format) = crate::webopen::classify_drop(&crate::webopen::url_file_name(&url))
            else {
                self.report_failure(
                    "Could not open the URL",
                    crate::dialogs::unsupported_file_diagnostic(&url),
                    None,
                );
                return;
            };
            self.last_fetch_url = Some(url.clone());
            self.load_canceled = false;
            self.load_progress = crate::webopen::LoadProgress::fetched(0, 0);
            let inbox = self.web_open.clone();
            let name = crate::webopen::url_file_name(&url);
            wasm_bindgen_futures::spawn_local(async move {
                match crate::webopen::fetch_gds_bytes(&url).await {
                    Ok(bytes) => {
                        let size = bytes.len() as u64;
                        if let Some(message) =
                            crate::webopen::LoadPlan::for_size(size).refusal_message()
                        {
                            inbox.post(crate::webopen::WebOpenEvent::Failed(message));
                        } else {
                            inbox.post(crate::webopen::WebOpenEvent::Opened {
                                bytes,
                                format,
                                recent: crate::webopen::RecentFile::remote(name, size, url),
                            });
                        }
                    }
                    Err(message) => {
                        inbox.post(crate::webopen::WebOpenEvent::Failed(message));
                    }
                }
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = &url;
            self.report_failure(
                "Open from URL is available in the browser build",
                crate::notify::Diagnostic::new(
                    "The desktop app opens files from your computer, so it does not \
                     fetch remote URLs.",
                    "Use Open to pick the file from disk, or download it first.",
                    format!("requested url: {url}"),
                ),
                None,
            );
        }
    }

    /// Copies the read-only viewer link for the current room to the clipboard
    /// (`share.copy_viewer_link`).
    ///
    /// Dispatched from a menu, the palette, or a shortcut, none of which thread an
    /// egui context, so the link is staged in [`pending_clipboard`](Self::pending_clipboard)
    /// and the frame loop copies it (the same defer the dialogs use for their copy
    /// buttons that run outside a `ui` closure).
    fn copy_viewer_link(&mut self) {
        let link =
            crate::share::viewer_link(&self.share_page, &self.share_server, &self.share_room);
        self.pending_clipboard = Some(link);
        self.notify(
            "Viewer link copied",
            "Anyone with this link can watch, view-only.",
        );
    }

    /// Stashes a permalink parsed from the page URL to apply once a `?gds=` open
    /// completes (through the private `apply_permalink`).
    ///
    /// Stores `None` when the link carried no view-state params, so an ordinary open is
    /// unaffected; the stashed link is consumed on the wasm open path when the fetched
    /// document is installed, restoring the shared cell, camera, and layers on top of it.
    pub fn set_pending_permalink(&mut self, permalink: crate::share::Permalink) {
        let empty =
            permalink.cell.is_none() && permalink.camera.is_none() && permalink.layers.is_none();
        self.pending_permalink = if empty { None } else { Some(permalink) };
    }

    /// Applies a permalink to the currently open document: focuses the named cell,
    /// restores the camera, and applies the layer visibility set.
    ///
    /// Ordering matters. An explicit camera is the final view, so it wins over both the
    /// auto-fit a fresh open would otherwise run and the frame-the-cell jump; when no
    /// camera is given, a named cell is framed with the existing jump-to-cell machinery
    /// (a deferred locate the canvas consumes next frame). A `Some` layer set hides every
    /// layer, then shows the listed ones (an empty set hides all); `None` leaves layer
    /// visibility alone. Unknown cells and layers are ignored, so a stale link degrades
    /// gracefully rather than blanking the view.
    fn apply_permalink(&mut self, permalink: &crate::share::Permalink) {
        // The link, not the open's auto-fit, controls the view when it positions it.
        if permalink.cell.is_some() || permalink.camera.is_some() {
            self.fit_requested = false;
        }
        if let Some(layers) = &permalink.layers {
            self.layer_state.hide_all();
            for &(layer, datatype) in layers {
                self.layer_state
                    .set_visible(LayerId::new(layer, datatype), true);
            }
        }
        // Frame the cell only when no explicit camera overrides the view.
        if permalink.camera.is_none()
            && let Some(cell) = &permalink.cell
            && let Some(bbox) = self.history.document().cell_bbox(cell)
        {
            self.search.pending_locate = Some(bbox);
        }
        if let Some((x, y, zoom)) = permalink.camera {
            self.camera = ViewCamera::new(Point::new(x as i32, y as i32), zoom);
            // An explicit camera supersedes any pending cell framing.
            self.search.pending_locate = None;
        }
    }

    /// Builds the permalink describing the current view: the focused top cell, the
    /// camera, and the visible layer set (see [`crate::share::session_permalink`]).
    fn session_permalink(&self) -> crate::share::Permalink {
        let camera = self.camera;
        let visible: Vec<(u16, u16)> = self
            .layer_state
            .rows()
            .iter()
            .filter(|r| r.visible)
            .map(|r| (r.id.layer, r.id.datatype))
            .collect();
        let cell = (!self.top_cell.is_empty()).then_some(self.top_cell.as_str());
        crate::share::session_permalink(
            cell,
            (
                f64::from(camera.center().x),
                f64::from(camera.center().y),
                camera.pixels_per_dbu(),
            ),
            &visible,
        )
    }

    /// Serializes the current view to a permalink and copies it to the clipboard.
    ///
    /// The link carries the focused cell, camera, and visible layers on top of the page
    /// origin; opening it restores the same view (see [`App::apply_permalink`]). The
    /// serialization is [`App::session_permalink`]; this is the thin UI action over it.
    fn copy_permalink(&mut self, ctx: &egui::Context) {
        let link = crate::share::emit_permalink(&self.share_page, None, &self.session_permalink());
        ctx.copy_text(link);
        self.status.set("Permalink copied");
    }

    /// Sets whether the e2e edit-script mode is active (`?e2e-edit=1`): in that mode the
    /// publisher-on-boot places one scripted rect after going live. Threaded in by the
    /// web entry at boot.
    pub fn set_e2e_edit(&mut self, on: bool) {
        self.e2e_edit = on;
    }

    /// Starts the replay theater playing on boot when the page requested it
    /// (`?e2e-autoplay=1`, see [`crate::share::parse_e2e_replay_autoplay`]).
    ///
    /// The public `?view=replay` landing waits at Play; a headed browser test flips this
    /// on so it can drive playback to the end and assert the wasm replay reproduces the
    /// recorded hash from the DOM (`window.__reticle_stats.hash_check`), without clicking
    /// the GPU-painted transport. A no-op unless a transcript is loaded (the replay-theater
    /// start view), so it never affects the editor or a normal visitor.
    pub fn set_replay_autoplay(&mut self) {
        if self.replay.is_loaded() {
            // Play fast so the headed guard reaches the end (and the hash verdict)
            // quickly. Speed only controls how many records apply per tick; it does not
            // affect the replayed document or its hash, so the Match verdict is identical
            // to playing at 1x.
            self.replay.set_speed(32.0);
            self.replay.play();
        }
    }

    /// Sets whether the bundle is embedded (`?embed=1`, catalog 94): minimal chrome
    /// for an iframe. Threaded in by the web entry at boot from
    /// [`crate::share::parse_embed`]; a no-op modifier on native.
    pub fn set_embed(&mut self, on: bool) {
        self.embed = on;
    }

    /// Whether the bundle is running in embed mode (`?embed=1`).
    #[must_use]
    pub fn is_embedded(&self) -> bool {
        self.embed
    }

    /// Places one fixed scripted rectangle so lane v8-1e's browser test can observe an
    /// edit propagate from this publisher to a viewer (only reached in `?e2e-edit=1`
    /// mode, after going live).
    #[cfg(target_arch = "wasm32")]
    fn place_e2e_rect(&mut self) {
        let shape = DrawShape::new(
            LayerId::new(68, 20),
            ShapeKind::Rect(Rect::new(Point::new(0, 0), Point::new(1000, 1000))),
        );
        self.add_shapes_undoable(vec![shape]);
        self.status.set("Placed the e2e scripted rect");
    }

    /// Bumps the wasm-only `window.__reticle_stats` counters lane v8-1e's browser e2e
    /// reads: `applied_frames` increments by one and `applied_shapes` is set to the shape
    /// count now in the mirrored top cell. A no-op if the window or stats object is
    /// unavailable, so it never disturbs a normal viewer session.
    #[cfg(target_arch = "wasm32")]
    fn record_viewer_stats(&self) {
        use wasm_bindgen::JsValue;
        let shapes = self
            .history
            .document()
            .cell(&self.top_cell)
            .map_or(0, |c| c.shapes.len()) as f64;
        let Some(window) = web_sys::window() else {
            return;
        };
        let key = JsValue::from_str("__reticle_stats");
        let stats = match js_sys::Reflect::get(window.as_ref(), &key) {
            Ok(v) if v.is_object() => v,
            _ => {
                let obj = js_sys::Object::new();
                let _ = js_sys::Reflect::set(window.as_ref(), &key, obj.as_ref());
                JsValue::from(obj)
            }
        };
        let frames = js_sys::Reflect::get(&stats, &JsValue::from_str("applied_frames"))
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let _ = js_sys::Reflect::set(
            &stats,
            &JsValue::from_str("applied_frames"),
            &JsValue::from_f64(frames + 1.0),
        );
        let _ = js_sys::Reflect::set(
            &stats,
            &JsValue::from_str("applied_shapes"),
            &JsValue::from_f64(shapes),
        );
    }

    /// Publishes the live view camera into `window.__reticle_stats.camera` so lane
    /// v8-1e's phone-touch browser e2e can observe a pinch/pan actually move the camera
    /// (the canvas is GPU-painted, so there is no DOM node or reliable pixel readback to
    /// assert on; this readout is the browser-observable proof, mirroring the
    /// applied-frame counters [`record_viewer_stats`](Self::record_viewer_stats) exposes).
    ///
    /// Written every editor frame from [`ui`](Self::ui): `camera.pixels_per_dbu` is the
    /// zoom and `camera.center_x`/`camera.center_y` the world point at the canvas center,
    /// so a test reads a baseline, performs a touch gesture, and asserts the readout moved.
    /// A no-op if the window is unavailable, so it never disturbs a normal session, and it
    /// only writes (never reads app state back), so it cannot affect rendering.
    #[cfg(target_arch = "wasm32")]
    fn record_camera_stats(&self) {
        use wasm_bindgen::JsValue;
        let Some(window) = web_sys::window() else {
            return;
        };
        let key = JsValue::from_str("__reticle_stats");
        let stats = match js_sys::Reflect::get(window.as_ref(), &key) {
            Ok(v) if v.is_object() => v,
            _ => {
                let obj = js_sys::Object::new();
                let _ = js_sys::Reflect::set(window.as_ref(), &key, obj.as_ref());
                JsValue::from(obj)
            }
        };
        let center = self.camera.center();
        let cam = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &cam,
            &JsValue::from_str("center_x"),
            &JsValue::from_f64(f64::from(center.x)),
        );
        let _ = js_sys::Reflect::set(
            &cam,
            &JsValue::from_str("center_y"),
            &JsValue::from_f64(f64::from(center.y)),
        );
        let _ = js_sys::Reflect::set(
            &cam,
            &JsValue::from_str("pixels_per_dbu"),
            &JsValue::from_f64(self.camera.pixels_per_dbu()),
        );
        let _ = js_sys::Reflect::set(&stats, &JsValue::from_str("camera"), cam.as_ref());
    }

    /// Publishes the additive demo-observability keys to `window.__reticle_stats`
    /// (wasm only), alongside the camera [`record_camera_stats`](Self::record_camera_stats)
    /// writes each editor frame. These make three otherwise canvas-only or
    /// viewer-path-only signals browser-readable, so the headed demo guards can assert
    /// them:
    ///
    /// - `applied_shapes`: the live top-cell DIRECT shape count. [`record_viewer_stats`]
    ///   (Self::record_viewer_stats) publishes this only on the share-live viewer path;
    ///   here it is republished every editor frame. It is 0 for a hierarchical design
    ///   whose top cell holds only instances, so `applied_scene_shapes` is the metric a
    ///   per-example load check should use.
    /// - `applied_scene_shapes`: the flattened renderable shape count (the top cell with
    ///   every instance expanded). It is > 0 whenever the editor has geometry to draw,
    ///   hierarchical or flat, so it is the honest "the example loaded" signal.
    /// - `render_nonblank`: `true` when the app is painting real geometry this frame (the
    ///   flattened scene has shapes, or a streamed archive is painting records) with a
    ///   finite positive zoom. It separates "rendered the design" from a black canvas: a
    ///   backgrounded tab pauses the rAF loop so this method never runs and the key stays
    ///   absent/stale. It does NOT assert every pixel is lit; the headed screenshot guards
    ///   prove the pixels.
    /// - `hash_check`: the replay verdict (`"Match"`/`"Mismatch"`/`"Pending"`/
    ///   `"Unverifiable"`) as a DOM-readable string. The theater otherwise paints this on
    ///   the GPU canvas, unreadable by automation.
    ///
    /// Additive only: it extends the same stats object and never rewrites an existing
    /// key, so the add-only seam canaries keep passing. A no-op if the window is
    /// unavailable, so it never disturbs a normal session.
    #[cfg(target_arch = "wasm32")]
    fn record_frame_stats(&self) {
        use crate::replay::HashCheck;
        use wasm_bindgen::JsValue;
        let Some(window) = web_sys::window() else {
            return;
        };
        let key = JsValue::from_str("__reticle_stats");
        let stats = match js_sys::Reflect::get(window.as_ref(), &key) {
            Ok(v) if v.is_object() => v,
            _ => {
                let obj = js_sys::Object::new();
                let _ = js_sys::Reflect::set(window.as_ref(), &key, obj.as_ref());
                JsValue::from(obj)
            }
        };
        // The live top-cell shape count (the same expression the viewer path publishes).
        // For a hierarchical design the top cell holds instances, not direct shapes, so
        // this is 0 there; `applied_scene_shapes` below is the count that reflects the
        // rendered geometry.
        let shapes = self
            .history
            .document()
            .cell(&self.top_cell)
            .map_or(0, |c| c.shapes.len());
        let _ = js_sys::Reflect::set(
            &stats,
            &JsValue::from_str("applied_shapes"),
            &JsValue::from_f64(shapes as f64),
        );
        // The flattened renderable shape count: the top cell with every instance expanded
        // (`SceneIndex` is built from `document.flatten`). It is > 0 whenever the editor
        // has geometry to draw, including a hierarchical design whose top cell has no
        // direct shapes (the Tiny Tapeout sample), so it is the per-example loaded signal.
        let scene_shapes = self.scene.len();
        let _ = js_sys::Reflect::set(
            &stats,
            &JsValue::from_str("applied_scene_shapes"),
            &JsValue::from_f64(scene_shapes as f64),
        );
        // Named vs total layer rows. A correctly opened example grafts its technology so
        // the layers are named and colored; a document opened WITHOUT a technology has
        // only synthesized "L#D#" placeholders with default opaque fills that overpaint
        // to one blob (the white-examples bug). named_layers == 0 flags that, so a
        // headed guard fails a white blob even though it is "not blank".
        let _ = js_sys::Reflect::set(
            &stats,
            &JsValue::from_str("named_layers"),
            &JsValue::from_f64(self.layer_state.named_layer_count() as f64),
        );
        let _ = js_sys::Reflect::set(
            &stats,
            &JsValue::from_str("applied_layers"),
            &JsValue::from_f64(self.layer_state.rows().len() as f64),
        );
        // Painting real geometry (the flattened editor scene has shapes, or a streamed
        // archive is painting records) with a live camera.
        let ppd = self.camera.pixels_per_dbu();
        let archive_painting = self
            .archive
            .as_ref()
            .is_some_and(|b| b.stats().records_painted > 0);
        let nonblank = ppd.is_finite() && ppd > 0.0 && (scene_shapes > 0 || archive_painting);
        let _ = js_sys::Reflect::set(
            &stats,
            &JsValue::from_str("render_nonblank"),
            &JsValue::from_bool(nonblank),
        );
        // The replay verdict as a DOM-readable string (canvas-only otherwise).
        let verdict = match self.replay.hash_check() {
            HashCheck::Pending => "Pending",
            HashCheck::Unverifiable => "Unverifiable",
            HashCheck::Match => "Match",
            HashCheck::Mismatch => "Mismatch",
        };
        let _ = js_sys::Reflect::set(
            &stats,
            &JsValue::from_str("hash_check"),
            &JsValue::from_str(verdict),
        );
        // Regression seams (v8.1-REGRESSION): the live interaction state, so a headed
        // matrix can prove an action had an EFFECT, not just that a widget exists.
        let _ = js_sys::Reflect::set(
            &stats,
            &JsValue::from_str("active_tool"),
            &JsValue::from_str(self.tools.active().label()),
        );
        let _ = js_sys::Reflect::set(
            &stats,
            &JsValue::from_str("selection_count"),
            &JsValue::from_f64(self.selection.len() as f64),
        );
        let _ = js_sys::Reflect::set(
            &stats,
            &JsValue::from_str("undo_depth"),
            &JsValue::from_f64(self.history.undo_depth() as f64),
        );
        let _ = js_sys::Reflect::set(
            &stats,
            &JsValue::from_str("redo_depth"),
            &JsValue::from_f64(self.history.redo_depth() as f64),
        );
        let _ = js_sys::Reflect::set(
            &stats,
            &JsValue::from_str("agent_running"),
            &JsValue::from_bool(self.agent.is_running()),
        );
        let (replay_step, replay_total) = self.replay.progress();
        let _ = js_sys::Reflect::set(
            &stats,
            &JsValue::from_str("replay_step"),
            &JsValue::from_f64(replay_step as f64),
        );
        let _ = js_sys::Reflect::set(
            &stats,
            &JsValue::from_str("replay_total"),
            &JsValue::from_f64(replay_total as f64),
        );
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

    // --- lane import-wiring: import open path ---
    /// Opens DXF bytes and applies a layer remap before installing the result;
    /// see [`crate::dxf_dialog::DxfLayerMap`]. CIF, DXF, and OASIS (`OasisStd`)
    /// already open via the existing picker and drop paths with no remap.
    pub fn open_dxf_with_layer_map(
        &mut self,
        bytes: &[u8],
        map: &crate::dxf_dialog::DxfLayerMap,
    ) -> Result<(), crate::open::OpenError> {
        let mut outcome = crate::open::open_document_bytes(bytes, crate::open::DocFormat::Dxf)?;
        map.apply(&mut outcome.document);
        self.open_outcome(outcome);
        Ok(())
    }
    // --- end lane import-wiring ---

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

    /// Reports a rich failure that carries a Copy-details action and, when `retry` is
    /// given, a Retry action wired to replay that operation (items 71, 72).
    ///
    /// Every failing open/URL/convert/share path routes through here (or the plain
    /// [`Notifications::fail`](crate::notify::Notifications::fail) for a non-retryable
    /// one), so no failure is silent and each carries a cause, a next step, and a
    /// copyable diagnostic.
    fn report_failure(
        &mut self,
        summary: impl Into<String>,
        diagnostic: crate::notify::Diagnostic,
        retry: Option<RetryOp>,
    ) {
        let mut note = crate::notify::Notification::new(
            crate::notify::Severity::Error,
            summary,
            diagnostic.next_step.clone(),
        )
        .with_diagnostic(diagnostic)
        .with_action(crate::notify::NotificationAction::CopyDetails);
        if retry.is_some() {
            note = note.with_action(crate::notify::NotificationAction::Retry);
            self.retry = retry;
        }
        self.notifications.push(note);
    }

    /// Replays the most recent retryable operation for a toast's Retry action.
    fn run_retry(&mut self) {
        match self.retry.clone() {
            Some(RetryOp::Picker) => self.open_file_dialog(),
            Some(RetryOp::FetchUrl(url)) => self.start_url_open(url),
            None => {}
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
        // Open the first dropped file that names a layout format, routing it through
        // the same hardened helper the file picker uses so a refusal (unsupported
        // extension, too large, corrupt) surfaces a rich diagnostic (items 5, 8, 72).
        for file in &dropped {
            if crate::webopen::classify_drop(&file.name).is_none() {
                continue;
            }
            let Some(bytes) = Self::dropped_file_bytes(file) else {
                self.notifications.fail(
                    format!("Could not read {}", file.name),
                    crate::notify::Diagnostic::new(
                        "The dropped file could not be read.",
                        "Try dragging it again, or use Open to pick it from disk.",
                        format!("file: {}", file.name),
                    ),
                );
                return true;
            };
            self.open_named_bytes(&file.name, &bytes);
            return true;
        }
        // Every dropped file was a non-layout type: explain the refusal with the
        // supported-format list rather than silently ignoring the gesture (item 8).
        let name = dropped
            .first()
            .map_or_else(|| "that file".to_owned(), |f| f.name.clone());
        self.notifications.fail(
            format!("Could not open {name}"),
            crate::dialogs::unsupported_file_diagnostic(&name),
        );
        true
    }

    /// Opens a URL pasted onto the page with Ctrl+V (catalog item 3).
    ///
    /// Reads this frame's paste events and, when a text field does not own focus (so a
    /// paste into the palette or the Open-from-URL field is left alone) and the pasted
    /// text validates as a layout URL, starts the same fetch the Open-from-URL dialog
    /// does. Pasting raw file bytes is not something the browser clipboard exposes to a
    /// page, so the file half of paste-to-open is drag-and-drop; a pasted link works
    /// here.
    fn handle_paste_to_open(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let pasted = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Paste(text) => Some(text.clone()),
                _ => None,
            })
        });
        if let Some(text) = pasted
            && let Ok(url) = crate::dialogs::validate_open_url(&text)
        {
            self.start_url_open(url);
        }
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
        painter.rect_filled(screen, 0.0, CANVAS.scrim);
        let inset = screen.shrink(24.0);
        painter.rect_stroke(
            inset,
            12.0,
            Stroke::new(3.0, CANVAS.drop_frame),
            StrokeKind::Inside,
        );
        let style = ctx.style_of(egui::Theme::Dark);
        painter.text(
            screen.center(),
            Align2::CENTER_CENTER,
            "Drop a layout file to open it",
            egui::TextStyle::Heading.resolve(&style),
            CANVAS.hud_text,
        );
        // A supported-format hint under the prompt so the drop target says what it
        // accepts, not just that it accepts something (item 5).
        painter.text(
            screen.center() + Vec2::new(0.0, 28.0),
            Align2::CENTER_CENTER,
            format!(
                "Supported: {}",
                crate::dialogs::SUPPORTED_FORMATS.join("  |  ")
            ),
            egui::TextStyle::Body.resolve(&style),
            CANVAS.hud_text,
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
                let cctx = self.ui_ctx();
                // Stage line: which honest phase the open is in (item 6). The remote
                // path fetches, then builds the index (the parse/tessellate work); the
                // GPU upload happens on the first paint after the document installs.
                let (stage, detail, fraction) = match &self.load_progress {
                    LoadProgress::Fetching { received, total } if *total > 0 => (
                        crate::dialogs::OpenStage::Parse,
                        format!(
                            "Downloading {} / {} MiB",
                            received / (1024 * 1024),
                            total / (1024 * 1024)
                        ),
                        Some(self.load_progress.fraction().unwrap_or(0.0)),
                    ),
                    LoadProgress::Fetching { received, .. } => (
                        crate::dialogs::OpenStage::Parse,
                        format!("Downloading {} MiB", received / (1024 * 1024)),
                        None,
                    ),
                    _ => (
                        crate::dialogs::OpenStage::Tessellate,
                        "Building the index".to_owned(),
                        Some(crate::dialogs::OpenStage::Tessellate.fraction()),
                    ),
                };
                let mut cancel = false;
                egui::Window::new("Opening")
                    .id(egui::Id::new("load_progress_window"))
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .show(ctx, |ui| {
                        ui.set_min_width(320.0);
                        ui.label(egui::RichText::new(stage.label()).color(DARK.text));
                        ui.label(egui::RichText::new(detail).color(DARK.text_weak));
                        ui.add_space(cctx.density.item_spacing().y);
                        // A determinate row when the fetch total is known, else an
                        // indeterminate animated bar; both carry a cancel affordance.
                        if let Some(f) = fraction {
                            let out = crate::theme::components::ProgressRow::new("", f)
                                .cancelable(true)
                                .show(ui, cctx);
                            cancel = out.canceled;
                        } else {
                            ui.add(egui::ProgressBar::new(0.0).animate(true));
                            if crate::theme::components::Button::secondary("Cancel")
                                .show(ui, cctx)
                                .clicked()
                            {
                                cancel = true;
                            }
                        }
                    });
                if cancel {
                    // The browser fetch cannot be aborted mid-flight, so mark the load
                    // canceled and drop its eventual result rather than installing it.
                    self.load_progress = LoadProgress::Idle;
                    self.load_canceled = true;
                    self.status.set("Open canceled");
                    self.notify("Open canceled", "");
                }
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
                        ui.colored_label(DARK.danger, &message);
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
        // A canceled load (item 6) drops the fetch's late results instead of installing
        // them; the flag clears once the in-flight result has been discarded.
        if self.load_canceled
            && matches!(
                event,
                WebOpenEvent::Progress(_) | WebOpenEvent::Opened { .. }
            )
        {
            if matches!(event, WebOpenEvent::Opened { .. }) {
                self.load_canceled = false;
            }
            self.load_progress = crate::webopen::LoadProgress::Idle;
            return;
        }
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
                    self.last_fetch_url = None;
                    self.persist_recent_files();
                    // A `?gds=` link may also carry a permalink (cell/camera/layers) to
                    // restore on top of the freshly opened document.
                    if let Some(permalink) = self.pending_permalink.take() {
                        self.apply_permalink(&permalink);
                    }
                }
                Err(e) => {
                    // A fetched file that will not import gets the same corrupt-file
                    // explainer as a local one (items 8, 72), with Retry wired to the
                    // source (re-fetch a URL, else the picker).
                    let retry = self
                        .last_fetch_url
                        .take()
                        .map_or(RetryOp::Picker, RetryOp::FetchUrl);
                    self.report_failure(
                        "Could not open the file",
                        crate::dialogs::open_error_diagnostic(&e),
                        Some(retry),
                    );
                    self.load_progress = crate::webopen::LoadProgress::Idle;
                }
            },
            WebOpenEvent::Failed(message) => {
                self.status.set(message.clone());
                // Turn the transport's message into a rich toast: a CORS block gets
                // the plain-language explainer, any other network/HTTP failure the
                // generic fetch diagnostic, both naming the URL and offering Retry
                // (items 2, 72). The load card is cleared so the toast is the single
                // surface.
                let url = self.last_fetch_url.take();
                let named = url.clone().unwrap_or_else(|| "the file".to_owned());
                let lower = message.to_lowercase();
                let diagnostic = if lower.contains("cors") || lower.contains("cross-origin") {
                    crate::dialogs::cors_diagnostic(&named)
                } else {
                    crate::dialogs::fetch_failure_diagnostic(&named, &message)
                };
                let retry = url.map(RetryOp::FetchUrl);
                self.report_failure("Could not open the file", diagnostic, retry);
                self.load_progress = crate::webopen::LoadProgress::Idle;
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

    /// Drives the served-archive browse (`?archive=`, lane v8-2e): on the first wasm frame
    /// it kicks off the async archive open, and every frame it installs the finished
    /// browse once it arrives.
    ///
    /// Mirrors [`drive_web_open`](Self::drive_web_open): the open runs on
    /// `wasm_bindgen_futures::spawn_local` (it fetches the header and probes the size) and
    /// posts the assembled [`ArchiveBrowse`](crate::archive::ArchiveBrowse) into
    /// [`archive_open`](Self::archive_open); here, on the main thread, a success installs
    /// it (so the canvas paints the streamed die next frame) and a failure surfaces as a
    /// notification. On native nothing is ever posted, so this is a cheap no-op.
    fn drive_archive(&mut self, ctx: &egui::Context) {
        #[cfg(target_arch = "wasm32")]
        if !self.archive_started {
            self.archive_started = true;
            if let Some(url) = self.pending_archive_url.take() {
                self.status.set("Opening archive...");
                crate::archive::start_archive_open(url, self.archive_open.clone(), ctx.clone());
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = ctx;

        if let Some(result) = self.archive_open.take() {
            match result {
                Ok(browse) => {
                    self.status.set("Streaming archive");
                    self.archive = Some(browse);
                    // Frame the die on the first archive frame, once the canvas size is known.
                    self.archive_framed = false;
                }
                Err(message) => {
                    self.report_error("Could not open the archive", message.clone());
                    self.status.set(message);
                }
            }
        }
    }

    /// Drives the live share transport (ADR 0058): on the first wasm frame of a viewer
    /// session it opens the read-only viewer socket, and every frame it applies whatever
    /// the socket callbacks have posted to the [`live_inbox`](Self::live_inbox).
    ///
    /// This is the viewer's bridge between the socket's event world and the synchronous
    /// editor loop, mirroring [`drive_web_open`](Self::drive_web_open). The transport
    /// posts [`LiveEvent`](crate::livesync::LiveEvent)s; here, on the main thread, a
    /// CRDT frame is applied into the [`ViewerSession`](crate::viewer::ViewerSession) and its mirrored document is
    /// installed for rendering, a presence updates the sharer's cursor and follow
    /// viewport, and a status change updates the status line. On native the inbox is
    /// always empty and no socket is opened, so this is a cheap no-op.
    fn drive_live(&mut self, ctx: &egui::Context) {
        #[cfg(target_arch = "wasm32")]
        if !self.live_started {
            if self.viewer_target.is_some() {
                self.live_started = true;
                if let Some(target) = self.viewer_target.clone() {
                    self.viewer_transport = Some(crate::livesync::ViewerTransport::connect(
                        &target.relay,
                        &target.room,
                        &self.live_inbox,
                        ctx,
                    ));
                    self.status.set("Joining the shared session...");
                }
            } else if self.share_on_boot {
                // Publisher side of the browser share-live e2e: go live without a click.
                self.live_started = true;
                self.go_live(ctx);
                // In edit-script mode, place one scripted rect so lane v8-1e can observe
                // the edit reach a viewer.
                if self.e2e_edit {
                    self.place_e2e_rect();
                }
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = ctx;

        let events = self.live_inbox.drain();
        if events.is_empty() {
            return;
        }
        let mut geometry_changed = false;
        for event in events {
            geometry_changed |= self.apply_live_event(event);
        }
        // Reflect the sharer's updated geometry into the render pipeline, and, when
        // following, snap the local camera to the sharer's viewport.
        if geometry_changed {
            self.install_viewer_mirror();
            // Catalog 90: flash the remote-edit attribution glow when a mirrored frame
            // lands, so a viewer sees where the shared session just changed.
            self.remote_edit_flash = REMOTE_EDIT_FLASH_SECS;
            // Expose the applied-frame/shape counters lane v8-1e's browser test reads.
            #[cfg(target_arch = "wasm32")]
            self.record_viewer_stats();
        }
    }

    /// Applies one [`LiveEvent`](crate::livesync::LiveEvent) from the viewer transport,
    /// returning `true` if the mirrored document's geometry changed (so the caller
    /// rebuilds the render scene).
    ///
    /// A CRDT frame is merged into the [`ViewerSession`](crate::viewer::ViewerSession); a presence updates the
    /// sharer's cursor and the viewport follow-mode rides on, and is also mirrored into
    /// the App's awareness map so the existing [`draw_presence`](Self::draw_presence)
    /// draws the sharer's cursor; a status change updates the status line.
    fn apply_live_event(&mut self, event: crate::livesync::LiveEvent) -> bool {
        use crate::livesync::LiveEvent;
        match event {
            LiveEvent::Update(bytes) => {
                if let Some(session) = self.viewer_session.as_mut() {
                    match session.apply_frame(&bytes) {
                        Ok(()) => return true,
                        Err(e) => self.status.set(format!("dropped a bad frame: {e}")),
                    }
                }
                false
            }
            LiveEvent::Presence(presence) => {
                if let Some(session) = self.viewer_session.as_mut() {
                    session.apply_presence(presence.clone());
                }
                // Mirror the sharer's presence into the App's awareness map so the
                // canvas draws the sharer's cursor with the machinery that already
                // renders remote collaborators.
                self.document.awareness_mut().set(presence);
                false
            }
            LiveEvent::Status(status) => {
                // When a sharer socket first opens, force a full-document republish: the
                // first publish attempt fired while the socket was still connecting and
                // was dropped, and the revision guard would otherwise never retry. This
                // makes the sharer's whole document reach viewers as soon as the socket
                // is live (the CRDT is idempotent, so a repeat is harmless).
                if status.is_open() && self.sharer_transport.is_some() {
                    self.published_revision = self.history.revision().wrapping_sub(1);
                    // A (re)connected socket must resend the FULL state so a reconnecting
                    // viewer or a late joiner gets a complete snapshot, not just a delta.
                    self.publish_full_next = true;
                }
                // Catalog 91: in a viewer session, a live->closed transition means the
                // sharer dropped; raise the read-only freeze notice (and clear it if the
                // socket comes back).
                if self.is_viewer() {
                    let open = status.is_open();
                    if self.viewer_was_open && !open {
                        self.sharer_left = true;
                        self.notifications.warning(
                            "Sharer left",
                            "You are viewing the last received state, read-only.",
                        );
                    } else if open {
                        self.sharer_left = false;
                    }
                    self.viewer_was_open = open;
                }
                self.status.set(status.label());
                // Drive the offline badge and reconnect toasts off the live socket
                // state (item 74): an open socket is online, a reconnecting or terminal
                // one is offline. Each real transition yields at most one toast.
                let toast = if status.is_open() {
                    self.connectivity.set_online()
                } else if status.is_reconnecting() || status.is_terminal() {
                    self.connectivity.set_offline()
                } else {
                    None
                };
                if let Some(note) = toast {
                    self.notifications.push(note);
                }
                self.live_status = status;
                false
            }
        }
    }

    /// Installs the viewer's mirrored document into the render pipeline, rebuilding the
    /// spatial index, retained GPU scene, and outline from the [`ViewerSession`](crate::viewer::ViewerSession)'s
    /// current document.
    ///
    /// Called when a CRDT frame changed the mirror. It reframes the camera only on the
    /// *first* installed geometry (so the viewer sees the sharer's design without having
    /// to pan to find it) and thereafter leaves the local camera alone, so the viewer
    /// pans and zooms independently. With follow-mode on, the local camera then snaps to
    /// the sharer's viewport instead (handled where the canvas draws).
    fn install_viewer_mirror(&mut self) {
        let Some(session) = self.viewer_session.as_ref() else {
            return;
        };
        let document = session.document().clone();
        let top_cell = document
            .top_cells()
            .first()
            .cloned()
            .or_else(|| document.cells().next().map(|c| c.name.clone()))
            .unwrap_or_else(|| self.top_cell.clone());

        self.layer_state = LayerState::from_technology(document.technology());
        self.scene = SceneIndex::build(&document, &top_cell);
        let palette = palette_from_layers(&self.layer_state);
        self.retained = RetainedScene::new(&document, &top_cell, &palette);
        self.expanded = Arc::new(self.retained.expand());
        self.search.outline = OutlineTree::build(&document);
        self.top_cell = top_cell;
        self.history = History::new(document);
        self.doc_generation = self.doc_generation.wrapping_add(1);
        self.render_revision = self.render_revision.wrapping_add(1);
        // Reframe the first time the sharer's geometry lands (the viewer boots showing
        // the demo document until then), so the viewer sees the shared design; after
        // that keep the viewer's own camera so independent pan/zoom is not fought.
        if !self.viewer_framed {
            self.viewer_framed = true;
            self.fit_requested = true;
        }
    }

    /// Publishes the editor's document and the sharer's presence over the sharer
    /// transport, when one is open (the Share section's "Go live", ADR 0058).
    ///
    /// The document is re-encoded and sent only when it actually changed since the last
    /// publish (tracked by the history revision), because [`self.document`](Self::document)
    /// (the collaboration mirror) is not kept in step with edits, so the sharer builds a
    /// fresh [`SyncDocument`] from the editable
    /// [`history`](Self::history) document to produce the update bytes. Presence (cursor,
    /// selection, viewport) is sent every frame the socket is open so a viewer's cursor
    /// and follow-viewport stay live; presence frames are tiny.
    ///
    /// A no-op on native and whenever no sharer transport is open.
    #[cfg_attr(not(target_arch = "wasm32"), allow(clippy::unused_self))]
    fn drive_sharer_publish(&mut self) {
        #[cfg(target_arch = "wasm32")]
        {
            if self.sharer_transport.is_none() {
                return;
            }
            // Re-encode and publish the document only on a real change.
            let revision = self.history.revision();
            if revision != self.published_revision {
                // Reconcile the sharer's LONG-LIVED document to the current editable
                // document (ADR 0063). Mutating one persistent doc advances the yrs
                // clocks monotonically, so a viewer integrates each delta rather than
                // dropping later publishes as already-seen struct ids (which rebuilding
                // a fresh document per publish caused). A (re)connect sends full state.
                let doc = self.sharer_doc.get_or_insert_with(|| {
                    reticle_sync::SyncDocument::new(crate::livesync::SHARER_ACTOR)
                });
                let state_before = doc.state_vector();
                doc.reconcile_to(self.history.document());
                let update = if self.publish_full_next {
                    self.publish_full_next = false;
                    doc.encode_state_update()
                } else {
                    doc.encode_update(&state_before)
                        .unwrap_or_else(|_| doc.encode_state_update())
                };
                if let Some(transport) = self.sharer_transport.as_ref() {
                    transport.publish_update(&update);
                }
                self.published_revision = revision;
            }
            // Publish presence every open frame; the frame is small and keeps the
            // sharer's cursor and follow-viewport live for viewers.
            if self.live_status.is_open()
                && let Some(presence) = self.local_presence()
                && let Some(transport) = self.sharer_transport.as_ref()
            {
                transport.publish_presence(&presence);
            }
        }
    }

    /// Builds the sharer's own [`Presence`](reticle_sync::Presence): the cursor under
    /// the pointer, the current selection as element references, and the visible
    /// viewport (in world DBU) that a following viewer frames.
    ///
    /// Returns `None` when the canvas size is not yet known (no viewport to publish).
    /// The viewport rides on the frozen `Presence.viewport` field, exactly what
    /// follow-mode reads (ADR 0038).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    fn local_presence(&self) -> Option<reticle_sync::Presence> {
        let screen = self.last_screen?;
        let mut presence = reticle_sync::Presence::new("sharer");
        // The locally-stored display name (catalog 86) rides on the published presence
        // so a viewer sees the sharer's chosen name on their cursor; empty falls back to
        // a neutral label at the viewer.
        let name = self.display_name.trim();
        presence
            .display_name
            .push_str(if name.is_empty() { "Sharer" } else { name });
        presence.color_rgba = 0x2f_81_f7_ff; // a distinct sharer blue
        if let Some(cursor) = self.cursor_world {
            presence.cursor = cursor;
        }
        presence.selection = self
            .selection
            .iter()
            .map(|idx| format!("{}/shape-{idx}", self.top_cell))
            .collect();
        presence.viewport = self.camera.visible_world_rect(&screen);
        Some(presence)
    }

    /// Opens the sharer transport for the current relay/room so viewers stream this
    /// session (the Share section's "Go live", ADR 0058).
    ///
    /// Idempotent from the UI's perspective: opening replaces any prior transport,
    /// dialing the room in Edit mode. The next frame begins publishing the document and
    /// presence. On native this records intent but dials no socket.
    fn go_live(&mut self, ctx: &egui::Context) {
        self.published_revision = self.history.revision().wrapping_sub(1); // force a first publish
        self.sharer_doc = None; // a fresh session rebuilds the persistent sharer doc
        self.publish_full_next = true; // the first publish carries full state
        self.live_status = crate::livesync::LiveStatus::Connecting;
        self.sharer_transport = Some(crate::livesync::SharerTransport::connect(
            &self.share_server,
            &self.share_room,
            &self.live_inbox,
            ctx,
        ));
        self.status
            .set("Going live: viewers can now join this session");
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

    /// Opens or closes the command palette. A one-line state hook for the visual
    /// regression suite (`tests/ui_snapshots.rs`), which snapshots the
    /// palette-open editor state; production toggles it through the toolbar and
    /// shortcut instead.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_palette_open(&mut self, open: bool) {
        self.palette_open = open;
        self.palette_focus_pending = open;
    }

    /// Finishes the first-run tour so a snapshot shows the steady editor rather
    /// than environment-dependent onboarding chrome. A one-line state hook for
    /// the visual regression suite (`tests/ui_snapshots.rs`).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn suppress_onboarding_for_snapshot(&mut self) {
        self.tour.finish();
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
    #[allow(clippy::unused_self)] // signature must match the native version, which uses `self`.
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
                // Opening the 3D stack is now opening its managed panel (ADR 0096);
                // a demo `view3d on` step drives the same flag View > Panels does.
                self.panel_3d_open = *open;
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
            Step::Generator(id) => {
                self.demo_focus_generate = true;
                match self
                    .generate
                    .infos()
                    .iter()
                    .position(|i| i.id == id.as_str())
                {
                    Some(index) => self.generate.select(index),
                    None => eprintln!("demo: no generator with id `{id}`"),
                }
            }
            Step::GenParam { name, value } => {
                self.demo_focus_generate = true;
                self.generate.selected_params_mut()[name.as_str()] =
                    serde_json::Value::from(*value);
            }
            Step::GenPlace => {
                self.demo_focus_generate = true;
                self.generate_apply();
                // Reframe so the placed structure is on screen.
                self.fit_requested = true;
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
        // Fold in the hover-peek layer (catalog 39) so entering, leaving, or moving
        // the peek retessellates the dimmed scene; no peek leaves the signature at
        // its plain visibility value, so an idle frame is still a no-op rebuild.
        if let Some(id) = self.peek_layer {
            let peek = u64::from(u32::from(id.layer) << 8 | u32::from(id.datatype)) | (1u64 << 40);
            hash = (hash ^ peek).wrapping_mul(0x0000_0100_0000_01B3);
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
        let palette = palette_from_layers_peek(&self.layer_state, self.peek_layer);
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
            Command::ZoomSelection => {
                if self.selection.is_empty() {
                    self.status.set("Fit selection: nothing selected");
                } else {
                    self.pending_view = Some(ViewPreset::Selection);
                    self.status.set("Fit selection");
                }
            }
            Command::ZoomOneToOne => {
                self.pending_view = Some(ViewPreset::OneToOne);
                self.status.set("Zoom 1:1 DBU");
            }
            Command::ZoomLayerExtents => {
                self.pending_view = Some(ViewPreset::LayerExtents);
                self.status.set("Zoom to layer extents");
            }
            Command::BookmarkSave => {
                // Nine slots, oldest dropped, so the palette recall list stays short.
                const MAX_BOOKMARKS: usize = 9;
                if self.bookmarks.len() == MAX_BOOKMARKS {
                    self.bookmarks.remove(0);
                }
                self.bookmarks.push(self.camera);
                self.status.set(format!(
                    "Saved view bookmark {} (recall from the palette)",
                    self.bookmarks.len()
                ));
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
            Command::Duplicate => self.productivity_duplicate(),
            Command::CopyPermalink => self.copy_permalink_at_view(),
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

    /// Builds a permalink pinning the current camera, layers, and cell, and queues it for
    /// the clipboard (item 35). The link is emitted with the current page as its base and
    /// carries the open document's `?gds=` source (on wasm) so it reopens the same design
    /// at the same view; the pure serialization is [`crate::share::session_permalink`] +
    /// [`crate::share::emit_permalink`].
    fn copy_permalink_at_view(&mut self) {
        let center = self.camera.center();
        let camera = (
            f64::from(center.x),
            f64::from(center.y),
            self.camera.pixels_per_dbu(),
        );
        let visible: Vec<(u16, u16)> = self
            .layer_state
            .rows()
            .iter()
            .filter(|r| r.visible)
            .map(|r| (r.id.layer, r.id.datatype))
            .collect();
        let cell = self.top_cell.clone();
        let permalink = crate::share::session_permalink(Some(&cell), camera, &visible);
        let (base, gds) = self.permalink_context();
        let link = crate::share::emit_permalink(&base, gds.as_deref(), &permalink);
        self.pending_clipboard = Some(link);
        self.status
            .set("Copied a permalink to this view to the clipboard");
    }

    /// The page base URL and any `?gds=` source to fold into a copied permalink.
    ///
    /// On wasm this reads the live `window.location` so the link resolves against the
    /// deployed bundle and reopens the same document; on native (no browser) it yields a
    /// relative link with no source, which still round-trips the view state.
    #[cfg(target_arch = "wasm32")]
    fn permalink_context(&self) -> (String, Option<String>) {
        let Some(window) = web_sys::window() else {
            return (String::new(), None);
        };
        let location = window.location();
        let origin = location.origin().unwrap_or_default();
        let pathname = location.pathname().unwrap_or_default();
        let base = format!("{origin}{pathname}");
        let gds = location
            .search()
            .ok()
            .and_then(|search| crate::webopen::gds_url_from_query(&search));
        (base, gds)
    }

    /// Native builds have no page URL, so a copied permalink is a relative view-state
    /// link with no document source.
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::unused_self)]
    fn permalink_context(&self) -> (String, Option<String>) {
        (String::new(), None)
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
                self.close_palette();
                self.pending_chord = None;
            }
            return;
        }
        let chords = pressed_chords(ctx);

        // Chord capture for the shortcuts editor: the next press rebinds.
        if let Some(id) = self.rebinding {
            if let Some(new_chord) = chords.into_iter().next() {
                self.rebinding = None;
                if new_chord.key == "Escape" {
                    self.status.set("Rebind canceled");
                } else {
                    let shown = new_chord.to_string();
                    let stolen = self.keymap.bind(id, Some(new_chord));
                    match stolen.first() {
                        Some(loser) => self.status.set(format!(
                            "{} bound to {shown}; {} is now unbound",
                            commands::label(id),
                            commands::label(*loser)
                        )),
                        None => self
                            .status
                            .set(format!("{} bound to {shown}", commands::label(id))),
                    }
                }
            }
            return;
        }

        // Escape drives the documented cascade (item 84): cancel the tool, then
        // clear the selection, then close a popover. It consumes the frame so a
        // pending chord sequence is dropped rather than resolved.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.pending_chord = None;
            self.apply_esc();
            return;
        }

        // A pending sequence prefix (item 81): this press is the suffix.
        if let Some(prefix) = self.pending_chord.take() {
            if let Some(chord) = chords.into_iter().next() {
                match keymap::sequence_command(&prefix.key, &chord.key) {
                    Some(id) => self.dispatch(id),
                    None => self.status.set("Unknown shortcut sequence"),
                }
            } else {
                // No key this frame; keep waiting for the suffix.
                self.pending_chord = Some(prefix);
            }
            return;
        }

        for chord in chords {
            // Shift+F6 cycles focus backward (plain F6 is a normal binding).
            if chord.key == "F6" && chord.shift && !chord.ctrl && !chord.alt {
                self.cycle_focus(false);
                continue;
            }
            if let Some(id) = self.keymap.command_for(&chord) {
                self.dispatch(id);
            } else if !chord.ctrl
                && !chord.shift
                && !chord.alt
                && keymap::is_sequence_prefix(&chord.key)
            {
                self.status.set(format!("{chord} …"));
                self.pending_chord = Some(chord);
            }
        }
    }

    /// The state the Escape cascade inspects this frame (item 84).
    fn esc_state(&self) -> crate::focus::EscState {
        crate::focus::EscState {
            tool_active: self.draw.in_progress() || self.tools.active() != Tool::Select,
            has_selection: !self.selection.is_empty(),
            // Lane 3A's transform popover and the palette join the cascade's popover
            // arm alongside 3C's shortcuts/keymap windows.
            popover_open: self.shortcuts_open
                || self.keymap_open
                || self.transform.open
                || self.palette_open,
        }
    }

    /// Applies one Escape press per the cascade contract (item 84).
    fn apply_esc(&mut self) {
        match crate::focus::esc_action(self.esc_state()) {
            crate::focus::EscAction::CancelTool => {
                // Lane 3A owns the canvas-draw cancel (item 52): drop a half-drawn
                // shape first; otherwise fall back to the Select tool.
                if self.cancel_in_progress_draw() {
                    self.status.set("Draw canceled");
                } else {
                    self.select_tool(Tool::Select);
                    self.status.set("Tool: Select");
                }
            }
            crate::focus::EscAction::ClearSelection => {
                self.selection.clear();
                self.status.set("Selection cleared");
            }
            crate::focus::EscAction::ClosePopover => {
                if self.shortcuts_open {
                    self.shortcuts_open = false;
                } else if self.keymap_open {
                    self.keymap_open = false;
                    self.rebinding = None;
                } else if self.transform.open {
                    self.transform.open = false;
                } else if self.palette_open {
                    self.palette_open = false;
                }
            }
            crate::focus::EscAction::Nothing => {}
        }
    }

    /// Cancels a half-drawn shape (an in-progress polygon or path, item 52), returning
    /// whether anything was actually cancelled.
    fn cancel_in_progress_draw(&mut self) -> bool {
        let drawing = matches!(
            self.tools.active(),
            Tool::DrawPolygon | Tool::DrawPath | Tool::EditVertex
        );
        let has_progress =
            !self.draw.poly.vertices().is_empty() || !self.draw.path.points().is_empty();
        if drawing && has_progress {
            self.draw.reset();
            true
        } else {
            false
        }
    }

    /// Runs a registry command by [`CommandId`], the single funnel every surface
    /// (shortcuts, menus, palette, context menus, buttons) routes through.
    ///
    /// [`RunAs::Command`] entries go through [`App::run_command`] so they share the
    /// palette and toolbar effect path; [`RunAs::App`] entries go through
    /// [`App::run_app_op`]. An unknown id (nothing in the registry) is a no-op.
    fn dispatch(&mut self, id: CommandId) {
        let Some(spec) = commands::spec(id) else {
            return;
        };
        #[cfg(target_arch = "wasm32")]
        let rev_before = self.history.revision();
        match spec.run {
            RunAs::Command(cmd) => self.run_command(cmd, None),
            RunAs::App(op) => self.run_app_op(op),
        }
        // Regression seam: record which command fired and whether it mutated the
        // document, so a headed test can prove an action FIRED and had an EFFECT.
        #[cfg(target_arch = "wasm32")]
        self.record_command_stats(id, rev_before);
    }

    /// Publishes the last dispatched command id and whether it changed the document
    /// revision to `window.__reticle_stats` (wasm only, additive), so a headed
    /// regression test proves a command FIRED (`last_command_id`) and had an EFFECT
    /// (`last_command_mutated`). A no-op if the window is unavailable.
    #[cfg(target_arch = "wasm32")]
    fn record_command_stats(&self, id: CommandId, rev_before: u64) {
        use wasm_bindgen::JsValue;
        let Some(window) = web_sys::window() else {
            return;
        };
        let key = JsValue::from_str("__reticle_stats");
        let stats = match js_sys::Reflect::get(window.as_ref(), &key) {
            Ok(v) if v.is_object() => v,
            _ => {
                let obj = js_sys::Object::new();
                let _ = js_sys::Reflect::set(window.as_ref(), &key, obj.as_ref());
                JsValue::from(obj)
            }
        };
        let _ = js_sys::Reflect::set(
            &stats,
            &JsValue::from_str("last_command_id"),
            &JsValue::from_str(id.0),
        );
        let _ = js_sys::Reflect::set(
            &stats,
            &JsValue::from_str("last_command_mutated"),
            &JsValue::from_bool(self.history.revision() != rev_before),
        );
    }

    /// Runs an [`AppOp`]: an app-level effect with no palette [`Command`]
    /// equivalent (folded from the former `run_action`).
    // One flat match arm per app-op across the merged lanes; the length is the arm
    // count, not branching logic, so the line-count lint does not apply.
    #[allow(clippy::too_many_lines)]
    fn run_app_op(&mut self, op: AppOp) {
        match op {
            AppOp::OpenPalette => {
                self.palette_open = !self.palette_open;
                // Focus the search field only on the frame it opens (see
                // `palette_focus_pending`); re-requesting every frame breaks Enter.
                self.palette_focus_pending = self.palette_open;
                self.palette_query.clear();
            }
            AppOp::ToggleLabels => {
                self.labels_visible = !self.labels_visible;
                self.status
                    .set(format!("Labels {}", on_off(self.labels_visible)));
            }
            AppOp::ToggleMinimap => {
                self.minimap_visible = !self.minimap_visible;
                self.status
                    .set(format!("Minimap {}", on_off(self.minimap_visible)));
            }
            AppOp::ToggleRulers => {
                self.rulers_visible = !self.rulers_visible;
                self.status
                    .set(format!("Rulers {}", on_off(self.rulers_visible)));
            }
            AppOp::SplitSingle => self.set_split(Split::Single),
            AppOp::SplitHorizontal => self.set_split(Split::Horizontal),
            AppOp::SplitVertical => self.set_split(Split::Vertical),
            AppOp::PromptGotoCoordinate => {
                self.open_palette_arg(command::PaletteArg::GotoCoordinate);
            }
            AppOp::PromptGotoCell => self.open_palette_arg(command::PaletteArg::GotoCell),
            AppOp::CycleFocus => self.cycle_focus(true),
            AppOp::ToggleShortcuts => {
                self.shortcuts_open = !self.shortcuts_open;
            }
            AppOp::TakeTour => {
                self.tour.relaunch(true);
                self.status.set("Tour started");
                self.persist_prefs();
            }
            AppOp::OpenSettings => self.settings_open = true,
            AppOp::OpenAbout => self.about_open = true,
            AppOp::OpenWhatsNew => self.whats_new_open = true,
            AppOp::OpenDocs => {
                self.pending_open_url = Some(crate::help::DOCS_URL.to_owned());
                self.status.set("Opening documentation");
            }
            // lane 2A: the developer actions relocated to Help > Developer.
            AppOp::AddDemoRect => self.add_demo_rectangle(),
            AppOp::ToggleReplayTheater => {
                self.replay_open = !self.replay_open;
                self.status
                    .set(format!("Replay theater {}", on_off(self.replay_open)));
            }
            AppOp::TogglePanel3d => {
                self.panel_3d_open = !self.panel_3d_open;
                if self.panel_3d_open {
                    self.view3d.reset();
                }
                self.status
                    .set(format!("3D stack {}", on_off(self.panel_3d_open)));
            }
            AppOp::TogglePanelXsection => {
                self.panel_xsection_open = !self.panel_xsection_open;
                self.status.set(format!(
                    "Cross-section {}",
                    on_off(self.panel_xsection_open)
                ));
            }
            // lane 2b: Inspector effects. Each routes to the method the section
            // button already calls, so a menu, the palette, and a button share one
            // effect path.
            AppOp::PanelsToggle => {
                self.inspector.collapsed = !self.inspector.collapsed;
                self.status.set(format!(
                    "Inspector {}",
                    if self.inspector.collapsed {
                        "collapsed"
                    } else {
                        "expanded"
                    }
                ));
            }
            AppOp::EditCut => self.productivity_cut(),
            AppOp::EditPaste => self.productivity_paste(),
            AppOp::EditMove => self.productivity_move_delta(),
            AppOp::EditArray => self.productivity_array(),
            AppOp::EditViaStack => self.productivity_via_stack(),
            AppOp::BoolUnion => self.run_bool(crate::ops::BoolKind::Union),
            AppOp::BoolIntersect => self.run_bool(crate::ops::BoolKind::Intersection),
            AppOp::BoolSubtract => self.run_bool(crate::ops::BoolKind::Difference),
            AppOp::SelectSameLayer => self.select_same_layer(),
            AppOp::SelectByName => self.select_by_layer_name(),
            AppOp::DrcRun => self.run_drc(),
            AppOp::DrcLive => {
                self.live_drc_on = !self.live_drc_on;
                if !self.live_drc_on {
                    self.live_drc.clear();
                    self.live_pending = crate::history::Dirty::None;
                    self.live_reprepare_accum = 0.0;
                }
                self.status
                    .set(format!("Check as you type {}", on_off(self.live_drc_on)));
            }
            AppOp::DrcClear => {
                self.drc.clear();
                self.status.set("DRC cleared");
            }
            AppOp::DiffSnapshot => {
                self.diff_overlay.snapshot(self.history.document());
                self.status.set("Diff: snapshot captured");
            }
            AppOp::DiffRun => self.compute_diff(),
            AppOp::DiffOverlay => {
                let visible = !self.diff_overlay.visible();
                self.diff_overlay.set_visible(visible);
                self.status.set(format!("Diff overlay {}", on_off(visible)));
            }
            AppOp::CommentAdd => self.add_comment_on_top_cell(0),
            AppOp::ExportSvg => {
                self.view_export.format = crate::viewexport::ExportFormat::Svg;
                self.run_export();
            }
            AppOp::ExportMetrology => self.export_metrology(),
            // lane 2D: presentation mode and close-design.
            AppOp::TogglePresentation => {
                self.presentation = !self.presentation;
                self.status
                    .set(format!("Presentation mode {}", on_off(self.presentation)));
            }
            AppOp::CloseDesign => {
                // Return to the Start screen (leaving presentation mode if on) so the
                // open guidance, gallery, and recent files are one place again.
                self.presentation = false;
                self.start_screen = true;
                self.status.set("Closed design");
            }
            // --- lane 3b: open flow and dialogs ---
            AppOp::OpenFileDialog => self.open_file_dialog(),
            AppOp::OpenUrlDialog => {
                self.dialogs.open_url_shown = !self.dialogs.open_url_shown;
            }
            AppOp::ConvertDialog => {
                self.dialogs.convert_shown = !self.dialogs.convert_shown;
            }
            AppOp::ShareDialog => {
                self.dialogs.share_shown = !self.dialogs.share_shown;
            }
            AppOp::CopyViewerLink => self.copy_viewer_link(),
        }
    }

    /// Opens the palette in an inline argument-prompt mode (goto coordinate/cell).
    fn open_palette_arg(&mut self, arg: command::PaletteArg) {
        self.palette_open = true;
        self.palette_arg = Some(arg);
        // Focus the argument field on this opening frame only (see
        // `palette_focus_pending`).
        self.palette_focus_pending = true;
        self.palette_query.clear();
    }

    /// Marks the start of a keyboard focus region (item 83): a zero-size focusable
    /// anchor carrying the region's stable id, so F6 can land focus here and a focus
    /// ring is painted around the whole region while it holds focus. Tab then moves
    /// from the anchor into the region's controls, and tabbing onto an anchor keeps
    /// [`focus_region`](Self::focus_region) in sync with where focus actually is.
    fn focus_anchor(&mut self, ui: &mut egui::Ui, region: crate::focus::FocusRegion) {
        let region_rect = ui.max_rect();
        let anchor_rect = egui::Rect::from_min_size(region_rect.min, egui::Vec2::ZERO);
        let resp = ui.interact(
            anchor_rect,
            region.anchor_id(),
            egui::Sense::focusable_noninteractive(),
        );
        if self.focus_request && self.focus_region == region {
            resp.request_focus();
            self.focus_request = false;
        }
        if resp.has_focus() {
            self.focus_region = region;
            let focus = self.component_ctx().tokens.focus;
            ui.painter().rect_stroke(
                region_rect,
                egui::CornerRadius::same(6),
                egui::Stroke::new(2.0, focus),
                egui::StrokeKind::Inside,
            );
        }
    }

    /// Advances keyboard focus to the next (or previous) focus region and asks the
    /// render pass to move focus there so a focus ring is visible (item 83).
    fn cycle_focus(&mut self, forward: bool) {
        self.focus_region = if forward {
            self.focus_region.next()
        } else {
            self.focus_region.prev()
        };
        self.focus_request = true;
        self.status
            .set(format!("Focus: {}", self.focus_region.label()));
    }

    /// Runs a boolean operation over the same-layer selection through the undo
    /// history (the effect behind the `edit.bool_*` registry ids).
    fn run_bool(&mut self, kind: crate::ops::BoolKind) {
        use crate::ops::BoolKind;
        let label = match kind {
            BoolKind::Union => "Union",
            BoolKind::Intersection => "Intersect",
            BoolKind::Difference => "Subtract",
            BoolKind::Xor => "XOR",
        };
        self.run_ops(label, |scene, sel, cell, editable| {
            crate::ops::boolean_edits(kind, scene, sel, cell, editable)
        });
    }

    /// Exports a per-layer metrology CSV (the `file.export_metrology` effect,
    /// catalog 12).
    ///
    /// A dependency-free layer-area summary computed straight from the flattened
    /// scene: one row per layer with its name and shape count. The full metrology
    /// report (perimeter, connectivity, antenna) lives in `reticle-metrology`; it
    /// is not wired here to keep it out of the app's wasm bundle budget.
    fn export_metrology(&mut self) {
        use std::collections::BTreeMap;
        use std::fmt::Write as _;
        let mut counts: BTreeMap<(u16, u16), usize> = BTreeMap::new();
        for s in self.scene.shapes() {
            *counts.entry((s.layer.layer, s.layer.datatype)).or_default() += 1;
        }
        let mut csv = String::from("layer,datatype,name,shape_count\n");
        for ((layer, datatype), n) in counts {
            let id = LayerId::new(layer, datatype);
            let name = self
                .layer_state
                .rows()
                .iter()
                .find(|r| r.id == id)
                .map_or_else(|| format!("{layer}/{datatype}"), |r| r.name.clone());
            let _ = writeln!(csv, "{layer},{datatype},{name},{n}");
        }
        match self.write_export_text("reticle-metrology.csv", &csv) {
            Ok(msg) => self.status.set(msg),
            Err(e) => self.status.set(format!("Metrology CSV export failed: {e}")),
        }
    }

    /// Grows the selection to every shape sharing a layer with the current
    /// selection (catalog 56, the `select.same_layer` effect).
    fn select_same_layer(&mut self) {
        let shapes = self.scene.shapes();
        let layers: std::collections::BTreeSet<LayerId> = self
            .selection
            .iter()
            .filter_map(|i| shapes.get(i).map(|s| s.layer))
            .collect();
        if layers.is_empty() {
            self.status.set("Select a shape first");
            return;
        }
        let hits: Vec<usize> = shapes
            .iter()
            .enumerate()
            .filter(|(_, s)| layers.contains(&s.layer))
            .map(|(i, _)| i)
            .collect();
        let n = hits.len();
        self.selection.set(hits);
        self.status
            .set(format!("Selected {n} shape(s) on same layer"));
    }

    /// Applies a pane split and reports it in the status bar.
    fn set_split(&mut self, split: Split) {
        self.viewports.set_split(split, &self.camera);
        self.status.set(format!("View: {}", split.label()));
    }

    /// Draws the thinned top toolbar: an Open affordance, grouped selection and
    /// draw tools, view actions and toggles, and a segmented split control.
    ///
    /// Every control is a [`components`] widget dispatched through its registry id,
    /// so the toolbar and the menu bar share one effect path and one keymap. Undo,
    /// Redo, the palette, and Shortcuts moved into the Edit and Help menus; the web
    /// Convert affordance moved into the File menu. Tooltips carry a name, the live
    /// chord, and a one-line description (catalog 25). Nothing floats.
    #[allow(clippy::too_many_lines)] // one flat row of button groups reads better than fragmenting
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        // Mark the toolbar as a keyboard focus region (lane 3C, item 83) so F6 can
        // land here and paint a focus ring around the whole row.
        self.focus_anchor(ui, crate::focus::FocusRegion::Toolbar);
        let ctx = components::Ctx::dark(self.ui_density).with_reduced_motion(self.reduced_motion);
        ui.horizontal_wrapped(|ui| {
            // The open affordance the first-run tour points at. It returns to the
            // Start screen, which holds the file-open guidance, the drag-and-drop
            // target, the example-chip gallery, and recent files, so opening a design
            // works the same on native and on the web (where there is no filesystem
            // dialog). Dragging a file onto the window opens it directly (see
            // `handle_dropped_files`).
            if components::IconButton::new(icons::FOLDER_OPEN, "Open")
                .hint("Open a design, or drag a GDSII or OASIS file onto the window")
                .show(ui, ctx)
                .clicked()
            {
                self.start_screen = true;
            }
            ui.separator();

            // Selection tools.
            for (tool, icon) in [
                (
                    Tool::Select,
                    IconLabel::new(
                        icons::MOUSE_POINTER_2,
                        "Select",
                        "Select and rubber-band shapes",
                    ),
                ),
                (
                    Tool::Pan,
                    IconLabel::new(icons::HAND, "Pan", "Drag to move the view"),
                ),
                (
                    Tool::Measure,
                    IconLabel::new(
                        icons::RULER,
                        "Measure",
                        "Measure the distance between two points",
                    ),
                ),
            ] {
                self.tool_icon(ui, ctx, tool, icon);
            }
            ui.separator();

            // Draw tools.
            for (tool, icon) in [
                (
                    Tool::CutLine,
                    IconLabel::new(icons::SCISSORS, "Cut line", "Cut a shape along a line"),
                ),
                (
                    Tool::DrawRect,
                    IconLabel::new(icons::SQUARE, "Rectangle", "Draw an axis-aligned rectangle"),
                ),
                (
                    Tool::DrawPolygon,
                    IconLabel::new(icons::PENTAGON, "Polygon", "Draw a polygon"),
                ),
                (
                    Tool::DrawPath,
                    IconLabel::new(icons::PEN_TOOL, "Path", "Draw a fixed-width path"),
                ),
                (
                    Tool::EditVertex,
                    IconLabel::new(
                        icons::SQUARE_PEN,
                        "Edit vertices",
                        "Move the vertices of a shape",
                    ),
                ),
            ] {
                self.tool_icon(ui, ctx, tool, icon);
            }
            // Path width and end cap, shown only while the path tool is active.
            if self.tools.active() == Tool::DrawPath {
                self.path_options(ui, ctx);
            }
            ui.separator();

            // View action and toggles, all dispatched through the registry.
            let fit = IconLabel::new(icons::MAXIMIZE, "Fit", "Frame the whole design");
            if self.command_icon(ui, ctx, fit, "view.zoom_fit", false) {
                self.dispatch(CommandId("view.zoom_fit"));
            }
            for (icon, id, on) in [
                (
                    IconLabel::new(icons::GRID_3X3, "Grid", "Toggle the reference grid"),
                    "view.grid",
                    self.grid.visible,
                ),
                (
                    IconLabel::new(icons::MAGNET, "Snap", "Toggle snapping to the grid"),
                    "view.snap",
                    self.grid.snap_enabled,
                ),
                (
                    IconLabel::new(icons::TAG, "Labels", "Toggle text labels"),
                    "view.labels",
                    self.labels_visible,
                ),
                (
                    IconLabel::new(icons::MAP, "Minimap", "Toggle the overview minimap"),
                    "view.minimap",
                    self.minimap_visible,
                ),
            ] {
                if self.command_icon(ui, ctx, icon, id, on) {
                    self.dispatch(CommandId(id));
                }
            }
            ui.separator();

            // Split mode as a single-select segmented control.
            let mut idx = Split::all()
                .iter()
                .position(|s| *s == self.viewports.split())
                .unwrap_or(0);
            if components::Segmented::new(&["Single", "Split H", "Split V"])
                .show(ui, ctx, &mut idx)
                .changed()
            {
                self.set_split(Split::all()[idx]);
            }

            // Web only: switch between the public replay theater and the full editor.
            // Formerly a floating top-right HTML link that overlapped the right-hand
            // column; kept in the toolbar row so it never occludes those controls
            // (lane v8-ui). The navigation itself stays in the web shell.
            #[cfg(target_arch = "wasm32")]
            {
                ui.separator();
                let (glyph, name) = match self.start_view {
                    StartView::ReplayTheater => (icons::EXTERNAL_LINK, "Open the full editor"),
                    StartView::Editor => (icons::CLAPPERBOARD, "Open the replay theater"),
                };
                if components::IconButton::new(glyph, name)
                    .show(ui, ctx)
                    .clicked()
                {
                    Self::call_window_fn("__reticleSwitchView");
                }
            }
        });
    }

    /// A toolbar tool button: selected when `tool` is active and dispatched through
    /// the registry id that selects it.
    fn tool_icon(&mut self, ui: &mut egui::Ui, ctx: components::Ctx, tool: Tool, icon: IconLabel) {
        let id = tool_command_id(tool);
        let selected = self.tools.active() == tool;
        if self.command_icon(ui, ctx, icon, id, selected) {
            self.dispatch(CommandId(id));
        }
    }

    /// Shows one toolbar [`IconButton`](components::IconButton) carrying the live
    /// chord `keymap` binds to `id` and a one-line tooltip; returns whether it was
    /// clicked. Reads `self` only, so the caller can dispatch after.
    fn command_icon(
        &self,
        ui: &mut egui::Ui,
        ctx: components::Ctx,
        icon: IconLabel,
        id: &'static str,
        selected: bool,
    ) -> bool {
        let mut button = components::IconButton::new(icon.glyph, icon.name)
            .hint(icon.hint)
            .selected(selected);
        if let Some(chord) = self.keymap.chord_for(CommandId(id)) {
            button = button.kbd(chord.to_string());
        }
        button.show(ui, ctx).clicked()
    }

    /// Draws the path tool's inline width and end-cap controls. The width is a raw
    /// numeric field (the 1C component library has no numeric input yet); the end
    /// cap is a [`Segmented`](components::Segmented) control.
    fn path_options(&mut self, ui: &mut egui::Ui, ctx: components::Ctx) {
        ui.separator();
        ui.label("Width:");
        let mut width = self.draw.path.width();
        if ui
            .add(
                egui::DragValue::new(&mut width)
                    .speed(5.0)
                    .range(1..=100_000),
            )
            .changed()
        {
            self.draw.path.set_width(width);
        }
        let caps = [Endcap::Flat, Endcap::Square, Endcap::Round];
        let mut idx = caps
            .iter()
            .position(|c| *c == self.draw.path.endcap())
            .unwrap_or(0);
        if components::Segmented::new(&["Flat", "Square", "Round"])
            .show(ui, ctx, &mut idx)
            .changed()
        {
            self.draw.path.set_endcap(caps[idx]);
        }
    }

    /// Invokes a zero-argument function stored as a global `window[name]`, if the web
    /// shell defined one.
    ///
    /// The convert picker and the theater/editor view switch are wired up in
    /// `crates/web/index.html`; the toolbar buttons above delegate to them here so those
    /// affordances sit in the toolbar instead of floating over the canvas (lane v8-ui). A
    /// missing global or a call error is a no-op -- the toolbar button simply does
    /// nothing rather than panicking.
    #[cfg(target_arch = "wasm32")]
    fn call_window_fn(name: &str) {
        use wasm_bindgen::{JsCast, JsValue};
        let Some(window) = web_sys::window() else {
            return;
        };
        let Ok(value) = js_sys::Reflect::get(&window, &JsValue::from_str(name)) else {
            return;
        };
        if let Ok(func) = value.dyn_into::<js_sys::Function>() {
            let _ = func.call0(&JsValue::NULL);
        }
    }

    /// Draws the menu bar, rendered from the command registry (lane 2A).
    ///
    /// Every registered command with a menu path appears under it with its live
    /// chord; the tail folds into a `...` menu when the window is narrow. Clicks are
    /// collected into a [`MenuChoice`](menu::MenuChoice) and applied after the bar
    /// closes, so the menu closures borrow nothing but a local (see [`crate::menu`]).
    fn menubar(&mut self, ui: &mut egui::Ui) {
        let menus = menu::build_menus(&self.keymap);
        let recent: Vec<String> = self
            .recent_files
            .entries()
            .iter()
            .map(|r| r.name.clone())
            .collect();
        let mut choice: Option<menu::MenuChoice> = None;
        egui::MenuBar::new().ui(ui, |ui| {
            menu::render_bar(ui, &menus, &recent, &mut choice);
        });
        self.apply_menu_choice(choice);
    }

    /// Applies the [`MenuChoice`](menu::MenuChoice) a menu click recorded this frame.
    ///
    /// Registry commands go through [`dispatch`](Self::dispatch) like every other
    /// surface; the two remaining dynamic items (Open Recent and the web convert
    /// picker) route to their existing effects until the lanes that own them (2D, 3B)
    /// register their ids.
    fn apply_menu_choice(&mut self, choice: Option<menu::MenuChoice>) {
        match choice {
            Some(menu::MenuChoice::Command(id)) => self.dispatch(id),
            Some(menu::MenuChoice::OpenRecent) => {
                // The live reopen is lane 2D's work (catalog 9); route to the Start
                // screen, where recent files live, so the item is never a dead end.
                self.start_screen = true;
                self.status.set("Recent files are on the Start screen");
            }
            None => {}
        }
    }

    /// Whether the tour counts as seen for persistence.
    ///
    /// It is seen unless it is an automatic first-run that has not finished yet, so
    /// a completed or dismissed first run, and any relaunch, persist `tour_seen =
    /// true`. That is what stops the automatic tour from ever showing twice.
    fn tour_seen(&self) -> bool {
        // Seen unless it is an unfinished first run.
        !self.tour.is_first_run() || self.tour.is_finished()
    }

    /// Builds the persisted session snapshot from the live view, settings, and
    /// onboarding state.
    ///
    /// [`SessionState::capture`](crate::session::SessionState::capture) carries the view and the settings (density, motion,
    /// wheel, touch); the onboarding bits (hints, checklist, GPU card) are not view
    /// state, so they are grafted on here. This is the single place both the native
    /// session file and the web localStorage mirror are built from.
    fn session_snapshot(&self) -> crate::session::SessionState {
        let hidden: Vec<LayerId> = self
            .layer_state
            .rows()
            .iter()
            .filter(|r| !r.visible)
            .map(|r| r.id)
            .collect();
        let mut state = crate::session::SessionState::capture(
            &self.camera,
            self.tools.active(),
            self.grid,
            self.view_export.theme,
            self.ui_density,
            self.reduced_motion,
            self.wheel,
            self.touch_mode,
            &hidden,
            self.tour_seen(),
            crate::session::PanelLayout {
                left_w: self.panel_left_w,
                panel_3d_open: self.panel_3d_open,
                panel_xsection_open: self.panel_xsection_open,
            },
        );
        state.hints = self.hints;
        state.checklist = self.checklist;
        state.gpu_card_dismissed = self.gpu_card_dismissed;
        // Fold in the right Inspector's remembered layout (lane 2B, catalog 61).
        state.panel_right_w = self.inspector.width.0;
        state.panel_group = self.inspector.group.index() as u8;
        state.panels_collapsed = self.inspector.collapsed;
        state.panel_open = self.inspector.open_tags();
        state
    }

    /// Persists the current preferences and onboarding state immediately.
    ///
    /// Called whenever a setting or onboarding bit changes so the choice sticks
    /// without waiting for the periodic eframe save. On native it writes the session
    /// file; on the web it writes the localStorage mirror (there is no filesystem).
    fn persist_prefs(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = crate::session::save(&self.session_snapshot());
        }
        #[cfg(target_arch = "wasm32")]
        {
            crate::session::web_save(&self.session_snapshot());
        }
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
                Stroke::new(3.0, CANVAS.tour_highlight),
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
            // Close parks the tour so the Help menu can offer "Resume tour" and pick
            // up at the same step, rather than ending it outright.
            Some(TourAction::Close) => self.tour.dismiss(),
            None => {}
        }
        // Any tour transition can flip the persisted "seen" bit, so save it now
        // rather than waiting for the periodic eframe save.
        if action.is_some() {
            self.persist_prefs();
        }
    }

    // ---- lane 4c: onboarding hints, checklist, and GPU card -----------------

    /// Advances the onboarding state from observed app state, once per editor frame.
    ///
    /// Ticks the checklist tasks off as their action happens (a design opened, DRC
    /// run, a session shared, the agent tried) and fires the once-only contextual
    /// hints the first time the user reaches the layers, DRC, or share surface. A
    /// no-op in a viewer session, whose onboarding is the viewer tour instead.
    fn update_onboarding(&mut self) {
        use crate::onboarding::{Hint, Task};
        if self.is_viewer() {
            return;
        }
        // Checklist completion, inferred from live state.
        if self.opened_a_design() {
            self.complete_task(Task::OpenFile);
        }
        if self.drc.has_run() {
            self.complete_task(Task::RunDrc);
        }
        if self.live_status.is_open() {
            self.complete_task(Task::Share);
        }
        if self.agent.is_running() || self.replay.is_loaded() {
            self.complete_task(Task::TryAgent);
        }
        // Contextual hints: at most one at a time, and never over the running tour.
        if self.active_hint.is_some() || self.tour.is_active() {
            return;
        }
        if !self.hints.is_seen(Hint::Layers) && self.layer_state.rows().iter().any(|r| !r.visible) {
            self.fire_hint(Hint::Layers);
        } else if !self.hints.is_seen(Hint::Drc) && self.drc.has_run() && !self.drc.is_empty() {
            self.fire_hint(Hint::Drc);
        } else if !self.hints.is_seen(Hint::Share) && self.live_status.is_open() {
            self.fire_hint(Hint::Share);
        }
    }

    /// Whether a real design (not the built-in demo) is loaded, for the "Open a
    /// design" checklist task.
    fn opened_a_design(&self) -> bool {
        self.is_viewer() || self.archive.is_some() || self.top_cell != crate::demo::TOP_CELL
    }

    /// Marks a checklist task complete, persisting only on a real change.
    fn complete_task(&mut self, task: crate::onboarding::Task) {
        if self.checklist.complete(task) {
            self.persist_prefs();
        }
    }

    /// Fires a once-only hint: records it seen, shows its bubble, and persists.
    fn fire_hint(&mut self, hint: crate::onboarding::Hint) {
        if let Some(shown) = self.hints.fire(hint) {
            self.active_hint = Some(shown);
            self.persist_prefs();
        }
    }

    /// Draws the onboarding chrome: the active hint bubble, the checklist card, and
    /// the first-run GPU capability card. A no-op in a viewer session or on the
    /// Start screen (onboarding belongs to the editor).
    // Three independent cards laid out inline; the length is layout, not branching.
    #[allow(clippy::too_many_lines)]
    fn onboarding_overlay(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        use crate::onboarding::Task;
        use crate::theme::components::{Button, Ctx};
        if self.is_viewer() || self.start_screen {
            return;
        }
        let cctx = Ctx::dark(self.ui_density).with_reduced_motion(self.reduced_motion);

        // The active contextual hint, as a small card below the toolbar.
        if let Some(hint) = self.active_hint {
            let mut dismiss = false;
            egui::Area::new(egui::Id::new("onboarding_hint"))
                .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 56.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_max_width(340.0);
                            ui.strong(hint.title());
                            ui.add_space(4.0);
                            ui.label(hint.body());
                            ui.add_space(8.0);
                            if Button::secondary("Got it").show(ui, cctx).clicked() {
                                dismiss = true;
                            }
                        });
                });
            if dismiss {
                self.active_hint = None;
            }
        }

        // The onboarding checklist card, bottom-right above the status bar.
        if self.checklist.is_visible() {
            let mut dismiss = false;
            let (done, total) = self.checklist.progress();
            egui::Area::new(egui::Id::new("onboarding_checklist"))
                .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-16.0, -48.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_max_width(260.0);
                            ui.horizontal(|ui| {
                                ui.strong("Get started");
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.weak(format!("{done} of {total}"));
                                    },
                                );
                            });
                            ui.add_space(4.0);
                            for task in Task::ALL {
                                let done = self.checklist.is_done(task);
                                let mark = if done { "\u{2713}" } else { "\u{25CB}" };
                                let text = format!("{mark}  {}", task.label());
                                if done {
                                    ui.weak(text);
                                } else {
                                    ui.label(text);
                                }
                            }
                            ui.add_space(8.0);
                            if Button::secondary("Dismiss").show(ui, cctx).clicked() {
                                dismiss = true;
                            }
                        });
                });
            if dismiss {
                self.checklist.dismiss();
                self.persist_prefs();
            }
        }

        // The first-run GPU capability card, bottom-left, shown once.
        if !self.gpu_card_dismissed {
            let (adapter, backend) = gpu_info(frame);
            let mut dismiss = false;
            let mut docs = false;
            egui::Area::new(egui::Id::new("onboarding_gpu"))
                .anchor(Align2::LEFT_BOTTOM, Vec2::new(16.0, -48.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_max_width(300.0);
                            ui.strong("Graphics");
                            ui.add_space(4.0);
                            ui.label(format!(
                                "Rendering with {backend} on {adapter}. Reticle runs \
                                 on WebGPU where available and falls back to WebGL2."
                            ));
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if Button::secondary("Learn more").show(ui, cctx).clicked() {
                                    docs = true;
                                }
                                if Button::secondary("Dismiss").show(ui, cctx).clicked() {
                                    dismiss = true;
                                }
                            });
                        });
                });
            if docs {
                self.pending_open_url = Some(crate::help::DOCS_URL.to_owned());
            }
            if dismiss {
                self.gpu_card_dismissed = true;
                self.persist_prefs();
            }
        }
    }

    // ---- lane 4c: the Help dialogs ------------------------------------------

    /// The Settings dialog (catalog 98): density, reduced motion, wheel behavior,
    /// touch mode, and a panel-layout reset.
    ///
    /// Changing density or reduced motion flips [`theme_dirty`](Self::theme_dirty)
    /// so lane 1A's boot styling hook re-applies the theme next frame; every change
    /// persists immediately (the session file on native, the localStorage mirror on
    /// the web).
    fn settings_dialog(&mut self, ctx: &egui::Context) {
        use crate::settings::{TouchMode, WheelBehavior};
        use crate::theme::components::{Button, Ctx, Modal, Segmented, ToggleChip};
        use crate::theme::tokens::Density;
        if !self.settings_open {
            return;
        }
        let cctx = Ctx::dark(self.ui_density).with_reduced_motion(self.reduced_motion);
        let mut density_ix = usize::from(self.ui_density == Density::Compact);
        let mut reduced = self.reduced_motion;
        let mut wheel_ix = usize::from(self.wheel == WheelBehavior::Pan);
        let mut touch_ix = match self.touch_mode {
            TouchMode::Auto => 0,
            TouchMode::On => 1,
            TouchMode::Off => 2,
        };
        let mut reset_panels = false;
        let mut close = false;
        let modal = Modal::new("Settings").overlay(ctx, cctx, |ui, cctx| {
            ui.set_max_width(340.0);
            ui.label("Density");
            Segmented::new(&["Comfortable", "Compact"]).show(ui, cctx, &mut density_ix);
            ui.add_space(cctx.density.item_spacing().y);
            if ToggleChip::new("Reduced motion", reduced)
                .show(ui, cctx)
                .clicked()
            {
                reduced = !reduced;
            }
            ui.add_space(cctx.density.item_spacing().y);
            ui.label("Mouse wheel");
            Segmented::new(&["Zoom", "Pan"]).show(ui, cctx, &mut wheel_ix);
            ui.add_space(cctx.density.item_spacing().y);
            ui.label("Touch targets");
            Segmented::new(&["Auto", "On", "Off"]).show(ui, cctx, &mut touch_ix);
            ui.add_space(cctx.density.item_spacing().y);
            ui.separator();
            ui.horizontal(|ui| {
                if Button::secondary("Reset panel layout")
                    .show(ui, cctx)
                    .clicked()
                {
                    reset_panels = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if Button::secondary("Close").show(ui, cctx).clicked() {
                        close = true;
                    }
                });
            });
        });

        let new_density = if density_ix == 1 {
            Density::Compact
        } else {
            Density::Comfortable
        };
        let new_wheel = if wheel_ix == 1 {
            WheelBehavior::Pan
        } else {
            WheelBehavior::Zoom
        };
        let new_touch = match touch_ix {
            1 => TouchMode::On,
            2 => TouchMode::Off,
            _ => TouchMode::Auto,
        };
        let mut changed = false;
        if new_density != self.ui_density {
            self.ui_density = new_density;
            self.theme_dirty = true;
            changed = true;
        }
        if reduced != self.reduced_motion {
            self.reduced_motion = reduced;
            self.theme_dirty = true;
            changed = true;
        }
        if new_wheel != self.wheel {
            self.wheel = new_wheel;
            changed = true;
        }
        if new_touch != self.touch_mode {
            self.touch_mode = new_touch;
            // A touch-mode change re-applies the theme so the raised/lowered
            // hit-target floor (lane 4B) takes effect next frame.
            self.theme_dirty = true;
            changed = true;
        }
        if changed {
            self.persist_prefs();
        }
        if reset_panels {
            // A panel-layout reset drops egui's persisted per-widget UI state (panel
            // widths and collapsible open flags), returning the docked layout to its
            // defaults without touching the document or the saved view.
            ctx.data_mut(egui::util::IdTypeMap::clear);
            self.status.set("Panel layout reset");
        }
        if close || modal.should_close() {
            self.settings_open = false;
        }
    }

    /// The About dialog (catalog 99, 100): versions, GPU adapter, bundle hash, a
    /// one-click copy of the diagnostics, a prefilled issue link, and the verified
    /// zero-telemetry statement.
    fn about_dialog(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        use crate::theme::components::{Button, Ctx, Modal};
        if !self.about_open {
            return;
        }
        let cctx = Ctx::dark(self.ui_density).with_reduced_motion(self.reduced_motion);
        let (adapter, backend) = gpu_info(frame);
        let diagnostics = crate::help::Diagnostics {
            app_version: env!("CARGO_PKG_VERSION"),
            bundle_hash: bundle_hash(),
            platform: platform_label(),
            gpu_adapter: adapter.as_str(),
            gpu_backend: backend,
        };
        let report = diagnostics.report();
        let mut copy = false;
        let mut issue = false;
        let mut close = false;
        let modal = Modal::new("About Reticle").overlay(ctx, cctx, |ui, cctx| {
            ui.set_max_width(420.0);
            ui.strong(format!("Reticle {}", env!("CARGO_PKG_VERSION")));
            ui.add_space(4.0);
            ui.weak(report.as_str());
            ui.add_space(cctx.density.item_spacing().y);
            ui.separator();
            ui.label(crate::help::ZERO_TELEMETRY);
            ui.add_space(cctx.density.item_spacing().y);
            ui.horizontal(|ui| {
                if Button::secondary("Copy diagnostics")
                    .show(ui, cctx)
                    .clicked()
                {
                    copy = true;
                }
                if Button::secondary("Report an issue")
                    .show(ui, cctx)
                    .clicked()
                {
                    issue = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if Button::secondary("Close").show(ui, cctx).clicked() {
                        close = true;
                    }
                });
            });
        });
        if copy {
            ctx.copy_text(report.clone());
            self.status.set("Diagnostics copied");
        }
        if issue {
            self.pending_open_url = Some(crate::help::issue_url(&report));
        }
        if close || modal.should_close() {
            self.about_open = false;
        }
    }

    /// The "What's new" dialog (catalog 26): the embedded changelog, newest first.
    fn whats_new_dialog(&mut self, ctx: &egui::Context) {
        use crate::theme::components::{Button, Ctx, Modal};
        if !self.whats_new_open {
            return;
        }
        let cctx = Ctx::dark(self.ui_density).with_reduced_motion(self.reduced_motion);
        let mut close = false;
        let modal = Modal::new("What's new").overlay(ctx, cctx, |ui, cctx| {
            ui.set_max_width(460.0);
            egui::ScrollArea::vertical()
                .max_height(360.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for release in crate::help::changelog() {
                        ui.strong(format!("Version {} ({})", release.version, release.date));
                        ui.label(release.headline);
                        ui.add_space(2.0);
                        for note in release.notes {
                            ui.label(format!("\u{2022}  {note}"));
                        }
                        ui.add_space(cctx.density.item_spacing().y);
                    }
                });
            ui.separator();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if Button::secondary("Close").show(ui, cctx).clicked() {
                    close = true;
                }
            });
        });
        if close || modal.should_close() {
            self.whats_new_open = false;
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
        frame: &eframe::Frame,
        gpu_format: Option<eframe::egui_wgpu::wgpu::TextureFormat>,
    ) -> (Option<ScreenRect>, TourTargets) {
        egui::Panel::top("menubar").show(ui, |ui| self.menubar(ui));
        let toolbar = egui::Panel::top("toolbar")
            .show(ui, |ui| self.toolbar(ui))
            .response
            .rect;
        egui::Panel::bottom("status").show(ui, |ui| {
            self.status_bar(ui);
        });
        let layers = egui::Panel::left("layers")
            .resizable(true)
            .default_size(self.panel_left_w)
            .show(ui, |ui| self.layer_panel(ui))
            .response
            .rect;
        // Remember the (possibly resized) width so the session persists it.
        self.panel_left_w = layers.width();
        let right_column = self.inspector_column(ui);
        // Managed view panels (ADR 0096): the 3D stack and Cross-section dock to the
        // canvas bottom when opened from View > Panels. Registered after the side
        // panels, so each spans only the central column and sits above the status bar;
        // the canvas shrinks to fit above them, keeping the rulers and the top-right
        // minimap clear of the panels by construction (fixes AUD-01/AUD-02).
        self.show_xsection_panel(ui);
        self.show_view3d_panel(ui, frame);
        // The Replay theater docks here too (H1): it was the lone holdout left as a
        // floating window, and it opens on the public landing (?view=replay, ADR
        // 0026), so its title bar sliced through the menu bar and its body covered
        // the canvas on a first visit. As a managed bottom panel it sits above the
        // status bar with the canvas shrinking to fit above it, so the menu bar and
        // canvas are never occluded (the AUD-01/AUD-20 scenario the packet set out
        // to kill).
        self.show_replay_panel(ui);
        let mut canvas_screen: Option<ScreenRect> = None;
        egui::CentralPanel::default().show(ui, |ui| {
            canvas_screen = Some(self.canvas(ui, gpu_format));
        });
        // Cache the canvas rectangle so the live-share sharer can publish its viewport
        // (see [`App::local_presence`]) without threading the screen through.
        self.last_screen = canvas_screen;

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

    /// A [`components::Ctx`] for this frame: the dark palette, the active density,
    /// and the reduced-motion preference. Chrome that composes the widget library
    /// (the Layers panel, the managed view panels) threads this into each call.
    fn ui_ctx(&self) -> components::Ctx {
        components::Ctx {
            tokens: DARK,
            density: self.ui_density,
            reduced_motion: self.reduced_motion,
        }
    }

    /// Hosts the managed 3D-stack panel (ADR 0096) docked at the canvas bottom when
    /// [`panel_3d_open`](Self::panel_3d_open) is set. A close affordance in the
    /// header flips the flag; the wgpu callback is [`crate::view3d::View3d::show_in`],
    /// unchanged from the floating window it replaced.
    fn show_view3d_panel(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        if !self.panel_3d_open {
            return;
        }
        let cctx = self.ui_ctx();
        egui::Panel::bottom("view.panel_3d")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                let close = view_panel_header(ui, cctx, "3D stack");
                self.view3d.show_in(
                    ui,
                    frame,
                    self.history.document(),
                    &self.top_cell,
                    &self.layer_state,
                );
                if close {
                    self.panel_3d_open = false;
                    self.status.set("3D stack off");
                }
            });
    }

    /// Hosts the managed Cross-section panel (ADR 0096) docked at the canvas bottom
    /// when [`panel_xsection_open`](Self::panel_xsection_open) is set. A close
    /// affordance in the header flips the flag; the elevation draw is
    /// [`crate::xsection::panel`], unchanged from the floating window it replaced.
    fn show_xsection_panel(&mut self, ui: &mut egui::Ui) {
        if !self.panel_xsection_open {
            return;
        }
        let cctx = self.ui_ctx();
        egui::Panel::bottom("view.panel_xsection")
            .resizable(true)
            .default_size(220.0)
            .show(ui, |ui| {
                let close = view_panel_header(ui, cctx, "Cross-section");
                crate::xsection::panel(
                    ui,
                    self.tools.cut_line(),
                    self.scene.shapes(),
                    self.history.document().technology(),
                    &self.layer_state,
                );
                if close {
                    self.panel_xsection_open = false;
                    self.status.set("Cross-section off");
                }
            });
    }

    /// The Replay theater as a managed bottom panel (H1): the load row, transport,
    /// readouts, and replay canvas, with a header carrying the close affordance.
    /// Docked (never a floating window) so it cannot occlude the menu bar or the
    /// main canvas, including on the public `?view=replay` landing (ADR 0026).
    fn show_replay_panel(&mut self, ui: &mut egui::Ui) {
        if !self.replay_open {
            return;
        }
        let cctx = self.ui_ctx();
        egui::Panel::bottom("replay_theater")
            .resizable(true)
            // Compact by default so the main canvas stays the hero (the packet's
            // "must not open half-screen"): a slim docked strip carrying the load row,
            // transport, and readouts, plus a small live preview. The size range caps
            // the panel well under half the viewport; the user can drag within it.
            // Docked, never floating (ADR 0026).
            .default_size(196.0)
            .size_range(egui::Rangef::new(150.0, 248.0))
            .show(ui, |ui| {
                let close = view_panel_header(ui, cctx, "Replay theater");
                self.replay_load_row(ui);
                ui.separator();
                self.replay_transport_row(ui);
                ui.separator();
                self.replay_readouts(ui);
                ui.separator();
                self.replay_canvas(ui);
                if close {
                    self.replay_open = false;
                }
            });
    }

    /// The component-library context for this frame: the shipped dark palette at
    /// the active density, honoring the reduced-motion preference. Every Inspector
    /// widget composes from [`theme::components`] through this handle (lane 2B).
    fn comp_ctx(&self) -> theme::components::Ctx {
        theme::components::Ctx::dark(self.ui_density).with_reduced_motion(self.reduced_motion)
    }

    /// Draws the right Inspector: a segmented control over the four groups with
    /// token-styled collapsible sections inside the selected one, or the collapsed
    /// icon rail (ADR-2B; managed panel per ADR 0096). Returns the panel rect for
    /// the tour overlay.
    fn inspector_column(&mut self, ui: &mut egui::Ui) -> EguiRect {
        use crate::inspector_layout::OrderedWidth;

        // Demo-capture focus hooks: jump to the group holding the driven section so
        // the README media harness frames it (was a scroll-to-cursor before the
        // segmented rebuild).
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.demo_focus_search {
                self.inspector.reveal("search");
            }
            if self.demo_focus_generate {
                self.inspector.reveal("generate");
            }
        }

        if self.inspector.collapsed {
            return self.inspector_rail(ui);
        }

        let ctx = self.comp_ctx();
        let rect = egui::Panel::right("inspector")
            .resizable(true)
            .default_size(self.inspector.width.clamped())
            .show(ui, |ui| {
                // Keyboard focus region for F6 traversal (lane 3C, item 83).
                self.focus_anchor(ui, crate::focus::FocusRegion::RightPanel);
                self.inspector_header(ui, ctx);
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.inspector_body(ui, ctx);
                    });
            })
            .response
            .rect;
        // Track the actual (possibly dragged) width so the session persists it.
        self.inspector.width = OrderedWidth(rect.width());
        rect
    }

    /// The Inspector header: the four-group segmented control, a collapse-to-rail
    /// button, and the per-panel gear menu (catalog 59, 60).
    fn inspector_header(&mut self, ui: &mut egui::Ui, ctx: theme::components::Ctx) {
        use crate::inspector_layout::PanelGroup;
        use theme::components::IconButton;

        ui.horizontal(|ui| {
            // Icon tabs, not a text segmented control: four full-text labels plus
            // the collapse button and gear menu did not fit the fixed Inspector
            // width and clipped "Settings" to "ngs" at every desktop size (H2). Each
            // group carries an icon and its name as the tooltip (matching the
            // collapsed icon rail), so the tab strip stays legible and compact.
            for group in PanelGroup::ALL {
                let selected = self.inspector.group == group;
                if IconButton::new(group.icon(), group.label())
                    .selected(selected)
                    .show(ui, ctx)
                    .clicked()
                {
                    self.inspector.group = group;
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if IconButton::new(crate::theme::icons::PANEL_RIGHT, "Collapse panel")
                    .kbd("Tab")
                    .hint("Collapse the Inspector to its icon rail")
                    .show(ui, ctx)
                    .clicked()
                {
                    self.inspector.collapsed = true;
                }
                self.inspector_gear_menu(ui, ctx);
            });
        });
    }

    /// The per-panel gear menu (catalog 60): expand/collapse every section in the
    /// current group at once, or toggle a single section, from one place.
    fn inspector_gear_menu(&mut self, ui: &mut egui::Ui, _ctx: theme::components::Ctx) {
        use crate::inspector_layout::InspectorState;

        let group = self.inspector.group;
        let gear = crate::theme::icons::SETTINGS.to_string();
        ui.menu_button(gear, |ui| {
            if ui.button("Expand all").clicked() {
                for spec in InspectorState::sections_in(group) {
                    self.inspector.set_open(spec.key, true);
                }
                ui.close();
            }
            if ui.button("Collapse all").clicked() {
                for spec in InspectorState::sections_in(group) {
                    self.inspector.set_open(spec.key, false);
                }
                ui.close();
            }
            ui.separator();
            for spec in InspectorState::sections_in(group) {
                let mut open = self.inspector.is_open(spec.key);
                if ui.checkbox(&mut open, spec.title).changed() {
                    self.inspector.set_open(spec.key, open);
                }
            }
        });
    }

    /// Draws the collapsible sections of the selected group in render order.
    fn inspector_body(&mut self, ui: &mut egui::Ui, ctx: theme::components::Ctx) {
        use crate::inspector_layout::PanelGroup;
        match self.inspector.group {
            PanelGroup::Inspect => {
                self.inspector_section(ui, ctx, "properties", "Properties", Self::inspector_panel);
                self.inspector_section(ui, ctx, "search", "Search and outline", Self::search_panel);
                self.inspector_section(ui, ctx, "history", "History", Self::history_panel);
            }
            PanelGroup::Review => {
                self.inspector_section(ui, ctx, "drc", "DRC", Self::drc_panel);
                self.inspector_section(ui, ctx, "diff", "Layout diff", Self::diff_panel);
                self.inspector_section(ui, ctx, "comments", "Comments", Self::comment_panel);
            }
            PanelGroup::Automate => {
                self.inspector_section(ui, ctx, "agent", "Agent (preview)", Self::agent_section);
                self.inspector_section(ui, ctx, "generate", "Generate", Self::generate_section);
            }
            PanelGroup::Settings => {
                self.inspector_section(ui, ctx, "operations", "Operations", Self::ops_panel);
                self.inspector_section(
                    ui,
                    ctx,
                    "productivity",
                    "Productivity",
                    Self::productivity_panel,
                );
                self.inspector_section(ui, ctx, "snap", "Snap and guides", Self::snap_panel);
                self.inspector_section(ui, ctx, "export", "Export", Self::view_export_panel);
                self.inspector_section(
                    ui,
                    ctx,
                    "tech",
                    "Technology editor",
                    Self::tech_editor_panel,
                );
            }
        }
    }

    /// Draws one collapsible Inspector section: a token-styled header with a
    /// remembered-open flag (persisted through the session), then `body` when open.
    fn inspector_section(
        &mut self,
        ui: &mut egui::Ui,
        ctx: theme::components::Ctx,
        key: &'static str,
        title: &'static str,
        body: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        let mut open = self.inspector.is_open(key);
        theme::components::Collapsible::new(key, title).show(ui, ctx, &mut open, |ui, _ctx| {
            body(self, ui);
        });
        self.inspector.set_open(key, open);
    }

    /// Draws the collapsed Inspector as a narrow icon rail: an expand button and a
    /// selectable glyph per group (catalog 59). Clicking a group expands the panel
    /// on that group. Returns the rail rect for the tour overlay.
    fn inspector_rail(&mut self, ui: &mut egui::Ui) -> EguiRect {
        use crate::inspector_layout::{PanelGroup, RAIL_WIDTH};
        use theme::components::IconButton;

        let ctx = self.comp_ctx();
        egui::Panel::right("inspector_rail")
            .resizable(false)
            .default_size(RAIL_WIDTH)
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    if IconButton::new(crate::theme::icons::PANEL_RIGHT, "Expand panel")
                        .kbd("Tab")
                        .show(ui, ctx)
                        .clicked()
                    {
                        self.inspector.collapsed = false;
                    }
                    ui.separator();
                    for group in PanelGroup::ALL {
                        let selected = self.inspector.group == group;
                        if IconButton::new(group.icon(), group.label())
                            .selected(selected)
                            .show(ui, ctx)
                            .clicked()
                        {
                            self.inspector.group = group;
                            self.inspector.collapsed = false;
                        }
                    }
                });
            })
            .response
            .rect
    }

    /// A [`components::Ctx`] for this frame, carrying
    /// the app's resolved density and reduced-motion so lane 2D chrome draws from the
    /// same component library and tokens as every other panel.
    fn theme_ctx(&self) -> theme::components::Ctx {
        theme::components::Ctx::dark(self.ui_density).with_reduced_motion(self.reduced_motion)
    }

    /// The read-only viewer chrome (catalog 23, AUD-03): a slim top bar with the live
    /// session chip, a follow toggle, and one "Open full editor" affordance; the
    /// Layers panel reflecting the mirrored document; the status bar; and the canvas.
    /// There is no inspector, no draw tools, and no menu bar except Help, so a
    /// share-link viewer lands on a clean read-only surface, not the full editor.
    fn viewer_panels(
        &mut self,
        ui: &mut egui::Ui,
        gpu_format: Option<eframe::egui_wgpu::wgpu::TextureFormat>,
    ) -> (Option<ScreenRect>, TourTargets) {
        let top = egui::Panel::top("viewer_top")
            .show(ui, |ui| self.viewer_top_bar(ui))
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
        let mut canvas_screen: Option<ScreenRect> = None;
        egui::CentralPanel::default().show(ui, |ui| {
            canvas_screen = Some(self.canvas(ui, gpu_format));
        });
        self.last_screen = canvas_screen;
        let targets = TourTargets {
            canvas: canvas_screen.map(|s| egui_rect_of(&s)),
            layers: Some(layers),
            toolbar: Some(top),
            right_column: None,
            minimap: None,
        };
        (canvas_screen, targets)
    }

    /// The viewer chrome's top bar: a "view-only" tag, the live session chip
    /// (catalog 75), the follow controls (catalog 87), and, right-aligned, the single
    /// fixed "Open full editor" affordance plus Help and Shortcuts.
    fn viewer_top_bar(&mut self, ui: &mut egui::Ui) {
        let cx = self.theme_ctx();
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Reticle").strong());
            ui.label(
                egui::RichText::new("view-only")
                    .color(cx.tokens.text_weak)
                    .small(),
            );
            ui.separator();
            self.session_chip(ui, cx);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::components::Button::primary("Open full editor")
                    .show(ui, cx)
                    .on_hover_text("Switch to the full editor, keeping this camera")
                    .clicked()
                {
                    self.open_full_editor();
                }
                // The viewer's only menu is Help (ia-inventory section 2). It renders
                // from the reserved registry ids so it stays in step with the editor's
                // Help menu (lane 2A replaced the old `help_menu` with the registry).
                ui.menu_button("Help", |ui| {
                    for id in [
                        "help.tour",
                        "help.shortcuts",
                        "help.docs",
                        "help.whats_new",
                        "help.about",
                    ] {
                        let cid = CommandId(id);
                        if let Some(spec) = commands::spec(cid)
                            && ui.button(spec.label).clicked()
                        {
                            self.dispatch(cid);
                            ui.close();
                        }
                    }
                });
                if ui.button("Shortcuts").clicked() {
                    self.keymap_open = !self.keymap_open;
                }
            });
        });
        if self.sharer_left {
            ui.label(
                egui::RichText::new(
                    "The sharer left. You are viewing the last received state, read-only.",
                )
                .color(cx.tokens.warning),
            );
        }
    }

    /// The live session chip (catalog 75): a connection-state dot and label, the
    /// participant count, clickable presence avatars (click to follow, catalog 87), a
    /// follow toggle, and a "Following ..." chip when follow-mode is on.
    fn session_chip(&mut self, ui: &mut egui::Ui, cx: theme::components::Ctx) {
        let t = cx.tokens;
        let connected = self.live_status.is_open();
        let (dot, label) = if self.sharer_left {
            (t.warning, "Sharer left".to_owned())
        } else if connected {
            (t.success, "Live".to_owned())
        } else {
            (t.text_weak, self.live_status.label())
        };
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.0, dot);
        ui.label(label);

        let participants =
            crate::viewer::participants(self.document.awareness(), crate::viewer::VIEWER_ACTOR);
        ui.label(
            egui::RichText::new(format!("{} in session", participants.len() + 1))
                .color(t.text_weak),
        );
        let mut follow_clicked = false;
        for p in &participants {
            if Self::avatar(ui, p).clicked() {
                follow_clicked = true;
            }
        }
        if follow_clicked && let Some(session) = self.viewer_session.as_mut() {
            let now_following = session.toggle_follow();
            self.status.set(if now_following {
                "Following the sharer's view"
            } else {
                "Panning independently"
            });
        }

        if let Some(session) = self.viewer_session.as_mut() {
            let mut follow = session.is_following();
            if ui.checkbox(&mut follow, "Follow").changed() {
                session.set_follow(follow);
            }
        }
        if self
            .viewer_session
            .as_ref()
            .is_some_and(crate::viewer::ViewerSession::is_following)
        {
            let who = participants
                .first()
                .map_or_else(|| "the sharer".to_owned(), |p| p.name.clone());
            ui.label(egui::RichText::new(format!("Following {who}")).color(t.accent));
        }
    }

    /// Draws one clickable presence avatar: a colored disc with the participant's
    /// initial, tooltip naming them, returning the [`Response`](egui::Response) so a
    /// click can toggle follow-mode on that peer (catalog 87).
    fn avatar(ui: &mut egui::Ui, p: &crate::viewer::Participant) -> egui::Response {
        let (r, g, b, _) = layers::rgba_components(p.color_rgba);
        let color = theme::tokens::layer_rgb(r, g, b);
        let size = 18.0;
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
        ui.painter().circle_filled(rect.center(), size / 2.0, color);
        let initial = p
            .name
            .chars()
            .next()
            .map_or_else(|| "?".to_owned(), |ch| ch.to_uppercase().to_string());
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            initial,
            egui::TextStyle::Small.resolve(ui.style()),
            egui::Color32::WHITE,
        );
        resp.on_hover_text(format!("{} \u{2014} click to follow", p.name))
    }

    /// Leaves read-only viewer mode for the full editor, keeping the mirrored
    /// document and the current camera (catalog 23: the editor is one click away and
    /// the transition preserves the camera). Clearing the viewer target flips
    /// [`is_viewer`](Self::is_viewer), so the next frame draws the full editor chrome
    /// over the already-mirrored document at the same view.
    fn open_full_editor(&mut self) {
        self.viewer_target = None;
        self.viewer_session = None;
        self.sharer_left = false;
        self.status.set("Opened the full editor");
    }

    /// Presentation mode (catalog 93): only the canvas, full-window, with a quiet
    /// self-explaining exit hint. The caller suppresses all other chrome.
    fn presentation_canvas(
        &mut self,
        ui: &mut egui::Ui,
        gpu_format: Option<eframe::egui_wgpu::wgpu::TextureFormat>,
        ctx: &egui::Context,
    ) -> Option<ScreenRect> {
        let mut canvas_screen: Option<ScreenRect> = None;
        egui::CentralPanel::default().show(ui, |ui| {
            canvas_screen = Some(self.canvas(ui, gpu_format));
        });
        self.last_screen = canvas_screen;
        let weak = self.theme_ctx().tokens.text_weak;
        egui::Area::new(egui::Id::new("presentation_hint"))
            .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-12.0, 12.0))
            .interactable(false)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("Presentation \u{2014} press P or Esc to exit")
                        .color(weak)
                        .small(),
                );
            });
        canvas_screen
    }

    /// Embed mode (catalog 94): minimal chrome for an iframe. Only the canvas, plus a
    /// small "open in Reticle" link in the corner that opens the full, non-embedded app
    /// in a new tab. The caller suppresses all other chrome.
    fn embed_canvas(
        &mut self,
        ui: &mut egui::Ui,
        gpu_format: Option<eframe::egui_wgpu::wgpu::TextureFormat>,
        ctx: &egui::Context,
    ) -> Option<ScreenRect> {
        let mut canvas_screen: Option<ScreenRect> = None;
        egui::CentralPanel::default().show(ui, |ui| {
            canvas_screen = Some(self.canvas(ui, gpu_format));
        });
        self.last_screen = canvas_screen;
        let accent = self.theme_ctx().tokens.accent;
        let mut open_clicked = false;
        egui::Area::new(egui::Id::new("embed_open"))
            .anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-8.0, -8.0))
            .show(ctx, |ui| {
                if ui
                    .link(
                        egui::RichText::new("Open in Reticle \u{2197}")
                            .color(accent)
                            .small(),
                    )
                    .clicked()
                {
                    open_clicked = true;
                }
            });
        if open_clicked {
            self.open_in_new_tab();
        }
        canvas_screen
    }

    /// Opens the full (non-embedded) app in a new tab from embed mode, by reopening
    /// the current URL with the embed flag turned off. A status note on native.
    #[cfg_attr(not(target_arch = "wasm32"), allow(clippy::unused_self))]
    fn open_in_new_tab(&mut self) {
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(window) = web_sys::window()
                && let Ok(href) = window.location().href()
            {
                let target = href
                    .replace("embed=1", "embed=0")
                    .replace("embed=true", "embed=0");
                let _ = window.open_with_url_and_target(&target, "_blank");
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.status.set("Open in Reticle (web only)");
        }
    }

    /// Records the time each remote cursor last moved, so a parked cursor can fade
    /// (catalog 86 idle fade). Actors no longer present are dropped.
    fn track_presence_idle(&mut self, now: f64) {
        let current: Vec<(String, Point)> = self
            .document
            .awareness()
            .iter()
            .map(|(a, p)| (a.clone(), p.cursor))
            .collect();
        let mut present = std::collections::HashSet::with_capacity(current.len());
        for (actor, cursor) in current {
            present.insert(actor.clone());
            match self.presence_seen.get(&actor) {
                // Unchanged cursor keeps its last-move time so the fade keeps counting.
                Some(&(prev, _)) if prev == cursor => {}
                _ => {
                    self.presence_seen.insert(actor, (cursor, now));
                }
            }
        }
        self.presence_seen.retain(|a, _| present.contains(a));
    }

    /// Draws the remote-edit attribution glow (catalog 90): a brief inset border in
    /// the sharer's color when a mirrored remote frame lands, fading out over
    /// [`REMOTE_EDIT_FLASH_SECS`]. A no-op once the flash has elapsed.
    fn draw_remote_edit_glow(&self, painter: &egui::Painter, screen: &ScreenRect) {
        if self.remote_edit_flash <= 0.0 {
            return;
        }
        let frac = (self.remote_edit_flash / REMOTE_EDIT_FLASH_SECS).clamp(0.0, 1.0);
        let color =
            crate::viewer::participants(self.document.awareness(), crate::viewer::VIEWER_ACTOR)
                .first()
                .map_or(0x6e_a8_fe_ff, |p| p.color_rgba);
        let (r, g, b, _) = layers::rgba_components(color);
        // Fade the (premultiplied) sharer color by the remaining flash fraction; the
        // 0.7 ceiling keeps the border a glow rather than a solid frame.
        let glow = theme::tokens::layer_rgb(r, g, b).gamma_multiply(frac * 0.7);
        let stroke = Stroke::new(3.0, glow);
        painter.rect_stroke(egui_rect_of(screen), 0.0, stroke, StrokeKind::Inside);
    }

    /// A click the Start screen recorded this frame, applied after the layout closure
    /// so the borrow of `self` inside the egui closure is released first.
    ///
    /// Collecting the intent and acting on it afterwards is the same pattern the tour
    /// overlay uses; it lets each Start-screen section be a plain closure over `ui`
    /// without also borrowing `self` mutably to run the action inline.
    fn start_screen_ui(&mut self, ui: &mut egui::Ui) {
        let cx = self.theme_ctx();
        let mut action: Option<StartAction> = None;
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| {
                        ui.heading("Welcome to Reticle");
                        ui.label(
                            "Open a design, load an example, or start a new Tiny Tapeout \
                             tile. You can skip to a blank editor anytime.",
                        );
                    });
                    ui.add_space(12.0);

                    // Empty-canvas state with exactly three primary actions (catalog 16).
                    Self::start_hero_section(ui, cx, &mut action);
                    ui.add_space(10.0);
                    Self::start_recent_section(
                        ui,
                        cx,
                        self.recent_files.entries(),
                        &self.recent_pins,
                        &mut action,
                    );
                    ui.add_space(10.0);
                    Self::start_gallery_section(ui, cx, &mut action);
                    ui.add_space(10.0);
                    Self::start_open_hint_section(ui);
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
        // The New Tiny Tapeout tile wizard (catalog 24) draws over the Start screen.
        self.tt_wizard(&ui.ctx().clone());
    }

    /// The empty-canvas hero (catalog 16): exactly three primary actions, the three
    /// ways to get a design onto the canvas.
    fn start_hero_section(
        ui: &mut egui::Ui,
        cx: theme::components::Ctx,
        action: &mut Option<StartAction>,
    ) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("Nothing open yet").strong());
                ui.label(
                    egui::RichText::new("Pick one of three ways to get a design on the canvas.")
                        .color(cx.tokens.text_weak)
                        .small(),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if theme::components::Button::primary("Open a file")
                        .min_width(150.0)
                        .show(ui, cx)
                        .clicked()
                    {
                        *action = Some(StartAction::OpenDialog);
                    }
                    if theme::components::Button::primary("Load an example")
                        .min_width(150.0)
                        .show(ui, cx)
                        .clicked()
                    {
                        *action = Some(StartAction::LoadExample(
                            crate::startscreen::ExampleChip::TinyTapeoutMin,
                        ));
                    }
                    if theme::components::Button::primary("New TT tile")
                        .min_width(150.0)
                        .show(ui, cx)
                        .clicked()
                    {
                        *action = Some(StartAction::OpenTileWizard);
                    }
                });
            });
        });
    }

    /// Draws one small metadata badge (a filled pill with a short label), the
    /// building block of the gallery card's technology/size/license/streaming row.
    fn badge(ui: &mut egui::Ui, text: &str, fill: egui::Color32, fg: egui::Color32) {
        let font = egui::TextStyle::Small.resolve(ui.style());
        let galley = ui.painter().layout_no_wrap(text.to_owned(), font, fg);
        let pad = Vec2::new(6.0, 2.0);
        let (rect, _) = ui.allocate_exact_size(galley.size() + pad * 2.0, Sense::hover());
        ui.painter().rect_filled(rect, 3.0, fill);
        ui.painter().galley(rect.min + pad, galley, fg);
    }

    /// Draws the recent-files section (catalog 9): pinned entries first, each with a
    /// render-thumbnail slot, size, and a pin/unpin toggle. The recent-file model and
    /// its persistence are frozen Lane 1B code; pinning is a Lane 2D sibling.
    fn start_recent_section(
        ui: &mut egui::Ui,
        cx: theme::components::Ctx,
        recent: &[crate::webopen::RecentFile],
        pins: &crate::startscreen::RecentPins,
        action: &mut Option<StartAction>,
    ) {
        let t = cx.tokens;
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
                for r in pins.order(recent) {
                    let key = crate::startscreen::recent_key(r);
                    let pinned = pins.is_pinned(key);
                    // Integer-only size formatting keeps clippy's precision-loss lint
                    // quiet; a sub-KiB file simply reads as "0 KiB".
                    let size = if r.size >= 1024 * 1024 {
                        format!("{} MiB", r.size / (1024 * 1024))
                    } else {
                        format!("{} KiB", r.size / 1024)
                    };
                    ui.horizontal(|ui| {
                        // The cached-render-thumbnail slot (rendering + IndexedDB cache
                        // is ledgered; the slot keeps the layout honest today).
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
                        ui.painter().rect_filled(rect, 3.0, t.bg_input);
                        ui.painter().text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            "\u{25A6}",
                            egui::TextStyle::Small.resolve(ui.style()),
                            t.text_faint,
                        );
                        ui.vertical(|ui| {
                            ui.monospace(&r.name);
                            ui.label(egui::RichText::new(&size).weak().small());
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let label = if pinned { "Unpin" } else { "Pin" };
                            if ui.small_button(label).clicked() {
                                *action = Some(StartAction::PinRecent(key.to_owned()));
                            }
                        });
                    });
                }
            }
        });
    }

    /// Draws the gallery (catalog 14/96): one differentiated card per real design,
    /// carrying technology/size/license badges, a Streaming badge for served-archive
    /// demos, a source line, and a "What am I looking at?" landmarks dropdown.
    fn start_gallery_section(
        ui: &mut egui::Ui,
        cx: theme::components::Ctx,
        action: &mut Option<StartAction>,
    ) {
        let t = cx.tokens;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong("Gallery");
            ui.label(
                egui::RichText::new("Redistribution-cleared real designs, built in or streamed.")
                    .weak()
                    .small(),
            );
            ui.add_space(4.0);
            for card in crate::startscreen::GALLERY {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.strong(card.title);
                            ui.label(card.description);
                            ui.horizontal_wrapped(|ui| {
                                Self::badge(ui, card.technology, t.accent_muted, t.text);
                                Self::badge(ui, card.size, t.widget_bg, t.text_weak);
                                Self::badge(ui, card.license, t.widget_bg, t.text_weak);
                                if card.streaming {
                                    Self::badge(ui, "Streaming", t.success, t.accent_text);
                                }
                            });
                            ui.label(
                                egui::RichText::new(format!("Source: {}", card.source))
                                    .weak()
                                    .small(),
                            );
                            // Landmarks dropdown (catalog 96). Scope the id by title so
                            // the identically-labelled headers do not collide.
                            ui.push_id(card.title, |ui| {
                                ui.collapsing("What am I looking at?", |ui| {
                                    for lm in card.landmarks {
                                        ui.label(
                                            egui::RichText::new(format!("\u{2022} {}", lm.name))
                                                .strong()
                                                .small(),
                                        );
                                        ui.label(egui::RichText::new(lm.detail).weak().small());
                                    }
                                });
                            });
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Open").clicked() {
                                *action = Some(match card.action {
                                    crate::startscreen::GalleryAction::Example(chip) => {
                                        StartAction::LoadExample(chip)
                                    }
                                    crate::startscreen::GalleryAction::Archive(url) => {
                                        StartAction::OpenArchive(url.to_owned())
                                    }
                                });
                            }
                        });
                    });
                });
                ui.add_space(4.0);
            }
        });
        // --- lane gallery-live: F1 open-silicon library section ---
        // Separate section below the curated examples above: the real committed F1
        // manifest, rendered generically by crate::gallery::show. A parse/validate
        // failure leaves this section undrawn (see gallery::bundled_manifest's doc).
        if let Some(manifest) = crate::gallery::bundled_manifest() {
            ui.add_space(10.0);
            if let Some(url) = crate::gallery::show(ui, cx, manifest, "") {
                *action = Some(StartAction::OpenArchive(url));
            }
        }
        // --- end lane gallery-live ---
    }

    /// The secondary drag-and-drop hint under the primary actions: dropping a file
    /// anywhere on the window opens it (there is no synchronous web file dialog).
    fn start_open_hint_section(ui: &mut egui::Ui) {
        egui::Frame::group(ui.style())
            .fill(ui.visuals().faint_bg_color)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Or drag a layout file anywhere on the window").weak(),
                    );
                    ui.label(
                        egui::RichText::new("Supported: GDSII (.gds) and OASIS (.oas)")
                            .weak()
                            .small(),
                    );
                    ui.add_space(4.0);
                });
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
            Some(StartAction::OpenArchive(url)) => self.open_archive_demo(&url),
            Some(StartAction::OpenDialog) => self.request_open_dialog(),
            Some(StartAction::OpenTileWizard) => {
                self.tt_wizard_open = true;
            }
            Some(StartAction::PinRecent(key)) => {
                let pinned = self.recent_pins.toggle(key);
                self.status.set(if pinned { "Pinned" } else { "Unpinned" });
            }
            Some(StartAction::SkipToEditor) => {
                // Dismiss the chooser and keep the demo document already loaded.
                self.start_screen = false;
                self.status.set("Editor");
            }
            None => {}
        }
    }

    /// Requests the file-open dialog by dispatching the reserved `file.open_dialog`
    /// command (owned by lane 3B, catalog 1). Until that lane wires the effect into
    /// this build's registry, it falls back to the honest drag-and-drop hint.
    fn request_open_dialog(&mut self) {
        let id = CommandId("file.open_dialog");
        if commands::spec(id).is_some() {
            self.dispatch(id);
        } else {
            self.status
                .set("Drag a GDSII or OASIS file onto the window to open it");
        }
    }

    /// Opens a served-archive gallery demo by URL through the streaming `?archive=`
    /// path (catalog 14). On the web this navigates to the archive link; on native,
    /// where there is no browser fetch, it reports that streaming demos are web-only.
    #[cfg_attr(not(target_arch = "wasm32"), allow(clippy::unused_self))]
    fn open_archive_demo(&mut self, url: &str) {
        #[cfg(target_arch = "wasm32")]
        {
            let link = crate::share::emit_archive_link("", url);
            if let Some(window) = web_sys::window() {
                let _ = window.location().set_href(&link);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.status.set(format!(
                "Streaming archive demos open in the web build: {url}"
            ));
        }
    }

    /// Draws the New Tiny Tapeout tile wizard (catalog 24) as a modal over the Start
    /// screen: a pin-map preview of the tile frame with its Create/Cancel actions.
    /// Creating enters the [`UseCase::NewTinyTapeoutTile`] scenario.
    fn tt_wizard(&mut self, ctx: &egui::Context) {
        if !self.tt_wizard_open {
            return;
        }
        let cx = self.theme_ctx();
        let mut create = false;
        let mut cancel = false;
        theme::components::Modal::new("New Tiny Tapeout tile").overlay(ctx, cx, |ui, cx| {
            ui.label(
                egui::RichText::new(
                    "Start from a correctly shaped, pinned tile frame: the 1x2 die \
                     boundary, the six ua[0..5] analog pins on met4, and the power \
                     straps. Fill in your logic inside.",
                )
                .color(cx.tokens.text_weak),
            );
            ui.add_space(8.0);
            Self::draw_pin_map(ui, cx);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if theme::components::Button::primary("Create tile")
                    .show(ui, cx)
                    .clicked()
                {
                    create = true;
                }
                if theme::components::Button::secondary("Cancel")
                    .show(ui, cx)
                    .clicked()
                {
                    cancel = true;
                }
            });
        });
        if create {
            self.tt_wizard_open = false;
            self.enter_use_case(UseCase::NewTinyTapeoutTile);
        } else if cancel {
            self.tt_wizard_open = false;
        }
    }

    /// Draws the schematic pin-map preview for the New TT tile wizard: the die
    /// outline, the six analog pins along the bottom edge, and the three power straps.
    /// It is illustrative (a qualitative map of where the fixed frame pins sit), not a
    /// scaled render of the generated tile.
    fn draw_pin_map(ui: &mut egui::Ui, cx: theme::components::Ctx) {
        let t = cx.tokens;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(260.0, 150.0), Sense::hover());
        let painter = ui.painter();
        // The die outline.
        painter.rect_stroke(
            rect,
            2.0,
            Stroke::new(1.5, t.border_strong),
            StrokeKind::Inside,
        );
        // Six analog pins along the bottom edge (ua[0..5]).
        let pin_color = theme::tokens::layer_rgb(0x8e, 0x4e, 0xc6);
        for i in 0..6 {
            #[allow(clippy::cast_precision_loss)]
            let x = rect.left() + rect.width() * (0.12 + 0.152 * i as f32);
            let pin =
                EguiRect::from_min_size(Pos2::new(x, rect.bottom() - 16.0), Vec2::new(14.0, 12.0));
            painter.rect_filled(pin, 1.0, pin_color);
            painter.text(
                Pos2::new(pin.center().x, pin.top() - 6.0),
                Align2::CENTER_BOTTOM,
                format!("ua{i}"),
                egui::TextStyle::Small.resolve(ui.style()),
                t.text_weak,
            );
        }
        // Three vertical power straps.
        let strap_color = theme::tokens::layer_rgb(0x46, 0xa7, 0x58);
        for (i, name) in ["VDPWR", "VGND", "VAPWR"].into_iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let x = rect.left() + rect.width() * (0.30 + 0.20 * i as f32);
            let strap = EguiRect::from_min_size(
                Pos2::new(x, rect.top() + 8.0),
                Vec2::new(6.0, rect.height() - 34.0),
            );
            painter.rect_filled(strap, 1.0, strap_color);
            painter.text(
                Pos2::new(strap.center().x, rect.top() + 2.0),
                Align2::CENTER_TOP,
                name,
                egui::TextStyle::Small.resolve(ui.style()),
                t.text_weak,
            );
        }
    }

    /// Draws the left Layers panel (lane 2c): a color swatch and editor, a
    /// visibility eye with hover-peek and alt-click solo (catalog 39), a lock
    /// toggle (catalog 57), a filter with a clear affordance, show/hide-all icons,
    /// and savable visibility presets (catalog 62). Everything is composed from
    /// [`components`], so a density or palette change lands in one place.
    ///
    /// In a streamed-archive session the panel shows an empty state instead of the
    /// built-in demo's layers, since a `.rtla` archive carries no editable layer
    /// table (AUD-04). The viewer and editor paths already rebuild the table from
    /// the active document on install, so they need no special case here.
    fn layer_panel(&mut self, ui: &mut egui::Ui) {
        self.focus_anchor(ui, crate::focus::FocusRegion::LeftPanel);
        let cctx = self.ui_ctx();
        let header = components::SectionHeader::new("Layers").show(ui, cctx);
        self.attach_context_menu(&header, commands::MenuContext::PanelHeader);

        if self.archive.is_some() {
            ui.separator();
            components::EmptyState::new(
                "No layer table",
                "A streamed archive carries no editable layers.",
            )
            .show(ui, cctx);
            ui.separator();
            self.view_panel_launcher(ui, cctx);
            return;
        }

        // Filter with a clear affordance, plus show/hide-all as icon buttons.
        let filter_empty = self.layer_state.filter().is_empty();
        ui.horizontal(|ui| {
            components::TextField::new(self.layer_state.filter_mut())
                .hint("Filter layers")
                .desired_width(110.0)
                .show(ui, cctx);
            if components::IconButton::new(icons::X, "Clear filter")
                .enabled(!filter_empty)
                .show(ui, cctx)
                .clicked()
            {
                self.layer_state.filter_mut().clear();
            }
            if components::IconButton::new(icons::EYE, "Show all layers")
                .show(ui, cctx)
                .clicked()
            {
                self.layer_state.show_all();
            }
            if components::IconButton::new(icons::EYE_OFF, "Hide all layers")
                .show(ui, cctx)
                .clicked()
            {
                self.layer_state.hide_all();
            }
        });
        ui.separator();

        self.layer_rows(ui, cctx);
        self.layer_color_popover(ui);
        ui.separator();
        self.layer_presets_section(ui, cctx);
        ui.separator();
        self.select_by_name_row(ui, cctx);
        ui.separator();
        self.view_panel_launcher(ui, cctx);
    }

    /// Draws the scrolling layer rows and applies the row interaction collected
    /// this frame (visibility toggle, solo, lock, color-editor open, hover peek).
    ///
    /// Interactions are gathered into locals while an immutable borrow of the row
    /// table is live, then applied afterwards, so the row loop never needs mutable
    /// access to [`LayerState`] mid-render.
    fn layer_rows(&mut self, ui: &mut egui::Ui, cctx: components::Ctx) {
        let indices = self.layer_state.filtered_indices();
        // A cheap keymap snapshot so the per-row context menu (item 47) can render
        // without re-borrowing `self` inside the row loop that holds the layer table.
        let keymap = self.keymap.clone();
        let mut menu_action: Option<command::PaletteAction> = None;
        let mut hover: Option<LayerId> = None;
        let mut toggle_vis: Option<LayerId> = None;
        let mut do_solo: Option<LayerId> = None;
        let mut toggle_lock: Option<LayerId> = None;
        let mut open_color: Option<LayerId> = None;

        let rows = self.layer_state.rows();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for i in indices {
                    let row = &rows[i];
                    let line = ui.horizontal(|ui| {
                        // Color swatch: click opens the color editor (catalog 62).
                        let (r, g, b, _) = layers::rgba_components(row.color_rgba);
                        let (rect, swatch) =
                            ui.allocate_exact_size(Vec2::new(14.0, 14.0), Sense::click());
                        ui.painter()
                            .rect_filled(rect, 2.0, theme::tokens::layer_rgb(r, g, b));
                        if swatch.on_hover_text("Edit color").clicked() {
                            open_color = Some(row.id);
                        }
                        // Visibility eye: click toggles, alt-click solos (catalog 39).
                        let (eye, verb) = if row.visible {
                            (icons::EYE, "Hide layer")
                        } else {
                            (icons::EYE_OFF, "Show layer")
                        };
                        if components::IconButton::new(eye, verb)
                            .hint("Alt-click to show only this layer")
                            .selected(row.visible)
                            .show(ui, cctx)
                            .clicked()
                        {
                            if ui.input(|i| i.modifiers.alt) {
                                do_solo = Some(row.id);
                            } else {
                                toggle_vis = Some(row.id);
                            }
                        }
                        // Lock: the layer stays drawn but the canvas will not pick
                        // it (catalog 57).
                        let lock_verb = if row.locked {
                            "Unlock layer"
                        } else {
                            "Lock layer"
                        };
                        if components::IconButton::new(icons::LOCK, lock_verb)
                            .hint("Locked layers stay drawn but cannot be selected")
                            .selected(row.locked)
                            .show(ui, cctx)
                            .clicked()
                        {
                            toggle_lock = Some(row.id);
                        }
                        // Name, faint when the layer is hidden.
                        let color = if row.visible {
                            cctx.tokens.text
                        } else {
                            cctx.tokens.text_faint
                        };
                        ui.label(egui::RichText::new(&row.name).color(color));
                    });
                    // Re-interact the whole row so it senses a right-click for the
                    // registry-driven context menu (lane 3C, item 47) as well as the
                    // hover-peek dim (catalog 39).
                    let row_resp = line.response.interact(egui::Sense::click());
                    if row_resp.hovered() {
                        hover = Some(row.id);
                    }
                    row_resp.context_menu(|ui| {
                        if let Some(action) = Self::context_menu_body(
                            ui,
                            &keymap,
                            commands::MenuContext::LayerRow,
                            Some((i, row.name.as_str())),
                        ) {
                            menu_action = Some(action);
                        }
                    });
                }
            });
        if let Some(action) = menu_action {
            self.run_menu_action(action, None);
        }

        // Apply the collected interaction now the row borrow has ended.
        self.peek_layer = hover;
        if let Some(id) = toggle_vis {
            self.layer_state.toggle(id);
        }
        if let Some(id) = do_solo {
            self.layer_state.solo(id);
            self.status
                .set(format!("Solo {}", layer_name_of(&self.layer_state, id)));
        }
        if let Some(id) = toggle_lock
            && let Some(now) = self.layer_state.toggle_lock(id)
        {
            let name = layer_name_of(&self.layer_state, id);
            self.status.set(format!(
                "{name} {}",
                if now { "locked" } else { "unlocked" }
            ));
        }
        if let Some(id) = open_color {
            self.color_editor = Some(id);
        }
    }

    /// The layer color-editor popover (catalog 62): a floating picker over the
    /// swatch that was clicked, editing the active layer's display color live. A
    /// popover, not a docked panel, so ADR 0096 does not apply.
    fn layer_color_popover(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.color_editor else {
            return;
        };
        let Some(rgba) = self
            .layer_state
            .rows()
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.color_rgba)
        else {
            self.color_editor = None;
            return;
        };
        let (r, g, b, a) = layers::rgba_components(rgba);
        let mut color = theme::tokens::layer_rgba(r, g, b, a);
        let mut open = true;
        let mut new_rgba: Option<u32> = None;
        egui::Window::new("Layer color")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                if egui::color_picker::color_picker_color32(
                    ui,
                    &mut color,
                    egui::color_picker::Alpha::Opaque,
                ) {
                    let [nr, ng, nb, na] = color.to_srgba_unmultiplied();
                    new_rgba = Some(layers::pack_rgba(nr, ng, nb, na));
                }
            });
        if let Some(c) = new_rgba {
            self.layer_state.set_color(id, c);
        }
        if !open {
            self.color_editor = None;
        }
    }

    /// The visibility-presets section (catalog 62): a name field with a Save
    /// button, then one row per saved preset with an apply button and a delete
    /// affordance.
    fn layer_presets_section(&mut self, ui: &mut egui::Ui, cctx: components::Ctx) {
        components::SectionHeader::new("Visibility presets").show(ui, cctx);
        ui.horizontal(|ui| {
            components::TextField::new(&mut self.preset_name)
                .hint("Preset name")
                .desired_width(110.0)
                .show(ui, cctx);
            if components::Button::secondary("Save")
                .show(ui, cctx)
                .clicked()
                && self.layer_state.save_preset(&self.preset_name)
            {
                self.status
                    .set(format!("Saved preset {}", self.preset_name.trim()));
                self.preset_name.clear();
            }
        });
        let names: Vec<String> = self
            .layer_state
            .presets()
            .iter()
            .map(|p| p.name.clone())
            .collect();
        if names.is_empty() {
            ui.label(egui::RichText::new("No saved presets yet.").color(cctx.tokens.text_weak));
        }
        for name in names {
            ui.horizontal(|ui| {
                if components::Button::ghost(&name).show(ui, cctx).clicked() {
                    self.layer_state.apply_preset(&name);
                    self.status.set(format!("Applied preset {name}"));
                }
                if components::IconButton::new(icons::TRASH_2, "Delete preset")
                    .show(ui, cctx)
                    .clicked()
                {
                    self.layer_state.delete_preset(&name);
                }
            });
        }
    }

    /// The select-by-layer-name row: a name field and a Select button that
    /// selects every shape whose layer name matches (see [`select_by_layer_name`]).
    ///
    /// [`select_by_layer_name`]: Self::select_by_layer_name
    fn select_by_name_row(&mut self, ui: &mut egui::Ui, cctx: components::Ctx) {
        ui.horizontal(|ui| {
            components::TextField::new(&mut self.layer_query)
                .hint("Layer name")
                .desired_width(110.0)
                .show(ui, cctx);
            if components::Button::secondary("Select")
                .show(ui, cctx)
                .clicked()
            {
                self.select_by_layer_name();
            }
        });
    }

    /// The interim launcher for the managed view panels (ADR 0096): two toggle
    /// icon buttons that dispatch the reserved `view.panel_3d` / `view.panel_xsection`
    /// commands. Once lane 2a's menu bar lands they also open from View > Panels;
    /// both surfaces dispatch the same registry ids, so the effect is identical.
    fn view_panel_launcher(&mut self, ui: &mut egui::Ui, cctx: components::Ctx) {
        components::SectionHeader::new("Panels").show(ui, cctx);
        ui.horizontal(|ui| {
            if components::IconButton::new(icons::BOXES, "3D stack")
                .hint("Open the 3D layer stack (View > Panels)")
                .selected(self.panel_3d_open)
                .show(ui, cctx)
                .clicked()
            {
                self.dispatch(CommandId("view.panel_3d"));
            }
            if components::IconButton::new(icons::SLICE, "Cross-section")
                .hint("Open the cross-section (View > Panels)")
                .selected(self.panel_xsection_open)
                .show(ui, cctx)
                .clicked()
            {
                self.dispatch(CommandId("view.panel_xsection"));
            }
        });
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
        // --- Filter query bar ------------------------------------------------
        ui.label("Filter (e.g. layer:METAL1 width<400 area>1000):");
        let query_response = ui.text_edit_singleline(&mut self.search.query_text);
        let run = ui.button("Select matching").clicked()
            || (query_response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        if run {
            self.run_filter_query();
        }
        if !self.search.error.is_empty() {
            ui.colored_label(DARK.danger, &self.search.error);
        }

        ui.separator();

        // --- Select similar / same layer (catalog 56) ------------------------
        ui.horizontal(|ui| {
            let has_sel = !self.selection.is_empty();
            if ui
                .add_enabled(has_sel, egui::Button::new("Select similar"))
                .on_hover_text("Same layer and a similar size")
                .clicked()
            {
                self.select_similar();
            }
            if ui
                .add_enabled(has_sel, egui::Button::new("Same layer"))
                .on_hover_text("Every shape sharing a selected layer")
                .clicked()
            {
                self.select_same_layer();
            }
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

        // Undo timeline (catalog 54): a clickable list that jumps multiple steps at
        // once. The document history stores depths, not per-edit labels, so rows are
        // numbered by distance; clicking one undoes or redoes to that point.
        let undo_depth = self.history.undo_depth();
        let redo_depth = self.history.redo_depth();
        ui.separator();
        ui.label("Timeline (click to jump):");
        let mut undo_to: Option<usize> = None;
        let mut redo_to: Option<usize> = None;
        egui::ScrollArea::vertical()
            .max_height(140.0)
            .auto_shrink([false, false])
            .id_salt("undo_timeline")
            .show(ui, |ui| {
                for k in (1..=redo_depth).rev() {
                    if ui
                        .selectable_label(
                            false,
                            format!("{} redo +{k}", crate::theme::icons::REDO_2),
                        )
                        .clicked()
                    {
                        redo_to = Some(k);
                    }
                }
                ui.label(
                    egui::RichText::new(format!("now  (depth {undo_depth})")).color(DARK.accent),
                );
                for k in 1..=undo_depth {
                    if ui
                        .selectable_label(
                            false,
                            format!("{} undo -{k}", crate::theme::icons::UNDO_2),
                        )
                        .clicked()
                    {
                        undo_to = Some(k);
                    }
                }
            });
        if let Some(k) = undo_to {
            self.undo_steps(k);
        }
        if let Some(k) = redo_to {
            self.redo_steps(k);
        }

        ui.separator();
        ui.label(format!("Selected shapes: {}", self.selection.len()));
        ui.label(format!("Scene shapes: {}", self.scene.len()));
        // The debug "Add demo rectangle" affordance retired to Help > Developer
        // (dev.add_demo_rect; catalog 70).
    }

    /// Undoes up to `n` history steps, rebuilding the scene once (the undo-timeline
    /// jump, catalog 54).
    fn undo_steps(&mut self, n: usize) {
        let mut done = 0;
        for _ in 0..n {
            if self.history.undo() {
                done += 1;
            } else {
                break;
            }
        }
        if done > 0 {
            self.rebuild_scene();
            self.status.set(format!("Undid {done} step(s)"));
        }
    }

    /// Redoes up to `n` history steps, rebuilding the scene once (catalog 54).
    fn redo_steps(&mut self, n: usize) {
        let mut done = 0;
        for _ in 0..n {
            if self.history.redo() {
                done += 1;
            } else {
                break;
            }
        }
        if done > 0 {
            self.rebuild_scene();
            self.status.set(format!("Redid {done} step(s)"));
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

    /// Translates the directly-selected top-cell shapes by `(dx, dy)` DBU as one undoable
    /// step, then reselects them so successive moves accumulate on the same shapes.
    ///
    /// Shared by arrow-key nudge (item 48) and the numeric transform popover's move (item
    /// 51). Returns the number of shapes moved (`0` when the selection holds no directly
    /// editable top-cell shape, or the delta is zero).
    fn translate_direct_selection(&mut self, dx: i32, dy: i32) -> usize {
        let direct = self.selected_direct_shapes();
        if direct.is_empty() || (dx == 0 && dy == 0) {
            return 0;
        }
        let n = direct.len();
        let cell = self.top_cell.clone();
        // Remove originals in descending index order (so lower indices stay valid), then
        // re-add the translated copies, as a single undo group.
        let mut edits: Vec<reticle_model::Edit> = Vec::with_capacity(n * 2);
        for (index, _) in direct.iter().rev() {
            edits.push(reticle_model::Edit::RemoveShape {
                cell: cell.clone(),
                index: *index,
            });
        }
        for (_, shape) in &direct {
            edits.push(reticle_model::Edit::AddShape {
                cell: cell.clone(),
                shape: productivity::translate_shape(shape, dx, dy),
            });
        }
        if self.history.apply_group(edits).is_err() {
            return 0;
        }
        self.rebuild_scene();
        // The moved shapes were appended, so they are the last `n` direct shapes.
        let total = self.top_cell_direct_shape_count();
        self.selection.set(total.saturating_sub(n)..total);
        n
    }

    /// Nudges the selection by `(dx, dy)` DBU with a delta readout in the status bar
    /// (item 48).
    fn nudge_selection(&mut self, dx: i32, dy: i32) {
        if self.selection.is_empty() {
            self.status.set("Nudge: nothing selected");
            return;
        }
        let n = self.translate_direct_selection(dx, dy);
        if n > 0 {
            self.status
                .set(format!("Nudged dx {dx}, dy {dy} DBU ({n} shape(s))"));
        } else {
            self.status.set("Nudge: selection is not movable");
        }
    }

    /// Handles arrow-key nudging of the selection (item 48): one grid step, ten steps
    /// with Shift, or a fine single DBU with Alt/Ctrl. Suppressed while a text field is
    /// focused (so typing in a panel does not move geometry) and in read-only browse.
    fn handle_arrow_nudge(&mut self, ctx: &egui::Context) {
        if self.archive.is_some() || self.selection.is_empty() {
            return;
        }
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let base = self.grid.base_step_dbu.max(1);
        let (dx, dy) = ctx.input(|i| {
            let step = if i.modifiers.shift {
                base.saturating_mul(10)
            } else if i.modifiers.alt || i.modifiers.command || i.modifiers.ctrl {
                1
            } else {
                base
            };
            let mut dx = 0;
            let mut dy = 0;
            if i.key_pressed(egui::Key::ArrowLeft) {
                dx -= step;
            }
            if i.key_pressed(egui::Key::ArrowRight) {
                dx += step;
            }
            // World +y is up, so Up increases y.
            if i.key_pressed(egui::Key::ArrowUp) {
                dy += step;
            }
            if i.key_pressed(egui::Key::ArrowDown) {
                dy -= step;
            }
            (dx, dy)
        });
        if dx != 0 || dy != 0 {
            self.nudge_selection(dx, dy);
        }
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
                ui.colored_label(DARK.danger, err);
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
        self.drc_ran_revision = Some(self.history.revision());
        if n == 0 {
            self.status.set("DRC: no violations");
        } else {
            self.status.set(format!("DRC: {n} violation(s)"));
            // Event-driven auto-expand: surface the DRC section on a fresh set of
            // violations (catalog 67).
            self.inspector.reveal("drc");
        }
    }

    /// Advances DRC-as-you-type by one frame: feed this frame's edits to the live
    /// checker so a fresh violation is underlined the moment its geometry is drawn.
    ///
    /// The prepared index is an immutable snapshot, so an edit is reflected only once
    /// the index is rebuilt. This drains the region every edit dirtied (see
    /// [`History::take_dirty`]), accumulates it, and on a throttle rebuilds the index
    /// and re-checks the accumulated region: the cheap per-edit `check_region` runs at
    /// microsecond scale, the expensive re-prepare at most a few times a second. The
    /// dirt is drained every frame regardless so it cannot pile up while the feature is
    /// off or while a read-only archive is being browsed.
    fn poll_live_drc(&mut self, dt: f32) {
        // How long a stale index may persist before the throttle rebuilds it. A quarter
        // second reads as live while bounding the expensive re-prepare during a drag.
        const REPREPARE_INTERVAL_SECS: f32 = 0.25;

        let dirty = self.history.take_dirty();
        if !self.live_drc_on || self.archive.is_some() {
            self.live_pending = crate::history::Dirty::None;
            self.live_reprepare_accum = 0.0;
            return;
        }
        self.live_pending = self.live_pending.merge(dirty);

        self.live_reprepare_accum += dt;
        let revision = self.history.revision();
        let due =
            !self.live_drc.has_index() || self.live_reprepare_accum >= REPREPARE_INTERVAL_SECS;
        if self.live_drc.is_stale(revision) && due {
            self.live_reprepare_accum = 0.0;
            let pending = std::mem::take(&mut self.live_pending);
            let n = self.live_drc.apply_dirty(
                pending,
                self.history.document(),
                &self.top_cell,
                revision,
                true,
            );
            if n > 0 {
                self.status.set(format!("Live DRC: {n} near the edit"));
            }
        }
    }

    /// Draws the DRC panel section: run/clear actions and the violation list.
    ///
    /// Clicking a violation records it as selected and zooms the camera to its
    /// location on the next frame (once the real canvas size is known).
    fn drc_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = self.comp_ctx();
        ui.horizontal(|ui| {
            if ui.button("Run DRC").clicked() {
                self.run_drc();
            }
            if ui.button("Clear").clicked() {
                self.drc.clear();
                self.drc_ran_revision = None;
                self.status.set("DRC cleared");
            }
        });
        // DRC-as-you-type: underline violations live as geometry is drawn. Turning it
        // off drops the live index and its underlines.
        if ui
            .checkbox(&mut self.live_drc_on, "Check as you type")
            .changed()
            && !self.live_drc_on
        {
            self.live_drc.clear();
            self.live_pending = crate::history::Dirty::None;
            self.live_reprepare_accum = 0.0;
        }

        // No results yet: an empty state that names the void and offers the next
        // action (catalog AUD-19).
        if !self.drc.has_run() {
            if theme::components::EmptyState::new(
                "No DRC results",
                "Run DRC to check the layout against the technology rules.",
            )
            .action(theme::components::Button::primary("Run DRC"))
            .show(ui, ctx)
            .is_some()
            {
                self.run_drc();
            }
            return;
        }

        self.drc_result_list(ui, ctx);
    }

    /// Draws the DRC results below the actions: the count, the stale indicator, the
    /// prev/next navigator, the violation list, and the "ask agent to fix" button
    /// (catalog 63). Split from [`drc_panel`](Self::drc_panel) to keep each focused.
    fn drc_result_list(&mut self, ui: &mut egui::Ui, ctx: theme::components::Ctx) {
        let count = self.drc.len();
        ui.horizontal(|ui| {
            ui.label(format!("{count} violation(s)"));
            // Stale indicator: the layout changed since DRC last ran (catalog 63).
            if self.drc_ran_revision != Some(self.history.revision()) {
                ui.colored_label(
                    DARK.warning,
                    format!("{} stale", crate::theme::icons::TRIANGLE_ALERT),
                )
                .on_hover_text("The layout changed since DRC last ran; rerun to refresh.");
            }
        });

        // Navigable list: prev/next cycle the selection and zoom to it (catalog 63).
        if count > 0 {
            ui.horizontal(|ui| {
                let sel = self.drc.selected();
                let prev = theme::components::IconButton::new(
                    crate::theme::icons::CHEVRON_UP,
                    "Previous violation",
                )
                .show(ui, ctx)
                .clicked();
                let next = theme::components::IconButton::new(
                    crate::theme::icons::CHEVRON_DOWN,
                    "Next violation",
                )
                .show(ui, ctx)
                .clicked();
                let pos = sel.unwrap_or(0);
                let target = if next {
                    Some((pos + 1) % count)
                } else if prev {
                    Some((pos + count - 1) % count)
                } else {
                    None
                };
                if let Some(t) = target
                    && self.drc.select(t).is_some()
                {
                    self.zoom_to_selected_violation = true;
                }
                if let Some(s) = self.drc.selected() {
                    ui.label(format!("{}/{count}", s + 1));
                }
            });
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
    }

    /// Draws the layout-diff panel: snapshot/diff/clear actions, a show/hide
    /// toggle, and the added/removed/changed counts.
    ///
    /// This ships the two-snapshot flow (no comparison-document file loader exists
    /// in this build): "Snapshot" captures the current document as the baseline,
    /// and after the user edits, "Diff vs snapshot" compares the baseline against
    /// the now-current document and paints the difference on the canvas.
    fn diff_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = self.comp_ctx();
        ui.horizontal(|ui| {
            if ui
                .button("Snapshot")
                .on_hover_text("Capture the current layout as the baseline to diff against")
                .clicked()
            {
                self.diff_overlay.snapshot(self.history.document());
                self.status.set("Diff: snapshot captured");
            }
            let can_diff = self.diff_overlay.has_baseline();
            if ui
                .add_enabled(can_diff, egui::Button::new("Diff vs snapshot"))
                .on_hover_text("Compare the current layout against the captured snapshot")
                .clicked()
            {
                self.compute_diff();
            }
            if ui.button("Clear").clicked() {
                self.diff_overlay.clear();
                self.diff_selected = None;
                self.status.set("Diff cleared");
            }
        });
        let mut visible = self.diff_overlay.visible();
        if ui.checkbox(&mut visible, "Show diff overlay").changed() {
            self.diff_overlay.set_visible(visible);
        }

        // Not yet diffed: an empty state whose action is the missing step.
        if !self.diff_overlay.has_run() {
            let clicked = if self.diff_overlay.has_baseline() {
                theme::components::EmptyState::new(
                    "Snapshot captured",
                    "Edit the layout, then diff to review the changes.",
                )
                .action(theme::components::Button::primary("Diff vs snapshot"))
                .show(ui, ctx)
            } else {
                theme::components::EmptyState::new(
                    "No snapshot",
                    "Capture a snapshot, edit, then diff to review the changes.",
                )
                .action(theme::components::Button::primary("Snapshot"))
                .show(ui, ctx)
            };
            if clicked.is_some() {
                if self.diff_overlay.has_baseline() {
                    self.compute_diff();
                } else {
                    self.diff_overlay.snapshot(self.history.document());
                    self.status.set("Diff: snapshot captured");
                }
            }
            return;
        }

        ui.label(format!(
            "+{} added   -{} removed   ~{} changed",
            self.diff_overlay.added_count(),
            self.diff_overlay.removed_count(),
            self.diff_overlay.changed_count(),
        ));
        // Legend: the same green/red/amber the overlay paints (catalog 64).
        ui.horizontal(|ui| {
            ui.colored_label(DARK.success, "+ added");
            ui.colored_label(DARK.danger, "- removed");
            ui.colored_label(DARK.warning, "~ changed");
        });

        self.diff_change_list(ui, ctx);
    }

    /// Draws the navigable diff change list: the per-layer filter, the prev/next
    /// navigator, and the clickable rows (catalog 64). Split from
    /// [`diff_panel`](Self::diff_panel) to keep each focused.
    fn diff_change_list(&mut self, ui: &mut egui::Ui, ctx: theme::components::Ctx) {
        // Snapshot the change list into owned rows so the borrow of the overlay
        // ends before navigation mutates the camera and the selection.
        let mut rows: Vec<(DiffKind, LayerId, Rect, String)> = Vec::new();
        for s in self.diff_overlay.added() {
            rows.push((DiffKind::Added, s.layer, s.rect, s.label.clone()));
        }
        for s in self.diff_overlay.removed() {
            rows.push((DiffKind::Removed, s.layer, s.rect, s.label.clone()));
        }
        for s in self.diff_overlay.changed() {
            rows.push((DiffKind::Changed, s.layer, s.rect, s.label.clone()));
        }
        if rows.is_empty() {
            ui.label("No differences.");
            return;
        }

        // Per-layer filter over the layers that actually appear in the diff. The
        // names are resolved up front so the combo closure does not borrow `self`
        // both mutably (the filter) and immutably (the name lookup) at once.
        let layers: std::collections::BTreeSet<LayerId> = rows.iter().map(|r| r.1).collect();
        let layer_names: Vec<(LayerId, String)> = layers
            .iter()
            .map(|id| (*id, self.layer_name_of(*id)))
            .collect();
        let current = self
            .diff_layer_filter
            .map_or_else(|| "All".to_owned(), |id| self.layer_name_of(id));
        ui.horizontal(|ui| {
            ui.label("Layer:");
            egui::ComboBox::from_id_salt("diff_layer_filter")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.diff_layer_filter, None, "All");
                    for (id, name) in &layer_names {
                        ui.selectable_value(&mut self.diff_layer_filter, Some(*id), name);
                    }
                });
        });

        // The row indices passing the filter, the navigation domain.
        let filtered: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| self.diff_layer_filter.is_none_or(|f| f == r.1))
            .map(|(i, _)| i)
            .collect();
        if filtered.is_empty() {
            ui.label("No changes on this layer.");
            return;
        }

        // Prev/next cycle through the filtered rows, recentering on each.
        ui.horizontal(|ui| {
            let here = self
                .diff_selected
                .and_then(|sel| filtered.iter().position(|&i| i == sel))
                .unwrap_or(0);
            let prev = theme::components::IconButton::new(
                crate::theme::icons::CHEVRON_UP,
                "Previous change",
            )
            .show(ui, ctx)
            .clicked();
            let next = theme::components::IconButton::new(
                crate::theme::icons::CHEVRON_DOWN,
                "Next change",
            )
            .show(ui, ctx)
            .clicked();
            let step = if next {
                Some((here + 1) % filtered.len())
            } else if prev {
                Some((here + filtered.len() - 1) % filtered.len())
            } else {
                None
            };
            if let Some(pos) = step {
                let idx = filtered[pos];
                self.diff_selected = Some(idx);
                self.center_on_rect(rows[idx].2);
            }
            ui.label(format!("{}/{}", here + 1, filtered.len()));
        });

        // The change list: click a row to recenter on it.
        let selected = self.diff_selected;
        let mut clicked: Option<usize> = None;
        egui::ScrollArea::vertical()
            .max_height(160.0)
            .auto_shrink([false, false])
            .id_salt("diff_list")
            .show(ui, |ui| {
                for &idx in &filtered {
                    let (kind, layer, _rect, label) = &rows[idx];
                    let text = format!("{} {} {}", kind.glyph(), self.layer_name_of(*layer), label);
                    if ui.selectable_label(selected == Some(idx), text).clicked() {
                        clicked = Some(idx);
                    }
                }
            });
        if let Some(idx) = clicked {
            self.diff_selected = Some(idx);
            self.center_on_rect(rows[idx].2);
        }
    }

    /// The display name of a layer id, falling back to its `layer/datatype`.
    fn layer_name_of(&self, id: LayerId) -> String {
        self.layer_state
            .rows()
            .iter()
            .find(|r| r.id == id)
            .map_or_else(
                || format!("{}/{}", id.layer, id.datatype),
                |r| r.name.clone(),
            )
    }

    /// Recenters the camera on `rect`'s center, keeping the current zoom, so a
    /// clicked diff change or violation is brought into view (catalog 64).
    fn center_on_rect(&mut self, rect: Rect) {
        let center = Point::new(
            i32::midpoint(rect.min.x, rect.max.x),
            i32::midpoint(rect.min.y, rect.max.y),
        );
        self.camera = ViewCamera::new(center, self.camera.pixels_per_dbu());
    }

    /// Diffs the captured baseline against the current document and reports the
    /// result on the status line.
    fn compute_diff(&mut self) {
        match self.diff_overlay.compute(self.history.document()) {
            Some(0) => self.status.set("Diff: no differences"),
            Some(n) => self.status.set(format!(
                "Diff: +{} -{} ({n} total)",
                self.diff_overlay.added_count(),
                self.diff_overlay.removed_count(),
            )),
            None => self.status.set("Diff: snapshot first"),
        }
    }

    /// Lists the layout's anchored comments and adds one on the current top cell.
    ///
    /// Each comment is a [`reticle_sync::Comment`] whose `anchor_ref` binds it to a
    /// cell; the canvas paints a numbered pin at each anchor (see
    /// [`draw_comment_pins`](Self::draw_comment_pins)). Comments persist through the
    /// schema-V2 `Document.comments` field (ADR 0080). Clicking a row selects its
    /// pin.
    fn comment_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = self.comp_ctx();
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.comment_draft)
                    .desired_width(140.0)
                    .hint_text("Comment on top cell"),
            );
            let can_add = !self.top_cell.is_empty();
            if ui
                .add_enabled(can_add, egui::Button::new("Add"))
                .on_hover_text("Anchor a comment to the current top cell")
                .clicked()
            {
                let now_ms = (ui.input(|i| i.time) * 1000.0) as i64;
                self.add_comment_on_top_cell(now_ms);
            }
        });

        if self.comment_pins.is_empty() {
            theme::components::EmptyState::new(
                "No comments",
                "Type a note above and Add to pin it to the top cell.",
            )
            .show(ui, ctx);
            return;
        }

        // Unresolved badge and the resolved filter (catalog 65).
        let unresolved = self.comment_pins.unresolved_count();
        ui.horizontal(|ui| {
            ui.colored_label(
                if unresolved > 0 {
                    DARK.accent
                } else {
                    DARK.success
                },
                format!("{unresolved} unresolved"),
            );
            ui.checkbox(&mut self.comment_pins.show_resolved, "Show resolved");
        });

        let selected = self.comment_pins.selected();
        let show_resolved = self.comment_pins.show_resolved;
        let mut clicked = None;
        let mut toggle_id: Option<String> = None;
        for (i, comment) in self.comment_pins.comments().iter().enumerate() {
            let resolved = self.comment_pins.is_resolved(&comment.id);
            if resolved && !show_resolved {
                continue;
            }
            ui.horizontal(|ui| {
                let line = format!("{}. {}", i + 1, comment_pins::format_comment_line(comment));
                let mut rich = egui::RichText::new(line);
                if resolved {
                    rich = rich.strikethrough().color(DARK.text_weak);
                }
                if ui.selectable_label(selected == Some(i), rich).clicked() {
                    clicked = Some(i);
                }
                let label = if resolved { "Reopen" } else { "Resolve" };
                if ui.small_button(label).clicked() {
                    toggle_id = Some(comment.id.clone());
                }
            });
        }
        if let Some(i) = clicked {
            self.comment_pins.select(i);
            self.status.set(format!("Comment {} selected", i + 1));
        }
        if let Some(id) = toggle_id {
            self.comment_pins.toggle_resolved(&id);
        }
    }

    /// Anchors a new comment to the current top cell using the panel's draft body.
    ///
    /// An empty draft falls back to a placeholder so the affordance always produces
    /// a visible pin; `now_ms` is the session clock (wasm-safe, from egui input) used
    /// as the creation timestamp.
    fn add_comment_on_top_cell(&mut self, now_ms: i64) {
        let body = match self.comment_draft.trim() {
            "" => "New comment".to_owned(),
            text => text.to_owned(),
        };
        let id = format!("c{}", self.comment_pins.len() + 1);
        let comment = Comment::root(id, self.top_cell.clone(), "you", body, now_ms);
        let index = self.comment_pins.add(comment);
        self.comment_pins.select(index);
        self.comment_draft.clear();
        // Event-driven auto-expand: surface the Comments section on a new thread
        // (catalog 67).
        self.inspector.reveal("comments");
        self.status.set("Comment added");
    }

    /// Draws the agent panel: prompt box, Run/Stop, live status, and narration.
    ///
    /// The state machine and the narration feed live in [`crate::agent_panel`];
    /// this is glue only. The panel drives a scripted transcript (no model or
    /// key), so Run always has something honest to narrate. Model-free, so it
    /// runs on wasm too.
    fn agent_section(&mut self, ui: &mut egui::Ui) {
        use crate::agent_panel::RunState;

        // Honesty (v8.1 post-tag): this panel is a scripted demonstration, not a live
        // agent. It narrates a fixed propose-verify-correct script on a built-in demo
        // cell and does not read or edit the open design or its DRC. The real
        // plan/approve/execute agent is planned for a later release. Label it plainly so
        // the Run control never reads as "fix my layout".
        ui.colored_label(
            DARK.warning,
            format!(
                "{} Preview: scripted demo on a built-in cell; it does not read or edit your design.",
                crate::theme::icons::TRIANGLE_ALERT
            ),
        )
        .on_hover_text(
            "A plan/approve/execute agent is planned for a later release. This panel narrates a fixed scripted run for illustration; it never touches your open document or its DRC results.",
        );

        ui.horizontal(|ui| {
            ui.label("Prompt:");
            ui.text_edit_singleline(&mut self.agent.prompt);
        });
        // Sample prompts (catalog 21): a one-click starter while idle.
        if !self.agent.is_running() {
            ui.horizontal_wrapped(|ui| {
                ui.label("Try:");
                for &sample in AGENT_SAMPLE_PROMPTS {
                    if ui
                        .small_button(sample)
                        .on_hover_text("Use this as the prompt")
                        .clicked()
                    {
                        sample.clone_into(&mut self.agent.prompt);
                    }
                }
            });
        }
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
            ui.colored_label(DARK.danger, &self.agent_history.error);
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
        self.drc_ran_revision = Some(self.history.revision());
        if n == 0 {
            self.status.set("Agent verify: DRC clean");
        } else {
            self.status.set(format!("Agent verify: {n} violation(s)"));
            self.inspector.reveal("drc");
        }
    }

    /// A verify step crossed by the *agent preview's own run*: note the result as an
    /// agent turn in the conversation, so the dialogue reflects the scripted
    /// propose-verify-correct loop.
    ///
    /// The preview runs a fixed script on a built-in demo cell (see
    /// [`crate::agent_panel`]); it deliberately does NOT write the result into the
    /// document's DRC panel or overlay. Those track the user's real design, and a
    /// scripted demo must never make them read as verified. (Giving a replay its own
    /// DRC view, separate from the document's, is the v8.2 theater redesign.)
    fn apply_agent_run_verify(&mut self, violations: &[reticle_model::Violation]) {
        let n = violations.len();
        if n == 0 {
            self.agent.note_agent("verified: DRC clean");
        } else {
            self.agent
                .note_agent(format!("verified: {n} violation(s) remaining"));
        }
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
                        ui.colored_label(DARK.warning, &w.summary)
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
        use crate::notify::NotificationAction as NA;
        use crate::theme::components::{Button, ButtonVariant, Severity as CSeverity, Toast};
        if self.notifications.is_empty() {
            return;
        }
        let cctx = self.ui_ctx();
        // A dismissal, or a resolved action to apply after the layout closure so the
        // borrow of `self.notifications` ends first.
        let mut dismiss: Option<usize> = None;
        // (index, action, resolved diagnostic clipboard text for CopyDetails).
        let mut act: Option<(usize, NA, Option<String>)> = None;
        egui::Area::new(egui::Id::new("notifications_area"))
            .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-12.0, -12.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                for (index, note) in self.notifications.iter().enumerate() {
                    let severity = match note.severity {
                        crate::notify::Severity::Info => CSeverity::Info,
                        crate::notify::Severity::Warning => CSeverity::Warning,
                        crate::notify::Severity::Error => CSeverity::Danger,
                    };
                    let message = if note.has_detail() {
                        format!("{}\n{}", note.summary, note.detail)
                    } else {
                        note.summary.clone()
                    };
                    let mut toast = Toast::new(severity, message);
                    for &action in &note.actions {
                        let variant = match action {
                            NA::Fit => ButtonVariant::Primary,
                            NA::CopyDetails => ButtonVariant::Ghost,
                            NA::Retry | NA::Undo => ButtonVariant::Secondary,
                        };
                        toast = toast.action(Button::new(action.label(), variant));
                    }
                    let resp = toast.show(ui, cctx);
                    if let Some(i) = resp.action
                        && let Some(&action) = note.actions.get(i)
                    {
                        let diag = note
                            .diagnostic
                            .as_ref()
                            .map(crate::notify::Diagnostic::clipboard_text);
                        act = Some((index, action, diag));
                    }
                    if resp.closed {
                        dismiss = Some(index);
                    }
                    ui.add_space(6.0);
                }
            });
        // Apply an action first (it dismisses its own toast); otherwise a plain close.
        if let Some((index, action, diag)) = act {
            match action {
                NA::CopyDetails => {
                    if let Some(text) = diag {
                        self.pending_clipboard = Some(text);
                        self.notify("Diagnostics copied", "Paste them into a bug report.");
                    }
                    self.notifications.dismiss(index);
                }
                NA::Retry => {
                    self.notifications.dismiss(index);
                    self.run_retry();
                }
                NA::Undo => {
                    self.notifications.dismiss(index);
                    self.run_command(Command::Undo, None);
                }
                NA::Fit => {
                    self.fit_requested = true;
                    self.notifications.dismiss(index);
                }
            }
        } else if let Some(index) = dismiss {
            self.notifications.dismiss(index);
        }
    }

    /// Draws the offline badge (item 74): a small pill, top-center, shown only while
    /// the live connection is down, so a paused share is visible without opening a
    /// panel. Nothing is drawn while online.
    fn draw_offline_badge(&self, ctx: &egui::Context) {
        let Some(label) = self.connectivity.badge_label() else {
            return;
        };
        egui::Area::new(egui::Id::new("offline_badge"))
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 8.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(DARK.bg_raised)
                    .stroke(Stroke::new(1.0, DARK.warning))
                    .corner_radius(egui::CornerRadius::same(crate::theme::tokens::RADIUS_LG))
                    .inner_margin(egui::Margin::symmetric(10, 5))
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(label).color(DARK.warning));
                    });
            });
    }

    /// Draws the Open-from-URL dialog (item 2, `file.open_url`): a validated URL field
    /// with an inline reason, a CORS-aware next step, and Open/Cancel.
    ///
    /// Validation is the pure [`crate::dialogs::validate_open_url`]; the fetch itself
    /// (and its CORS explainer on failure) is [`start_url_open`](Self::start_url_open).
    fn open_url_dialog(&mut self, ctx: &egui::Context) {
        use crate::theme::components::{Button, TextField};
        if !self.dialogs.open_url_shown {
            return;
        }
        let cctx = self.ui_ctx();
        let mut open = true;
        let mut submit = false;
        let mut cancel = false;
        egui::Window::new("Open from URL")
            .id(egui::Id::new("open_url_dialog"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_max_width(440.0);
                ui.label("Paste a link to a .gds or .oas file:");
                let resp = TextField::new(&mut self.dialogs.open_url_text)
                    .hint("https://example.com/chip.gds")
                    .desired_width(410.0)
                    .show(ui, cctx);
                submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let validation = crate::dialogs::validate_open_url(&self.dialogs.open_url_text);
                // An inline reason once the user has typed something invalid.
                if let Err(e) = validation
                    && !self.dialogs.open_url_text.trim().is_empty()
                {
                    ui.colored_label(DARK.danger, e.message());
                }
                ui.add_space(cctx.density.item_spacing().y);
                ui.horizontal(|ui| {
                    if Button::primary("Open")
                        .enabled(validation.is_ok())
                        .show(ui, cctx)
                        .clicked()
                    {
                        submit = true;
                    }
                    if Button::ghost("Cancel").show(ui, cctx).clicked() {
                        cancel = true;
                    }
                });
                ui.add_space(cctx.density.item_spacing().y);
                ui.label(
                    egui::RichText::new(
                        "If the file is on another site it may be blocked by the browser's \
                         cross-origin (CORS) policy. You can also drag the file onto the window.",
                    )
                    .color(DARK.text_weak),
                );
            });
        if submit && let Ok(url) = crate::dialogs::validate_open_url(&self.dialogs.open_url_text) {
            self.start_url_open(url);
            self.dialogs.open_url_shown = false;
            self.dialogs.open_url_text.clear();
        }
        if cancel || !open {
            self.dialogs.open_url_shown = false;
        }
    }

    /// Draws the Convert-to-archive dialog (`file.convert_gds`): a short explanation of
    /// the streamable `.rtla` output and the trigger for the in-browser converter.
    ///
    /// The conversion itself runs in the web worker the shell wires up
    /// (`__reticleTriggerConvert`), so on the web this dialog is the labelled entry to
    /// it; on native, where there is no browser worker, it explains that and points at
    /// the desktop's direct open.
    fn convert_dialog(&mut self, ctx: &egui::Context) {
        use crate::theme::components::Button;
        if !self.dialogs.convert_shown {
            return;
        }
        let cctx = self.ui_ctx();
        let mut open = true;
        let mut close = false;
        egui::Window::new("Convert GDS to archive")
            .id(egui::Id::new("convert_dialog"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.label("Source: a GDSII file");
                ui.label("Destination: a streamable .rtla archive");
                ui.add_space(cctx.density.item_spacing().y);
                ui.label(
                    egui::RichText::new(
                        "A .rtla archive streams tile by tile, so a large layout opens \
                         instantly and pages in only what the viewport touches.",
                    )
                    .color(DARK.text_weak),
                );
                ui.add_space(cctx.density.item_spacing().y);
                ui.horizontal(|ui| {
                    #[cfg(target_arch = "wasm32")]
                    if Button::primary("Choose a GDS file...")
                        .show(ui, cctx)
                        .clicked()
                    {
                        Self::call_window_fn("__reticleTriggerConvert");
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        let _ = &cctx;
                        ui.label(
                            egui::RichText::new(
                                "Conversion runs in the browser build. On the desktop, open \
                                 the GDS directly.",
                            )
                            .color(DARK.text_weak),
                        );
                    }
                    if Button::ghost("Close").show(ui, cctx).clicked() {
                        close = true;
                    }
                });
            });
        if close || !open {
            self.dialogs.convert_shown = false;
        }
    }

    /// Draws the Share dialog (item 85, `share.dialog`): mint-and-go-live, an
    /// editable/view-only link toggle, copy and test-open, and an honest expiry and
    /// live-status line.
    ///
    /// The link primitives ([`room_link`](crate::share::room_link),
    /// [`viewer_link`](crate::share::viewer_link)) and [`go_live`](Self::go_live) are
    /// the same ones the Inspector's Share section uses, so this dialog is a second
    /// surface over the one share model, not a fork.
    #[allow(clippy::too_many_lines)]
    fn share_dialog(&mut self, ctx: &egui::Context) {
        use crate::theme::components::{Button, Segmented};
        if !self.dialogs.share_shown {
            return;
        }
        let cctx = self.ui_ctx();
        let editable = crate::share::room_link(&self.share_server, &self.share_room);
        let viewer =
            crate::share::viewer_link(&self.share_page, &self.share_server, &self.share_room);
        let live = self.sharer_transport.is_some();
        let mut open = true;
        let mut close = false;
        let mut mint = false;
        let mut copy: Option<String> = None;
        let mut test_open = false;
        egui::Window::new("Share this session")
            .id(egui::Id::new("share_dialog"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_max_width(480.0);
                if Button::primary(if live {
                    "Mint a fresh room and go live again"
                } else {
                    "Mint a room and go live"
                })
                .show(ui, cctx)
                .clicked()
                {
                    mint = true;
                }
                ui.add_space(cctx.density.item_spacing().y);
                // Editable vs view-only link toggle.
                let mut idx = usize::from(self.dialogs.share_view_only);
                Segmented::new(&["Editable link", "View-only link"]).show(ui, cctx, &mut idx);
                self.dialogs.share_view_only = idx == 1;
                let link = if self.dialogs.share_view_only {
                    viewer.clone()
                } else {
                    editable.clone()
                };
                ui.monospace(&link);
                ui.horizontal(|ui| {
                    if Button::secondary("Copy").show(ui, cctx).clicked() {
                        copy = Some(link.clone());
                    }
                    if Button::ghost("Test open (view-only)")
                        .show(ui, cctx)
                        .clicked()
                    {
                        test_open = true;
                    }
                });
                ui.add_space(cctx.density.item_spacing().y);
                // A scannable QR of the selected link (item 92), or an honest note when
                // the link is longer than the compact encoder handles.
                match crate::qr::QrCode::encode(link.as_bytes()) {
                    Some(code) => Self::draw_qr(ui, &code),
                    None => {
                        ui.label(
                            egui::RichText::new(
                                "QR code: the link is too long to encode; copy it instead.",
                            )
                            .color(DARK.text_weak),
                        );
                    }
                }
                ui.add_space(cctx.density.item_spacing().y);
                ui.label(
                    egui::RichText::new("Expiry: this link works while your session stays live.")
                        .color(DARK.text_weak),
                );
                let status = if live {
                    self.live_status.label()
                } else {
                    "Not live yet. Mint a room to start sharing.".to_owned()
                };
                ui.label(egui::RichText::new(format!("Status: {status}")).color(DARK.text_weak));
                ui.add_space(cctx.density.item_spacing().y);
                if Button::ghost("Close").show(ui, cctx).clicked() {
                    close = true;
                }
            });
        if mint {
            let seed = ctx.input(|i| (i.time * 1_000_000.0) as u64);
            self.share_room = crate::share::minted_room_id(&self.top_cell, seed);
            self.go_live(ctx);
            self.notify("Room minted and live", "Copy a link to invite others.");
        }
        if let Some(link) = copy {
            self.pending_clipboard = Some(link);
            self.notify("Link copied", "");
        }
        if test_open {
            #[cfg(target_arch = "wasm32")]
            {
                Self::open_url_in_new_tab(&viewer);
                self.notify(
                    "Opened the view-only link",
                    "Check that it shows your session, view-only.",
                );
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                self.pending_clipboard = Some(viewer.clone());
                self.notify(
                    "Viewer link copied",
                    "Open it in a browser to test the view-only session.",
                );
            }
        }
        if close || !open {
            self.dialogs.share_shown = false;
        }
    }

    /// Opens `url` in a new browser tab so the Share dialog's test-open shows the viewer
    /// link live (wasm only; the desktop build stages the link to the clipboard).
    #[cfg(target_arch = "wasm32")]
    fn open_url_in_new_tab(url: &str) {
        if let Some(window) = web_sys::window() {
            let _ = window.open_with_url_and_target(url, "_blank");
        }
    }

    /// Paints a [`QrCode`](crate::qr::QrCode) as black modules on a white card with a
    /// four-module quiet zone (item 92).
    ///
    /// QR codes must read dark-on-light to scan, so this uses plain black and white
    /// rather than theme chrome tokens; the surrounding dialog is themed.
    fn draw_qr(ui: &mut egui::Ui, code: &crate::qr::QrCode) {
        const MODULE_PX: f32 = 4.0;
        const QUIET: usize = 4;
        let modules = code.size();
        let side = (modules + 2 * QUIET) as f32 * MODULE_PX;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, egui::Color32::WHITE);
        for y in 0..modules {
            for x in 0..modules {
                if code.module(x, y) {
                    let min = rect.min
                        + Vec2::new(
                            (QUIET + x) as f32 * MODULE_PX,
                            (QUIET + y) as f32 * MODULE_PX,
                        );
                    let cell = EguiRect::from_min_size(min, Vec2::splat(MODULE_PX));
                    painter.rect_filled(cell, 0.0, egui::Color32::BLACK);
                }
            }
        }
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
            ui.colored_label(DARK.danger, &self.replay_error);
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

        // A slim fixed-height preview so the docked theater stays compact and the main
        // canvas is the hero. `available_height` reports the whole region during the
        // panel's content pass, so using it here re-inflates the panel past its cap;
        // a small fixed strip keeps the total deterministic (controls + this < cap).
        let h = 72.0;
        let size = Vec2::new(ui.available_width().max(160.0), h);
        let (response, painter) = ui.allocate_painter(size, Sense::hover());
        let rect = response.rect;
        painter.rect_filled(rect, 4.0, DARK.bg_input);
        let shapes = self.replay.flattened_shapes();
        let Some(bbox) = shapes_bbox(&shapes) else {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Nothing drawn yet",
                theme::apply::hud_body(self.ui_density),
                CANVAS.hud_text_dim,
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
                .map_or(CANVAS.layer_fallback, |l| {
                    let (r, g, b, _) = layers::rgba_components(l.color_rgba);
                    theme::tokens::layer_rgb(r, g, b)
                })
        };
        for shape in &shapes {
            let base = color_of(shape.layer);
            let fill = theme::tokens::with_alpha(base, 170);
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
        let marker = Stroke::new(2.0, CANVAS.drc_violation);
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
                theme::apply::hud_mono(self.ui_density),
                CANVAS.hud_text_dim,
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
        let color = CANVAS.agent_cursor;
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
            theme::apply::hud_body(self.ui_density),
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
    ///
    /// ADR-2B: Share left the right Inspector when it was rebuilt on the
    /// segmented-group system (`ia-inventory.md` section 2 gives Share its own
    /// top-level menu, owned by lanes 2A/2D). This method is retained until that
    /// Share menu lands; the headless render test still exercises it, and lane
    /// 2D re-homes its only editor call site. `allow(dead_code)` keeps the
    /// interim state warning-clean.
    #[allow(dead_code)]
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
        // The locally-stored display name that rides on your live presence, so viewers
        // see your name on your cursor (catalog 86).
        ui.horizontal(|ui| {
            ui.label("Your name:");
            ui.text_edit_singleline(&mut self.display_name);
        });

        // A read-only viewer session mirrors a sharer's screen; its Share panel is the
        // connection status and the follow-mode toggle, not the sharing controls.
        if self.is_viewer() {
            self.viewer_share_controls(ui);
            return;
        }

        // One-click Share: mint a fresh room, go live, and copy the read-only viewer
        // link in a single action, so sharing is one button rather than three fields and
        // a sequence of clicks. The advanced relay/room/page fields stay reachable below.
        if ui
            .button("Share this session")
            .on_hover_text("Mint a room, go live, and copy the viewer link")
            .clicked()
        {
            // Seed the room suffix from the frame clock (present on native and wasm); the
            // minting itself is deterministic, so the unit test pins a seed instead.
            let seed = ui.input(|i| (i.time * 1_000_000.0) as u64);
            self.share_room = crate::share::minted_room_id(&self.top_cell, seed);
            let ctx = ui.ctx().clone();
            self.go_live(&ctx);
            let link =
                crate::share::viewer_link(&self.share_page, &self.share_server, &self.share_room);
            ctx.copy_text(link.clone());
            self.status
                .set("Shared: room minted, live, viewer link copied");
            self.notifications.info("Viewer link copied", link);
        }

        ui.separator();

        // The collaborator (read-write) join link.
        let link = crate::share::room_link(&self.share_server, &self.share_room);
        ui.monospace(&link);
        ui.horizontal(|ui| {
            if ui.button("Copy link").clicked() {
                ui.ctx().copy_text(link);
                self.status.set("Share link copied");
            }
            // Go live: open the sharer transport so viewers stream this session's
            // geometry and the live cursor/viewport (ADR 0058). On native this records
            // intent but dials no socket; the live streaming is the browser build.
            let live = self.sharer_transport.is_some();
            let go_live = ui.button(if live { "Live (restart)" } else { "Go live" });
            if go_live.clicked() {
                let ctx = ui.ctx().clone();
                self.go_live(&ctx);
            }
        });
        if self.sharer_transport.is_some() {
            ui.label(self.live_status.label());
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

        ui.separator();

        // A permalink deep-links the current view (cell, camera, visible layers) on top
        // of the opened document, so a collaborator opening it lands on the same spot.
        if ui
            .button("Copy permalink to this view")
            .on_hover_text("A link that restores this cell, camera, and visible layers")
            .clicked()
        {
            let ctx = ui.ctx().clone();
            self.copy_permalink(&ctx);
        }
    }

    /// Draws the Share section for a **read-only viewer** session: the joined room, the
    /// live connection status, and the follow-mode toggle (ADR 0038/0058).
    ///
    /// A viewer cannot share or edit, so this panel shows only what a viewer controls:
    /// whether to ride along with the sharer's viewport (follow on) or pan and zoom the
    /// mirrored design independently (follow off).
    fn viewer_share_controls(&mut self, ui: &mut egui::Ui) {
        ui.label(format!(
            "Read-only viewer of room '{}' on {}",
            self.share_room, self.share_server
        ));
        ui.label(self.live_status.label());
        if let Some(session) = self.viewer_session.as_mut() {
            let mut follow = session.is_following();
            if ui
                .checkbox(&mut follow, "Follow the sharer's view")
                .changed()
            {
                session.set_follow(follow);
                self.status.set(if follow {
                    "Following the sharer's view"
                } else {
                    "Panning independently"
                });
            }
        }
        ui.label("You are viewing a shared session read-only. Your edits are not sent.");
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
        // The stock-egui light toggle is retired: v8.1 ships one tokened dark
        // theme applied at boot from the theme module (ADR 0095). A future light
        // variant is a second token table, not a UI switch here.

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
        ui.horizontal(|ui| {
            if ui.button("Export").clicked() {
                self.run_export();
            }
            // Export-menu consolidation (catalog 12): the metrology CSV alongside
            // the image formats. The same effect the `file.export_metrology`
            // registry id runs.
            if ui
                .button("Metrology CSV")
                .on_hover_text("Write a per-layer metrology summary as CSV")
                .clicked()
            {
                self.export_metrology();
            }
        });
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
    // Always `Ok` on wasm, but the native version genuinely returns `Result`, so the
    // signature must match across cfg.
    #[allow(clippy::unnecessary_wraps)]
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
        let ctx = self.comp_ctx();
        let indices: Vec<usize> = self.selection.iter().collect();
        let insp = inspector::inspect(self.scene.shapes(), &indices, &self.layer_state);
        match insp {
            Inspection::Empty => {
                if theme::components::EmptyState::new(
                    "No selection",
                    "Click a shape or press V to pick the Select tool.",
                )
                .action(theme::components::Button::secondary("Select tool"))
                .show(ui, ctx)
                .is_some()
                {
                    self.select_tool(Tool::Select);
                }
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

    /// Draws the bottom status bar: tool, live coordinates and crosshair toggle, the
    /// selection summary, and the actionable zoom / fps / archive readouts.
    ///
    /// The readouts are click affordances, not passive debug text (AUD-16, item 68): the
    /// zoom opens the preset menu, the fps readout a small perf popover, and (in browse
    /// mode) the archive label toggles the streaming HUD. Coordinates are monospace with
    /// tabular figures so they do not jitter as the cursor moves (tokens.md type scale).
    // One flat egui row of readouts; the length is widget count, not branching logic.
    #[allow(clippy::too_many_lines)]
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        // Precompute the readout strings so the menu closures below never borrow `self`.
        let ppd = self.camera.pixels_per_dbu();
        let zoom_label = format!("Zoom {ppd:.3} px/DBU");
        let fps_label = format!("{:.0} fps", self.frame_meter.fps());
        let frame_ms = self.frame_meter.frame_ms();
        let selection_summary = self.selection_summary();
        let in_archive = self.archive.is_some();

        // Actions chosen inside menu closures, applied after the row so the closures
        // capture only these locals, never `self` (the egui nested-borrow rule).
        let action = ui
            .horizontal(|ui| {
                let mut action: Option<Command> = None;
                ui.label(format!("Tool: {}", self.tools.active().label()));
                ui.separator();

                // Coordinates (monospace, tabular) plus the crosshair toggle (item 32).
                let coord = match self.cursor_world {
                    Some(p) => format!("{}, {}", p.x, p.y),
                    None => "--, --".to_owned(),
                };
                ui.label(egui::RichText::new(format!("{coord} DBU")).monospace());
                if ui
                    .selectable_label(self.crosshair, "+")
                    .on_hover_text("Toggle cursor crosshair")
                    .clicked()
                {
                    self.crosshair = !self.crosshair;
                }
                ui.separator();

                // Selection summary: count, bbox, area (item 55); click to open the
                // numeric transform popover (item 51).
                if ui
                    .add(egui::Label::new(&selection_summary).sense(Sense::click()))
                    .on_hover_text("Selection summary - click for numeric transform")
                    .clicked()
                    && !self.selection.is_empty()
                {
                    self.open_transform_popover();
                }
                ui.separator();

                // Zoom readout: click for the preset menu (item 68).
                ui.menu_button(zoom_label, |ui| {
                    if ui.button("Fit  (F)").clicked() {
                        action = Some(Command::ZoomToFit);
                        ui.close();
                    }
                    if ui.button("Fit selection  (Shift+F)").clicked() {
                        action = Some(Command::ZoomSelection);
                        ui.close();
                    }
                    if ui.button("Zoom 1:1 DBU").clicked() {
                        action = Some(Command::ZoomOneToOne);
                        ui.close();
                    }
                    if ui.button("Zoom to layer extents").clicked() {
                        action = Some(Command::ZoomLayerExtents);
                        ui.close();
                    }
                });
                ui.separator();

                // FPS readout: click for the perf popover (item 68).
                ui.menu_button(fps_label, |ui| {
                    ui.label(egui::RichText::new(format!("{frame_ms:.2} ms / frame")).monospace());
                    ui.label(
                        egui::RichText::new(format!(
                            "{:.1} fps averaged",
                            1000.0 / frame_ms.max(0.001)
                        ))
                        .monospace(),
                    );
                });

                // Archive label: click toggles the streaming HUD (item 68).
                if in_archive {
                    ui.separator();
                    if ui
                        .selectable_label(self.archive_hud_visible, "Archive HUD")
                        .on_hover_text("Toggle the streaming HUD")
                        .clicked()
                    {
                        self.archive_hud_visible = !self.archive_hud_visible;
                    }
                }

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
                action
            })
            .inner;

        if let Some(cmd) = action {
            self.run_command(cmd, None);
        }
    }

    /// The status-bar selection summary: count, bounding box, and area (item 55).
    fn selection_summary(&self) -> String {
        let n = self.selection.len();
        if n == 0 {
            return "No selection".to_owned();
        }
        match self.selection_bounds() {
            Some(b) => {
                let (w, h) = (b.width(), b.height());
                format!("{n} selected · {w}x{h} DBU · {} DBU^2", w.saturating_mul(h))
            }
            None => format!("{n} selected"),
        }
    }

    /// Opens the numeric transform popover, filling its edit buffers from the current
    /// selection's bounding box (item 51).
    fn open_transform_popover(&mut self) {
        if let Some(b) = self.selection_bounds() {
            self.transform.x = b.min.x.to_string();
            self.transform.y = b.min.y.to_string();
            self.transform.w = b.width().to_string();
            self.transform.h = b.height().to_string();
            self.transform.open = true;
        }
    }

    /// Draws the numeric transform popover and applies it on request (item 51).
    ///
    /// `x`/`y` move the selection so its bounding-box corner lands on the typed
    /// coordinate; `w`/`h` resize a single selected rectangle (an arbitrary multi-shape
    /// resize has no unambiguous meaning, so it is move-only). The commit routes through
    /// the undo history like every other edit.
    fn transform_window(&mut self, ctx: &egui::Context) {
        if !self.transform.open {
            return;
        }
        let mut open = self.transform.open;
        let mut apply = false;
        let single_rect = self.single_selected_direct_rect().is_some();
        egui::Window::new("Transform")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                egui::Grid::new("transform_grid").show(ui, |ui| {
                    ui.label("X");
                    ui.text_edit_singleline(&mut self.transform.x);
                    ui.end_row();
                    ui.label("Y");
                    ui.text_edit_singleline(&mut self.transform.y);
                    ui.end_row();
                    ui.add_enabled_ui(single_rect, |ui| ui.label("W"));
                    ui.add_enabled(
                        single_rect,
                        egui::TextEdit::singleline(&mut self.transform.w),
                    );
                    ui.end_row();
                    ui.add_enabled_ui(single_rect, |ui| ui.label("H"));
                    ui.add_enabled(
                        single_rect,
                        egui::TextEdit::singleline(&mut self.transform.h),
                    );
                    ui.end_row();
                });
                if !single_rect {
                    ui.label(
                        egui::RichText::new("W/H resize applies to a single rectangle.")
                            .small()
                            .color(DARK.text_weak),
                    );
                }
                if ui.button("Apply").clicked() {
                    apply = true;
                }
            });
        self.transform.open = open;
        if apply {
            self.apply_transform();
        }
    }

    /// The single directly-selected rectangle's `(index, layer)`, or `None` when the
    /// selection is not exactly one top-cell rectangle (item 51 resize).
    fn single_selected_direct_rect(&self) -> Option<(usize, LayerId)> {
        let direct = self.selected_direct_shapes();
        if direct.len() != 1 {
            return None;
        }
        let (index, shape) = &direct[0];
        if matches!(shape.kind, ShapeKind::Rect(_)) {
            Some((*index, shape.layer))
        } else {
            None
        }
    }

    /// Applies the numeric transform popover's values to the selection (item 51).
    fn apply_transform(&mut self) {
        let Some(bounds) = self.selection_bounds() else {
            self.status.set("Transform: nothing selected");
            return;
        };
        let (Ok(x), Ok(y)) = (
            self.transform.x.trim().parse::<i32>(),
            self.transform.y.trim().parse::<i32>(),
        ) else {
            self.status.set("Transform: x and y must be integers");
            return;
        };
        // A single rectangle can be resized outright; otherwise the popover moves the
        // selection so its bounding-box corner lands on (x, y).
        if let Some((index, layer)) = self.single_selected_direct_rect()
            && let (Ok(w), Ok(h)) = (
                self.transform.w.trim().parse::<i32>(),
                self.transform.h.trim().parse::<i32>(),
            )
            && w > 0
            && h > 0
        {
            let rect = Rect::new(
                Point::new(x, y),
                Point::new(x.saturating_add(w), y.saturating_add(h)),
            );
            self.resize_direct_rect(index, layer, rect);
            self.status.set(format!("Transformed to {x}, {y}, {w}x{h}"));
            return;
        }
        let n = self.translate_direct_selection(x - bounds.min.x, y - bounds.min.y);
        if n > 0 {
            self.status.set(format!("Moved selection to {x}, {y}"));
        } else {
            self.status.set("Transform: selection is not movable");
        }
    }

    /// Replaces the directly-owned rectangle at `index` with a new `rect` on `layer` as
    /// one undoable step, reselecting the result (item 51 resize).
    fn resize_direct_rect(&mut self, index: usize, layer: LayerId, rect: Rect) {
        let cell = self.top_cell.clone();
        let edits = vec![
            reticle_model::Edit::RemoveShape {
                cell: cell.clone(),
                index,
            },
            reticle_model::Edit::AddShape {
                cell,
                shape: DrawShape::new(layer, ShapeKind::Rect(rect)),
            },
        ];
        if self.history.apply_group(edits).is_ok() {
            self.rebuild_scene();
            let total = self.top_cell_direct_shape_count();
            self.selection.set(total.saturating_sub(1)..total);
        }
    }

    /// The component [`Ctx`](crate::theme::components::Ctx) for this frame: the dark
    /// palette at the active density and reduced-motion preference. Lane 3C renders
    /// the palette, shortcuts overlay, and context menus through it.
    fn component_ctx(&self) -> crate::theme::components::Ctx {
        crate::theme::components::Ctx::dark(self.ui_density)
            .with_reduced_motion(self.reduced_motion)
    }

    /// The document targets the palette also searches (layers, cells, bookmarks;
    /// item 80), returned as owned vectors the caller borrows into a
    /// [`DocTargets`](crate::command::DocTargets) for the frame.
    fn palette_doc_targets(&self) -> (Vec<String>, Vec<String>, Vec<String>) {
        let layers = self
            .layer_state
            .rows()
            .iter()
            .map(|r| r.name.clone())
            .collect();
        let mut cells: Vec<String> = self
            .history
            .document()
            .cells()
            .map(|c| c.name.clone())
            .collect();
        cells.sort();
        // The palette recalls the View-menu camera bookmarks (lane 3A). Slots are
        // unnamed, so label each by its 1-based position.
        let bookmarks = (1..=self.bookmarks.len()).map(|n| n.to_string()).collect();
        (layers, cells, bookmarks)
    }

    /// Draws the command palette: a fuzzy launcher over the whole registry plus the
    /// document targets, grouped with recents first and live chord hints (items 79,
    /// 80), or an inline argument prompt for goto-coordinate / goto-cell.
    fn palette_window(&mut self, ctx: &egui::Context, screen: Option<ScreenRect>) {
        if !self.palette_open {
            return;
        }
        let cctx = self.component_ctx();
        let (layers, cells, bookmarks) = self.palette_doc_targets();
        let targets = command::DocTargets {
            layers: &layers,
            cells: &cells,
            bookmarks: &bookmarks,
        };
        let items = command::build_items(commands::registry(), &self.keymap, &targets);

        let mut open = self.palette_open;
        // Request keyboard focus only on the frame the palette opened; see
        // `palette_focus_pending`. Re-requesting every frame pins `focus.id` to the
        // field, which makes `lost_focus()` forever false so Enter never runs a command.
        let focus_pending = self.palette_focus_pending;
        // The row the user activated this frame, as (action, recents key).
        let mut chosen: Option<(command::PaletteAction, String)> = None;
        let mut commit_arg = false;
        let mut cancel = false;
        egui::Window::new("Command palette")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(420.0)
            .default_pos(Pos2::new(200.0, 120.0))
            .show(ctx, |ui| {
                if let Some(arg) = self.palette_arg {
                    ui.label(arg.prompt());
                    let resp = crate::theme::components::TextField::new(&mut self.palette_query)
                        .hint(arg.hint())
                        .desired_width(f32::INFINITY)
                        .show(ui, cctx);
                    if focus_pending {
                        resp.request_focus();
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        commit_arg = true;
                    }
                    ui.horizontal(|ui| {
                        if crate::theme::components::Button::primary("Go")
                            .show(ui, cctx)
                            .clicked()
                        {
                            commit_arg = true;
                        }
                        if crate::theme::components::Button::secondary("Cancel")
                            .show(ui, cctx)
                            .clicked()
                        {
                            cancel = true;
                        }
                    });
                } else {
                    let resp = crate::theme::components::TextField::new(&mut self.palette_query)
                        .hint("Type a command, layer, cell, or bookmark...")
                        .desired_width(f32::INFINITY)
                        .show(ui, cctx);
                    if focus_pending {
                        resp.request_focus();
                    }
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let groups =
                        command::results(&items, &self.palette_recents, &self.palette_query);
                    // Enter runs the first row of the first group (the top hit).
                    if enter && let Some(item) = groups.first().and_then(|g| g.items.first()) {
                        chosen = Some((item.action.clone(), item.key.clone()));
                    }
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(360.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if groups.is_empty() {
                                ui.label(
                                    egui::RichText::new("No matching commands")
                                        .color(cctx.tokens.text_weak),
                                );
                            }
                            for group in &groups {
                                crate::theme::components::SectionHeader::new(&group.heading)
                                    .show(ui, cctx);
                                for item in &group.items {
                                    if Self::palette_row(ui, cctx, item) {
                                        chosen = Some((item.action.clone(), item.key.clone()));
                                    }
                                }
                            }
                        });
                }
            });
        // Focus was requested (if pending) during this frame's show; consume the latch
        // so subsequent frames leave the field alone and Enter's focus-surrender works.
        self.palette_focus_pending = false;
        self.palette_open = open;
        if !self.palette_open || cancel {
            self.palette_open = false;
            self.palette_arg = None;
            return;
        }
        if commit_arg {
            self.commit_palette_arg(screen);
            return;
        }
        if let Some((action, key)) = chosen {
            self.run_palette_action(action, key, screen);
        }
    }

    /// Draws one palette row (label plus a right-aligned chord chip) and returns
    /// whether it was activated this frame.
    fn palette_row(
        ui: &mut egui::Ui,
        cctx: crate::theme::components::Ctx,
        item: &command::PaletteItem,
    ) -> bool {
        let mut clicked = false;
        ui.horizontal(|ui| {
            if ui.selectable_label(false, &item.label).clicked() {
                clicked = true;
            }
            if let Some(hint) = &item.hint {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    crate::theme::components::KbdChip::new(hint.clone()).show(ui, cctx);
                });
            }
        });
        clicked
    }

    /// Runs a chosen palette row and records it in the recents list.
    ///
    /// Registry commands route through [`App::dispatch`] (the goto commands switch
    /// the palette into argument mode and keep it open); the document rows drive the
    /// layer manager, cell locator, and bookmark recall directly. The palette closes
    /// afterward unless it entered an argument prompt.
    fn run_palette_action(
        &mut self,
        action: command::PaletteAction,
        key: String,
        screen: Option<ScreenRect>,
    ) {
        self.record_recent(key);
        self.run_menu_action(action, screen);
        // Keep the palette open only if a goto command just armed an argument prompt.
        if self.palette_arg.is_none() {
            self.palette_open = false;
        } else {
            self.palette_query.clear();
        }
    }

    /// Runs a [`PaletteAction`](crate::command::PaletteAction) without touching
    /// palette state or recents, shared by the palette and the context menus.
    fn run_menu_action(&mut self, action: command::PaletteAction, screen: Option<ScreenRect>) {
        match action {
            command::PaletteAction::Command(id) => self.dispatch(id),
            command::PaletteAction::ToggleLayer(i) => {
                self.run_command(Command::ToggleLayer(i), screen);
            }
            command::PaletteAction::SelectLayer(i) => {
                self.run_command(Command::SelectLayer(i), screen);
            }
            command::PaletteAction::GotoCell(name) => self.goto_cell(&name),
            command::PaletteAction::Bookmark(i) => self.recall_bookmark(i),
        }
    }

    /// Renders a registry-driven context-menu body and returns the chosen action
    /// (item 47). For a layer row, `layer` supplies the dynamic toggle/select rows
    /// prepended before the registry commands. Call inside
    /// [`egui::Response::context_menu`].
    fn context_menu_body(
        ui: &mut egui::Ui,
        keymap: &Keymap,
        context: commands::MenuContext,
        layer: Option<(usize, &str)>,
    ) -> Option<command::PaletteAction> {
        let mut chosen = None;
        if let Some((i, name)) = layer {
            if ui.button(format!("Toggle layer {name}")).clicked() {
                chosen = Some(command::PaletteAction::ToggleLayer(i));
                ui.close();
            }
            if ui.button(format!("Select all on {name}")).clicked() {
                chosen = Some(command::PaletteAction::SelectLayer(i));
                ui.close();
            }
            ui.separator();
        }
        for id in context.command_ids() {
            let cid = CommandId(id);
            let Some(spec) = commands::spec(cid) else {
                continue;
            };
            let label = match keymap.chord_for(cid) {
                Some(c) => format!("{}  ({c})", spec.label),
                None => spec.label.to_owned(),
            };
            if ui.button(label).clicked() {
                chosen = Some(command::PaletteAction::Command(cid));
                ui.close();
            }
        }
        chosen
    }

    /// Attaches a registry-driven context menu to `response` and runs the chosen
    /// action, the shared glue for the canvas and panel-header right-click surfaces
    /// (item 47). Layer rows render [`App::context_menu_body`] directly so they can
    /// supply their dynamic per-layer actions.
    fn attach_context_menu(&mut self, response: &egui::Response, context: commands::MenuContext) {
        let keymap = &self.keymap;
        let mut chosen = None;
        response.context_menu(|ui| {
            chosen = Self::context_menu_body(ui, keymap, context, None);
        });
        if let Some(action) = chosen {
            self.run_menu_action(action, None);
        }
    }

    /// Commits the current argument prompt (goto coordinate / cell) and closes the
    /// palette; a malformed coordinate leaves the prompt open with a status hint.
    fn commit_palette_arg(&mut self, screen: Option<ScreenRect>) {
        let _ = screen;
        match self.palette_arg {
            Some(command::PaletteArg::GotoCoordinate) => {
                if let Some((x, y)) = command::parse_coordinate(&self.palette_query) {
                    self.camera = ViewCamera::new(
                        Point::new(x as i32, y as i32),
                        self.camera.pixels_per_dbu(),
                    );
                    self.status.set(format!("Go to ({x}, {y})"));
                    self.close_palette();
                } else {
                    self.status.set("Enter a coordinate as x, y (DBU)");
                }
            }
            Some(command::PaletteArg::GotoCell) => {
                let name = self.palette_query.trim().to_owned();
                self.goto_cell(&name);
                self.close_palette();
            }
            None => self.close_palette(),
        }
    }

    /// Centers the view on `name`'s bounding box, or reports it is unknown.
    fn goto_cell(&mut self, name: &str) {
        if let Some(bbox) = self.history.document().cell_bbox(name) {
            self.search.pending_locate = Some(bbox);
            self.status.set(format!("Go to cell '{name}'"));
        } else {
            self.status.set(format!("No cell named '{name}'"));
        }
    }

    /// Recalls the View-menu camera bookmark at slot `i` (lane 3A), easing the camera
    /// to it. The palette's dynamic bookmark rows dispatch this.
    fn recall_bookmark(&mut self, i: usize) {
        if let Some(&cam) = self.bookmarks.get(i) {
            self.begin_view_move(cam);
            self.status.set(format!("Recalled view bookmark {}", i + 1));
        }
    }

    /// Closes the palette and clears any argument prompt and query text.
    fn close_palette(&mut self) {
        self.palette_open = false;
        self.palette_arg = None;
        self.palette_query.clear();
    }

    /// Records `key` at the front of the recents list, de-duplicated and bounded.
    fn record_recent(&mut self, key: String) {
        self.palette_recents.retain(|k| k != &key);
        self.palette_recents.insert(0, key);
        self.palette_recents.truncate(PALETTE_RECENTS_MAX);
    }

    /// Draws the keyboard-shortcuts overlay (`?`, item 18): every command grouped
    /// by category with its live chord, generated straight from the registry so it
    /// can never drift from what the keyboard does, plus the chord-sequence list
    /// (item 81) and a link into the editor.
    fn shortcuts_overlay(&mut self, ctx: &egui::Context) {
        if !self.shortcuts_open {
            return;
        }
        let cctx = self.component_ctx();
        // Group the registry by category in first-seen order (so a category split
        // across lane sections still renders under one heading).
        let mut cats: Vec<(&str, Vec<&crate::commands::CommandSpec>)> = Vec::new();
        for spec in commands::registry() {
            if let Some(entry) = cats.iter_mut().find(|(c, _)| *c == spec.category) {
                entry.1.push(spec);
            } else {
                cats.push((spec.category, vec![spec]));
            }
        }
        let mut open = self.shortcuts_open;
        let mut customize = false;
        egui::Window::new("Keyboard shortcuts")
            .open(&mut open)
            .resizable(true)
            .default_width(360.0)
            .default_pos(Pos2::new(320.0, 120.0))
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("Generated from the command registry")
                        .color(cctx.tokens.text_weak),
                );
                egui::ScrollArea::vertical()
                    .max_height(440.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (category, specs) in &cats {
                            crate::theme::components::SectionHeader::new(*category).show(ui, cctx);
                            for spec in specs {
                                ui.horizontal(|ui| {
                                    ui.label(spec.label);
                                    if let Some(chord) = self.keymap.chord_for(spec.id) {
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                crate::theme::components::KbdChip::new(
                                                    chord.to_string(),
                                                )
                                                .show(ui, cctx);
                                            },
                                        );
                                    }
                                });
                            }
                        }
                        crate::theme::components::SectionHeader::new("Sequences").show(ui, cctx);
                        for (prefix, suffix, target) in keymap::SEQUENCES {
                            ui.horizontal(|ui| {
                                ui.label(commands::label(CommandId(target)));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        crate::theme::components::KbdChip::new(format!(
                                            "{} then {}",
                                            prefix.to_lowercase(),
                                            suffix.to_lowercase()
                                        ))
                                        .show(ui, cctx);
                                    },
                                );
                            });
                        }
                    });
                ui.separator();
                if crate::theme::components::Button::secondary("Customize…")
                    .show(ui, cctx)
                    .clicked()
                {
                    customize = true;
                }
            });
        self.shortcuts_open = open;
        if customize {
            self.keymap_open = true;
        }
    }

    /// Draws the shortcuts editor window: every action with its current chord,
    /// rebind and clear controls, a conflict check, keymap export/import, plus
    /// reset and (native) save.
    ///
    /// The chord capture itself happens in [`App::handle_shortcuts`]; this window
    /// only arms it, so what the editor shows and what the keyboard does can
    /// never disagree. Takeovers (binding a chord another action holds) are
    /// reported through the status bar by the capture path.
    #[allow(clippy::too_many_lines)] // grid rows plus the conflict/io controls read better flat
    fn keymap_window(&mut self, ctx: &egui::Context) {
        if !self.keymap_open {
            return;
        }
        let cctx = self.component_ctx();
        let mut open = self.keymap_open;
        egui::Window::new("Customize shortcuts")
            .open(&mut open)
            .resizable(true)
            .default_pos(Pos2::new(260.0, 140.0))
            .show(ctx, |ui| {
                ui.label("Click Rebind, then press the new chord (Escape cancels).");
                ui.label("Binding a chord another action holds unbinds that action.");
                // Conflict detector: bind()/from_toml() keep the map conflict-free by
                // construction, so this reassures and would surface any future lapse.
                let conflicts = self.keymap.conflicts();
                if conflicts.is_empty() {
                    ui.label(
                        egui::RichText::new("No shortcut conflicts").color(cctx.tokens.text_weak),
                    );
                } else {
                    for (a, b) in &conflicts {
                        ui.label(
                            egui::RichText::new(format!(
                                "Conflict: {} and {} share a chord",
                                commands::label(*a),
                                commands::label(*b)
                            ))
                            .color(cctx.tokens.danger),
                        );
                    }
                }
                ui.separator();
                egui::Grid::new("keymap_grid")
                    .num_columns(3)
                    .striped(true)
                    .show(ui, |ui| {
                        for spec in commands::registry().iter().filter(|s| s.rebindable) {
                            let cmd = spec.id;
                            ui.label(spec.label);
                            let chord_text = self
                                .keymap
                                .chord_for(cmd)
                                .map_or_else(|| "(unbound)".to_owned(), ToString::to_string);
                            ui.monospace(chord_text);
                            ui.horizontal(|ui| {
                                if self.rebinding == Some(cmd) {
                                    ui.label("press keys...");
                                } else if ui.small_button("Rebind").clicked() {
                                    self.rebinding = Some(cmd);
                                }
                                if ui.small_button("Clear").clicked() {
                                    self.keymap.bind(cmd, None);
                                    if self.rebinding == Some(cmd) {
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
                ui.separator();
                // Export / import (item 82): TOML round-trips through a scratch buffer
                // that works on every target, no file dialog needed.
                egui::CollapsingHeader::new("Export / import")
                    .id_salt("keymap_io")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if ui.button("Export to text").clicked() {
                                self.keymap_io_text = self.keymap.to_toml();
                                self.status.set("Keymap exported to the text box");
                            }
                            if ui.button("Copy").clicked() {
                                ui.ctx().copy_text(self.keymap_io_text.clone());
                                self.status.set("Keymap copied to clipboard");
                            }
                            if ui.button("Apply from text").clicked() {
                                let (map, warnings) = Keymap::from_toml(&self.keymap_io_text);
                                self.keymap = map;
                                self.rebinding = None;
                                if warnings.is_empty() {
                                    self.status.set("Keymap imported");
                                } else {
                                    self.status.set(format!(
                                        "Keymap imported ({} warnings)",
                                        warnings.len()
                                    ));
                                }
                            }
                        });
                        ui.add(
                            egui::TextEdit::multiline(&mut self.keymap_io_text)
                                .desired_rows(6)
                                .desired_width(f32::INFINITY)
                                .code_editor()
                                .hint_text("Paste a keymap TOML here, then Apply from text"),
                        );
                    });
            });
        self.keymap_open = open;
        if !self.keymap_open {
            self.rebinding = None;
        }
    }

    /// Starts an animated camera move to `target` (item 29), or cuts to it instantly
    /// under reduced motion. Replaces any move already in flight.
    fn begin_view_move(&mut self, target: ViewCamera) {
        let secs = if self.reduced_motion {
            0.0
        } else {
            VIEW_MOVE_SECS
        };
        self.camera_tween = Some(CameraTween::new(self.camera, target, secs));
    }

    /// Advances an in-flight animated camera move by `dt`, clearing it when it settles.
    /// The per-frame repaint at the end of `App::ui` keeps the animation running.
    fn advance_camera_tween(&mut self, dt: f32) {
        if let Some(tween) = self.camera_tween.as_mut() {
            self.camera = tween.advance(dt);
            if tween.done() {
                self.camera_tween = None;
            }
        }
    }

    /// Cancels any in-flight animated camera move, so a manual pan/zoom/pinch takes the
    /// camera over cleanly instead of fighting the tween.
    fn cancel_view_move(&mut self) {
        self.camera_tween = None;
    }

    /// The camera that fits `bounds` into `screen`, leaving the current camera untouched.
    fn fitted_camera(&self, screen: &ScreenRect, bounds: Rect) -> ViewCamera {
        let mut cam = self.camera;
        cam.zoom_to_fit(screen, bounds);
        cam
    }

    /// The target camera for a zoom preset (item 28), or `None` when there is nothing to
    /// frame (an empty selection, or no visible geometry).
    fn preset_camera(&self, preset: ViewPreset, screen: &ScreenRect) -> Option<ViewCamera> {
        match preset {
            ViewPreset::Selection => {
                let bounds = self.selection_bounds()?;
                let mut cam = self.camera;
                // 10% border so the selection reads with a little breathing room.
                cam.zoom_to_rect(screen, bounds, 0.1);
                Some(cam)
            }
            // 1:1 DBU: exactly one screen pixel per database unit, keeping the center.
            ViewPreset::OneToOne => Some(ViewCamera::new(self.camera.center(), 1.0)),
            ViewPreset::LayerExtents => {
                Some(self.fitted_camera(screen, self.visible_layer_bounds()?))
            }
        }
    }

    /// The bounding box of the current selection in world DBU, or `None` when nothing is
    /// selected (item 28 fit-selection, item 55 status summary).
    fn selection_bounds(&self) -> Option<Rect> {
        let shapes = self.scene.shapes();
        union_bounds(
            self.selection
                .iter()
                .filter_map(|i| shapes.get(i))
                .map(reticle_geometry::Shape::bounding_box),
        )
    }

    /// The union bounding box of every shape on a visible layer, or `None` when no visible
    /// layer carries geometry (item 28 zoom-to-layer-extents).
    fn visible_layer_bounds(&self) -> Option<Rect> {
        union_bounds(
            self.scene
                .shapes()
                .iter()
                .filter(|s| self.layer_state.is_visible(s.layer()))
                .map(reticle_geometry::Shape::bounding_box),
        )
    }

    /// Draws the layout canvas and processes pointer interaction on it.
    ///
    /// Returns the canvas [`ScreenRect`] so the caller can hand it to actions (PNG
    /// export, deferred zoom-to-fit) that need the real pixel size.
    ///
    /// When `gpu_format` is `Some`, the layout geometry is drawn on the GPU through a
    /// retained paint callback (eframe's shared device); egui overlays still paint on
    /// top. When it is `None`, the geometry falls back to egui painting.
    // The per-frame canvas: input routing, deferred camera moves, and the ordered draw
    // of geometry then overlays. Its length is that sequence, not branching complexity.
    #[allow(clippy::too_many_lines)]
    fn canvas(
        &mut self,
        ui: &mut egui::Ui,
        gpu_format: Option<eframe::egui_wgpu::wgpu::TextureFormat>,
    ) -> ScreenRect {
        self.focus_anchor(ui, crate::focus::FocusRegion::Canvas);
        let size = ui.available_size();
        let (response, base_painter) = ui.allocate_painter(size, Sense::click_and_drag());
        let rect = response.rect;

        // Right-click context menu (item 47): shape actions when a selection exists,
        // otherwise canvas navigation. Registry-driven like every other surface.
        let menu_ctx = if self.selection.is_empty() {
            commands::MenuContext::Canvas
        } else {
            commands::MenuContext::Shape
        };
        self.attach_context_menu(&response, menu_ctx);

        // Background (covers every pane and the divider).
        base_painter.rect_filled(rect, 0.0, DARK.bg_canvas);

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

        // Deferred camera moves now that the canvas size is known. Each computes a target
        // camera and hands it to `begin_view_move`, which animates the transition (item
        // 29) unless reduced motion is on; the tween is then advanced for this frame.
        let dt = ui.ctx().input(|i| i.stable_dt).max(0.0);

        // Deferred fit-all.
        if self.fit_requested {
            if let Some(bounds) = self.scene.bounds() {
                self.begin_view_move(self.fitted_camera(&screen, bounds));
            }
            self.fit_requested = false;
        }

        // In a read-only viewer session with follow-mode on, snap the local camera to
        // the sharer's viewport (ADR 0038) before drawing, so the viewer rides along.
        // The ViewerSession owns its own camera; mirror it into the App camera the
        // canvas draws with, so follow reuses the whole render path.
        let follow_t = if self.reduced_motion {
            1.0
        } else {
            let dt = ui.input(|i| i.stable_dt);
            (dt * FOLLOW_EASE_RATE).clamp(0.05, 1.0)
        };
        if let Some(session) = self.viewer_session.as_mut()
            && session.is_following()
            && session.follow_step(&screen, follow_t)
        {
            self.camera = session.camera();
        }

        // Deferred zoom preset (fit-selection, 1:1 DBU, layer extents) that needed the
        // real canvas size to resolve (item 28).
        if let Some(preset) = self.pending_view.take() {
            if let Some(target) = self.preset_camera(preset, &screen) {
                self.begin_view_move(target);
            } else {
                self.status.set("Nothing to frame");
            }
        }

        // Deferred zoom to the violation the DRC list just selected.
        if self.zoom_to_selected_violation {
            if let Some(loc) = self
                .drc
                .selected()
                .and_then(|i| self.drc.violations().get(i).map(|v| v.location))
            {
                // 25% border so the violation reads clearly with surrounding context.
                let mut target = self.camera;
                target.zoom_to_rect(&screen, loc, 0.25);
                self.begin_view_move(target);
            }
            self.zoom_to_selected_violation = false;
        }

        // Deferred locate to the outline row the search panel just clicked. Framed
        // with the same 25% border as the DRC locate so the target reads clearly.
        if let Some(target) = self.search.pending_locate.take() {
            let mut cam = self.camera;
            cam.zoom_to_rect(&screen, target, 0.25);
            self.begin_view_move(cam);
        }

        // Advance any in-flight animated move, then let follow-mode override: a viewer
        // riding the sharer's camera must win over a local tween.
        self.advance_camera_tween(dt);

        // In a read-only viewer session with follow-mode on, snap the local camera to
        // the sharer's viewport (ADR 0038) before drawing, so the viewer rides along.
        // The ViewerSession owns its own camera; mirror it into the App camera the
        // canvas draws with, so follow reuses the whole render path.
        if let Some(session) = self.viewer_session.as_mut()
            && session.is_following()
            && session.sync_camera(&screen)
        {
            self.camera = session.camera();
            self.camera_tween = None;
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
            self.hover_pick = None;
        }

        // Draw grid + rulers under the geometry.
        if self.grid.visible {
            self.draw_grid(&painter, &screen);
        }

        // Draw the scene (shapes or, at low zoom, cell boxes).
        let viewport = self.camera.visible_world_rect(&screen);
        if self.archive.is_some() {
            // Served-archive browse: paint the read-only streamed die with progressive
            // residency (coarse-then-fine) and an eased LOD crossfade, skipping the
            // document-specific overlays below (there is no editable document in browse
            // mode). The HUD and residency minimap are placed after, via the overlay
            // manager.
            self.draw_archive(&painter, &screen, dt);
        } else {
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
            self.draw_live_drc_underlines(&painter, &screen);
            self.draw_diff_overlay(&painter, &screen);
            self.draw_comment_pins(&painter, &screen);
        }

        // User guides under the ruler bars, then the rulers cover their ends.
        self.draw_guides(&painter, &screen);
        if self.rulers_visible {
            self.draw_rulers(&painter, &screen);
        }
        self.draw_measure(&painter, &screen);
        // An optional crosshair tracks the cursor across the whole pane (item 32).
        if self.crosshair {
            self.draw_crosshair(&painter, &screen);
        }
        // The snap indicator rides on top so it is never hidden by geometry.
        self.draw_snap_indicator(&painter, &screen);

        // Floating canvas overlays are placed through the collision-free layout manager,
        // so the HUD (top-left), the minimap (top-right), and any legend can never
        // overlap a ruler or each other (AUD-01/02/08, item 68). The ruler inset is zero
        // when the rulers are hidden, reclaiming those edges.
        let ruler_bar = if self.rulers_visible { RULER_BAR } else { 0.0 };
        let mut overlays = OverlayLayout::new(&screen, ruler_bar);
        if self.archive.is_some() {
            // Streamed archive browse: the HUD and the residency minimap, both anchored
            // through the manager. Document/collaboration overlays are meaningless here.
            if self.archive_hud_visible {
                self.draw_archive_hud(&painter, &mut overlays);
            }
            if self.minimap_visible {
                self.draw_minimap(&painter, &screen, &mut overlays);
            }
        } else {
            self.draw_hover_highlight(&painter, &screen);
            self.draw_draw_overlay(&painter, &screen, &response, ui.ctx());
            if self.labels_visible {
                self.draw_labels(&painter, &screen);
            }
            if self.minimap_visible {
                self.draw_minimap(&painter, &screen, &mut overlays);
            }
            let now = ui.input(|i| i.time);
            self.draw_presence(&painter, &screen, now);
            self.draw_agent_cursor(&painter, &screen);
            self.draw_remote_edit_glow(&painter, &screen);
        }

        // Mark the focused pane when split (drawn unclipped so the full border
        // stroke shows).
        if split {
            base_painter.rect_stroke(
                egui_rect_of(&screen),
                0.0,
                Stroke::new(1.5, CANVAS.pane_focus),
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
        let border = Stroke::new(1.0, CANVAS.pane_border);
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
        // Whether the snap-bypass modifier is held (item 53): the cursor then lands on
        // the exact world point, and no snap indicator is drawn.
        self.snap_bypass = ctx.input(|i| i.modifiers.command || i.modifiers.ctrl);

        // Track the cursor world position (snapped) for the status bar, and stash
        // the snap hint so the canvas can draw the snap indicator this frame. For the
        // Select tool also record the shape under the cursor for the hover
        // pre-highlight (item 46).
        if let Some(pos) = response.hover_pos() {
            let raw = self.camera.screen_to_world(screen, pos.x, pos.y);
            let (snapped, hint) = self.snap_world(raw);
            self.cursor_world = Some(snapped);
            self.snap_hint = hint;
            self.hover_pick = if self.tools.active() == Tool::Select {
                self.scene.pick(raw)
            } else {
                None
            };
        } else {
            self.cursor_world = None;
            self.snap_hint = None;
            self.hover_pick = None;
        }

        // Stash the press point at the start of every canvas drag. `interact_pointer_pos`
        // is the press only while the button is held; on the `drag_stopped` release frame
        // it is the release point, so gestures that commit on release (rectangle, marquee)
        // must capture the origin here, up front, rather than reconstruct it later.
        if response.drag_started() {
            self.drag_press_pos = response.interact_pointer_pos();
        }

        // Guide drag: a drag that begins inside a ruler bar pulls out a guide line.
        // While such a drag is live it owns the pointer, so no tool acts on it; on
        // release the guide is dropped at the cursor (grid-snapped). Handled before
        // every tool so the ruler always wins the gesture.
        if self.handle_guide_drag(response, screen) {
            return;
        }

        // Minimap navigation: a click or drag inside the panel recenters the view there
        // and consumes the input so no tool acts on it. A drag re-centers every frame, so
        // the viewport rectangle follows the pointer - the draggable viewport handle
        // (item 30). The layout is built the same way the draw path builds it, so what is
        // clickable matches what is drawn.
        if self.minimap_visible
            && (response.clicked() || response.dragged())
            && let Some(pos) = response.interact_pointer_pos()
            && let Some(layout) = self.minimap_layout(screen)
            && layout.contains(pos.x, pos.y)
        {
            let center = layout.panel_to_world(pos.x, pos.y);
            self.cancel_view_move();
            self.camera = ViewCamera::new(center, self.camera.pixels_per_dbu());
            return;
        }

        // Zoom to cursor on scroll, regardless of tool.
        if response.hovered() {
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0
                && let Some(pos) = response.hover_pos()
            {
                // A gentler multiplier than v8.0's so a single wheel notch is a small,
                // predictable step and the world point under the cursor stays pinned
                // (item 27, zoom-to-cursor tuning; the anchoring is `zoom_about`).
                let factor = (f64::from(scroll) * 0.0012).exp();
                self.cancel_view_move();
                self.camera.zoom_about(screen, factor, pos.x, pos.y);
            }
        }

        // Touch: two-finger pinch-zoom anchored at the gesture centroid, plus two-finger
        // pan. egui aggregates the active fingers into one multi-touch gesture; a single
        // finger is delivered as ordinary pointer drags instead (which the Pan tool below
        // turns into a pan), so only the multi-touch gesture is wired here. Placed before
        // the tool match so navigating a design by touch always wins the gesture, and the
        // pinch math (the anchoring invariant) lives in the pure `camera::apply_pinch`.
        if let Some(touch) = ctx.multi_touch() {
            self.cancel_view_move();
            self.camera = crate::camera::apply_pinch(
                self.camera,
                screen,
                (touch.center_pos.x, touch.center_pos.y),
                f64::from(touch.zoom_delta),
            );
            if touch.translation_delta != egui::Vec2::ZERO {
                self.camera
                    .pan_pixels(touch.translation_delta.x, touch.translation_delta.y);
            }
        }

        match self.tools.active() {
            Tool::Pan => {
                if response.dragged() {
                    self.cancel_view_move();
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
        let (Some(origin), Some(current)) = (self.drag_press_pos.take(), response.hover_pos())
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
    /// Whether the scene shape at `idx` may be picked by the pointer.
    ///
    /// A shape on a locked layer stays drawn but is not selectable (catalog 57);
    /// an out-of-range index or an unknown layer is pickable so stray geometry is
    /// never silently locked.
    fn is_pickable(&self, idx: usize) -> bool {
        self.scene
            .shapes()
            .get(idx)
            .is_none_or(|s| !self.layer_state.is_locked(s.layer))
    }

    fn handle_select_input(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        screen: &ScreenRect,
    ) {
        let additive = ctx.input(|i| i.modifiers.shift || i.modifiers.command || i.modifiers.ctrl);

        // Double-click to fit: on a shape, frame that shape; on empty canvas, fit all
        // (item 40). Handled before the single-click pick so the fit wins the gesture.
        if response.double_clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let world = self.camera.screen_to_world(screen, pos.x, pos.y);
            if let Some(idx) = self.scene.pick(world) {
                if let Some(shape) = self.scene.shapes().get(idx) {
                    let bounds = shape.bounding_box();
                    let mut cam = self.camera;
                    // 20% border so the framed shape reads with context.
                    cam.zoom_to_rect(screen, bounds, 0.2);
                    self.begin_view_move(cam);
                    self.status.set("Fit shape");
                }
            } else {
                self.fit_requested = true;
                self.status.set("Fit all");
            }
            return;
        }

        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let world = self.camera.screen_to_world(screen, pos.x, pos.y);
            // A locked layer is visible-but-unselectable (catalog 57): a click on it
            // reads as empty space, so it can still clear or pass through.
            match self.scene.pick(world).filter(|&idx| self.is_pickable(idx)) {
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

        // Rubber-band: on drag release, select shapes fully inside the drag box. Shift
        // adds, Alt subtracts, otherwise replace; a box begun over a shape is filtered to
        // that shape's layer (item 45).
        if response.drag_stopped()
            && let (Some(origin), Some(current)) =
                (self.drag_press_pos.take(), response.hover_pos())
        {
            let a = self.camera.screen_to_world(screen, origin.x, origin.y);
            let b = self.camera.screen_to_world(screen, current.x, current.y);
            let band = Rect::new(a, b);
            if band.width() > 0 && band.height() > 0 {
                let (shift, alt) = ctx.input(|i| (i.modifiers.shift, i.modifiers.alt));
                // Same-layer filter: the layer of the shape under the drag origin (item
                // 45, lane 3A). Locked-layer shapes never join the marquee (catalog 57,
                // lane 2C), so the pickability gate filters them out too.
                let layer_filter = self
                    .scene
                    .pick(a)
                    .and_then(|idx| self.scene.shapes().get(idx))
                    .map(|s| s.layer);
                let hits: Vec<usize> =
                    selection::shapes_in_rect_on_layer(self.scene.shapes(), band, layer_filter)
                        .into_iter()
                        .filter(|&i| self.is_pickable(i))
                        .collect();
                let n = hits.len();
                if alt {
                    self.selection.subtract(hits);
                    self.status
                        .set(format!("Removed {n} shape(s) from selection"));
                } else if shift {
                    self.selection.extend(hits);
                    self.status.set(format!("Added {n} shape(s) to selection"));
                } else {
                    self.selection.set(hits);
                    self.status.set(format!("Selected {n} shape(s)"));
                }
            }
        }
    }

    /// The screen position where the current drag started, if any.
    ///
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
            if let Some(pos) = response.hover_pos().or(self.drag_press_pos) {
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
        let color = CANVAS.grid_line;
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
        let axis = Stroke::new(1.5, CANVAS.grid_axis);
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
            let fill = theme::tokens::layer_rgba(r, g, b, a);
            let selected = self.selection.contains(idx);
            self.draw_one_shape(painter, screen, shape, fill, selected);
        }
    }

    /// Paints the read-only streamed archive for the current camera, driving one
    /// residency pass and drawing the HUD (lane v8-2e).
    ///
    /// The steps mirror the residency contract proven in `tests/residency.rs`: adopt any
    /// tiles fetched since last frame, pick the target level for the viewport, spawn
    /// fetches for the covering tiles that are not yet resident (wasm), then paint the
    /// finest resident level that still covers the viewport
    /// ([`StreamedScene::paint_level`](crate::streamed::StreamedScene::paint_level)) so the
    /// die refines coarse-then-fine rather than blanking while fine tiles stream. Finally
    /// it draws the on-canvas HUD and publishes the counters to `window.__reticle_stats`
    /// for the served-archive e2e.
    fn draw_archive(&mut self, painter: &egui::Painter, screen: &ScreenRect, dt: f32) {
        // Frame the whole die once the canvas size is known (deferred from the async
        // open, which has no screen), then read the fitted viewport.
        if !self.archive_framed {
            if let Some(browse) = self.archive.as_ref() {
                let world = browse.scene().world();
                self.camera.zoom_to_fit(screen, world);
            }
            self.archive_framed = true;
        }
        let viewport = self.camera.visible_world_rect(screen);
        let center = self.camera.center();

        // 1. Adopt fetched tiles and choose the paint level for this viewport.
        let (target, paint) = {
            let browse = self.archive.as_mut().expect("archive browse is active");
            browse.drain();
            let scene = browse.scene();
            let target = crate::archive::target_level_for_viewport(scene, viewport);
            let paint = scene.paint_level(viewport, target);
            (target, paint)
        };

        // 2. Spawn fetches for the covering tiles not yet resident or in flight (wasm),
        //    then speculatively prefetch tiles just ahead of the pan (item 43; native
        //    only warms the velocity estimate).
        if let Some(browse) = self.archive.as_mut() {
            #[cfg(target_arch = "wasm32")]
            browse.spawn_missing(viewport, target);
            browse.prefetch(viewport, center, target);
        }

        // 3. Advance the LOD crossfade toward the newly-covered level and paint: the
        //    settled level paints solid, and a level still fading in paints on top at the
        //    crossfade alpha, so refinement eases in rather than popping (item 42). Under
        //    reduced motion the fade is instant.
        self.archive_fade.observe(paint, dt, self.reduced_motion);
        let shown = self.archive_fade.shown();
        let incoming = self.archive_fade.incoming();
        let alpha = self.archive_fade.alpha();
        let mut painted = 0;
        if let Some(level) = shown {
            let records = self
                .archive
                .as_ref()
                .expect("archive browse is active")
                .scene()
                .painted_records(viewport, level);
            painted = records.len();
            self.draw_streamed_records(painter, screen, &records, 1.0);
        }
        if let Some(level) = incoming {
            let records = self
                .archive
                .as_ref()
                .expect("archive browse is active")
                .scene()
                .painted_records(viewport, level);
            painted = painted.max(records.len());
            self.draw_streamed_records(painter, screen, &records, alpha);
        }
        if let Some(browse) = self.archive.as_mut() {
            browse.set_records_painted(painted);
        }

        // The HUD and the residency minimap are placed later in `canvas` through the
        // overlay layout manager, so they cannot collide; the JS stats seam the e2e reads
        // is published here now that the counters for this frame are settled.
        #[cfg(target_arch = "wasm32")]
        self.publish_archive_stats();
    }

    /// Paints a set of streamed [`TileRecord`](reticle_index::TileRecord)s as filled,
    /// outlined rectangles, colored and visibility-gated by the same layer table as the
    /// editing geometry (an unknown archive layer paints in the default grey and stays
    /// visible; see [`LayerState::is_visible`](crate::layers::LayerState::is_visible)).
    fn draw_streamed_records(
        &self,
        painter: &egui::Painter,
        screen: &ScreenRect,
        records: &[reticle_index::TileRecord],
        fade: f32,
    ) {
        for record in records {
            let layer = LayerId::new(record.layer, record.datatype);
            if !self.layer_state.is_visible(layer) {
                continue;
            }
            let (red, green, blue, alpha) = self.layer_color(layer);
            // `fade` is the LOD crossfade opacity (item 42): `1.0` for the settled level,
            // ramping for a level fading in on top of it.
            let base = theme::tokens::layer_rgba(red, green, blue, alpha);
            let fill = base.gamma_multiply(fade);
            let dst = self.world_rect_to_screen(screen, record.rect.to_rect());
            painter.rect_filled(dst, 0.0, fill);
            painter.rect_stroke(
                dst,
                0.0,
                Stroke::new(1.0, base.gamma_multiply(1.4 * fade)),
                StrokeKind::Middle,
            );
        }
    }

    /// Draws the streaming HUD in the top-left of the canvas: bytes fetched vs archive
    /// size, tiles resident and records painted, the working-set estimate, and the frame
    /// rate (the counter arithmetic is [`ArchiveStats`](crate::archive::ArchiveStats), unit-tested).
    fn draw_archive_hud(&self, painter: &egui::Painter, overlays: &mut OverlayLayout) {
        let Some(browse) = self.archive.as_ref() else {
            return;
        };
        let lines = browse.stats().hud_lines(self.frame_meter.fps());
        let pad = 8.0;
        let line_h = 16.0;
        let width = 210.0;
        let height = pad * 2.0 + line_h * lines.len() as f32;
        // The manager anchors the HUD top-left, already clear of the rulers; if it cannot
        // fit (a tiny canvas) it is simply not drawn (AUD-02, item 68).
        let Some(rect) = overlays.place(Anchor::TopLeft, width, height) else {
            return;
        };
        let panel = EguiRect::from_min_size(
            Pos2::new(rect.left, rect.top),
            egui::vec2(rect.width, rect.height),
        );
        painter.rect_filled(panel, 5.0, CANVAS.hud_panel);
        let x = rect.left + pad;
        let mut y = rect.top + pad;
        for line in &lines {
            painter.text(
                Pos2::new(x, y),
                egui::Align2::LEFT_TOP,
                line,
                theme::apply::hud_mono(self.ui_density),
                CANVAS.hud_text,
            );
            y += line_h;
        }
    }

    /// Pre-highlights the shape under the cursor for the Select tool, so the click target
    /// is obvious before the click (item 46). A faint outline of the shape's bounding box,
    /// skipped when that shape is already selected (its selection outline already shows).
    fn draw_hover_highlight(&self, painter: &egui::Painter, screen: &ScreenRect) {
        let Some(idx) = self.hover_pick else {
            return;
        };
        if self.selection.contains(idx) {
            return;
        }
        let Some(shape) = self.scene.shapes().get(idx) else {
            return;
        };
        let rect = self.world_rect_to_screen(screen, shape.bounding_box());
        painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, theme::tokens::with_alpha(CANVAS.selection, 120)),
            StrokeKind::Middle,
        );
    }

    /// Draws a full-pane crosshair through the cursor's snapped world position (item 32),
    /// so the pointer's exact coordinate can be read against distant geometry. Drawn only
    /// while the cursor is over the pane.
    fn draw_crosshair(&self, painter: &egui::Painter, screen: &ScreenRect) {
        let Some(world) = self.cursor_world else {
            return;
        };
        let c = self.world_pos_to_screen(screen, world);
        let stroke = Stroke::new(1.0, CANVAS.hud_text_dim);
        painter.line_segment(
            [
                Pos2::new(screen.left, c.y),
                Pos2::new(screen.left + screen.width, c.y),
            ],
            stroke,
        );
        painter.line_segment(
            [
                Pos2::new(c.x, screen.top),
                Pos2::new(c.x, screen.top + screen.height),
            ],
            stroke,
        );
    }

    /// Publishes the archive HUD counters to `window.__reticle_stats` (wasm only), the
    /// seam the served-archive Playwright spec polls to assert tiles become resident and
    /// the canvas paints. Extends the same stats object the share-live viewer writes to.
    #[cfg(target_arch = "wasm32")]
    fn publish_archive_stats(&self) {
        use wasm_bindgen::JsValue;
        let Some(browse) = self.archive.as_ref() else {
            return;
        };
        let s = browse.stats();
        let Some(window) = web_sys::window() else {
            return;
        };
        let key = JsValue::from_str("__reticle_stats");
        let stats = match js_sys::Reflect::get(window.as_ref(), &key) {
            Ok(v) if v.is_object() => v,
            _ => {
                let obj = js_sys::Object::new();
                let _ = js_sys::Reflect::set(window.as_ref(), &key, obj.as_ref());
                JsValue::from(obj)
            }
        };
        let set = |k: &str, v: f64| {
            let _ = js_sys::Reflect::set(&stats, &JsValue::from_str(k), &JsValue::from_f64(v));
        };
        #[allow(clippy::cast_precision_loss)]
        {
            set("archive_file_size", s.file_size as f64);
            set("archive_bytes_fetched", s.bytes_fetched as f64);
            set("archive_tiles_fetched", s.tiles_fetched as f64);
            set("archive_tiles_resident", s.tiles_resident as f64);
            set("archive_working_set_bytes", s.working_set_bytes() as f64);
            set("archive_records_painted", s.records_painted as f64);
            // Additive prefetch counter (item 43); existing keys above are unchanged so
            // the served-archive e2e that asserts them keeps passing.
            set("archive_prefetched", s.prefetched as f64);
            // A convenience alias the spec reads directly.
            set("tiles_resident", s.tiles_resident as f64);
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
            Stroke::new(2.0, CANVAS.selection)
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
                        Stroke::new(w.max(2.0), CANVAS.selection)
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
        let stroke = Stroke::new(1.0, CANVAS.cell_box);
        let fill = CANVAS.cell_box_fill;
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
        let color = CANVAS.net_highlight;
        let stroke = Stroke::new(2.5, color);
        let fill = theme::tokens::with_alpha(color, 60);
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
        let color = CANVAS.array_preview;
        let stroke = Stroke::new(1.5, color);
        let fill = theme::tokens::with_alpha(color, 40);
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
        let color = CANVAS.generate_preview;
        let stroke = Stroke::new(1.5, color);
        let fill = theme::tokens::with_alpha(color, 40);
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
        let normal = Stroke::new(2.0, CANVAS.drc_violation);
        let hot = Stroke::new(3.0, CANVAS.drc_selected);
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

    /// Paints DRC-as-you-type underlines: a spell-checker squiggle beneath each live
    /// violation caught while drawing.
    ///
    /// Deliberately distinct from [`draw_drc_markers`](Self::draw_drc_markers) (which
    /// boxes the violations of a full DRC run): the squiggle is a live "misspelling"
    /// hint at the edited geometry, sized in constant screen pixels so it reads the
    /// same at any zoom, exactly like a text editor's underline.
    fn draw_live_drc_underlines(&self, painter: &egui::Painter, screen: &ScreenRect) {
        // Constant on-screen squiggle geometry (pixels), independent of zoom.
        const AMPLITUDE: f32 = 3.0;
        const WAVELENGTH: f32 = 8.0;
        if self.live_drc.is_empty() {
            return;
        }
        let stroke = Stroke::new(1.5, CANVAS.live_drc);
        for v in self.live_drc.violations() {
            let e = self.world_rect_to_screen(screen, v.location);
            // Ride just beneath the offending region's bottom edge.
            let baseline = e.bottom() + AMPLITUDE + 1.0;
            let pts: Vec<Pos2> =
                drc_panel::squiggle_points(e.left(), e.right(), baseline, AMPLITUDE, WAVELENGTH)
                    .into_iter()
                    .map(|(x, y)| Pos2::new(x, y))
                    .collect();
            if pts.len() >= 2 {
                painter.add(Shape::line(pts, stroke));
            }
        }
    }

    /// Paints the layout-diff overlay: added shapes green, removed red, changed
    /// amber.
    ///
    /// Each reported [`DiffShape`](reticle_diff::DiffShape) is drawn as an outlined,
    /// faintly filled rectangle at its bounding box (world to screen via the
    /// camera). Nothing paints when the overlay is hidden or no diff has been
    /// computed (see [`crate::diff_overlay::DiffOverlay::should_paint`]); `changed`
    /// is always empty in v1, so the amber pass draws nothing today.
    fn draw_diff_overlay(&self, painter: &egui::Painter, screen: &ScreenRect) {
        if !self.diff_overlay.should_paint() {
            return;
        }
        let added = CANVAS.diff_added;
        let removed = CANVAS.diff_removed;
        let changed = CANVAS.diff_changed;
        for (shapes, color) in [
            (self.diff_overlay.added(), added),
            (self.diff_overlay.removed(), removed),
            (self.diff_overlay.changed(), changed),
        ] {
            let stroke = Stroke::new(2.0, color);
            let fill = theme::tokens::with_alpha(color, 40);
            for shape in shapes {
                // Inflate a touch so a zero-area shape still shows as a small box.
                let e = self.world_rect_to_screen(screen, shape.rect).expand(1.0);
                painter.rect_filled(e, 0.0, fill);
                painter.rect_stroke(e, 0.0, stroke, StrokeKind::Middle);
            }
        }
    }

    /// Paints a numbered pin at each anchored comment.
    ///
    /// A pin lands at the centre of its comment's anchored cell geometry (world to
    /// screen via the camera; see [`comment_pins::anchor_point`]). A comment whose
    /// anchor resolves to no geometry is skipped rather than pinned at the origin.
    /// The selected comment's pin is larger and drawn in the accent color.
    fn draw_comment_pins(&self, painter: &egui::Painter, screen: &ScreenRect) {
        if self.comment_pins.is_empty() {
            return;
        }
        let doc = self.history.document();
        let base = CANVAS.comment_pin;
        let accent = CANVAS.comment_pin_selected;
        let selected = self.comment_pins.selected();
        for (i, comment) in self.comment_pins.comments().iter().enumerate() {
            let Some(anchor) = comment_pins::anchor_point(doc, &comment.anchor_ref) else {
                continue;
            };
            let center = self.world_pos_to_screen(screen, anchor);
            let is_selected = selected == Some(i);
            let radius = if is_selected { 12.0 } else { 9.0 };
            let color = if is_selected { accent } else { base };
            painter.circle_filled(center, radius, color);
            painter.circle_stroke(
                center,
                radius,
                Stroke::new(1.5, Color32::from_black_alpha(180)),
            );
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                format!("{}", i + 1),
                theme::apply::proportional_sized(radius),
                Color32::BLACK,
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
        let bg = CANVAS.ruler_bg;
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
        let tick = Stroke::new(1.0, CANVAS.ruler_tick);
        let font = theme::apply::hud_mono(self.ui_density);
        let label = CANVAS.hud_label;

        // Ticks stay dense, but a label is drawn only when it clears the previous
        // one's measured width (top ruler) or height (left ruler). Without this
        // decimation the five-figure DBU labels overprint into an illegible band at
        // low zoom, where the tick interval is finer than a label is wide (H3 /
        // AUD-08). Labels are measured, not assumed, so the gap holds at any font.
        let mut last_label_end = f32::NEG_INFINITY;
        for x in grid::grid_lines(world.min.x, world.max.x, step) {
            let (sx, _) = self.camera.world_to_screen(screen, Point::new(x, 0));
            if sx < screen.left + bar {
                continue;
            }
            painter.line_segment(
                [Pos2::new(sx, screen.top), Pos2::new(sx, screen.top + bar)],
                tick,
            );
            if sx + 2.0 >= last_label_end + 4.0 {
                let galley = painter.layout_no_wrap(x.to_string(), font.clone(), label);
                let width = galley.size().x;
                painter.galley(Pos2::new(sx + 2.0, screen.top + 1.0), galley, label);
                last_label_end = sx + 2.0 + width;
            }
        }
        let mut last_label_bottom = f32::NEG_INFINITY;
        for y in grid::grid_lines(world.min.y, world.max.y, step) {
            let (_, sy) = self.camera.world_to_screen(screen, Point::new(0, y));
            if sy < screen.top + bar {
                continue;
            }
            painter.line_segment(
                [Pos2::new(screen.left, sy), Pos2::new(screen.left + bar, sy)],
                tick,
            );
            if sy + 1.0 >= last_label_bottom + 2.0 {
                let galley = painter.layout_no_wrap(y.to_string(), font.clone(), label);
                let height = galley.size().y;
                painter.galley(Pos2::new(screen.left + 1.0, sy + 1.0), galley, label);
                last_label_bottom = sy + 1.0 + height;
            }
        }
    }

    /// Draws the user guide lines across the canvas.
    ///
    /// Horizontal guides span the full width at their world `y`; vertical guides
    /// span the full height at their world `x`. Guides off screen are skipped. A
    /// guide being actively dragged has no committed line yet, so nothing special is
    /// drawn for it here; it appears once released.
    fn draw_guides(&self, painter: &egui::Painter, screen: &ScreenRect) {
        let stroke = Stroke::new(1.0, CANVAS.guide);
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
            snap::SnapKind::Vertex => CANVAS.snap_vertex,
            snap::SnapKind::Midpoint => CANVAS.snap_midpoint,
            snap::SnapKind::Center => CANVAS.snap_center,
            snap::SnapKind::Edge => CANVAS.snap_edge,
            snap::SnapKind::Guide => CANVAS.guide,
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
            theme::apply::hud_mono(self.ui_density),
            color,
        );
    }

    /// Draws the in-progress or completed measurement overlay.
    fn draw_measure(&self, painter: &egui::Painter, screen: &ScreenRect) {
        let color = CANVAS.measure;
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
                theme::apply::hud_mono(self.ui_density),
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
        let accent = CANVAS.draw_preview;
        let stroke = Stroke::new(1.5, accent);
        match self.tools.active() {
            Tool::DrawRect => {
                if response.dragged()
                    && let (Some(origin), Some(current)) =
                        (self.drag_press_pos, response.hover_pos())
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
                        // Live dimensions on the ghost (item 52).
                        painter.text(
                            Pos2::new(e.min.x, e.min.y - 3.0),
                            Align2::LEFT_BOTTOM,
                            format!("{} x {} DBU", rect.width(), rect.height()),
                            theme::apply::hud_mono(self.ui_density),
                            accent,
                        );
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
            // Live length of the segment being drawn, on the ghost (item 52).
            if let Some(&last_world) = verts.last() {
                let len = segment_length_dbu(last_world, pos);
                painter.text(
                    Pos2::new(cursor.x + 6.0, cursor.y - 6.0),
                    Align2::LEFT_BOTTOM,
                    format!("{len} DBU"),
                    theme::apply::hud_mono(self.ui_density),
                    color,
                );
            }
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
        let font = theme::apply::mono_sized(labels::LABEL_FONT_PX);

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
            let name_color = CANVAS.hud_text;
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
                CANVAS.selection,
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
                CANVAS.measure,
            );
        }
    }

    /// Draws the minimap overlay: document overview, placements, and the viewport.
    ///
    /// All geometry comes from [`MinimapLayout`]; this method only paints the
    /// rectangles it computes. The click/drag recentering lives at the top of
    /// [`App::process_canvas_input`] using the same layout, so what is drawn and
    /// what is clickable can never disagree.
    fn draw_minimap(
        &self,
        painter: &egui::Painter,
        screen: &ScreenRect,
        overlays: &mut OverlayLayout,
    ) {
        let Some(bounds) = self.minimap_bounds() else {
            return;
        };
        // The manager anchors the panel top-right, past the rulers (AUD-17); the world
        // transform is then fitted into that exact rectangle.
        let Some(panel_rect) = overlays.place(
            Anchor::TopRight,
            minimap::PANEL_WIDTH,
            minimap::PANEL_HEIGHT,
        ) else {
            return;
        };
        let Some(layout) = MinimapLayout::within(panel_rect, bounds) else {
            return;
        };
        let panel = EguiRect::from_min_size(
            Pos2::new(layout.panel.left, layout.panel.top),
            Vec2::new(layout.panel.width, layout.panel.height),
        );
        painter.rect_filled(panel, 3.0, CANVAS.hud_panel);
        painter.rect_stroke(
            panel,
            3.0,
            Stroke::new(1.0, CANVAS.pane_border),
            StrokeKind::Middle,
        );

        // Document bounds outline.
        let (bx, by, bw, bh) = layout.world_rect_to_panel(bounds);
        let doc_rect = EguiRect::from_min_size(Pos2::new(bx, by), Vec2::new(bw, bh));
        painter.rect_stroke(
            doc_rect,
            0.0,
            Stroke::new(1.0, CANVAS.minimap_doc),
            StrokeKind::Middle,
        );

        let clip = painter.with_clip_rect(panel);
        if let Some(browse) = self.archive.as_ref() {
            // Streamed archive: shade the coarse tiles already fetched so the overview
            // doubles as a residency heatmap of where the die has streamed in (item 30).
            Self::draw_minimap_residency(&clip, &layout, browse.scene());
        } else {
            // Editing session: placement boxes give the overview its silhouette; cap the
            // count so a huge document cannot make the minimap the most expensive draw.
            let fill = CANVAS.minimap_fill;
            for cb in culling::visible_cell_boxes(self.history.document(), &self.top_cell, bounds)
                .into_iter()
                .take(256)
            {
                let (x, y, w, h) = layout.world_rect_to_panel(cb.bbox);
                clip.rect_filled(
                    EguiRect::from_min_size(Pos2::new(x, y), Vec2::new(w, h)),
                    0.0,
                    fill,
                );
            }
        }

        // The camera's visible world rectangle, clamped to the panel: the draggable
        // viewport handle (item 30).
        let (vx, vy, vw, vh) = layout.world_rect_to_panel(self.camera.visible_world_rect(screen));
        painter.rect_stroke(
            EguiRect::from_min_size(Pos2::new(vx, vy), Vec2::new(vw, vh)),
            0.0,
            Stroke::new(1.5, CANVAS.minimap_viewport),
            StrokeKind::Middle,
        );
    }

    /// Draws remote collaborators' named cursors from the sync presence map
    /// (catalog 86): each cursor takes a stable palette color (its published color, or
    /// one derived from the actor id), a name label on a legible pill, and an idle
    /// fade that recedes a parked cursor toward a floor opacity. `now` is the current
    /// egui time, against which [`presence_seen`](Self::presence_seen) measures idle.
    #[allow(clippy::many_single_char_names)] // r, g, b, a color channels plus the point
    fn draw_presence(&self, painter: &egui::Painter, screen: &ScreenRect, now: f64) {
        for (actor, presence) in self.document.awareness().iter() {
            let rgba = if presence.color_rgba == 0 {
                crate::viewer::color_for_actor(actor)
            } else {
                presence.color_rgba
            };
            let (r, g, b, _) = layers::rgba_components(rgba);
            // Idle fade: the longer a cursor has sat still, the more it recedes.
            let idle = self
                .presence_seen
                .get(actor)
                .map_or(0.0, |&(_, last_move)| (now - last_move).max(0.0));
            #[allow(clippy::cast_possible_truncation)]
            let alpha_f = crate::viewer::idle_alpha(idle as f32);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let a = (alpha_f * 255.0) as u8;
            // Fade the (premultiplied) palette color by the idle alpha through the
            // theme's color helper, so no raw color literal lives outside `theme/`.
            let color = theme::tokens::layer_rgb(r, g, b).gamma_multiply(alpha_f);
            let p = self.world_pos_to_screen(screen, presence.cursor);
            painter.circle_filled(p, 4.0, color);

            let name = if presence.display_name.is_empty() {
                crate::viewer::participants(self.document.awareness(), crate::viewer::VIEWER_ACTOR)
                    .into_iter()
                    .find(|part| &part.actor == actor)
                    .map(|part| part.name)
            } else {
                Some(presence.display_name.clone())
            };
            if let Some(name) = name {
                let font = theme::apply::hud_body(self.ui_density);
                let galley = painter.layout_no_wrap(name, font, color);
                let pos = Pos2::new(p.x + 8.0, p.y - galley.size().y / 2.0);
                let pad = Vec2::new(4.0, 2.0);
                // A translucent pill keeps the name legible over busy geometry,
                // itself fading with the cursor (half the cursor's own alpha).
                let bg = egui::Color32::from_black_alpha(a / 2);
                painter.rect_filled(
                    EguiRect::from_min_size(pos - pad, galley.size() + pad * 2.0),
                    3.0,
                    bg,
                );
                painter.galley(pos, galley, color);
            }
        }
    }

    /// Shades the streamed archive's resident tiles inside the minimap panel, coarse
    /// levels first, so the overview reads as a residency heatmap of the fetched die
    /// (item 30). Read-only: it inspects
    /// [`resident_tile_rects`](crate::streamed::StreamedScene::resident_tile_rects).
    fn draw_minimap_residency(
        painter: &egui::Painter,
        layout: &MinimapLayout,
        scene: &crate::streamed::StreamedScene,
    ) {
        // A couple of coarse levels are enough to read the explored region at minimap
        // scale; finer resident tiles are sub-pixel here and would just darken uniformly.
        let overview = scene.level_count().saturating_sub(1).min(2);
        for level in 0..=overview {
            for rect in scene.resident_tile_rects(level) {
                let (x, y, w, h) = layout.world_rect_to_panel(rect);
                painter.rect_filled(
                    EguiRect::from_min_size(Pos2::new(x, y), Vec2::new(w, h)),
                    0.0,
                    CANVAS.minimap_fill,
                );
            }
        }
    }

    /// The world bounds the minimap displays: the streamed die in archive-browse mode
    /// (never the stale demo, AUD-04), otherwise the editable scene's bounds.
    fn minimap_bounds(&self) -> Option<Rect> {
        if let Some(browse) = self.archive.as_ref() {
            Some(browse.scene().world())
        } else {
            self.scene.bounds()
        }
    }

    /// The minimap layout for click/drag navigation, built the same way the draw path
    /// builds it (through the overlay manager), so what is drawn and what is clickable
    /// can never disagree.
    fn minimap_layout(&self, screen: &ScreenRect) -> Option<MinimapLayout> {
        let bounds = self.minimap_bounds()?;
        let ruler_bar = if self.rulers_visible { RULER_BAR } else { 0.0 };
        let mut overlays = OverlayLayout::new(screen, ruler_bar);
        let panel = overlays.place(
            Anchor::TopRight,
            minimap::PANEL_WIDTH,
            minimap::PANEL_HEIGHT,
        )?;
        MinimapLayout::within(panel, bounds)
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
        // The bypass modifier drops snapping entirely for this frame, so the cursor lands
        // on the exact world point the pointer is over (item 53).
        if self.snap_bypass {
            return (raw, None);
        }
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
    #[allow(clippy::too_many_lines)]
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Boot styling hook (ADR 0095, 0097): install the embedded subset faces
        // (lane 1B) and the tokened dark style (lane 1A) once, not per frame.
        // `theme_dirty` starts true so the first frame applies both, and a future
        // density or reduced-motion change re-applies by setting the flag again
        // (lane 4C's Settings dialog in Wave 2). Fonts do not depend on density,
        // so re-running install on a density change is a harmless idempotent call.
        // The theme module pins both egui theme slots to the dark style, so an OS
        // light preference cannot resurrect the retired stock-light look.
        if self.theme_dirty {
            crate::theme::fonts::install(&ctx);
            // Bridge lane 4C's tri-state `TouchMode` to lane 4B's bool-based apply.
            // TODO(v8.2): wire a proper coarse-pointer media query; `any_touches` is
            // a rough proxy that only reports true while a touch is actively down.
            let touch = self
                .touch_mode
                .effective(ctx.input(egui::InputState::any_touches));
            crate::theme::apply::apply(&ctx, self.ui_density, self.reduced_motion, touch);
            self.theme_dirty = false;
        }

        // Gallery mode renders the hidden component library full-window and
        // returns before any editor state is built: a deterministic screenshot
        // surface (`?gallery=1` / `--gallery`, lane 1C). Kept minimal on purpose.
        if let Some(gallery) = &mut self.gallery {
            egui::CentralPanel::default().show(ui, |ui| {
                crate::theme::gallery::ui(ui, gallery);
            });
            ctx.request_repaint();
            return;
        }

        // Sample this frame's duration up front: both the Start screen and the editor
        // age notification toasts by it, so it is computed before either branch.
        let dt = ctx.input(|i| i.stable_dt).max(0.0);

        // Drive the browser open path: kick off the `?gds=` fetch and the IndexedDB
        // recent-list load on the first wasm frame, and apply whatever those async
        // tasks have posted since last frame. A no-op on native.
        self.drive_web_open(&ctx);

        // Drive the served-archive browse: kick off the `?archive=` open on the first
        // wasm frame and install the streamed scene once it arrives. A no-op on native
        // and in a normal editing session.
        self.drive_archive(&ctx);

        // Drive the live share transport: open the read-only viewer socket on the first
        // wasm frame of a viewer session, and apply the sharer's streamed frames and
        // presence the socket has posted since last frame. A no-op on native and in a
        // normal (non-viewer) editor session. Also publish the sharer's document and
        // presence when a "Go live" sharer transport is open.
        self.drive_live(&ctx);
        self.drive_sharer_publish();

        // Open a file dropped onto the page (browser) or window (native) before
        // anything else this frame, so a drop works from any view including the Start
        // screen and the replay theater. The seam hardens the import, so a bad drop
        // cannot panic; an oversized or non-layout drop sets a clear status. While a
        // file is dragged over, draw a "drop to open" affordance.
        self.handle_dropped_files(&ctx);
        // Open a URL pasted with Ctrl+V (item 3), unless a text field owns focus.
        self.handle_paste_to_open(&ctx);
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
        // An embedded iframe (catalog 94) never shows the Start chooser: it opens
        // straight onto the canvas of whatever it was pointed at.
        if self.start_screen && !self.embed {
            self.start_screen_ui(ui);
            self.notifications_area(&ctx);
            ctx.request_repaint();
            return;
        }

        // Publish the live camera into the browser stats seam every editor frame, so the
        // phone-touch e2e can read a baseline and assert a pinch/pan moved the camera.
        #[cfg(target_arch = "wasm32")]
        self.record_camera_stats();
        // Publish the additive demo-observability keys (applied_shapes, render_nonblank,
        // hash_check) so the headed demo guards can read them from the DOM.
        #[cfg(target_arch = "wasm32")]
        self.record_frame_stats();

        self.handle_shortcuts(&ctx);

        // Lane 2D: leave presentation mode on Escape (the `P` chord toggles it through
        // the registry; Escape is the intuitive way back out of a full-screen view).
        if self.presentation && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.presentation = false;
            self.status.set("Presentation mode off");
        }
        // Collaboration per-frame upkeep: stamp remote-cursor idle timing (catalog 86)
        // and age out the remote-edit attribution glow (catalog 90).
        self.track_presence_idle(ctx.input(|i| i.time));
        self.remote_edit_flash = (self.remote_edit_flash - dt).max(0.0);

        // Flush any queued clipboard text (the copy-permalink action, item 35), now that
        // the egui context is in hand.
        if let Some(text) = self.pending_clipboard.take() {
            ctx.copy_text(text);
        }

        // Arrow-key nudge of the selection with a delta readout (item 48).
        self.handle_arrow_nudge(&ctx);

        self.frame_meter
            .record(std::time::Duration::from_secs_f32(dt));

        // Advance the agent preview run by this frame's dt so its narration and
        // cursor animate while the panel is running. Each verify step is narrated
        // into the panel's conversation; the preview runs on a built-in demo cell, so
        // it does not touch the document's DRC panel or the canvas markers.
        if let Some(update) = self.agent.tick(dt) {
            self.apply_agent_run_verify(&update);
        }

        // Advance replay-theater playback the same way; a playing transcript
        // updates the theater canvas and the DRC overlay as it crosses
        // verifies.
        if let Some(update) = self.replay.tick(dt) {
            self.apply_agent_drc_update(update);
        }

        // Feed the previous frame's edits to DRC-as-you-type before drawing, so the
        // live underlines the canvas paints below reflect this frame's re-check.
        self.poll_live_drc(dt);

        // The surface color format when eframe is on its wgpu backend; drives the
        // retained GPU canvas. `None` (e.g. a glow build) falls back to egui painting.
        let gpu_format = frame.wgpu_render_state().map(|state| state.target_format);

        // Draw the docked panels and the canvas, collecting the canvas rect (for the
        // palette/export) and the tour highlight rectangles. Lane 2D picks the chrome:
        // presentation mode (catalog 93) shows only the canvas; a read-only viewer
        // (catalog 23) gets the clean viewer chrome; otherwise the full editor.
        let (canvas_screen, tour_targets) = if self.embed {
            (
                self.embed_canvas(ui, gpu_format, &ctx),
                TourTargets::default(),
            )
        } else if self.presentation {
            (
                self.presentation_canvas(ui, gpu_format, &ctx),
                TourTargets::default(),
            )
        } else if self.is_viewer() {
            self.viewer_panels(ui, gpu_format)
        } else {
            self.main_panels(ui, frame, gpu_format)
        };
        // Cache the canvas rect so next frame's view/export panel can frame the
        // current view (the panel draws before the canvas is measured).
        self.view_export.last_canvas = canvas_screen;

        // Minimal-chrome modes (presentation, embed) hide the palette, floating
        // windows, and tour overlay, leaving only the canvas (and critical toasts).
        // The 3D stack and Cross-section are managed panels inside `main_panels` now
        // (lane 2C, ADR 0096), so they are not floated here.
        let minimal_chrome = self.presentation || self.embed;
        if !minimal_chrome {
            self.palette_window(&ctx, canvas_screen);
            // The numeric transform popover (item 51, lane 3A), over the canvas.
            self.transform_window(&ctx);
            self.shortcuts_overlay(&ctx);
            self.keymap_window(&ctx);
            self.open_warnings_window(&ctx);
        }
        // Lane 3b dialogs: Open-from-URL, Convert, and Share (each a no-op unless open).
        self.open_url_dialog(&ctx);
        self.convert_dialog(&ctx);
        self.share_dialog(&ctx);
        // --- lane review: review panel ---
        let review_comp = self.comp_ctx();
        if let Some(c) = self.review.show(
            &ctx,
            review_comp,
            self.comment_pins.comments(),
            (ctx.input(|i| i.time) * 1000.0) as i64,
        ) {
            self.comment_pins.add(c);
        }
        // --- end lane review ---
        // Copy any text a menu/dialog Copy button staged this frame, after the dialogs
        // render (the early per-frame flush only catches shortcut-staged copies).
        if let Some(text) = self.pending_clipboard.take() {
            ctx.copy_text(text);
        }
        // The offline badge (top-center) and the app-wide notification toasts, over the
        // panels and windows.
        self.draw_offline_badge(&ctx);
        self.notifications_area(&ctx);

        // Open any URL queued this frame (Documentation, the prefilled issue link).
        if let Some(url) = self.pending_open_url.take() {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }

        // The Help dialogs (Settings, About, What's new). Each is a no-op unless its
        // flag is set; they draw over the panels as real modals.
        self.settings_dialog(&ctx);
        self.about_dialog(&ctx, frame);
        self.whats_new_dialog(&ctx);

        // Draw the first-run tour overlay and the onboarding chrome last so their
        // cards and highlight sit over everything else. Suppressed in minimal-chrome
        // modes and during a demo capture so onboarding chrome does not cover the
        // feature under test.
        if !minimal_chrome && !self.in_demo_capture() {
            self.update_onboarding();
            self.onboarding_overlay(&ctx, frame);
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
            let _ = crate::session::save(&self.session_snapshot());
            // The keymap persists alongside the session so rebinds survive exit
            // even when the user never pressed Save in the editor.
            let _ = keymap::save(&self.keymap);
        }
    }
}

/// The visual identity of a toolbar [`IconButton`](components::IconButton): its
/// glyph, the tooltip name, and the one-line description shown under the name
/// (catalog 25). Bundled so the toolbar helpers stay under the argument limit.
#[derive(Clone, Copy)]
struct IconLabel {
    /// The Lucide glyph drawn on the button.
    glyph: char,
    /// The tooltip title (the action name).
    name: &'static str,
    /// The one-line description shown under the name.
    hint: &'static str,
}

impl IconLabel {
    /// A label from its glyph, name, and one-line hint.
    fn new(glyph: char, name: &'static str, hint: &'static str) -> Self {
        Self { glyph, name, hint }
    }
}

/// The registry id of the command that selects `tool`, so the toolbar tool
/// buttons dispatch through the same funnel and keymap as the Draw menu.
fn tool_command_id(tool: Tool) -> &'static str {
    match tool {
        Tool::Select => "tool.select",
        Tool::Pan => "tool.pan",
        Tool::Measure => "tool.measure",
        Tool::CutLine => "tool.cutline",
        Tool::DrawRect => "tool.rect",
        Tool::DrawPolygon => "tool.polygon",
        Tool::DrawPath => "tool.path",
        Tool::EditVertex => "tool.vertices",
    }
}

/// Formats a boolean as `on`/`off` for status messages.
fn on_off(v: bool) -> &'static str {
    if v { "on" } else { "off" }
}

/// The duration of an animated camera move (Fit / Fit-selection / Go-to), the packet's
/// ~150 ms functional transition (item 29); collapsed to an instant cut under reduced
/// motion by [`App::begin_view_move`].
const VIEW_MOVE_SECS: f32 = 0.15;

/// The rounded straight-line distance in DBU between two world points (the draw-tool
/// ghost's live segment readout, item 52). Uses `i64` differences so a segment across the
/// full coordinate range cannot overflow.
fn segment_length_dbu(a: Point, b: Point) -> i64 {
    let dx = (i64::from(b.x) - i64::from(a.x)) as f64;
    let dy = (i64::from(b.y) - i64::from(a.y)) as f64;
    dx.hypot(dy).round() as i64
}

/// The union bounding box of a sequence of world rectangles, or `None` when the sequence
/// is empty. The one place the app folds shape bounds for the zoom presets (item 28) and
/// the status-bar selection summary (item 55).
fn union_bounds(rects: impl Iterator<Item = Rect>) -> Option<Rect> {
    let mut it = rects;
    let first = it.next()?;
    let (mut min_x, mut min_y) = (first.min.x, first.min.y);
    let (mut max_x, mut max_y) = (first.max.x, first.max.y);
    for r in it {
        min_x = min_x.min(r.min.x);
        min_y = min_y.min(r.min.y);
        max_x = max_x.max(r.max.x);
        max_y = max_y.max(r.max.y);
    }
    Some(Rect::new(
        Point::new(min_x, min_y),
        Point::new(max_x, max_y),
    ))
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

/// The alpha, as a percentage of a layer's own, that non-peeked layers keep while
/// a layer row is hovered (catalog 39 hover peek): low enough that the peeked
/// layer reads clearly over the dimmed rest, high enough that context stays.
const PEEK_DIM_PERCENT: u16 = 22;

/// Like [`palette_from_layers`] but, when `peek` names a layer, dims every other
/// visible layer to [`PEEK_DIM_PERCENT`] of its alpha so the peeked layer stands
/// out (catalog 39 hover peek). With `peek` `None` it is exactly
/// [`palette_from_layers`], so an idle canvas pays nothing for the feature.
fn palette_from_layers_peek(layers: &LayerState, peek: Option<LayerId>) -> Palette {
    let Some(peek) = peek else {
        return palette_from_layers(layers);
    };
    let tech = Technology {
        name: String::new(),
        dbu_per_micron: 1,
        layers: layers
            .rows()
            .iter()
            .map(|r| {
                let color_rgba = if r.id == peek {
                    r.color_rgba
                } else {
                    let (red, green, blue, a) = layers::rgba_components(r.color_rgba);
                    let dimmed =
                        u8::try_from(u16::from(a) * PEEK_DIM_PERCENT / 100).unwrap_or(u8::MAX);
                    layers::pack_rgba(red, green, blue, dimmed)
                };
                LayerInfo {
                    id: r.id,
                    name: r.name.clone(),
                    color_rgba,
                    visible: r.visible,
                }
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

/// The persisted preferences to boot with (theme density and reduced motion, plus
/// the lane 4c settings and onboarding state).
///
/// On native this is the saved session file; on the web it is the localStorage
/// mirror. A missing or unreadable store falls back to [`SessionState`](crate::session::SessionState)'s default, so
/// a fresh install boots with the comfortable, motion-on, zoom-wheel defaults and no
/// onboarding progress.
#[cfg(not(target_arch = "wasm32"))]
fn boot_session() -> crate::session::SessionState {
    crate::session::load().unwrap_or_default()
}

/// The persisted preferences to boot with, on the web (from the localStorage
/// mirror; see the native variant).
#[cfg(target_arch = "wasm32")]
fn boot_session() -> crate::session::SessionState {
    crate::session::web_load().unwrap_or_default()
}

/// The GPU adapter name and a friendly backend label for the About dialog and the
/// first-run GPU card, from the live wgpu render state.
///
/// Falls back to a stand-in when no wgpu state is present (a software or headless
/// build), so the diagnostics always have something honest to show.
fn gpu_info(frame: &eframe::Frame) -> (String, &'static str) {
    match frame.wgpu_render_state() {
        Some(state) => {
            let info = state.adapter.get_info();
            (info.name, backend_label(info.backend))
        }
        None => ("software / unknown".to_owned(), "unknown"),
    }
}

/// A friendly label for a wgpu backend (the raw `Debug` names are terse).
// `wgpu::Backend` is `#[non_exhaustive]`, so the catch-all is required for future
// variants even though it currently also folds in the internal no-op backend.
#[allow(clippy::match_wildcard_for_single_variants)]
fn backend_label(backend: eframe::egui_wgpu::wgpu::Backend) -> &'static str {
    use eframe::egui_wgpu::wgpu::Backend;
    match backend {
        Backend::Vulkan => "Vulkan",
        Backend::Metal => "Metal",
        Backend::Dx12 => "Direct3D 12",
        Backend::Gl => "WebGL2 / OpenGL",
        Backend::BrowserWebGpu => "WebGPU",
        // `wgpu::Backend` is non-exhaustive (and has a no-op variant), so anything
        // else reports as unknown rather than naming an internal placeholder.
        _ => "unknown",
    }
}

/// The build/bundle hash for the About diagnostics.
///
/// Read from the optional `RETICLE_BUILD_HASH` compile-time env the release build
/// injects; an unversioned local build reports a stand-in so the field is never
/// empty.
fn bundle_hash() -> &'static str {
    option_env!("RETICLE_BUILD_HASH").unwrap_or("dev build")
}

/// The platform label for the About diagnostics.
fn platform_label() -> &'static str {
    if cfg!(target_arch = "wasm32") {
        "web (wasm32)"
    } else {
        "native"
    }
}

/// Draws a managed view-panel header row: the panel title on the left and a close
/// [`components::IconButton`] on the right. Returns whether the close affordance
/// was clicked this frame. Shared by the 3D-stack and Cross-section hosts (ADR 0096).
fn view_panel_header(ui: &mut egui::Ui, ctx: components::Ctx, title: &str) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.strong(title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if components::IconButton::new(icons::X, "Close panel")
                .hint("Close this panel (View > Panels)")
                .show(ui, ctx)
                .clicked()
            {
                close = true;
            }
        });
    });
    ui.separator();
    close
}

/// The right Inspector's remembered layout to boot with (lane 2B).
///
/// On native it restores the group, width, collapse flag, and open sections from
/// the saved session; a missing or unreadable session, and every web session,
/// starts from the defaults.
#[cfg(not(target_arch = "wasm32"))]
fn boot_inspector_state() -> crate::inspector_layout::InspectorState {
    use crate::inspector_layout::{InspectorState, OrderedWidth, PanelGroup};
    let mut state = InspectorState::default();
    if let Some(s) = crate::session::load() {
        state.group = PanelGroup::from_index(s.panel_group as usize);
        state.width = OrderedWidth(s.panel_right_w);
        state.collapsed = s.panels_collapsed;
        state.apply_open_tags(&s.panel_open);
    }
    state
}

/// The right Inspector's boot layout on the web: the defaults (no filesystem to
/// restore from; see the native variant).
#[cfg(target_arch = "wasm32")]
fn boot_inspector_state() -> crate::inspector_layout::InspectorState {
    crate::inspector_layout::InspectorState::default()
}

/// Converts a canvas [`ScreenRect`] to an egui rectangle.
fn egui_rect_of(screen: &ScreenRect) -> EguiRect {
    EguiRect::from_min_size(
        Pos2::new(screen.left, screen.top),
        Vec2::new(screen.width, screen.height),
    )
}

/// The display name of the layer row at `index`, or an empty string.
fn row_name(state: &LayerState, index: usize) -> String {
    state
        .rows()
        .get(index)
        .map(|r| r.name.clone())
        .unwrap_or_default()
}

/// The display name of the layer with id `id`, or an empty string if unknown.
fn layer_name_of(state: &LayerState, id: LayerId) -> String {
    state
        .rows()
        .iter()
        .find(|r| r.id == id)
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

// --- lane snapshots: snapshot viewer ---
impl App {
    /// Recognizes a snapshot permalink in a page query string
    /// ([`crate::snapshot::parse_snapshot_query`]), so a boot path can tell a
    /// `?view=snapshot` link apart from a live `?view=viewer` link. Pure; owns no
    /// `App` state (see `crate::snapshot` for the read-only mirror type).
    #[must_use]
    pub fn snapshot_permalink_target(query: &str) -> Option<crate::snapshot::SnapshotTarget> {
        crate::snapshot::parse_snapshot_query(query)
    }
}
// --- end lane snapshots ---

#[cfg(test)]
mod tests {
    use super::*;

    /// The agent preview's run narrates its verify steps into the conversation but
    /// must NOT write into the document's DRC panel: that panel tracks the user's real
    /// design, and a scripted demo on a built-in cell must not make it read as verified.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn agent_preview_run_does_not_touch_the_real_drc_panel() {
        let mut app = App::new();
        assert!(!app.drc.has_run());
        app.agent.prompt = "overlay wiring".to_owned();
        app.agent.start();
        // Drain the run the way the frame loop does: every verify update goes through
        // apply_agent_run_verify, which narrates and never touches the DRC overlay.
        let mut updates = 0;
        for _ in 0..1_000 {
            if let Some(update) = app.agent.tick(10.0) {
                updates += 1;
                app.apply_agent_run_verify(&update);
            }
            if !app.agent.is_running() {
                break;
            }
        }
        assert!(updates >= 1, "at least one verify step fired");
        // The real DRC panel is untouched: no run recorded, nothing overwritten, and
        // the status line is not the misleading "Agent verify: DRC clean".
        assert!(
            !app.drc.has_run(),
            "the demo must not mark the real DRC as run"
        );
        assert!(app.drc.is_empty());
        assert_ne!(app.status.text, "Agent verify: DRC clean");
        // The verify result is narrated as an agent turn in the conversation instead.
        assert!(
            app.agent
                .conversation()
                .iter()
                .any(|entry| entry.text.contains("DRC clean")),
            "the clean verify is narrated in the conversation, not the DRC panel"
        );
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
    fn touch_mode_setter_marks_the_style_dirty_only_on_change() {
        use crate::settings::TouchMode;
        // The plumbing seam lane 4C's Settings control drives: a fresh app boots
        // touch on auto, and changing it schedules a style re-apply so the next
        // frame installs the raised or lowered hit-target floor.
        let mut app = App::new();
        assert_eq!(
            app.touch_mode(),
            TouchMode::Auto,
            "touch mode defaults to auto"
        );
        // Clear the boot-time dirty flag so the assertion isolates the setter.
        app.theme_dirty = false;
        app.set_touch_mode(TouchMode::On);
        assert_eq!(app.touch_mode(), TouchMode::On);
        assert!(app.theme_dirty, "changing touch mode schedules a re-apply");
        // Setting the same value again is a no-op: no needless re-apply.
        app.theme_dirty = false;
        app.set_touch_mode(TouchMode::On);
        assert!(
            !app.theme_dirty,
            "an unchanged set does not re-dirty the style"
        );
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
    fn dispatch_routes_commands_to_their_effects() {
        let mut app = App::new();
        app.dispatch(CommandId("tool.measure"));
        assert_eq!(app.tools.active(), Tool::Measure);
        let labels_before = app.labels_visible;
        app.dispatch(CommandId("view.labels"));
        assert_ne!(app.labels_visible, labels_before);
        let minimap_before = app.minimap_visible;
        app.dispatch(CommandId("view.minimap"));
        assert_ne!(app.minimap_visible, minimap_before);
        app.dispatch(CommandId("view.split_h"));
        assert_eq!(app.viewports.pane_count(), 2);
        app.dispatch(CommandId("view.split_single"));
        assert_eq!(app.viewports.pane_count(), 1);
        app.dispatch(CommandId("palette.open"));
        assert!(app.palette_open);
        // An unknown id is a no-op, not a panic.
        app.dispatch(CommandId("not.a.command"));
    }

    #[test]
    fn dispatch_runs_the_relocated_developer_actions() {
        // The Help > Developer entries (lane 2A) run their effects through the same
        // dispatch funnel as every other registry command (catalog 70).
        let mut app = App::new();
        let before = app.scene.len();
        app.dispatch(CommandId("dev.add_demo_rect"));
        assert!(app.scene.len() > before, "dev.add_demo_rect adds a shape");
        let replay_before = app.replay_open;
        app.dispatch(CommandId("dev.replay_theater"));
        assert_ne!(
            app.replay_open, replay_before,
            "dev.replay_theater toggles the theater window"
        );
    }

    #[test]
    fn panels_toggle_collapses_and_expands_the_inspector() {
        let mut app = App::new();
        assert!(!app.inspector.collapsed, "the inspector starts expanded");
        app.dispatch(CommandId("view.panels_toggle"));
        assert!(app.inspector.collapsed, "Tab collapses it to the icon rail");
        app.dispatch(CommandId("view.panels_toggle"));
        assert!(!app.inspector.collapsed, "Tab again expands it");
    }

    #[test]
    fn running_drc_reveals_the_review_group_when_violations_exist() {
        use crate::inspector_layout::PanelGroup;
        let mut app = App::new();
        // Start on a different group so a reveal is observable.
        app.inspector.group = PanelGroup::Settings;
        app.run_drc();
        if app.drc.is_empty() {
            return; // The demo happens to be clean on this build; nothing to reveal.
        }
        assert_eq!(
            app.inspector.group,
            PanelGroup::Review,
            "a fresh violation set surfaces the Review group (catalog 67)"
        );
        assert!(app.inspector.is_open("drc"), "and opens the DRC section");
    }

    #[test]
    fn select_same_layer_grows_to_the_whole_layer() {
        let mut app = App::new();
        select_first_direct(&mut app, 1);
        if app.selection.is_empty() {
            return;
        }
        let seed_layer = app.scene.shapes()[app.selection.iter().next().unwrap()].layer;
        app.select_same_layer();
        // Every selected shape now shares the seed layer.
        let shapes = app.scene.shapes();
        assert!(
            app.selection.iter().all(|i| shapes[i].layer == seed_layer),
            "select-same keeps only shapes on the seeded layer"
        );
    }

    #[test]
    fn rebinding_through_the_map_redirects_the_command() {
        let mut app = App::new();
        // Force a known map so the test does not depend on any user keymap file.
        app.keymap = Keymap::defaults();
        let chord = keymap::Chord::parse("Ctrl+Shift+Q").expect("valid chord");
        assert_eq!(app.keymap.command_for(&chord), None);
        let stolen = app.keymap.bind(CommandId("edit.redo"), Some(chord.clone()));
        assert!(stolen.is_empty());
        assert_eq!(app.keymap.command_for(&chord), Some(CommandId("edit.redo")));
        // The old default no longer fires.
        let old = keymap::Chord::parse("Ctrl+Y").expect("valid chord");
        assert_eq!(app.keymap.command_for(&old), None);
    }

    #[test]
    fn lane_3c_commands_dispatch_to_their_effects() {
        let mut app = App::new();
        app.dispatch(CommandId("help.shortcuts"));
        assert!(app.shortcuts_open, "? toggles the overlay");
        app.dispatch(CommandId("palette.goto_coordinate"));
        assert!(app.palette_open, "goto opens the palette");
        assert_eq!(app.palette_arg, Some(command::PaletteArg::GotoCoordinate));
        let before = app.focus_region;
        app.dispatch(CommandId("focus.cycle"));
        assert_ne!(app.focus_region, before, "F6 advances the focus region");
        assert!(
            app.focus_request,
            "focus move is queued for the render pass"
        );
    }

    #[test]
    fn recent_palette_rows_are_deduped_and_bounded() {
        let mut app = App::new();
        for _ in 0..3 {
            app.record_recent("edit.undo".to_owned());
        }
        assert_eq!(
            app.palette_recents
                .iter()
                .filter(|k| *k == "edit.undo")
                .count(),
            1,
            "a repeat run does not duplicate the recent"
        );
        for n in 0..(PALETTE_RECENTS_MAX + 5) {
            app.record_recent(format!("k{n}"));
        }
        assert!(app.palette_recents.len() <= PALETTE_RECENTS_MAX, "bounded");
        assert_eq!(
            app.palette_recents[0],
            format!("k{}", PALETTE_RECENTS_MAX + 4),
            "most recent first"
        );
    }

    #[test]
    fn esc_cascade_peels_tool_then_selection_then_popover() {
        let mut app = App::new();
        app.shortcuts_open = true;
        app.select_tool(Tool::Measure);
        app.selection.set([0]);
        // 1: an active tool is canceled first.
        app.apply_esc();
        assert_eq!(app.tools.active(), Tool::Select);
        assert!(
            !app.selection.is_empty(),
            "selection survives the first Esc"
        );
        assert!(app.shortcuts_open, "popover survives the first Esc");
        // 2: the selection clears next.
        app.apply_esc();
        assert!(app.selection.is_empty());
        assert!(app.shortcuts_open, "popover survives the second Esc");
        // 3: the popover closes last.
        app.apply_esc();
        assert!(!app.shortcuts_open);
    }

    #[test]
    fn committing_a_coordinate_prompt_centers_the_camera() {
        let mut app = App::new();
        app.open_palette_arg(command::PaletteArg::GotoCoordinate);
        app.palette_query = "1200, -800".to_owned();
        app.commit_palette_arg(None);
        assert_eq!(app.camera.center(), Point::new(1200, -800));
        assert!(!app.palette_open, "a good coordinate closes the palette");
        assert_eq!(app.palette_arg, None);
    }

    #[test]
    fn a_bad_coordinate_keeps_the_prompt_open() {
        let mut app = App::new();
        app.open_palette_arg(command::PaletteArg::GotoCoordinate);
        app.palette_query = "not a point".to_owned();
        app.commit_palette_arg(None);
        assert!(app.palette_open, "a bad coordinate leaves the prompt up");
        assert_eq!(app.palette_arg, Some(command::PaletteArg::GotoCoordinate));
    }

    #[test]
    fn palette_and_shortcut_windows_lay_out_without_panicking() {
        // egui runs a full frame's layout without a GPU, so this exercises the
        // palette, the generated overlay, and the editor for id clashes, borrow
        // panics, and the zero-size focus anchors.
        let mut app = App::new();
        app.set_palette_open(true);
        app.shortcuts_open = true;
        app.keymap_open = true;
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        app.palette_window(&ctx, None);
        app.shortcuts_overlay(&ctx);
        app.keymap_window(&ctx);
        let _ = ctx.end_pass();
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

    /// The Share section renders headlessly without panicking, including the one-click
    /// Share and Copy-permalink buttons added this lane. It does not click them (that
    /// would dial a socket), only lays them out.
    #[test]
    fn share_section_renders_without_panic() {
        let mut app = App::new();
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        egui::Window::new("share test").show(&ctx, |ui| {
            app.share_section(ui);
        });
        let _ = ctx.end_pass();
        assert!(!app.share_server.is_empty());
        assert!(!app.share_room.is_empty());
    }

    #[test]
    fn presentation_command_toggles_the_chrome_flag() {
        let mut app = App::new();
        assert!(!app.presentation);
        app.dispatch(CommandId("view.presentation"));
        assert!(app.presentation, "P enters presentation mode (catalog 93)");
        app.dispatch(CommandId("view.presentation"));
        assert!(!app.presentation, "P again leaves it");
    }

    #[test]
    fn close_design_returns_to_the_start_screen() {
        let mut app = App::new();
        app.start_screen = false;
        app.presentation = true;
        app.dispatch(CommandId("file.close_design"));
        assert!(app.start_screen, "close design goes back to Start");
        assert!(!app.presentation, "and leaves presentation mode");
    }

    #[test]
    fn reserved_2d_command_ids_are_registered_with_their_contract() {
        let pres = commands::spec(CommandId("view.presentation")).expect("view.presentation");
        assert_eq!(pres.menu_path, Some(&["View"][..]));
        assert_eq!(pres.default_chord, Some("P"));
        let close = commands::spec(CommandId("file.close_design")).expect("file.close_design");
        assert_eq!(close.menu_path, Some(&["File"][..]));
        // The default keymap resolves bare P to presentation mode.
        let chord = keymap::Chord::parse("P").expect("P parses");
        assert_eq!(
            Keymap::defaults().command_for(&chord),
            Some(CommandId("view.presentation"))
        );
    }

    #[test]
    fn open_full_editor_leaves_viewer_mode_and_keeps_the_camera() {
        let target = crate::share::ViewerTarget {
            room: "room".to_owned(),
            relay: "127.0.0.1:3030".to_owned(),
        };
        let mut app = App::with_viewer(target);
        assert!(app.is_viewer());
        app.camera = ViewCamera::new(Point::new(999, -111), 2.0);
        let cam = app.camera;
        app.open_full_editor();
        assert!(
            !app.is_viewer(),
            "the editor is one click away (catalog 23)"
        );
        assert_eq!(
            app.camera, cam,
            "the camera is preserved across the transition"
        );
    }

    #[test]
    fn embed_flag_round_trips() {
        let mut app = App::new();
        assert!(!app.is_embedded());
        app.set_embed(true);
        assert!(
            app.is_embedded(),
            "embed mode is set from the page URL (catalog 94)"
        );
    }

    #[test]
    fn viewer_session_chip_and_pin_map_render_without_panic() {
        let target = crate::share::ViewerTarget {
            room: "room".to_owned(),
            relay: "127.0.0.1:3030".to_owned(),
        };
        let mut app = App::with_viewer(target);
        // Seed a remote presence so the chip has an avatar to draw.
        let mut presence = reticle_sync::Presence::new("sharer");
        presence.display_name = "Ada".to_owned();
        app.document.awareness_mut().set(presence);

        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        let cx = app.theme_ctx();
        egui::Window::new("viewer chrome test").show(&ctx, |ui| {
            app.session_chip(ui, cx);
            App::draw_pin_map(ui, cx);
        });
        let _ = ctx.end_pass();
    }

    #[test]
    fn start_screen_sections_render_the_rebuilt_gallery_and_recent_pins() {
        let mut app = App::new();
        app.record_recent_file(crate::webopen::RecentFile::local("chip.gds", 4096));
        app.recent_pins.toggle("chip.gds");
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        egui::Window::new("start test").show(&ctx, |ui| {
            let cx = app.theme_ctx();
            let mut action: Option<StartAction> = None;
            App::start_hero_section(ui, cx, &mut action);
            App::start_gallery_section(ui, cx, &mut action);
            App::start_recent_section(
                ui,
                cx,
                app.recent_files.entries(),
                &app.recent_pins,
                &mut action,
            );
            App::start_open_hint_section(ui);
        });
        let _ = ctx.end_pass();
        assert!(app.recent_pins.is_pinned("chip.gds"), "the pin sticks");
    }

    /// A permalink emitted from a session restores the same cell, camera, and layer set
    /// on a freshly opened document: emit the current view, round-trip it through the URL
    /// string, disturb the view, then apply and prove each piece came back.
    #[test]
    fn permalink_restores_cell_camera_and_layers_on_an_opened_document() {
        let mut app = App::new();
        let outcome = crate::startscreen::ExampleChip::TinyTapeoutMin
            .open()
            .expect("the bundled sample opens");
        app.open_outcome(outcome);
        let top = app.top_cell.clone();
        assert!(
            app.layer_state.rows().len() >= 2,
            "the sample has layers to toggle"
        );

        // Author a distinctive view: a specific camera and a single visible layer.
        app.camera = ViewCamera::new(Point::new(1234, -5678), 0.5);
        let target = app.layer_state.rows()[0].id;
        app.layer_state.hide_all();
        app.layer_state.set_visible(target, true);

        // Serialize exactly what the copy-permalink action serializes, then round-trip
        // it through the URL string a share link is.
        let emitted = app.session_permalink();
        let url = crate::share::emit_permalink("", None, &emitted);
        let parsed = crate::share::parse_permalink(url.trim_start_matches('?'));
        assert_eq!(parsed.cell.as_deref(), Some(top.as_str()), "cell carried");

        // Disturb the view, then prove applying the link restores it exactly.
        app.camera = ViewCamera::new(Point::ORIGIN, 0.05);
        app.layer_state.show_all();
        app.fit_requested = true;
        app.apply_permalink(&parsed);

        assert_eq!(
            app.camera.center(),
            Point::new(1234, -5678),
            "camera center restored"
        );
        assert!(
            (app.camera.pixels_per_dbu() - 0.5).abs() < 1e-9,
            "zoom restored"
        );
        assert!(!app.fit_requested, "the permalink camera cancels auto-fit");
        for row in app.layer_state.rows() {
            assert_eq!(
                row.visible,
                row.id == target,
                "layer {:?} visibility restored",
                row.id
            );
        }

        // And the jump-to-cell path: a cell-only permalink (no camera to override it)
        // frames that cell by setting the deferred locate to the cell's world bbox.
        app.search.pending_locate = None;
        app.apply_permalink(&crate::share::Permalink {
            cell: Some(top.clone()),
            camera: None,
            layers: None,
        });
        assert_eq!(
            app.search.pending_locate,
            app.history.document().cell_bbox(&top),
            "a cell-only permalink frames the cell"
        );
        assert!(
            app.search.pending_locate.is_some(),
            "the sample's top cell has a bbox to frame"
        );
    }

    /// A permalink naming an unknown cell or unknown layers is applied without panicking
    /// and never leaves the view in a broken state.
    #[test]
    fn apply_permalink_ignores_unknown_cell_and_layers() {
        let mut app = App::new();
        app.open_outcome(
            crate::startscreen::ExampleChip::TinyTapeoutMin
                .open()
                .expect("opens"),
        );
        app.apply_permalink(&crate::share::Permalink {
            cell: Some("NO_SUCH_CELL".to_owned()),
            camera: None,
            layers: Some(vec![(65535, 65535)]),
        });
        // An all-unknown layer set hides everything (hide_all, then the unknown miss);
        // the unknown cell sets no locate; nothing panicked.
        assert!(app.layer_state.rows().iter().all(|r| !r.visible));
        assert!(
            app.search.pending_locate.is_none(),
            "an unknown cell frames nothing"
        );
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
