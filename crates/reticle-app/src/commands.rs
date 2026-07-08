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
];

/// The full command registry.
///
/// Every menu, the palette, the shortcuts overlay, and the keymap read this one
/// table. Entries are ordered by the per-lane sections in the `REGISTRY` array.
#[must_use]
pub fn registry() -> &'static [CommandSpec] {
    REGISTRY
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
    fn reserved_ids_have_their_contracted_menu_paths() {
        // A spot check of the section-4 contract: id -> menu path.
        let cases = [
            ("edit.undo", &["Edit"][..]),
            ("view.split_h", &["View", "Split"][..]),
            ("file.export_png", &["File", "Export"][..]),
            ("palette.open", &["Help"][..]),
        ];
        for (id, path) in cases {
            let spec = super::spec(CommandId(id)).expect("id in registry");
            assert_eq!(spec.menu_path, Some(path), "menu path for {id}");
        }
    }
}
