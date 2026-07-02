//! Rebindable keyboard shortcuts: a TOML keymap with conflict detection.
//!
//! Every keyboard-driven action is an [`Action`]; the [`Keymap`] maps each one to
//! at most one [`Chord`] (modifiers plus a key, like `Ctrl+Shift+Z`). The app
//! resolves each key press through [`Keymap::action_for`] instead of hardcoded
//! `if` chains, so a user can rebind everything from the shortcuts window.
//!
//! The map is saved as TOML under the OS config directory (next to the session
//! file). Following this crate's no-serde rule, the writer emits plain TOML and
//! the reader parses the small subset the writer produces: `#` comments, a
//! `[bindings]` table, and `key = "string"` pairs. Chord key names follow egui's
//! canonical `Key::name()` strings (`A`..`Z`, `0`..`9`, `F1`.., `Escape`, ...) so
//! the glue layer can match events with a string compare.
//!
//! Conflicts cannot persist: binding a chord that another action holds unbinds
//! that action and reports it, and [`Keymap::from_toml`] applies the same rule to
//! hand-edited files, returning a warning per resolved conflict. The pure
//! [`Keymap::conflicts`] validator backs the unit tests.

use std::collections::BTreeMap;
use std::fmt;

/// Every rebindable action in the app, in display (and serialization) order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Action {
    /// Toggle the command palette.
    OpenPalette,
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
    /// Switch to the Select tool.
    ToolSelect,
    /// Switch to the Pan tool.
    ToolPan,
    /// Switch to the Measure tool.
    ToolMeasure,
    /// Toggle the canvas text-label overlay.
    ToggleLabels,
    /// Toggle the minimap overview panel.
    ToggleMinimap,
    /// Collapse the canvas to a single pane.
    SplitSingle,
    /// Split the canvas into two side-by-side panes.
    SplitHorizontal,
    /// Split the canvas into two stacked panes.
    SplitVertical,
}

impl Action {
    /// Every action, in the order the editor lists and the file serializes them.
    #[must_use]
    pub fn all() -> [Action; 14] {
        [
            Action::OpenPalette,
            Action::Undo,
            Action::Redo,
            Action::ZoomToFit,
            Action::ToggleGrid,
            Action::ToggleSnap,
            Action::ToolSelect,
            Action::ToolPan,
            Action::ToolMeasure,
            Action::ToggleLabels,
            Action::ToggleMinimap,
            Action::SplitSingle,
            Action::SplitHorizontal,
            Action::SplitVertical,
        ]
    }

    /// The stable TOML key for this action.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::OpenPalette => "palette",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::ZoomToFit => "zoom_to_fit",
            Self::ToggleGrid => "toggle_grid",
            Self::ToggleSnap => "toggle_snap",
            Self::ToolSelect => "tool_select",
            Self::ToolPan => "tool_pan",
            Self::ToolMeasure => "tool_measure",
            Self::ToggleLabels => "toggle_labels",
            Self::ToggleMinimap => "toggle_minimap",
            Self::SplitSingle => "split_single",
            Self::SplitHorizontal => "split_horizontal",
            Self::SplitVertical => "split_vertical",
        }
    }

    /// The human-readable label shown in the shortcuts editor.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenPalette => "Command palette",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::ZoomToFit => "Zoom to fit",
            Self::ToggleGrid => "Toggle grid",
            Self::ToggleSnap => "Toggle snapping",
            Self::ToolSelect => "Tool: Select",
            Self::ToolPan => "Tool: Pan",
            Self::ToolMeasure => "Tool: Measure",
            Self::ToggleLabels => "Toggle labels",
            Self::ToggleMinimap => "Toggle minimap",
            Self::SplitSingle => "View: single pane",
            Self::SplitHorizontal => "View: split horizontal",
            Self::SplitVertical => "View: split vertical",
        }
    }

    /// Parses a TOML key back to an action.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Action> {
        Action::all().into_iter().find(|a| a.tag() == tag)
    }
}

/// Key names accepted beyond single letters and digits; these match egui's
/// canonical `Key::name()` strings, so chord lookup is a string compare.
const NAMED_KEYS: &[&str] = &[
    "Escape",
    "Tab",
    "Backspace",
    "Enter",
    "Space",
    "Insert",
    "Delete",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Up",
    "Down",
    "Left",
    "Right",
    "Minus",
    "Plus",
    "Equals",
    "Comma",
    "Period",
    "Slash",
    "Backslash",
    "Semicolon",
    "Colon",
    "OpenBracket",
    "CloseBracket",
    "Backtick",
    "Quote",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
];

