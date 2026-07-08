//! The command palette: a searchable catalog of actions.
//!
//! The palette (opened with Ctrl+P) lists every action the app can run and filters
//! it by a search query. The catalog and the filtering are pure and live here;
//! *executing* a [`Command`] mutates app state and is done by the app in
//! [`crate::app`]. Splitting it this way keeps the fuzzy-match behavior testable.

use crate::commands::{CommandId, CommandSpec, Scope};
use crate::keymap::Keymap;
use crate::tool::Tool;

/// A single palette action.
///
/// Variants that need a target (switch to a tool, toggle a layer) carry it, so the
/// executor has everything it needs without re-parsing the label. Every payload is
/// a small `Copy` value, so `Command` is `Copy` and can be passed by value freely.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    /// Switch the active canvas tool.
    SetTool(Tool),
    /// Toggle the visibility of the layer at the given technology-table index.
    ToggleLayer(usize),
    /// Undo the last edit.
    Undo,
    /// Redo the last undone edit.
    Redo,
    /// Fit the whole design to the viewport.
    ZoomToFit,
    /// Toggle the background grid.
    ToggleGrid,
    /// Toggle cursor snapping to the grid.
    ToggleSnap,
    /// Clear the current selection.
    ClearSelection,
    /// Select every shape on the layer at the given technology-table index.
    SelectLayer(usize),
    /// Export the current view to a PNG file (native only; a no-op on web).
    ExportPng,
}

/// An inline argument prompt the palette can switch into.
///
/// `palette.goto_coordinate` and `palette.goto_cell` do not run immediately when
/// chosen; they put the palette into an argument-entry mode where the query line
/// collects a coordinate or a cell name and Enter commits it. The app owns the
/// commit effect (moving the camera); this enum only names which prompt is active.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaletteArg {
    /// Prompt for an `x,y` (or `x y`) coordinate in DBU to center the view on.
    GotoCoordinate,
    /// Prompt for a cell name to locate.
    GotoCell,
}

impl PaletteArg {
    /// The prompt shown above the argument field.
    #[must_use]
    pub fn prompt(self) -> &'static str {
        match self {
            PaletteArg::GotoCoordinate => "Go to coordinate (x, y in DBU):",
            PaletteArg::GotoCell => "Go to cell (name):",
        }
    }

    /// The placeholder hint shown in the empty argument field.
    #[must_use]
    pub fn hint(self) -> &'static str {
        match self {
            PaletteArg::GotoCoordinate => "e.g. 1200, -800",
            PaletteArg::GotoCell => "cell name",
        }
    }
}

/// What choosing a palette row does.
///
/// Registry commands route back through the app's `dispatch` funnel by id, so the
/// palette shares the one execution path with menus and shortcuts. The dynamic
/// document rows (layers, cells, bookmarks) carry the small target they act on.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PaletteAction {
    /// Run the registry command with this id through `App::dispatch`.
    Command(CommandId),
    /// Toggle visibility of the layer at this technology-table index.
    ToggleLayer(usize),
    /// Select every shape on the layer at this index.
    SelectLayer(usize),
    /// Center the view on the named cell (item 80).
    GotoCell(String),
    /// Recall the camera bookmark at this slot (item 80).
    Bookmark(usize),
}

/// One row the palette can display, match, and run.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PaletteItem {
    /// A stable key for recents tracking and de-duplication (`"edit.undo"`,
    /// `"layer.toggle.3"`).
    pub key: String,
    /// The text shown in the row and matched against the query.
    pub label: String,
    /// The group heading (a registry category, or a document group like `Layers`).
    pub category: String,
    /// The current chord for a bound command, shown as a hint chip; `None` when
    /// the command is unbound or the row is a document target.
    pub hint: Option<String>,
    /// What running the row does.
    pub action: PaletteAction,
}

/// The document-derived targets the palette also searches (item 80): layer names
/// indexed by technology-table position, cell names, and camera-bookmark labels.
#[derive(Clone, Copy, Debug, Default)]
pub struct DocTargets<'a> {
    /// Layer names in technology-table order (index is the toggle/select target).
    pub layers: &'a [String],
    /// Cell names reachable in the current document.
    pub cells: &'a [String],
    /// Camera-bookmark labels in slot order (index is the recall target).
    pub bookmarks: &'a [String],
}

/// How many recent rows the palette surfaces at the top (item 79).
pub const RECENTS_SHOWN: usize = 6;

/// Whether a command's [`Scope`] is runnable in the current build, so the palette
/// hides actions that cannot run here (PNG export needs native file IO).
#[must_use]
fn scope_available(scope: Scope) -> bool {
    match scope {
        Scope::Global => true,
        Scope::NativeOnly => cfg!(not(target_arch = "wasm32")),
        Scope::WasmOnly => cfg!(target_arch = "wasm32"),
    }
}

