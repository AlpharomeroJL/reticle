//! The menu bar, rendered from the command registry (Wave 2, lane 2A).
//!
//! The menu bar is data, not hand-wired lists: [`build_menus`] walks
//! [`crate::commands::registry`], groups every entry that carries a `menu_path`
//! into a fixed-order tree of top-level menus and nested submenus, and stamps each
//! leaf with the live chord from the [`Keymap`]. A command
//! added to the registry shows up here with no change to this module, so the menu,
//! the palette (3C), and the shortcuts overlay never drift apart. The parity test
//! at the bottom asserts exactly that: every rendered leaf resolves to a registry
//! id, and every registry command with a `menu_path` is rendered.
//!
//! Rendering ([`render_bar`]) uses egui's [`MenuBar`](egui::MenuBar) with
//! [`MenuButton`](egui::containers::menu::MenuButton) /
//! [`SubMenuButton`](egui::containers::menu::SubMenuButton); each leaf is an
//! [`egui::Button`] with its chord as `shortcut_text`. A click never mutates the
//! app inside the menu closure: it records a [`MenuChoice`] the caller applies once
//! the bar has closed (the same collect-then-apply shape the tour and Start screen
//! use), so the menu closures borrow nothing but the local choice.
//!
//! When the window is too narrow to hold every top-level menu, [`plan_overflow`]
//! folds the tail into a trailing `...` menu so nothing wraps or floats.
//!
//! A few menu items are not registry commands: the dynamic **Open Recent** list and
//! the web **Convert GDS** picker (owned by lane 3B; reached here through the
//! existing web-shell call until 3B registers `file.convert_gds`). These are
//! injected at render time and are the documented exceptions to "every menu item is
//! a registry id"; the parity test covers the registry-derived tree only. (Take the
//! tour is a registry command now, `help.tour`, so it renders from the tree.)

use crate::commands::{self, CommandId, Scope};
use crate::keymap::Keymap;
use eframe::egui::{self, Widget as _};

/// The fixed left-to-right order of the top-level menus. A menu appears only when
/// the registry contributes at least one command to it; anything the registry
/// places outside this list is appended after (defensive, never expected).
const TOP_ORDER: &[&str] = &[
    "File", "Edit", "View", "Select", "Draw", "Verify", "Share", "Help",
];

/// The estimated width, in points, reserved for the trailing `...` overflow menu
/// button when [`plan_overflow`] has to fold. It is an estimate: the real button
/// sizes itself, so a small mismatch only shifts when the fold begins by a menu.
const ELLIPSIS_WIDTH: f32 = 40.0;

/// One node in the static, registry-derived menu tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuNode {
    /// A runnable command: its registry [`CommandId`], the label to show, and the
    /// live chord text (`None` when the command ships or is left unbound).
    Item {
        /// The registry id dispatched when the item is clicked.
        id: CommandId,
        /// The label shown in the menu.
        label: &'static str,
        /// The current chord, shown right-aligned as `shortcut_text`.
        chord: Option<String>,
    },
    /// A nested submenu with its own children (`View > Split`, `Help > Developer`).
    Sub {
        /// The submenu label.
        label: &'static str,
        /// The submenu's children, in registry order.
        children: Vec<MenuNode>,
    },
}

/// A top-level menu: its label (`"File"`) and its children.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Menu {
    /// The menu label shown in the bar.
    pub label: &'static str,
    /// The menu's children, in registry order.
    pub nodes: Vec<MenuNode>,
}

/// What a menu click asks the app to do, recorded inside the menu closure and
/// applied by [`App::apply_menu_choice`](crate::app) once the bar has closed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuChoice {
    /// Dispatch a registry command through the app's `dispatch` funnel.
    Command(CommandId),
    /// Open a recent file: routed to the Start screen, where recent files are
    /// actionable (the live reopen is lane 2D's catalog item 9).
    OpenRecent,
    /// Trigger the in-browser GDS convert picker (web only; lane 3B's effect).
    #[cfg(target_arch = "wasm32")]
    ConvertGds,
}

/// Whether a command's [`Scope`] is reachable on the current build target.
fn scope_visible(scope: Scope) -> bool {
    match scope {
        Scope::Global => true,
        Scope::NativeOnly => cfg!(not(target_arch = "wasm32")),
        Scope::WasmOnly => cfg!(target_arch = "wasm32"),
    }
}

