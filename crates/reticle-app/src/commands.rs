//! The command registry: one static table every surface renders from.
//!
//! A [`CommandSpec`] describes a single user command: its stable [`CommandId`]
//! (`"edit.undo"`), the label and category shown in menus and the palette, where
//! it lives in the menu tree, its default chord, and how to run it. Menus (Wave 2
//! lane 2A), context menus and the palette (3C), and the shortcuts overlay all
//! read [`registry`] rather than hardcoding their own action lists, so a command
//! added here shows up everywhere at once.
//!
//! Running a command is deliberately kept out of this module: [`CommandSpec::run`]
//! is data ([`RunAs`]) that the app in [`crate::app`] interprets in its `dispatch`
//! funnel, routing [`RunAs::Command`] through the existing palette-command path and
//! [`RunAs::App`] through the folded app-op logic. Keeping the table pure keeps it
//! testable and keeps this module free of `egui`/state dependencies.
//!
//! ## Adding entries (Wave 2 lanes)
//!
//! The `REGISTRY` array is split into per-lane sections marked with `// ---
//! <lane> ---` comments. Append your ids into your own section using your id
//! prefixes; do not reorder or edit another lane's rows. Reference any id from
//! anywhere (menus, palette, buttons) by value: `self.dispatch(CommandId("view.grid"))`.
//! The ids and menu paths owned by other lanes are the cross-lane contract in
//! `docs/design/ia-inventory.md` section 4; Gate 2 asserts the merged table
//! against it.
//!
//! Enabled/checked predicates (for greying out or check-marking menu items) are
//! intentionally NOT fields here yet: they need to read `&App` state, which would
//! couple this pure table to app internals. Lane 2A adds them alongside the menu
//! renderer that needs them (see RESULT.md deferral note).

use crate::command::Command;

/// A stable, human-readable command identifier such as `"edit.undo"`.
///
/// The wrapped string is the id used in menus, the palette, saved keymaps, and the
/// cross-lane reserved-id table. It is `Ord` (a plain string compare) so it can key
/// the [`crate::keymap::Keymap`] `BTreeMap`, and `Copy` so it passes by value freely.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct CommandId(pub &'static str);

/// Where a command is available across build targets.
///
/// The palette and menus filter [`Scope::NativeOnly`]/[`Scope::WasmOnly`] entries
/// out of the build that cannot run them (PNG export needs native file IO, for
/// instance), leaving [`Scope::Global`] commands everywhere.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    /// Available in every build.
    Global,
    /// Native builds only (needs a filesystem, GPU blocking, etc.).
    NativeOnly,
    /// Web builds only.
    WasmOnly,
}