/// Builds every palette row from the command registry plus the document targets.
///
/// Registry rows come first, in registry order, each carrying its live chord hint
/// from `keymap`; then the generated per-layer toggle/select rows, cell rows, and
/// bookmark rows are appended (item 80). Grouping, recents, and fuzzy ranking are
/// applied later by [`results`]; this function just assembles the flat catalog.
#[must_use]
pub fn build_items(
    specs: &[CommandSpec],
    keymap: &Keymap,
    targets: &DocTargets,
) -> Vec<PaletteItem> {
    let mut items = Vec::new();
    for spec in specs {
        if !spec.palette_visible() || !scope_available(spec.scope) {
            continue;
        }
        items.push(PaletteItem {
            key: spec.id.0.to_owned(),
            label: spec.label.to_owned(),
            category: spec.category.to_owned(),
            hint: keymap.chord_for(spec.id).map(ToString::to_string),
            action: PaletteAction::Command(spec.id),
        });
    }
    for (i, name) in targets.layers.iter().enumerate() {
        items.push(PaletteItem {
            key: format!("layer.toggle.{i}"),
            label: format!("Toggle layer {name}"),
            category: "Layers".to_owned(),
            hint: None,
            action: PaletteAction::ToggleLayer(i),
        });
        items.push(PaletteItem {
            key: format!("layer.select.{i}"),
            label: format!("Select all on {name}"),
            category: "Layers".to_owned(),
            hint: None,
            action: PaletteAction::SelectLayer(i),
        });
    }
    for name in targets.cells {
        items.push(PaletteItem {
            key: format!("cell.{name}"),
            label: format!("Go to cell {name}"),
            category: "Cells".to_owned(),
            hint: None,
            action: PaletteAction::GotoCell(name.clone()),
        });
    }
    for (i, label) in targets.bookmarks.iter().enumerate() {
        items.push(PaletteItem {
            key: format!("bookmark.{i}"),
            label: format!("Go to bookmark {label}"),
            category: "Bookmarks".to_owned(),
            hint: None,
            action: PaletteAction::Bookmark(i),
        });
    }
    items
}

/// A named, ordered group of palette rows for rendering.
#[derive(Debug)]
pub struct PaletteGroup<'a> {
    /// The section heading (`Recent`, a registry category, or a document group).
    pub heading: String,
    /// The rows in this group, already ordered.
    pub items: Vec<&'a PaletteItem>,
}

/// Groups and orders the palette rows for the current `query` (item 79).
///
/// With a blank query the rows are shown by group: a `Recent` group first (the
/// most-recently-run rows, de-duplicated, capped at [`RECENTS_SHOWN`]) when any
/// recents exist, then every row grouped by category in first-seen order. With a
/// query the rows are fuzzy-filtered (a case-insensitive subsequence, tightest
/// span first) and grouped by category, the category with the best match leading;
/// no `Recent` group is shown because the ranking already floats the best hits.
#[must_use]
pub fn results<'a>(
    items: &'a [PaletteItem],
    recents: &[String],
    query: &str,
) -> Vec<PaletteGroup<'a>> {
    if query.trim().is_empty() {
        blank_results(items, recents)
    } else {
        filtered_results(items, query)
    }
}

/// The blank-query layout: a `Recent` group (when any recents resolve) followed by
/// every row grouped by category in first-seen order.
fn blank_results<'a>(items: &'a [PaletteItem], recents: &[String]) -> Vec<PaletteGroup<'a>> {
    let mut groups = Vec::new();
    let mut recent_items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for key in recents {
        if !seen.insert(key.as_str()) {
            continue;
        }
        if let Some(item) = items.iter().find(|i| &i.key == key) {
            recent_items.push(item);
            if recent_items.len() >= RECENTS_SHOWN {
                break;
            }
        }
    }
    if !recent_items.is_empty() {
        groups.push(PaletteGroup {
            heading: "Recent".to_owned(),
            items: recent_items,
        });
    }
    for item in items {
        push_into_category(&mut groups, item);
    }
    groups
}

/// The query layout: fuzzy-filter the rows, rank tightest span first, and group by
/// category with the best-matching category leading.
fn filtered_results<'a>(items: &'a [PaletteItem], query: &str) -> Vec<PaletteGroup<'a>> {
    let needle = query.to_lowercase();
    let mut scored: Vec<(usize, usize, &PaletteItem)> = items
        .iter()
        .filter_map(|i| {
            subsequence_span(&i.label.to_lowercase(), &needle).map(|s| (s, i.label.len(), i))
        })
        .collect();
    scored.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then_with(|| a.2.label.cmp(&b.2.label))
    });
    let mut groups = Vec::new();
    for (_, _, item) in scored {
        push_into_category(&mut groups, item);
    }
    groups
}