/// Builds the menu tree from the command registry, stamping each leaf with the
/// chord `keymap` currently binds to it.
///
/// Every registry entry with a `menu_path` reachable on this build target becomes
/// a leaf under its path; submenus are created on first use and keep registry
/// order. Top-level menus are ordered by `TOP_ORDER`. The result is pure data:
/// [`render_bar`] draws it and the parity test checks it.
#[must_use]
pub fn build_menus(keymap: &Keymap) -> Vec<Menu> {
    let mut menus: Vec<Menu> = Vec::new();
    for spec in commands::registry() {
        let Some(path) = spec.menu_path else { continue };
        if !scope_visible(spec.scope) || path.is_empty() {
            continue;
        }
        let chord = keymap.chord_for(spec.id).map(ToString::to_string);
        let leaf = MenuNode::Item {
            id: spec.id,
            label: spec.label,
            chord,
        };
        // Walk down (creating as needed) the top menu then each intermediate
        // submenu named by path[1..], and push the leaf into the deepest one.
        let top = top_menu_mut(&mut menus, path[0]);
        insert_into(&mut top.nodes, &path[1..], leaf);
    }
    order_top_level(menus)
}

/// Returns the existing top-level menu named `label`, creating an empty one at the
/// end if it does not exist yet.
fn top_menu_mut<'a>(menus: &'a mut Vec<Menu>, label: &'static str) -> &'a mut Menu {
    if let Some(i) = menus.iter().position(|m| m.label == label) {
        &mut menus[i]
    } else {
        menus.push(Menu {
            label,
            nodes: Vec::new(),
        });
        menus.last_mut().expect("just pushed")
    }
}

