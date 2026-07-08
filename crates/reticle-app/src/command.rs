//! The command palette: a searchable catalog of actions.
//!
//! The palette (opened with Ctrl+P) lists every action the app can run and filters
//! it by a search query. The catalog and the filtering are pure and live here;
//! *executing* a [`Command`] mutates app state and is done by the app in
//! [`crate::app`]. Splitting it this way keeps the fuzzy-match behavior testable.

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
    /// Frame the current selection's bounding box (Shift+F, `view.zoom_selection`).
    ZoomSelection,
    /// Reset the zoom to one screen pixel per DBU, keeping the center (`view.zoom_one_to_one`).
    ZoomOneToOne,
    /// Frame the union bounding box of every visible layer (`view.zoom_layer_extents`).
    ZoomLayerExtents,
    /// Save the current view as a numbered bookmark (`view.bookmark_save`).
    BookmarkSave,
    /// Recall the saved view bookmark at the given slot (palette-only, item 34).
    RecallBookmark(usize),
    /// Toggle the background grid.
    ToggleGrid,
    /// Toggle cursor snapping to the grid.
    ToggleSnap,
    /// Clear the current selection.
    ClearSelection,
    /// Select every shape on the layer at the given technology-table index.
    SelectLayer(usize),
    /// Duplicate the current selection at a small offset (Ctrl+D, `edit.duplicate`).
    Duplicate,
    /// Copy a permalink pinning the current view and layers to the clipboard
    /// (`share.copy_permalink`, item 35).
    CopyPermalink,
    /// Export the current view to a PNG file (native only; a no-op on web).
    ExportPng,
}

/// A palette entry: a command paired with the label shown in the list.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CommandEntry {
    /// The label shown in the palette and matched against the query.
    pub label: String,
    /// The command run when the entry is chosen.
    pub command: Command,
}

impl CommandEntry {
    /// Creates an entry from a label and command.
    #[must_use]
    pub fn new(label: impl Into<String>, command: Command) -> Self {
        Self {
            label: label.into(),
            command,
        }
    }
}

/// Whether an action is available in the current build.
///
/// PNG export needs a GPU and native file IO, so it is only offered off the web.
#[must_use]
pub fn export_supported() -> bool {
    cfg!(not(target_arch = "wasm32"))
}

/// Builds the full command catalog for the given layer names.
///
/// One toggle/select entry is generated per layer (indexed by technology-table
/// position) so the palette can drive the layer manager. The PNG-export entry is
/// only included where [`export_supported`] is true, keeping the web palette free of
/// actions that cannot run.
#[must_use]
pub fn catalog(layer_names: &[String]) -> Vec<CommandEntry> {
    let mut entries = vec![
        CommandEntry::new("Tool: Select", Command::SetTool(Tool::Select)),
        CommandEntry::new("Tool: Pan", Command::SetTool(Tool::Pan)),
        CommandEntry::new("Tool: Measure", Command::SetTool(Tool::Measure)),
        CommandEntry::new("Tool: Cut line", Command::SetTool(Tool::CutLine)),
        CommandEntry::new("Tool: Draw rectangle", Command::SetTool(Tool::DrawRect)),
        CommandEntry::new("Tool: Draw polygon", Command::SetTool(Tool::DrawPolygon)),
        CommandEntry::new("Tool: Draw path", Command::SetTool(Tool::DrawPath)),
        CommandEntry::new("Tool: Edit vertices", Command::SetTool(Tool::EditVertex)),
        CommandEntry::new("Edit: Undo", Command::Undo),
        CommandEntry::new("Edit: Redo", Command::Redo),
        CommandEntry::new("Edit: Duplicate", Command::Duplicate),
        CommandEntry::new("View: Zoom to fit", Command::ZoomToFit),
        CommandEntry::new("View: Fit selection", Command::ZoomSelection),
        CommandEntry::new("View: Zoom 1:1 DBU", Command::ZoomOneToOne),
        CommandEntry::new("View: Zoom to layer extents", Command::ZoomLayerExtents),
        CommandEntry::new("View: Save view bookmark", Command::BookmarkSave),
        CommandEntry::new("View: Toggle grid", Command::ToggleGrid),
        CommandEntry::new("View: Toggle snapping", Command::ToggleSnap),
        CommandEntry::new("Share: Copy permalink at this view", Command::CopyPermalink),
        CommandEntry::new("Select: Clear selection", Command::ClearSelection),
    ];
    for (i, name) in layer_names.iter().enumerate() {
        entries.push(CommandEntry::new(
            format!("Layer: Toggle {name}"),
            Command::ToggleLayer(i),
        ));
        entries.push(CommandEntry::new(
            format!("Select: All on {name}"),
            Command::SelectLayer(i),
        ));
    }
    if export_supported() {
        entries.push(CommandEntry::new("File: Export PNG", Command::ExportPng));
    }
    entries
}