/// The canonical key name for a chord token, or `None` if unrecognized.
///
/// Single alphanumeric characters canonicalize to uppercase (`z` -> `Z`); longer
/// tokens must match a [`NAMED_KEYS`] entry case-insensitively.
fn canonical_key(token: &str) -> Option<String> {
    let t = token.trim();
    let mut chars = t.chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && c.is_ascii_alphanumeric()
    {
        return Some(c.to_ascii_uppercase().to_string());
    }
    NAMED_KEYS
        .iter()
        .find(|k| k.eq_ignore_ascii_case(t))
        .map(|k| (*k).to_owned())
}

/// A keyboard chord: modifier flags plus one key, e.g. `Ctrl+Shift+Z`.
///
/// `key` holds the canonical egui key name. Equality is exact on all four
/// fields, so `Ctrl+Z` and `Ctrl+Shift+Z` are different chords.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Chord {
    /// Whether Ctrl (or the platform command key) is held.
    pub ctrl: bool,
    /// Whether Shift is held.
    pub shift: bool,
    /// Whether Alt is held.
    pub alt: bool,
    /// The canonical key name (`Z`, `F1`, `Escape`, ...).
    pub key: String,
}

impl Chord {
    /// Parses a chord like `Ctrl+Shift+Z`, case-insensitively.
    ///
    /// Modifier tokens are `Ctrl`/`Control`/`Cmd`/`Command`, `Shift`, and
    /// `Alt`/`Option`; exactly one non-modifier token must remain and it must
    /// canonicalize to a known key. Returns `None` for anything else (empty
    /// text, dangling `+`, two keys, or an unknown key name).
    #[must_use]
    pub fn parse(text: &str) -> Option<Chord> {
        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut key: Option<String> = None;
        for token in text.split('+') {
            let t = token.trim();
            if t.is_empty() {
                return None;
            }
            match t.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" | "command" => ctrl = true,
                "shift" => shift = true,
                "alt" | "option" => alt = true,
                _ => {
                    if key.is_some() {
                        return None;
                    }
                    key = Some(canonical_key(t)?);
                }
            }
        }
        Some(Chord {
            ctrl,
            shift,
            alt,
            key: key?,
        })
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            write!(f, "Ctrl+")?;
        }
        if self.shift {
            write!(f, "Shift+")?;
        }
        if self.alt {
            write!(f, "Alt+")?;
        }
        write!(f, "{}", self.key)
    }
}

/// The action-to-chord map, with at most one chord per action and (by
/// construction) at most one action per chord.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Keymap {
    /// The current bindings; an absent action is unbound.
    bindings: BTreeMap<Action, Chord>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::defaults()
    }
}

/// A convenience for building the default table.
fn chord(text: &str) -> Chord {
    Chord::parse(text).expect("default chords are valid")
}

impl Keymap {
    /// The built-in bindings, matching the app's historical shortcuts plus the
    /// newer overlay and split actions. `toggle_snap` ships unbound.
    #[must_use]
    pub fn defaults() -> Self {
        let mut bindings = BTreeMap::new();
        bindings.insert(Action::OpenPalette, chord("Ctrl+P"));
        bindings.insert(Action::Undo, chord("Ctrl+Z"));
        bindings.insert(Action::Redo, chord("Ctrl+Y"));
        bindings.insert(Action::ZoomToFit, chord("F"));
        bindings.insert(Action::ToggleGrid, chord("Ctrl+G"));
        bindings.insert(Action::ToolSelect, chord("V"));
        bindings.insert(Action::ToolPan, chord("S"));
        bindings.insert(Action::ToolMeasure, chord("M"));
        bindings.insert(Action::ToggleLabels, chord("L"));
        bindings.insert(Action::ToggleMinimap, chord("N"));
        bindings.insert(Action::SplitSingle, chord("Ctrl+1"));
        bindings.insert(Action::SplitHorizontal, chord("Ctrl+2"));
        bindings.insert(Action::SplitVertical, chord("Ctrl+3"));
        Self { bindings }
    }

    /// The chord bound to `action`, if any.
    #[must_use]
    pub fn chord_for(&self, action: Action) -> Option<&Chord> {
        self.bindings.get(&action)
    }