/// An app-level effect that has no palette [`Command`] equivalent.
///
/// These are the arms the old `run_action` handled directly (toggling the palette,
/// splitting panes, toggling overlays); the app's `run_app_op` interprets them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppOp {
    /// Toggle the command palette window.
    OpenPalette,
    /// Toggle the canvas text-label overlay.
    ToggleLabels,
    /// Toggle the minimap overview panel.
    ToggleMinimap,
    /// Toggle the top/left ruler bars (lane 3a, `view.rulers`).
    ToggleRulers,
    /// Collapse the canvas to a single pane.
    SplitSingle,
    /// Split the canvas into two side-by-side panes.
    SplitHorizontal,
    /// Split the canvas into two stacked panes.
    SplitVertical,
    // --- lane 3c ---
    /// Open the palette prompting for a coordinate to jump to (`palette.goto_coordinate`).
    PromptGotoCoordinate,
    /// Open the palette prompting for a cell to jump to (`palette.goto_cell`).
    PromptGotoCell,
    /// Advance keyboard focus to the next focus region (`focus.cycle`, F6).
    CycleFocus,
    /// Toggle the generated keyboard-shortcuts overlay (`help.shortcuts`, ?).
    ToggleShortcuts,
    // lane 4c: help and onboarding effects (no palette `Command` equivalent).
    /// Relaunch the guided tour from the first step (both editor chapters, or the
    /// viewer walkthrough in a viewer session).
    TakeTour,
    /// Open the Settings dialog (catalog 98).
    OpenSettings,
    /// Open the About dialog (catalog 99, 100).
    OpenAbout,
    /// Open the "What's new" dialog (catalog 26).
    OpenWhatsNew,
    /// Open the documentation in the browser (catalog, Help > Documentation).
    OpenDocs,
    // --- lane 2A: developer actions (Help > Developer) ---
    /// Insert a demo rectangle through the undo history (the retired debug-register
    /// action, relocated to Help > Developer; catalog 70).
    AddDemoRect,
    /// Toggle the replay-theater window (developer entry point).
    ToggleReplayTheater,
    /// Toggle the managed 3D-stack panel (lane 2c, ADR 0096).
    TogglePanel3d,
    /// Toggle the managed Cross-section panel (lane 2c, ADR 0096).
    TogglePanelXsection,
    // lane 2b: Inspector effects with no palette `Command` equivalent.
    /// Toggle the right Inspector between full and its collapsed icon rail.
    PanelsToggle,
    /// Cut the selection to the clipboard.
    EditCut,
    /// Paste the clipboard, offset by the productivity delta.
    EditPaste,
    /// Move the selection by the productivity delta.
    EditMove,
    /// Array the selection into the productivity grid.
    EditArray,
    /// Place a via stack at the productivity center.
    EditViaStack,
    /// Boolean-union the same-layer selection.
    BoolUnion,
    /// Boolean-intersect the same-layer selection.
    BoolIntersect,
    /// Boolean-subtract the same-layer selection.
    BoolSubtract,
    /// Grow the selection to every shape sharing a selected layer.
    SelectSameLayer,
    /// Select every shape whose layer name matches the Search field.
    SelectByName,
    /// Run DRC over the top cell.
    DrcRun,
    /// Toggle DRC-as-you-type.
    DrcLive,
    /// Clear the DRC violation list and markers.
    DrcClear,
    /// Capture the current layout as the diff baseline.
    DiffSnapshot,
    /// Diff the current layout against the captured baseline.
    DiffRun,
    /// Toggle the diff overlay's visibility.
    DiffOverlay,
    /// Anchor a comment to the current top cell.
    CommentAdd,
    /// Export the current view as SVG.
    ExportSvg,
    /// Export a metrology CSV of the current design.
    ExportMetrology,
    // --- lane 2D: viewer / start / presentation ---
    /// Toggle presentation mode: hide all chrome and show only the canvas
    /// (catalog 93, lane 2D, id `view.presentation`).
    TogglePresentation,
    /// Close the current design and return to the Start screen (lane 2D, id
    /// `file.close_design`).
    CloseDesign,
    // --- lane 3b: open flow and dialogs ---
    /// Open the native/browser file picker (rfd), feeding bytes into the hardened
    /// open path (`file.open_dialog`).
    OpenFileDialog,
    /// Toggle the Open-from-URL dialog (`file.open_url`).
    OpenUrlDialog,
    /// Toggle the Convert-to-archive dialog (`file.convert_gds`).
    ConvertDialog,
    /// Toggle the Share dialog (`share.dialog`).
    ShareDialog,
    /// Copy the read-only viewer link to the clipboard (`share.copy_viewer_link`).
    CopyViewerLink,
    // --- lane pcell-inspect: PCell inspector actions ---
    /// Reveal the Inspector's PCell section (`pcell.edit_params`).
    PcellEditParams,
    /// Refresh the status-bar `param_hash` readout for the selected PCell
    /// (`pcell.regenerate`); this lane never calls the sandboxed producer, which is
    /// the `pcell-produce` lane's Gate 2 wiring.
    PcellRegenerate,
    // --- end lane pcell-inspect ---
    // --- lane trace-ui: net-trace panel (F3 consumer; ADR 0103) ---
    /// Loads the net-at-point readout (fixture stand-in until Gate 2; `trace.at_point`).
    TraceAtPoint,
    /// Loads the net-extent readout (fixture stand-in until Gate 2; `trace.net_extent`).
    TraceNetExtent,
    /// Loads the shorts/opens report (fixture stand-in until Gate 2; `trace.shorts_opens`).
    TraceShortsOpens,
    /// Advances the shorts/opens navigator to the next row (`trace.next`).
    TraceNext,
    /// Moves the shorts/opens navigator to the previous row (`trace.prev`).
    TracePrev,
    // --- end lane trace-ui ---
    // --- lane nl-edit: natural-language edit command bar ---
    /// Parse the natural-language edit bar's current text (see
    /// [`crate::nl_edit`]) and apply it through the undo history as one step
    /// (`nl_edit.submit`).
    NlEditSubmit,
    // --- end lane nl-edit ---
    // --- lane agent-panel: real agent commands (item 3) ---
    /// Plan an agent run for the current prompt (`agent.plan`): stage a real plan on
    /// native with a model, or start the scripted preview otherwise.
    AgentPlan,
    /// Approve the staged plan and execute it against the live model (`agent.approve`).
    AgentApprove,
    /// Approve the staged plan step (`agent.approve_step`); the interactive loop approves
    /// the whole staged run, so this drives the same execute as `agent.approve`.
    AgentApproveStep,
    /// Stop the agent run, the native runner or the scripted preview (`agent.stop`).
    AgentStop,
    /// Open the last agent run in the replay theater (`agent.replay`).
    AgentReplay,
    // --- end lane agent-panel ---
}

/// How a command runs: either through the palette [`Command`] path or as an
/// [`AppOp`] the app handles directly. The app's `dispatch` matches on this.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RunAs {
    /// Route through `App::run_command`, sharing the palette/toolbar effect path.
    Command(Command),
    /// Route through `App::run_app_op` for effects with no palette command.
    App(AppOp),
}

/// One registry entry: everything a menu, the palette, or the shortcuts overlay
/// needs to display and run a command.
#[derive(Clone, Copy, Debug)]
pub struct CommandSpec {
    /// The stable id (`"edit.undo"`).
    pub id: CommandId,
    /// The human-readable label shown in menus, the palette, and the overlay.
    pub label: &'static str,
    /// The grouping used for palette sections and the shortcuts overlay.
    pub category: &'static str,
    /// The menu location, as a path of nested labels (`["View", "Split"]`).
    /// `None` means the command is palette-only (nothing to render in a menu).
    pub menu_path: Option<&'static [&'static str]>,
    /// The default chord, as text parsed by [`crate::keymap::Chord::parse`]; `None`
    /// ships unbound.
    pub default_chord: Option<&'static str>,
    /// Whether the shortcuts editor may rebind this command. Non-rebindable
    /// commands never appear in the keymap file or the editor.
    pub rebindable: bool,
    /// How to run the command.
    pub run: RunAs,
    /// Where the command is available across build targets.
    pub scope: Scope,
}