/// Filters `entries` to those matching `query`, best matches first.
///
/// Matching is a case-insensitive *subsequence* test (the query characters appear
/// in order, not necessarily adjacent), the usual command-palette feel. An empty
/// query returns every entry in catalog order. Results are ranked by match tightness
/// (a shorter matched span ranks higher), then by label length, then alphabetically,
/// so the ordering is deterministic.
#[must_use]
pub fn filter<'a>(entries: &'a [CommandEntry], query: &str) -> Vec<&'a CommandEntry> {
    if query.trim().is_empty() {
        return entries.iter().collect();
    }
    let needle = query.to_lowercase();
    let mut scored: Vec<(usize, usize, &CommandEntry)> = entries
        .iter()
        .filter_map(|e| {
            subsequence_span(&e.label.to_lowercase(), &needle).map(|span| (span, e.label.len(), e))
        })
        .collect();
    scored.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then_with(|| a.2.label.cmp(&b.2.label))
    });
    scored.into_iter().map(|(_, _, e)| e).collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        vec!["METAL1".to_owned(), "POLY".to_owned()]
    }

    #[test]
    fn catalog_includes_core_actions() {
        let c = catalog(&names());
        assert!(c.iter().any(|e| e.command == Command::Undo));
        assert!(c.iter().any(|e| e.command == Command::ZoomToFit));
        assert!(
            c.iter()
                .any(|e| e.command == Command::SetTool(Tool::Measure))
        );
    }

    #[test]
    fn catalog_has_per_layer_entries() {
        let c = catalog(&names());
        assert!(c.iter().any(|e| e.command == Command::ToggleLayer(0)));
        assert!(c.iter().any(|e| e.command == Command::SelectLayer(1)));
    }

    #[test]
    fn empty_query_returns_all() {
        let c = catalog(&names());
        assert_eq!(filter(&c, "").len(), c.len());
        assert_eq!(filter(&c, "   ").len(), c.len());
    }

    #[test]
    fn subsequence_matching_finds_actions() {
        let c = catalog(&names());
        // "undo" as a subsequence of "Edit: Undo".
        let hits = filter(&c, "undo");
        assert!(hits.iter().any(|e| e.command == Command::Undo));
        // Non-adjacent subsequence "ztf" -> "Zoom to fit".
        let z = filter(&c, "ztf");
        assert!(z.iter().any(|e| e.command == Command::ZoomToFit));
    }

    #[test]
    fn no_match_returns_empty() {
        let c = catalog(&names());
        assert!(filter(&c, "zzzzqqqq").is_empty());
    }

    #[test]
    fn matching_is_case_insensitive() {
        let c = catalog(&names());
        assert!(!filter(&c, "PAN").is_empty());
        assert!(!filter(&c, "pan").is_empty());
    }

    #[test]
    fn subsequence_span_prefers_tighter_matches() {
        // "ab" matches "axxxb" (span 5) and "ab" (span 2); tighter wins the sort.
        let entries = vec![
            CommandEntry::new("axxxb", Command::Undo),
            CommandEntry::new("ab", Command::Redo),
        ];
        let hits = filter(&entries, "ab");
        assert_eq!(hits[0].label, "ab");
    }

    #[test]
    fn export_entry_present_off_wasm() {
        let c = catalog(&names());
        let has_export = c.iter().any(|e| e.command == Command::ExportPng);
        assert_eq!(has_export, export_supported());
    }
}