    /// The action bound to `chord`, if any.
    #[must_use]
    pub fn action_for(&self, chord: &Chord) -> Option<Action> {
        self.bindings
            .iter()
            .find(|(_, c)| *c == chord)
            .map(|(a, _)| *a)
    }

    /// Binds `action` to `chord` (or unbinds it with `None`).
    ///
    /// Any *other* action holding the same chord is unbound first; the returned
    /// list names those actions so the caller can report the takeover. The
    /// invariant that a chord maps to at most one action therefore always holds.
    pub fn bind(&mut self, action: Action, chord: Option<Chord>) -> Vec<Action> {
        let mut stolen = Vec::new();
        if let Some(c) = &chord {
            stolen = self
                .bindings
                .iter()
                .filter(|(a, b)| **a != action && *b == c)
                .map(|(a, _)| *a)
                .collect();
            for a in &stolen {
                self.bindings.remove(a);
            }
        }
        match chord {
            Some(c) => {
                self.bindings.insert(action, c);
            }
            None => {
                self.bindings.remove(&action);
            }
        }
        stolen
    }

    /// Every pair of actions sharing a chord (empty when the invariant holds).
    ///
    /// [`Keymap::bind`] and [`Keymap::from_toml`] never produce conflicts; this
    /// validator exists so that fact is testable rather than assumed.
    #[must_use]
    pub fn conflicts(&self) -> Vec<(Action, Action)> {
        let entries: Vec<(&Action, &Chord)> = self.bindings.iter().collect();
        let mut out = Vec::new();
        for (i, (a, ca)) in entries.iter().enumerate() {
            for (b, cb) in entries.iter().skip(i + 1) {
                if ca == cb {
                    out.push((**a, **b));
                }
            }
        }
        out
    }

    /// Serializes the keymap as TOML, one line per action in [`Action::all`]
    /// order; unbound actions serialize as an empty string.
    #[must_use]
    pub fn to_toml(&self) -> String {
        use fmt::Write as _;
        let mut out = String::from(
            "# Reticle keymap: actions bound to chords like \"Ctrl+Shift+Z\".\n\
             # An empty string leaves the action unbound.\n\n[bindings]\n",
        );
        for action in Action::all() {
            let value = self
                .chord_for(action)
                .map(ToString::to_string)
                .unwrap_or_default();
            let _ = writeln!(out, "{} = \"{value}\"", action.tag());
        }
        out
    }

    /// Parses a keymap from the TOML subset [`Keymap::to_toml`] writes, starting
    /// from the defaults and applying each entry over them.
    ///
    /// The parser is deliberately tolerant so a hand-edited file degrades
    /// gracefully: unknown actions, malformed lines, unquoted values, and
    /// unparsable chords each produce a warning and leave that action's default
    /// in place. An empty string unbinds. When two entries claim the same chord
    /// the later one wins and the earlier action is unbound with a warning, so a
    /// loaded map never carries a conflict. Keys before any `[section]` header
    /// are treated as bindings; keys under a foreign section are ignored.
    #[must_use]
    pub fn from_toml(text: &str) -> (Self, Vec<String>) {
        let mut map = Self::defaults();
        let mut warnings = Vec::new();
        let mut in_bindings = true;
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                in_bindings = line == "[bindings]";
                continue;
            }
            if !in_bindings {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                warnings.push(format!("ignored malformed line `{line}`"));
                continue;
            };
            let k = k.trim();
            let Some(action) = Action::from_tag(k) else {
                warnings.push(format!("unknown action `{k}`"));
                continue;
            };
            let Some(value) = quoted_value(v) else {
                warnings.push(format!("expected a quoted chord for `{k}`"));
                continue;
            };
            if value.is_empty() {
                map.bind(action, None);
                continue;
            }
            match Chord::parse(&value) {
                Some(c) => {
                    for loser in map.bind(action, Some(c)) {
                        warnings.push(format!(
                            "`{}` and `{}` shared a chord; `{}` is now unbound",
                            loser.tag(),
                            action.tag(),
                            loser.tag()
                        ));
                    }
                }
                None => warnings.push(format!("invalid chord `{value}` for `{k}`")),
            }
        }
        (map, warnings)
    }
}