impl CommandSpec {
    /// Whether this command is reachable from the command palette.
    ///
    /// Every current command is palette-visible (the palette is the universal
    /// launcher); palette-only commands added by later lanes set `menu_path` to
    /// `None` and stay palette-visible. The parity test uses this to prove nothing
    /// is orphaned (reachable from neither a menu nor the palette).
    #[must_use]
    pub fn palette_visible(&self) -> bool {
        true
    }
}

// The registry, split into per-lane sections. Wave 2 lanes append into their own
// section; 1E owns the rows below. Ids, menu paths, and chords match the reserved
// table in docs/design/ia-inventory.md section 4 exactly.
static REGISTRY: &[CommandSpec] = &[
    // --- 1E: edit ---
    CommandSpec {
        id: CommandId("edit.undo"),
        label: "Undo",
        category: "Edit",
        menu_path: Some(&["Edit"]),
        default_chord: Some("Ctrl+Z"),
        rebindable: true,
        run: RunAs::Command(Command::Undo),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("edit.redo"),
        label: "Redo",
        category: "Edit",
        menu_path: Some(&["Edit"]),
        default_chord: Some("Ctrl+Y"),
        rebindable: true,
        run: RunAs::Command(Command::Redo),
        scope: Scope::Global,
    },
    // --- 1E: view ---
    CommandSpec {
        id: CommandId("view.zoom_fit"),
        label: "Fit",
        category: "View",
        menu_path: Some(&["View"]),
        default_chord: Some("F"),
        rebindable: true,
        run: RunAs::Command(Command::ZoomToFit),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("view.grid"),
        label: "Grid",
        category: "View",
        menu_path: Some(&["View"]),
        default_chord: Some("Ctrl+G"),
        rebindable: true,
        run: RunAs::Command(Command::ToggleGrid),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("view.snap"),
        label: "Snap",
        category: "View",
        menu_path: Some(&["View"]),
        default_chord: None,
        rebindable: true,
        run: RunAs::Command(Command::ToggleSnap),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("view.labels"),
        label: "Labels",
        category: "View",
        menu_path: Some(&["View"]),
        default_chord: Some("L"),
        rebindable: true,
        run: RunAs::App(AppOp::ToggleLabels),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("view.minimap"),
        label: "Minimap",
        category: "View",
        menu_path: Some(&["View"]),
        default_chord: Some("N"),
        rebindable: true,
        run: RunAs::App(AppOp::ToggleMinimap),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("view.split_single"),
        label: "Single pane",
        category: "View",
        menu_path: Some(&["View", "Split"]),
        default_chord: Some("Ctrl+1"),
        rebindable: true,
        run: RunAs::App(AppOp::SplitSingle),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("view.split_h"),
        label: "Split horizontal",
        category: "View",
        menu_path: Some(&["View", "Split"]),
        default_chord: Some("Ctrl+2"),
        rebindable: true,
        run: RunAs::App(AppOp::SplitHorizontal),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("view.split_v"),
        label: "Split vertical",
        category: "View",
        menu_path: Some(&["View", "Split"]),
        default_chord: Some("Ctrl+3"),
        rebindable: true,
        run: RunAs::App(AppOp::SplitVertical),
        scope: Scope::Global,
    },
    // --- 1E: select ---
    CommandSpec {
        id: CommandId("select.clear"),
        label: "Clear selection",
        category: "Select",
        menu_path: Some(&["Select"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::Command(Command::ClearSelection),
        scope: Scope::Global,
    },
    // --- 1E: draw (tools) ---
    CommandSpec {
        id: CommandId("tool.select"),
        label: "Select tool",
        category: "Draw",
        menu_path: Some(&["Draw"]),
        default_chord: Some("V"),
        rebindable: true,
        run: RunAs::Command(Command::SetTool(crate::tool::Tool::Select)),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("tool.pan"),
        label: "Pan tool",
        category: "Draw",
        menu_path: Some(&["Draw"]),
        default_chord: Some("S"),
        rebindable: true,
        run: RunAs::Command(Command::SetTool(crate::tool::Tool::Pan)),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("tool.measure"),
        label: "Measure tool",
        category: "Draw",
        menu_path: Some(&["Draw"]),
        default_chord: Some("M"),
        rebindable: true,
        run: RunAs::Command(Command::SetTool(crate::tool::Tool::Measure)),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("tool.cutline"),
        label: "Cut line tool",
        category: "Draw",
        menu_path: Some(&["Draw"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::Command(Command::SetTool(crate::tool::Tool::CutLine)),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("tool.rect"),
        label: "Rectangle tool",
        category: "Draw",
        menu_path: Some(&["Draw"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::Command(Command::SetTool(crate::tool::Tool::DrawRect)),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("tool.polygon"),
        label: "Polygon tool",
        category: "Draw",
        menu_path: Some(&["Draw"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::Command(Command::SetTool(crate::tool::Tool::DrawPolygon)),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("tool.path"),
        label: "Path tool",
        category: "Draw",
        menu_path: Some(&["Draw"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::Command(Command::SetTool(crate::tool::Tool::DrawPath)),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("tool.vertices"),
        label: "Edit vertices tool",
        category: "Draw",
        menu_path: Some(&["Draw"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::Command(Command::SetTool(crate::tool::Tool::EditVertex)),
        scope: Scope::Global,
    },
    // --- 1E: file ---
    CommandSpec {
        id: CommandId("file.export_png"),
        label: "Export view as PNG",
        category: "File",
        menu_path: Some(&["File", "Export"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::Command(Command::ExportPng),
        scope: Scope::NativeOnly,
    },
    // --- 1E: help ---
    CommandSpec {
        id: CommandId("palette.open"),
        label: "Command palette",
        category: "Help",
        menu_path: Some(&["Help"]),
        default_chord: Some("Ctrl+P"),
        rebindable: true,
        run: RunAs::App(AppOp::OpenPalette),
        scope: Scope::Global,
    },
    // --- 3C: palette / keys ---
    CommandSpec {
        id: CommandId("palette.goto_coordinate"),
        label: "Go to coordinate...",
        category: "Go to",
        menu_path: None,
        default_chord: None,
        rebindable: true,
        run: RunAs::App(AppOp::PromptGotoCoordinate),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("palette.goto_cell"),
        label: "Go to cell...",
        category: "Go to",
        menu_path: None,
        default_chord: None,
        rebindable: true,
        run: RunAs::App(AppOp::PromptGotoCell),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("focus.cycle"),
        label: "Cycle focus region",
        category: "View",
        menu_path: None,
        default_chord: Some("F6"),
        rebindable: true,
        run: RunAs::App(AppOp::CycleFocus),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("help.shortcuts"),
        label: "Keyboard shortcuts",
        category: "Help",
        menu_path: Some(&["Help"]),
        default_chord: Some("?"),
        rebindable: true,
        run: RunAs::App(AppOp::ToggleShortcuts),
        scope: Scope::Global,
    },
    // --- lane 4c: help and onboarding ---
    CommandSpec {
        id: CommandId("help.tour"),
        label: "Take the tour",
        category: "Help",
        menu_path: Some(&["Help"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::TakeTour),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("help.docs"),
        label: "Documentation",
        category: "Help",
        menu_path: Some(&["Help"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::OpenDocs),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("help.whats_new"),
        label: "What's new",
        category: "Help",
        menu_path: Some(&["Help"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::OpenWhatsNew),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("help.settings"),
        label: "Settings...",
        category: "Help",
        menu_path: Some(&["Help"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::OpenSettings),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("help.about"),
        label: "About Reticle",
        category: "Help",
        menu_path: Some(&["Help"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::OpenAbout),
        scope: Scope::Global,
    },
    // --- 2A: developer (Help > Developer) ---
    // The debug-register actions retired from the History and Agent panels into a
    // product-grade Help > Developer submenu (catalog 70). These ids are owned by
    // lane 2A per the reserved table; their effects run through `run_app_op`.
    CommandSpec {
        id: CommandId("dev.add_demo_rect"),
        label: "Insert demo rectangle",
        category: "Developer",
        menu_path: Some(&["Help", "Developer"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::AddDemoRect),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("dev.replay_theater"),
        label: "Replay theater",
        category: "Developer",
        menu_path: Some(&["Help", "Developer"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::ToggleReplayTheater),
        scope: Scope::Global,
    },
    // --- 2c: view panels ---
    CommandSpec {
        id: CommandId("view.panel_3d"),
        label: "3D stack panel",
        category: "View",
        menu_path: Some(&["View", "Panels"]),
        default_chord: None,
        rebindable: true,
        run: RunAs::App(AppOp::TogglePanel3d),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("view.panel_xsection"),
        label: "Cross-section panel",
        category: "View",
        menu_path: Some(&["View", "Panels"]),
        default_chord: None,
        rebindable: true,
        run: RunAs::App(AppOp::TogglePanelXsection),
        scope: Scope::Global,
    },
    // --- 2B: inspector (file export consolidation, edit ops, select, verify,
    // comments, panel toggle). Effects live in `App::run_app_op`; ids and menu
    // paths match ia-inventory.md section 4. ---
    CommandSpec {
        id: CommandId("file.export_svg"),
        label: "Export view as SVG",
        category: "File",
        menu_path: Some(&["File", "Export"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::ExportSvg),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("file.export_metrology"),
        label: "Export metrology CSV",
        category: "File",
        menu_path: Some(&["File", "Export"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::ExportMetrology),
        scope: Scope::NativeOnly,
    },
    CommandSpec {
        id: CommandId("edit.cut"),
        label: "Cut",
        category: "Edit",
        menu_path: Some(&["Edit"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::EditCut),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("edit.paste"),
        label: "Paste",
        category: "Edit",
        menu_path: Some(&["Edit"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::EditPaste),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("edit.move_exact"),
        label: "Move...",
        category: "Edit",
        menu_path: Some(&["Edit"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::EditMove),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("edit.array"),
        label: "Array...",
        category: "Edit",
        menu_path: Some(&["Edit"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::EditArray),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("edit.via_stack"),
        label: "Via stack...",
        category: "Edit",
        menu_path: Some(&["Edit"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::EditViaStack),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("edit.bool_union"),
        label: "Boolean union",
        category: "Edit",
        menu_path: Some(&["Edit", "Boolean"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::BoolUnion),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("edit.bool_intersect"),
        label: "Boolean intersect",
        category: "Edit",
        menu_path: Some(&["Edit", "Boolean"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::BoolIntersect),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("edit.bool_subtract"),
        label: "Boolean subtract",
        category: "Edit",
        menu_path: Some(&["Edit", "Boolean"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::BoolSubtract),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("select.same_layer"),
        label: "Same layer as selection",
        category: "Select",
        menu_path: Some(&["Select"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::SelectSameLayer),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("select.by_name"),
        label: "Select by name...",
        category: "Select",
        menu_path: Some(&["Select"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::SelectByName),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("view.panels_toggle"),
        label: "Toggle panels",
        category: "View",
        menu_path: Some(&["View", "Panels"]),
        default_chord: Some("Tab"),
        rebindable: true,
        run: RunAs::App(AppOp::PanelsToggle),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("verify.drc_run"),
        label: "Run DRC",
        category: "Verify",
        menu_path: Some(&["Verify"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::DrcRun),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("verify.drc_live"),
        label: "Check as you type",
        category: "Verify",
        menu_path: Some(&["Verify"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::DrcLive),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("verify.drc_clear"),
        label: "Clear DRC results",
        category: "Verify",
        menu_path: Some(&["Verify"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::DrcClear),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("verify.diff_snapshot"),
        label: "Snapshot for diff",
        category: "Verify",
        menu_path: Some(&["Verify"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::DiffSnapshot),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("verify.diff_run"),
        label: "Diff vs snapshot",
        category: "Verify",
        menu_path: Some(&["Verify"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::DiffRun),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("verify.diff_overlay"),
        label: "Show diff overlay",
        category: "Verify",
        menu_path: Some(&["Verify"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::DiffOverlay),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("comment.add"),
        label: "Add comment",
        category: "Share",
        menu_path: Some(&["Share", "Comments"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::CommentAdd),
        scope: Scope::Global,
    },
    // --- lane 2d: viewer / start / presentation ---
    CommandSpec {
        id: CommandId("view.presentation"),
        label: "Presentation mode",
        category: "View",
        menu_path: Some(&["View"]),
        default_chord: Some("P"),
        rebindable: true,
        run: RunAs::App(AppOp::TogglePresentation),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("file.close_design"),
        label: "Close design",
        category: "File",
        menu_path: Some(&["File"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::CloseDesign),
        scope: Scope::Global,
    },
    // --- 3A: canvas navigation, view presets, overlays ---
    CommandSpec {
        id: CommandId("view.zoom_selection"),
        label: "Fit selection",
        category: "View",
        menu_path: Some(&["View"]),
        default_chord: Some("Shift+F"),
        rebindable: true,
        run: RunAs::Command(Command::ZoomSelection),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("view.zoom_one_to_one"),
        label: "Zoom 1:1 DBU",
        category: "View",
        menu_path: Some(&["View"]),
        default_chord: None,
        rebindable: true,
        run: RunAs::Command(Command::ZoomOneToOne),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("view.zoom_layer_extents"),
        label: "Zoom to layer extents",
        category: "View",
        menu_path: Some(&["View"]),
        default_chord: None,
        rebindable: true,
        run: RunAs::Command(Command::ZoomLayerExtents),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("view.bookmark_save"),
        label: "Save view bookmark",
        category: "View",
        menu_path: Some(&["View", "Bookmarks"]),
        default_chord: None,
        rebindable: true,
        run: RunAs::Command(Command::BookmarkSave),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("view.rulers"),
        label: "Rulers",
        category: "View",
        menu_path: Some(&["View"]),
        default_chord: None,
        rebindable: true,
        run: RunAs::App(AppOp::ToggleRulers),
        scope: Scope::Global,
    },
    // --- 3A: edit ---
    CommandSpec {
        id: CommandId("edit.duplicate"),
        label: "Duplicate",
        category: "Edit",
        menu_path: Some(&["Edit"]),
        default_chord: Some("Ctrl+D"),
        rebindable: true,
        run: RunAs::Command(Command::Duplicate),
        scope: Scope::Global,
    },
    // --- 3A: share ---
    CommandSpec {
        id: CommandId("share.copy_permalink"),
        label: "Copy permalink at this view",
        category: "Share",
        menu_path: Some(&["Share"]),
        default_chord: None,
        rebindable: true,
        run: RunAs::Command(Command::CopyPermalink),
        scope: Scope::Global,
    },
    // --- lane 3b: open flow, dialogs, share ---
    // Ids, labels, menu paths, and chords match the reserved table in
    // docs/design/ia-inventory.md section 4 exactly (Gate 2 asserts this).
    CommandSpec {
        id: CommandId("file.open_dialog"),
        label: "Open...",
        category: "File",
        menu_path: Some(&["File"]),
        default_chord: Some("Ctrl+O"),
        rebindable: true,
        run: RunAs::App(AppOp::OpenFileDialog),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("file.open_url"),
        label: "Open from URL...",
        category: "File",
        menu_path: Some(&["File"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::OpenUrlDialog),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("file.convert_gds"),
        label: "Convert GDS to archive...",
        category: "File",
        menu_path: Some(&["File"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::ConvertDialog),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("share.dialog"),
        label: "Share this session...",
        category: "Share",
        menu_path: Some(&["Share"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::ShareDialog),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("share.copy_viewer_link"),
        label: "Copy viewer link",
        category: "Share",
        menu_path: Some(&["Share"]),
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::CopyViewerLink),
        scope: Scope::Global,
    },
    // --- lane pcell-inspect: PCell inspector actions (moved from the reserved
    // campaign-ids table above; ids and labels unchanged from the F6 contract,
    // ADR 0106). ---
    CommandSpec {
        id: CommandId("pcell.edit_params"),
        label: "Edit PCell parameters",
        category: "PCell",
        menu_path: None,
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::PcellEditParams),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("pcell.regenerate"),
        label: "Regenerate PCell",
        category: "PCell",
        menu_path: None,
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::PcellRegenerate),
        scope: Scope::Global,
    },
    // --- end lane pcell-inspect ---
    // --- lane trace-ui: net-trace panel (F3 consumer; ADR 0103) ---
    // Moved from RESERVED_CAMPAIGN_IDS (ids and labels unchanged, ADR 0106); no
    // default chord assigned, matching the reserved table's `chord: None`.
    CommandSpec {
        id: CommandId("trace.at_point"),
        label: "Trace net at point",
        category: "Trace",
        menu_path: None,
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::TraceAtPoint),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("trace.net_extent"),
        label: "Show net extent",
        category: "Trace",
        menu_path: None,
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::TraceNetExtent),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("trace.shorts_opens"),
        label: "Shorts and opens list",
        category: "Trace",
        menu_path: None,
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::TraceShortsOpens),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("trace.next"),
        label: "Next trace result",
        category: "Trace",
        menu_path: None,
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::TraceNext),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("trace.prev"),
        label: "Previous trace result",
        category: "Trace",
        menu_path: None,
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::TracePrev),
        scope: Scope::Global,
    },
    // --- end lane trace-ui ---
    // --- lane nl-edit: natural-language edit command bar ---
    // Palette-only (no menu row, no default chord): the bar's Enter/Run action
    // calls `App::nl_edit_submit` directly, and this registry row makes the same
    // action reachable and discoverable from the command palette too.
    CommandSpec {
        id: CommandId("nl_edit.submit"),
        label: "Run natural-language edit",
        category: "Edit",
        menu_path: None,
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::NlEditSubmit),
        scope: Scope::Global,
    },
    // --- end lane nl-edit ---
    // --- lane agent-panel: real agent commands, moved live from the reserved table
    // (item 3, ADR 0106). Palette-only (no menu path) and no chord, matching their
    // reserved contract; the effects run through `run_app_op`. ---
    CommandSpec {
        id: CommandId("agent.plan"),
        label: "Plan agent run",
        category: "Agent",
        menu_path: None,
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::AgentPlan),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("agent.approve"),
        label: "Approve plan",
        category: "Agent",
        menu_path: None,
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::AgentApprove),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("agent.approve_step"),
        label: "Approve step",
        category: "Agent",
        menu_path: None,
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::AgentApproveStep),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("agent.stop"),
        label: "Stop agent",
        category: "Agent",
        menu_path: None,
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::AgentStop),
        scope: Scope::Global,
    },
    CommandSpec {
        id: CommandId("agent.replay"),
        label: "Replay agent run",
        category: "Agent",
        menu_path: None,
        default_chord: None,
        rebindable: false,
        run: RunAs::App(AppOp::AgentReplay),
        scope: Scope::Global,
    },
    // --- end lane agent-panel ---
];

/// The full command registry.
///
/// Every menu, the palette, the shortcuts overlay, and the keymap read this one
/// table. Entries are ordered by the per-lane sections in the `REGISTRY` array.
#[must_use]
pub fn registry() -> &'static [CommandSpec] {
    REGISTRY
}

/// A surface a right-click can raise a context menu on (lane 3C, item 47).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuContext {
    /// Empty canvas: navigation and tool actions.
    Canvas,
    /// A selected shape on the canvas.
    Shape,
    /// A row in the Layers panel (dynamic per-layer actions are added by the caller).
    LayerRow,
    /// A panel header (Inspector / Layers title bars).
    PanelHeader,
}

impl MenuContext {
    /// The registry command ids a right-click on this surface offers, so context
    /// menus are driven by the registry rather than a hand-kept parallel list
    /// (item 47). The renderer skips any id not present in the current build and,
    /// for [`MenuContext::LayerRow`], prepends the dynamic toggle/select actions.
    #[must_use]
    pub fn command_ids(self) -> &'static [&'static str] {
        match self {
            MenuContext::Canvas => &[
                "tool.select",
                "view.zoom_fit",
                "select.clear",
                "palette.goto_coordinate",
                "palette.open",
            ],
            MenuContext::Shape => &[
                "view.zoom_fit",
                "select.clear",
                "tool.vertices",
                "palette.open",
            ],
            MenuContext::LayerRow => &["view.labels", "view.minimap"],
            MenuContext::PanelHeader => &["palette.open", "help.shortcuts", "focus.cycle"],
        }
    }
}

/// The spec for `id`, or `None` if no command has that id.
#[must_use]
pub fn spec(id: CommandId) -> Option<&'static CommandSpec> {
    REGISTRY.iter().find(|s| s.id == id)
}

/// The label for `id`, falling back to the raw id string for an unknown id so
/// status messages always have something to show.
#[must_use]
pub fn label(id: CommandId) -> &'static str {
    spec(id).map_or(id.0, |s| s.label)
}

/// The registry `CommandId` for a rebindable command whose id string is `s`.
///
/// Returns `None` for an unknown id or a command that is not rebindable, so the
/// keymap never binds a chord to something the editor cannot manage. The returned
/// id borrows the registry's `'static` string, not the caller's slice, so it can
/// be stored in the keymap.
#[must_use]
pub fn rebindable_id(s: &str) -> Option<CommandId> {
    REGISTRY
        .iter()
        .find(|spec| spec.rebindable && spec.id.0 == s)
        .map(|spec| spec.id)
}

// --- campaign f6: reserved command ids ---------------------------------------
//
// Every user-reachable command a v8.2 campaign phase will add is reserved here
// now (the F6 contract), so no phase invents an id and every planned action has
// one agreed name, menu location, and (later) chord. These are NOT registry
// commands: they carry no `run` target and never render, so the menu-parity and
// keymap tests are untouched. When a lane ships one it MOVES the id into
// `REGISTRY` with a real `run` and deletes the reserved row; the
// `reserved_campaign_ids_*` test enforces that reserved and live ids stay
// disjoint. See ADR 0106.

/// A reserved-but-not-yet-implemented command: `(id, label, owner lane, menu
/// path, chord)`. The id is dotted lowercase; `menu_path` is `None` for a
/// palette-only command; `chord` is `None` until the owning lane binds one
/// (assigned at implementation so it cannot clash with a live chord).
pub type ReservedId = (
    &'static str,
    &'static str,
    &'static str,
    Option<&'static [&'static str]>,
    Option<&'static str>,
);

/// The reserved campaign command ids across all phases (the F6 contract; ADR 0106).
static RESERVED_CAMPAIGN_IDS: &[ReservedId] = &[
    // Phase 1: Open Silicon, Review, Formats.
    (
        "file.import_oasis",
        "Import OASIS...",
        "import-wiring",
        Some(&["File", "Import"]),
        None,
    ),
    (
        "file.import_cif",
        "Import CIF...",
        "import-wiring",
        Some(&["File", "Import"]),
        None,
    ),
    (
        "file.import_dxf",
        "Import DXF...",
        "import-wiring",
        Some(&["File", "Import"]),
        None,
    ),
    (
        "file.export_stl",
        "Export STL...",
        "export3d",
        Some(&["File", "Export"]),
        None,
    ),
    (
        "file.export_gltf",
        "Export glTF...",
        "export3d",
        Some(&["File", "Export"]),
        None,
    ),
    (
        "gallery.open",
        "Open library gallery",
        "gallery",
        Some(&["File"]),
        None,
    ),
    (
        "gallery.open_die",
        "Open die from gallery",
        "gallery",
        None,
        None,
    ),
    (
        "review.mint_link",
        "Create review link...",
        "review",
        Some(&["Share"]),
        None,
    ),
    ("review.next_change", "Next change", "review", None, None),
    (
        "review.prev_change",
        "Previous change",
        "review",
        None,
        None,
    ),
    ("review.approve", "Approve review", "review", None, None),
    (
        "review.request_changes",
        "Request changes",
        "review",
        None,
        None,
    ),
    ("review.comment", "Add review comment", "review", None, None),
    (
        "snapshot.create",
        "Create snapshot...",
        "snapshots",
        Some(&["Share"]),
        None,
    ),
    ("snapshot.open", "Open snapshot", "snapshots", None, None),
    // Phase 3: Depth.
    (
        "waveform.run_oracle",
        "Run simulation oracle",
        "waveform-ui",
        None,
        None,
    ),
    (
        "waveform.export_csv",
        "Export waveforms CSV...",
        "waveform-ui",
        None,
        None,
    ),
    (
        "file.export_spice",
        "Export SPICE netlist...",
        "xschem",
        Some(&["File", "Export"]),
        None,
    ),
    (
        "xschem.import_probe",
        "Import xschem probe...",
        "xschem",
        None,
        None,
    ),
    (
        "classroom.bring_everyone",
        "Bring everyone here",
        "classroom",
        None,
        None,
    ),
    (
        "classroom.follow",
        "Follow instructor",
        "classroom",
        None,
        None,
    ),
    (
        "classroom.unlock_student",
        "Unlock student",
        "classroom",
        None,
        None,
    ),
    // Phase 4: Reach.
    ("plugin.browse", "Browse plugins", "plugin-ui", None, None),
    ("plugin.install", "Install plugin", "plugin-ui", None, None),
    ("plugin.enable", "Enable plugin", "plugin-ui", None, None),
    ("plugin.disable", "Disable plugin", "plugin-ui", None, None),
    (
        "underlay.load",
        "Load die-photo underlay...",
        "underlay",
        Some(&["File"]),
        None,
    ),
    ("underlay.align", "Align underlay", "underlay", None, None),
    (
        "underlay.opacity",
        "Underlay opacity",
        "underlay",
        None,
        None,
    ),
    ("embed.toggle", "Toggle embed mode", "embed", None, None),
];

/// The reserved campaign command ids (see [`ReservedId`] and ADR 0106).
#[must_use]
pub fn reserved_command_ids() -> &'static [ReservedId] {
    RESERVED_CAMPAIGN_IDS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{Chord, Keymap};

    #[test]
    fn every_id_is_unique() {
        let mut seen = std::collections::HashSet::new();
        for spec in registry() {
            assert!(seen.insert(spec.id.0), "duplicate id {}", spec.id.0);
        }
    }

    #[test]
    fn every_default_chord_parses() {
        for spec in registry() {
            if let Some(text) = spec.default_chord {
                assert!(
                    Chord::parse(text).is_some(),
                    "default chord {text:?} for {} does not parse",
                    spec.id.0
                );
            }
        }
    }

    #[test]
    fn default_chords_do_not_conflict() {
        // Reuse the keymap conflict validator over the defaults the registry
        // produces: two commands must never ship with the same chord.
        assert!(
            Keymap::defaults().conflicts().is_empty(),
            "registry default chords must not conflict"
        );
    }

    #[test]
    fn nothing_is_orphaned() {
        // Every command is reachable from a menu or the palette (or both).
        for spec in registry() {
            assert!(
                spec.menu_path.is_some() || spec.palette_visible(),
                "{} is reachable from neither a menu nor the palette",
                spec.id.0
            );
        }
    }

    #[test]
    fn spec_lookup_round_trips() {
        for spec in registry() {
            assert_eq!(super::spec(spec.id).map(|s| s.id), Some(spec.id));
        }
        assert!(super::spec(CommandId("nope.nonsense")).is_none());
    }

    #[test]
    fn rebindable_id_only_resolves_rebindable_commands() {
        assert_eq!(rebindable_id("edit.undo"), Some(CommandId("edit.undo")));
        // select.clear is a real command but not rebindable.
        assert_eq!(rebindable_id("select.clear"), None);
        assert_eq!(rebindable_id("not.a.command"), None);
    }

    #[test]
    fn lane_3b_reserved_ids_match_section_4() {
        // The cross-lane contract Gate 2 asserts: id -> (label, menu path, chord)
        // exactly as ia-inventory.md section 4 spells it for lane 3b.
        let cases: [(&str, &str, &[&str], Option<&str>); 5] = [
            ("file.open_dialog", "Open...", &["File"], Some("Ctrl+O")),
            ("file.open_url", "Open from URL...", &["File"], None),
            (
                "file.convert_gds",
                "Convert GDS to archive...",
                &["File"],
                None,
            ),
            ("share.dialog", "Share this session...", &["Share"], None),
            (
                "share.copy_viewer_link",
                "Copy viewer link",
                &["Share"],
                None,
            ),
        ];
        for (id, label, path, chord) in cases {
            let s = super::spec(CommandId(id)).expect("reserved id in registry");
            assert_eq!(s.label, label, "label for {id}");
            assert_eq!(s.menu_path, Some(path), "menu path for {id}");
            assert_eq!(s.default_chord, chord, "chord for {id}");
        }
    }

    #[test]
    fn reserved_ids_have_their_contracted_menu_paths() {
        // A spot check of the section-4 contract: id -> menu path.
        let cases = [
            ("edit.undo", &["Edit"][..]),
            ("view.split_h", &["View", "Split"][..]),
            ("view.panel_3d", &["View", "Panels"][..]),
            ("view.panel_xsection", &["View", "Panels"][..]),
            ("file.export_png", &["File", "Export"][..]),
            ("palette.open", &["Help"][..]),
        ];
        for (id, path) in cases {
            let spec = super::spec(CommandId(id)).expect("id in registry");
            assert_eq!(spec.menu_path, Some(path), "menu path for {id}");
        }
    }

    #[test]
    fn every_context_menu_id_is_a_registered_command() {
        // Context menus are registry-driven: every id they name must resolve to a
        // real command so the menu never shows a dead row (item 47).
        for context in [
            MenuContext::Canvas,
            MenuContext::Shape,
            MenuContext::LayerRow,
            MenuContext::PanelHeader,
        ] {
            let ids = context.command_ids();
            assert!(!ids.is_empty(), "{context:?} menu must offer something");
            for id in ids {
                assert!(
                    super::spec(CommandId(id)).is_some(),
                    "{context:?} menu id {id} must be in the registry"
                );
            }
        }
    }

    #[test]
    fn lane_3c_reserved_ids_match_the_section_4_contract() {
        // ia-inventory section 4: id -> (menu path, default chord). The goto and
        // focus commands are palette-only (no menu path); help.shortcuts lives on
        // the Help menu.
        let goto_coord = super::spec(CommandId("palette.goto_coordinate")).expect("registered");
        assert_eq!(goto_coord.menu_path, None);
        assert_eq!(goto_coord.default_chord, None);

        let goto_cell = super::spec(CommandId("palette.goto_cell")).expect("registered");
        assert_eq!(goto_cell.menu_path, None);
        assert_eq!(goto_cell.default_chord, None);

        let cycle = super::spec(CommandId("focus.cycle")).expect("registered");
        assert_eq!(cycle.menu_path, None);
        assert_eq!(cycle.default_chord, Some("F6"));

        let shortcuts = super::spec(CommandId("help.shortcuts")).expect("registered");
        assert_eq!(shortcuts.menu_path, Some(&["Help"][..]));
        assert_eq!(shortcuts.default_chord, Some("?"));
    }

    #[test]
    fn reserved_campaign_ids_are_well_formed_unique_and_disjoint_from_the_registry() {
        // The F6 contract (ADR 0106): every planned campaign command id is reserved now,
        // is well-formed, unique, and NOT yet a live registry command. When a lane ships
        // one it moves the id into REGISTRY and deletes the reserved row, so reserved and
        // live ids stay disjoint. This keeps the parity and keymap tests above untouched.
        let mut seen = std::collections::HashSet::new();
        for &(id, label, owner, menu_path, _chord) in reserved_command_ids() {
            assert!(id.contains('.'), "reserved id `{id}` must be dotted");
            assert!(
                id.bytes().all(|b| b.is_ascii_lowercase()
                    || b.is_ascii_digit()
                    || b == b'.'
                    || b == b'_'),
                "reserved id `{id}` must be lowercase dotted ascii"
            );
            assert!(!label.is_empty(), "reserved id `{id}` needs a label");
            assert!(!owner.is_empty(), "reserved id `{id}` needs an owner lane");
            if let Some(path) = menu_path {
                assert!(
                    !path.is_empty(),
                    "reserved id `{id}` has an empty menu path"
                );
            }
            assert!(seen.insert(id), "duplicate reserved id `{id}`");
            assert!(
                spec(CommandId(id)).is_none(),
                "reserved id `{id}` is already a live command; move it out of the reserved table when it ships"
            );
        }
        assert!(!seen.is_empty(), "the reserved table must not be empty");
    }
}