/// Inserts `leaf` into `nodes`, descending through the submenu names in `path`
/// (creating each missing submenu). An empty `path` pushes the leaf directly.
fn insert_into(nodes: &mut Vec<MenuNode>, path: &[&'static str], leaf: MenuNode) {
    let Some((head, rest)) = path.split_first() else {
        nodes.push(leaf);
        return;
    };
    // Find or create the submenu named `head`, then recurse into it.
    let existing = nodes
        .iter()
        .position(|n| matches!(n, MenuNode::Sub { label, .. } if label == head));
    let idx = if let Some(i) = existing {
        i
    } else {
        nodes.push(MenuNode::Sub {
            label: head,
            children: Vec::new(),
        });
        nodes.len() - 1
    };
    if let MenuNode::Sub { children, .. } = &mut nodes[idx] {
        insert_into(children, rest, leaf);
    }
}

/// Reorders top-level menus by [`TOP_ORDER`]; any menu whose label is not listed
/// there keeps its build order and follows the known ones.
fn order_top_level(mut menus: Vec<Menu>) -> Vec<Menu> {
    let rank = |label: &str| TOP_ORDER.iter().position(|t| *t == label);
    menus.sort_by(|a, b| match (rank(a.label), rank(b.label)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    menus
}

/// Plans how many leading top-level menus fit in `avail` points before the tail
/// must fold into a trailing `...` menu.
///
/// `label_widths` is the rendered width of each top-level menu button, in order.
/// The return value `n` is how many to draw directly; menus `n..` fold into the
/// `...` menu. When everything fits, `n == label_widths.len()` and no `...` button
/// is drawn; otherwise room for the `...` button (`ELLIPSIS_WIDTH`) is reserved
/// first, so the fold affordance itself never overflows.
#[must_use]
pub fn plan_overflow(label_widths: &[f32], avail: f32) -> usize {
    let total: f32 = label_widths.iter().sum();
    if total <= avail {
        return label_widths.len();
    }
    let budget = (avail - ELLIPSIS_WIDTH).max(0.0);
    let mut used = 0.0;
    let mut n = 0;
    for &w in label_widths {
        if used + w > budget {
            break;
        }
        used += w;
        n += 1;
    }
    n
}

/// Draws the whole menu bar into `ui`, folding the tail into a `...` menu when the
/// bar is too narrow. Clicks are recorded into `choice`; `recent` is the current
/// recent-file list for the dynamic Open Recent submenu.
///
/// Call inside [`MenuBar::ui`](egui::MenuBar::ui).
pub fn render_bar(
    ui: &mut egui::Ui,
    menus: &[Menu],
    recent: &[String],
    choice: &mut Option<MenuChoice>,
) {
    let avail = ui.available_width();
    let widths: Vec<f32> = menus
        .iter()
        .map(|m| menu_button_width(ui, m.label))
        .collect();
    let shown = plan_overflow(&widths, avail).min(menus.len());
    for menu in &menus[..shown] {
        egui::containers::menu::MenuButton::new(menu.label)
            .ui(ui, |ui| render_menu_content(ui, menu, recent, choice));
    }
    if shown < menus.len() {
        egui::containers::menu::MenuButton::new("...").ui(ui, |ui| {
            for menu in &menus[shown..] {
                egui::containers::menu::SubMenuButton::new(menu.label)
                    .ui(ui, |ui| render_menu_content(ui, menu, recent, choice));
            }
        });
    }
}

/// The width one top-level menu button needs: its label galley plus the button
/// padding and the inter-item gap.
fn menu_button_width(ui: &egui::Ui, label: &str) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, egui::Color32::PLACEHOLDER);
    galley.size().x + 2.0 * ui.spacing().button_padding.x + ui.spacing().item_spacing.x
}

/// Renders one top-level menu's contents: its registry-derived nodes followed by
/// the dynamic, non-registry items that belong under it.
fn render_menu_content(
    ui: &mut egui::Ui,
    menu: &Menu,
    recent: &[String],
    choice: &mut Option<MenuChoice>,
) {
    render_nodes(ui, &menu.nodes, choice);
    // The File menu carries the only non-registry items: the dynamic Open Recent
    // list and, on the web, the in-browser GDS convert picker.
    if menu.label == "File" {
        ui.separator();
        egui::containers::menu::SubMenuButton::new("Open Recent").ui(ui, |ui| {
            if recent.is_empty() {
                ui.add_enabled(false, egui::Button::new("No recent files"));
            } else {
                for name in recent {
                    if egui::Button::new(name.as_str()).ui(ui).clicked() {
                        *choice = Some(MenuChoice::OpenRecent);
                    }
                }
            }
        });
        // Web only: the in-browser GDS convert picker (lane 3B owns
        // `file.convert_gds`; this reaches the shipped web-shell call directly).
        #[cfg(target_arch = "wasm32")]
        if egui::Button::new("Convert GDS to archive...")
            .ui(ui)
            .clicked()
        {
            *choice = Some(MenuChoice::ConvertGds);
        }
    }
}

/// Renders a list of menu nodes: each leaf as a [`egui::Button`] carrying its
/// chord, each submenu as a [`SubMenuButton`](egui::containers::menu::SubMenuButton).
fn render_nodes(ui: &mut egui::Ui, nodes: &[MenuNode], choice: &mut Option<MenuChoice>) {
    for node in nodes {
        match node {
            MenuNode::Item { id, label, chord } => {
                let mut button = egui::Button::new(*label);
                if let Some(c) = chord {
                    button = button.shortcut_text(c.as_str());
                }
                if button.ui(ui).clicked() {
                    *choice = Some(MenuChoice::Command(*id));
                }
            }
            MenuNode::Sub { label, children } => {
                egui::containers::menu::SubMenuButton::new(*label)
                    .ui(ui, |ui| render_nodes(ui, children, choice));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collects every leaf as `(id, label, path)` where `path` is the full label
    /// path down to (but not including) the leaf, e.g. `["View", "Split"]`.
    fn leaves(menus: &[Menu]) -> Vec<(CommandId, &'static str, Vec<&'static str>)> {
        fn walk(
            nodes: &[MenuNode],
            path: &mut Vec<&'static str>,
            out: &mut Vec<(CommandId, &'static str, Vec<&'static str>)>,
        ) {
            for node in nodes {
                match node {
                    MenuNode::Item { id, label, .. } => out.push((*id, label, path.clone())),
                    MenuNode::Sub { label, children } => {
                        path.push(label);
                        walk(children, path, out);
                        path.pop();
                    }
                }
            }
        }
        let mut out = Vec::new();
        for menu in menus {
            let mut path = vec![menu.label];
            walk(&menu.nodes, &mut path, &mut out);
        }
        out
    }

    #[test]
    fn top_level_menus_follow_the_fixed_order() {
        let menus = build_menus(&Keymap::defaults());
        let labels: Vec<&str> = menus.iter().map(|m| m.label).collect();
        // The rendered order is a subsequence of the canonical order.
        let mut ranks: Vec<usize> = labels
            .iter()
            .map(|l| TOP_ORDER.iter().position(|t| t == l).expect("known menu"))
            .collect();
        let sorted = {
            let mut r = ranks.clone();
            r.sort_unstable();
            r
        };
        assert_eq!(
            ranks, sorted,
            "menus must be in canonical order: {labels:?}"
        );
        ranks.dedup();
        assert_eq!(ranks.len(), labels.len(), "no duplicate top-level menus");
        // The commands 1E ships today land these menus at minimum.
        for expect in ["File", "Edit", "View", "Select", "Draw", "Help"] {
            assert!(labels.contains(&expect), "menu {expect} is missing");
        }
    }

    #[test]
    fn every_leaf_resolves_to_a_registry_id() {
        let menus = build_menus(&Keymap::defaults());
        for (id, _, _) in leaves(&menus) {
            assert!(
                commands::spec(id).is_some(),
                "menu leaf {} has no registry entry",
                id.0
            );
        }
    }

    #[test]
    fn every_menu_command_appears_once_with_its_contracted_path() {
        let menus = build_menus(&Keymap::defaults());
        let found = leaves(&menus);
        for spec in commands::registry() {
            let Some(path) = spec.menu_path else { continue };
            if !scope_visible(spec.scope) {
                continue;
            }
            let hits: Vec<_> = found.iter().filter(|(id, _, _)| *id == spec.id).collect();
            assert_eq!(hits.len(), 1, "{} must appear exactly once", spec.id.0);
            let (_, _, got_path) = hits[0];
            assert_eq!(
                got_path.as_slice(),
                path,
                "menu path for {} does not match its registry menu_path",
                spec.id.0
            );
        }
    }

    /// Indexes every leaf by its id string for a chord lookup.
    fn index_by_id<'a>(
        nodes: &'a [MenuNode],
        out: &mut std::collections::HashMap<&'a str, &'a MenuNode>,
    ) {
        for node in nodes {
            match node {
                MenuNode::Item { id, .. } => {
                    out.insert(id.0, node);
                }
                MenuNode::Sub { children, .. } => index_by_id(children, out),
            }
        }
    }

    #[test]
    fn leaf_chords_reflect_the_keymap() {
        let keymap = Keymap::defaults();
        let menus = build_menus(&keymap);
        let mut by_id = std::collections::HashMap::new();
        for menu in &menus {
            index_by_id(&menu.nodes, &mut by_id);
        }
        // A bound command shows its chord; an unbound one shows nothing.
        if let Some(MenuNode::Item { chord, .. }) = by_id.get("edit.undo") {
            assert_eq!(chord.as_deref(), Some("Ctrl+Z"));
        } else {
            panic!("edit.undo not in the menu");
        }
        if let Some(MenuNode::Item { chord, .. }) = by_id.get("view.snap") {
            assert_eq!(chord.as_deref(), None, "snap ships unbound");
        } else {
            panic!("view.snap not in the menu");
        }
    }

    #[test]
    fn developer_actions_live_under_help_developer() {
        let menus = build_menus(&Keymap::defaults());
        let found = leaves(&menus);
        for (id, label, path) in &found {
            if id.0 == "dev.add_demo_rect" {
                assert_eq!(*label, "Insert demo rectangle");
                assert_eq!(path.as_slice(), &["Help", "Developer"]);
            }
        }
        assert!(
            found.iter().any(|(id, _, _)| id.0 == "dev.add_demo_rect"),
            "dev.add_demo_rect must be in the Help > Developer submenu"
        );
        assert!(
            found.iter().any(|(id, _, _)| id.0 == "dev.replay_theater"),
            "dev.replay_theater must be in the Help > Developer submenu"
        );
    }

    #[test]
    fn overflow_keeps_everything_when_it_fits() {
        let widths = [30.0, 30.0, 30.0];
        assert_eq!(plan_overflow(&widths, 200.0), 3, "all fit, no fold");
        assert_eq!(plan_overflow(&widths, 90.0), 3, "exact fit, no fold");
        assert_eq!(plan_overflow(&[], 100.0), 0);
    }

    #[test]
    fn overflow_folds_the_tail_and_reserves_the_ellipsis() {
        let widths = [30.0, 30.0, 30.0, 30.0];
        // 120 total > 100 avail, so we fold and reserve ELLIPSIS_WIDTH (40): budget
        // 60 holds two menus.
        assert_eq!(plan_overflow(&widths, 100.0), 2);
        // Too narrow for even one menu plus the ellipsis reserve.
        assert_eq!(plan_overflow(&widths, 45.0), 0);
        // Wide enough for three but not the fourth: still folds (reserve applies).
        assert_eq!(plan_overflow(&widths, 119.0), 2);
    }
}