/// Extracts the contents of the first double-quoted string in `v`, ignoring
/// anything after the closing quote (such as a trailing comment).
fn quoted_value(v: &str) -> Option<String> {
    let v = v.trim();
    let rest = v.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

/// The path of the keymap file under the user's config directory, native only.
///
/// Lives next to the session file at `<config>/reticle/keymap.toml`; returns
/// `None` if no config directory can be determined.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn keymap_path() -> Option<std::path::PathBuf> {
    crate::session::config_dir().map(|d| d.join("reticle").join("keymap.toml"))
}

/// Saves `map` to the keymap file, creating parent directories as needed.
///
/// # Errors
///
/// Returns any IO error from creating the directory or writing the file.
#[cfg(not(target_arch = "wasm32"))]
pub fn save(map: &Keymap) -> std::io::Result<()> {
    let Some(path) = keymap_path() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no config directory",
        ));
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, map.to_toml())
}

/// Loads the keymap file, or `None` if it is absent or unreadable.
///
/// Returns the parsed map plus any warnings from tolerant parsing, so the app
/// can surface how much of a hand-edited file was honored.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn load() -> Option<(Keymap, Vec<String>)> {
    let path = keymap_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    Some(Keymap::from_toml(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_chord_with_modifiers() {
        let c = Chord::parse("Ctrl+Shift+Z").expect("parses");
        assert!(c.ctrl && c.shift && !c.alt);
        assert_eq!(c.key, "Z");
    }

    #[test]
    fn parse_is_case_insensitive_and_canonicalizes() {
        assert_eq!(
            Chord::parse("ctrl+shift+z"),
            Chord::parse("CTRL+SHIFT+Z"),
            "case must not matter"
        );
        assert_eq!(Chord::parse("escape").expect("named key").key, "Escape");
        assert_eq!(Chord::parse("f1").expect("f-key").key, "F1");
        // Command/option are aliases for ctrl/alt.
        let mac = Chord::parse("Cmd+Option+A").expect("aliases");
        assert!(mac.ctrl && mac.alt);
    }

    #[test]
    fn parse_rejects_garbage() {
        assert_eq!(Chord::parse(""), None);
        assert_eq!(Chord::parse("Ctrl+"), None, "dangling plus");
        assert_eq!(Chord::parse("Ctrl+Shift"), None, "no key");
        assert_eq!(Chord::parse("A+B"), None, "two keys");
        assert_eq!(Chord::parse("Ctrl+Wombat"), None, "unknown key");
    }

    #[test]
    fn display_round_trips_canonically() {
        for text in ["Ctrl+Shift+Z", "F", "Alt+F4", "Ctrl+1", "Space"] {
            let c = Chord::parse(text).expect("parses");
            assert_eq!(c.to_string(), text);
            assert_eq!(Chord::parse(&c.to_string()).as_ref(), Some(&c));
        }
        // Non-canonical input prints canonically.
        let c = Chord::parse("shift+ctrl+q").expect("parses");
        assert_eq!(c.to_string(), "Ctrl+Shift+Q");
    }

    #[test]
    fn defaults_cover_the_historical_shortcuts() {
        let m = Keymap::defaults();
        assert_eq!(m.chord_for(Action::Undo), Chord::parse("Ctrl+Z").as_ref());
        assert_eq!(
            m.chord_for(Action::OpenPalette),
            Chord::parse("Ctrl+P").as_ref()
        );
        assert_eq!(m.chord_for(Action::ZoomToFit), Chord::parse("F").as_ref());
        assert_eq!(m.chord_for(Action::ToolMeasure), Chord::parse("M").as_ref());
        assert_eq!(m.chord_for(Action::ToggleSnap), None, "snap ships unbound");
        assert!(m.conflicts().is_empty(), "defaults must not conflict");
    }

    #[test]
    fn action_lookup_respects_modifiers_exactly() {
        let m = Keymap::defaults();
        let undo = Chord::parse("Ctrl+Z").expect("parses");
        assert_eq!(m.action_for(&undo), Some(Action::Undo));
        let shifted = Chord::parse("Ctrl+Shift+Z").expect("parses");
        assert_eq!(
            m.action_for(&shifted),
            None,
            "extra modifier must not match"
        );
    }

    #[test]
    fn bind_steals_a_conflicting_chord_and_reports_it() {
        let mut m = Keymap::defaults();
        // Give Redo the Undo chord: Undo must lose it and be reported.
        let stolen = m.bind(Action::Redo, Chord::parse("Ctrl+Z"));
        assert_eq!(stolen, vec![Action::Undo]);
        assert_eq!(m.chord_for(Action::Undo), None);
        assert_eq!(
            m.action_for(&Chord::parse("Ctrl+Z").expect("parses")),
            Some(Action::Redo)
        );
        assert!(m.conflicts().is_empty());
    }

    #[test]
    fn bind_none_unbinds() {
        let mut m = Keymap::defaults();
        let stolen = m.bind(Action::Undo, None);
        assert!(stolen.is_empty());
        assert_eq!(m.chord_for(Action::Undo), None);
        assert_eq!(m.action_for(&Chord::parse("Ctrl+Z").expect("ok")), None);
    }

    #[test]
    fn rebinding_the_same_action_is_not_a_conflict() {
        let mut m = Keymap::defaults();
        let stolen = m.bind(Action::Undo, Chord::parse("Ctrl+U"));
        assert!(stolen.is_empty());
        assert_eq!(m.chord_for(Action::Undo), Chord::parse("Ctrl+U").as_ref());
    }

    #[test]
    fn toml_round_trips_including_unbound_actions() {
        let mut m = Keymap::defaults();
        m.bind(Action::Undo, Chord::parse("Ctrl+Shift+U"));
        m.bind(Action::ToolPan, None);
        let text = m.to_toml();
        let (parsed, warnings) = Keymap::from_toml(&text);
        assert_eq!(parsed, m);
        assert!(
            warnings.is_empty(),
            "own output must parse clean: {warnings:?}"
        );
    }

    #[test]
    fn from_toml_applies_over_defaults_and_warns_on_junk() {
        let text = "# comment\n[bindings]\nundo = \"Ctrl+U\"\nwombat = \"Ctrl+W\"\nredo = \"NotAKey\"\ntool_pan = not quoted\n";
        let (m, warnings) = Keymap::from_toml(text);
        assert_eq!(m.chord_for(Action::Undo), Chord::parse("Ctrl+U").as_ref());
        // Bad entries keep their defaults.
        assert_eq!(m.chord_for(Action::Redo), Chord::parse("Ctrl+Y").as_ref());
        assert_eq!(m.chord_for(Action::ToolPan), Chord::parse("S").as_ref());
        assert_eq!(warnings.len(), 3, "unknown action, bad chord, unquoted");
    }

    #[test]
    fn from_toml_resolves_file_conflicts_with_a_warning() {
        // Both actions claim Ctrl+Q; the later entry wins.
        let text = "[bindings]\nundo = \"Ctrl+Q\"\nredo = \"Ctrl+Q\"\n";
        let (m, warnings) = Keymap::from_toml(text);
        assert_eq!(
            m.action_for(&Chord::parse("Ctrl+Q").expect("ok")),
            Some(Action::Redo)
        );
        assert_eq!(m.chord_for(Action::Undo), None);
        assert!(warnings.iter().any(|w| w.contains("undo")));
        assert!(m.conflicts().is_empty());
    }

    #[test]
    fn from_toml_unbinds_on_empty_string() {
        let (m, warnings) = Keymap::from_toml("[bindings]\nzoom_to_fit = \"\"\n");
        assert_eq!(m.chord_for(Action::ZoomToFit), None);
        assert!(warnings.is_empty());
    }

    #[test]
    fn from_toml_of_empty_text_is_the_defaults() {
        let (m, warnings) = Keymap::from_toml("");
        assert_eq!(m, Keymap::defaults());
        assert!(warnings.is_empty());
    }

    #[test]
    fn foreign_sections_are_ignored() {
        let text = "[bindings]\nundo = \"Ctrl+U\"\n[colors]\nundo = \"Ctrl+J\"\n";
        let (m, _) = Keymap::from_toml(text);
        assert_eq!(m.chord_for(Action::Undo), Chord::parse("Ctrl+U").as_ref());
    }

    #[test]
    fn every_action_has_a_unique_tag_that_round_trips() {
        let mut seen = std::collections::HashSet::new();
        for action in Action::all() {
            assert!(seen.insert(action.tag()), "duplicate tag {}", action.tag());
            assert_eq!(Action::from_tag(action.tag()), Some(action));
            assert!(!action.label().is_empty());
        }
        assert_eq!(Action::from_tag("nonsense"), None);
    }
}