/// Appends `item` to its category group, creating the group (in first-seen order)
/// if this is the first row for that category. The `Recent` group is never touched.
fn push_into_category<'a>(groups: &mut Vec<PaletteGroup<'a>>, item: &'a PaletteItem) {
    if let Some(group) = groups
        .iter_mut()
        .find(|g| g.heading != "Recent" && g.heading == item.category)
    {
        group.items.push(item);
    } else {
        groups.push(PaletteGroup {
            heading: item.category.clone(),
            items: vec![item],
        });
    }
}

/// If `needle` is a subsequence of `haystack`, returns the length of the smallest
/// span in `haystack` (from the first matched char to the last) that contains the
/// match; otherwise `None`. Both arguments must already be lowercased.
fn subsequence_span(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = haystack.chars().collect();
    let need: Vec<char> = needle.chars().collect();
    let mut first = None;
    let mut ni = 0;
    for (hi, &hc) in hay.iter().enumerate() {
        if hc == need[ni] {
            if first.is_none() {
                first = Some(hi);
            }
            ni += 1;
            if ni == need.len() {
                return Some(hi - first.unwrap_or(hi) + 1);
            }
        }
    }
    None
}

/// Parses a `goto coordinate` argument into an `(x, y)` pair in DBU.
///
/// Accepts a comma- or whitespace-separated pair (`"1200, -800"`, `"1200 -800"`),
/// trimming surrounding space. Returns `None` for anything that is not exactly two
/// integers, so the palette can keep the prompt open on bad input.
#[must_use]
pub fn parse_coordinate(text: &str) -> Option<(i64, i64)> {
    let parts: Vec<&str> = text
        .split([',', ' ', '\t'])
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() != 2 {
        return None;
    }
    let x = parts[0].parse().ok()?;
    let y = parts[1].parse().ok()?;
    Some((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{self, CommandId};

    fn targets<'a>(
        layers: &'a [String],
        cells: &'a [String],
        bookmarks: &'a [String],
    ) -> DocTargets<'a> {
        DocTargets {
            layers,
            cells,
            bookmarks,
        }
    }

    fn find<'a>(items: &'a [PaletteItem], key: &str) -> &'a PaletteItem {
        items
            .iter()
            .find(|i| i.key == key)
            .unwrap_or_else(|| panic!("no palette item with key {key}"))
    }

    #[test]
    fn build_items_sources_registry_commands_with_live_chord_hints() {
        let items = build_items(
            commands::registry(),
            &Keymap::defaults(),
            &DocTargets::default(),
        );
        let undo = find(&items, "edit.undo");
        assert_eq!(undo.label, "Undo");
        assert_eq!(undo.category, "Edit");
        assert_eq!(undo.hint.as_deref(), Some("Ctrl+Z"));
        assert_eq!(undo.action, PaletteAction::Command(CommandId("edit.undo")));
    }

    #[test]
    fn build_items_leaves_unbound_commands_without_a_hint() {
        let items = build_items(
            commands::registry(),
            &Keymap::defaults(),
            &DocTargets::default(),
        );
        // The rectangle tool ships unbound.
        assert_eq!(find(&items, "tool.rect").hint, None);
    }

    #[test]
    fn build_items_appends_generated_layer_rows() {
        let layers = vec!["MET1".to_owned(), "POLY".to_owned()];
        let items = build_items(
            commands::registry(),
            &Keymap::defaults(),
            &targets(&layers, &[], &[]),
        );
        assert!(
            items
                .iter()
                .any(|i| i.action == PaletteAction::ToggleLayer(0) && i.label.contains("MET1"))
        );
        assert!(
            items
                .iter()
                .any(|i| i.action == PaletteAction::SelectLayer(1) && i.label.contains("POLY"))
        );
    }

    #[test]
    fn build_items_appends_cell_and_bookmark_targets() {
        let cells = vec!["ALU".to_owned()];
        let bookmarks = vec!["Overview".to_owned()];
        let items = build_items(
            commands::registry(),
            &Keymap::defaults(),
            &targets(&[], &cells, &bookmarks),
        );
        assert!(
            items
                .iter()
                .any(|i| i.action == PaletteAction::GotoCell("ALU".to_owned()))
        );
        assert!(
            items
                .iter()
                .any(|i| i.action == PaletteAction::Bookmark(0) && i.label.contains("Overview"))
        );
    }

    #[test]
    fn blank_query_leads_with_a_recent_group_in_order() {
        let items = build_items(
            commands::registry(),
            &Keymap::defaults(),
            &DocTargets::default(),
        );
        let recents = vec!["edit.undo".to_owned(), "tool.pan".to_owned()];
        let groups = results(&items, &recents, "");
        assert_eq!(groups[0].heading, "Recent");
        assert_eq!(groups[0].items[0].key, "edit.undo");
        assert_eq!(groups[0].items[1].key, "tool.pan");
    }

    #[test]
    fn recent_group_dedupes_and_ignores_unknown_keys() {
        let items = build_items(
            commands::registry(),
            &Keymap::defaults(),
            &DocTargets::default(),
        );
        let recents = vec![
            "edit.undo".to_owned(),
            "edit.undo".to_owned(),
            "no.such.command".to_owned(),
        ];
        let groups = results(&items, &recents, "");
        assert_eq!(groups[0].heading, "Recent");
        assert_eq!(groups[0].items.len(), 1, "de-duplicated, unknown dropped");
    }

    #[test]
    fn blank_query_without_recents_starts_with_a_category_group() {
        let items = build_items(
            commands::registry(),
            &Keymap::defaults(),
            &DocTargets::default(),
        );
        let groups = results(&items, &[], "");
        assert!(!groups.is_empty());
        assert_ne!(groups[0].heading, "Recent");
        // Every group carries at least one row.
        assert!(groups.iter().all(|g| !g.items.is_empty()));
    }

    #[test]
    fn blank_query_groups_every_row_by_category() {
        let layers = vec!["MET1".to_owned()];
        let items = build_items(
            commands::registry(),
            &Keymap::defaults(),
            &targets(&layers, &[], &[]),
        );
        let groups = results(&items, &[], "");
        let total: usize = groups.iter().map(|g| g.items.len()).sum();
        assert_eq!(total, items.len(), "no row is dropped when grouping");
        assert!(groups.iter().any(|g| g.heading == "Layers"));
    }

    #[test]
    fn query_fuzzy_filters_and_groups_without_a_recent_group() {
        let items = build_items(
            commands::registry(),
            &Keymap::defaults(),
            &DocTargets::default(),
        );
        let groups = results(&items, &["edit.undo".to_owned()], "undo");
        assert!(groups.iter().all(|g| g.heading != "Recent"));
        assert!(
            groups
                .iter()
                .flat_map(|g| &g.items)
                .any(|i| i.key == "edit.undo")
        );
    }

    #[test]
    fn query_matches_a_non_adjacent_subsequence() {
        let items = build_items(
            commands::registry(),
            &Keymap::defaults(),
            &DocTargets::default(),
        );
        // "clsl" is a subsequence of "Clear selection".
        let groups = results(&items, &[], "clsl");
        assert!(
            groups
                .iter()
                .flat_map(|g| &g.items)
                .any(|i| i.key == "select.clear")
        );
    }

    #[test]
    fn query_with_no_match_yields_no_groups() {
        let items = build_items(
            commands::registry(),
            &Keymap::defaults(),
            &DocTargets::default(),
        );
        assert!(results(&items, &[], "zzzqqqwww").is_empty());
    }

    #[test]
    fn parse_coordinate_accepts_comma_or_space_separated_pairs() {
        assert_eq!(parse_coordinate("1200, -800"), Some((1200, -800)));
        assert_eq!(parse_coordinate("  1200   -800 "), Some((1200, -800)));
        assert_eq!(parse_coordinate("0,0"), Some((0, 0)));
    }

    #[test]
    fn parse_coordinate_rejects_non_pairs() {
        assert_eq!(parse_coordinate(""), None);
        assert_eq!(parse_coordinate("1200"), None, "one value");
        assert_eq!(parse_coordinate("1200, 3, 4"), None, "three values");
        assert_eq!(parse_coordinate("x, y"), None, "not integers");
    }

    #[test]
    fn query_ranks_the_tighter_match_first() {
        let items = vec![
            PaletteItem {
                key: "a".to_owned(),
                label: "axxxb".to_owned(),
                category: "Test".to_owned(),
                hint: None,
                action: PaletteAction::Command(CommandId("a")),
            },
            PaletteItem {
                key: "b".to_owned(),
                label: "ab".to_owned(),
                category: "Test".to_owned(),
                hint: None,
                action: PaletteAction::Command(CommandId("b")),
            },
        ];
        let groups = results(&items, &[], "ab");
        assert_eq!(groups[0].items[0].label, "ab", "tighter span wins");
    }
}
