//! Rebindable keyboard shortcuts: a TOML keymap with conflict detection.
//!
//! Every keyboard-driven command is identified by its registry [`CommandId`]
//! (`"edit.undo"`); the [`Keymap`] maps each id to at most one [`Chord`]
//! (modifiers plus a key, like `Ctrl+Shift+Z`). The app resolves each key press
//! through [`Keymap::command_for`] and dispatches the id, so a user can rebind
//! everything from the shortcuts window and menus/palette/shortcuts all agree.
//!
//! The map is saved as TOML under the OS config directory (next to the session
//! file). Following this crate's no-serde rule, the writer emits plain TOML and
//! the reader parses the small subset the writer produces: `#` comments, a
//! `[bindings]` table, and `key = "string"` pairs. Chord key names follow egui's
//! canonical `Key::name()` strings (`A`..`Z`, `0`..`9`, `F1`.., `Escape`, ...) so
//! the glue layer can match events with a string compare.
//!
//! [`Keymap::to_toml`] writes the dotted registry ids. [`Keymap::from_toml`] also
//! accepts the 14 short legacy tags that pre-registry `keymap.toml` files used
//! (see [`LEGACY_ALIASES`]), so a saved keymap keeps loading after the migration.
//!
//! Conflicts cannot persist: binding a chord that another command holds unbinds
//! that command and reports it, and [`Keymap::from_toml`] applies the same rule to
//! hand-edited files, returning a warning per resolved conflict. The pure
//! [`Keymap::conflicts`] validator backs the unit tests.

use crate::commands::{self, CommandId};
use std::collections::BTreeMap;
use std::fmt;

/// Legacy TOML keys accepted for backward compatibility, each mapped to the
/// dotted registry id that replaced it.
///
/// `keymap.toml` files written before the command registry used these 14 short
/// tags; [`Keymap::from_toml`] resolves them through this table so those files
/// keep loading. [`Keymap::to_toml`] always writes the new dotted ids, so a saved
/// file migrates on the first save.
pub const LEGACY_ALIASES: &[(&str, &str)] = &[
    ("palette", "palette.open"),
    ("undo", "edit.undo"),
    ("redo", "edit.redo"),
    ("zoom_to_fit", "view.zoom_fit"),
    ("toggle_grid", "view.grid"),
    ("toggle_snap", "view.snap"),
    ("tool_select", "tool.select"),
    ("tool_pan", "tool.pan"),
    ("tool_measure", "tool.measure"),
    ("toggle_labels", "view.labels"),
    ("toggle_minimap", "view.minimap"),
    ("split_single", "view.split_single"),
    ("split_horizontal", "view.split_h"),
    ("split_vertical", "view.split_v"),
];

/// Resolves a TOML key (a legacy short tag or a dotted id) to the rebindable
/// registry [`CommandId`] it names, or `None` if it names no rebindable command.
///
/// The returned id borrows the registry's `'static` string, never the caller's
/// slice, so it can be stored in the map.
fn resolve_key(key: &str) -> Option<CommandId> {
    let dotted = LEGACY_ALIASES
        .iter()
        .find(|(tag, _)| *tag == key)
        .map_or(key, |(_, id)| *id);
    commands::rebindable_id(dotted)
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

/// The command-to-chord map, with at most one chord per command and (by
/// construction) at most one command per chord.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Keymap {
    /// The current bindings; an absent command id is unbound.
    bindings: BTreeMap<CommandId, Chord>,
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
    /// The built-in bindings, taken straight from the registry: every rebindable
    /// [`CommandSpec`](crate::commands::CommandSpec) with a `default_chord`. The
    /// registry is the single source of truth, so `view.snap` (no default) ships
    /// unbound.
    #[must_use]
    pub fn defaults() -> Self {
        let mut bindings = BTreeMap::new();
        for spec in commands::registry() {
            if spec.rebindable
                && let Some(text) = spec.default_chord
            {
                bindings.insert(spec.id, chord(text));
            }
        }
        Self { bindings }
    }

    /// The chord bound to `id`, if any.
    #[must_use]
    pub fn chord_for(&self, id: CommandId) -> Option<&Chord> {
        self.bindings.get(&id)
    }

    /// The command bound to `chord`, if any.
    #[must_use]
    pub fn command_for(&self, chord: &Chord) -> Option<CommandId> {
        self.bindings
            .iter()
            .find(|(_, c)| *c == chord)
            .map(|(id, _)| *id)
    }

    /// The number of bound commands.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Whether no command is bound.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Binds `id` to `chord` (or unbinds it with `None`).
    ///
    /// Any *other* command holding the same chord is unbound first; the returned
    /// list names those commands so the caller can report the takeover. The
    /// invariant that a chord maps to at most one command therefore always holds.
    pub fn bind(&mut self, id: CommandId, chord: Option<Chord>) -> Vec<CommandId> {
        let mut stolen = Vec::new();
        if let Some(c) = &chord {
            stolen = self
                .bindings
                .iter()
                .filter(|(other, b)| **other != id && *b == c)
                .map(|(other, _)| *other)
                .collect();
            for other in &stolen {
                self.bindings.remove(other);
            }
        }
        match chord {
            Some(c) => {
                self.bindings.insert(id, c);
            }
            None => {
                self.bindings.remove(&id);
            }
        }
        stolen
    }

    /// Every pair of commands sharing a chord (empty when the invariant holds).
    ///
    /// [`Keymap::bind`] and [`Keymap::from_toml`] never produce conflicts; this
    /// validator exists so that fact is testable rather than assumed.
    #[must_use]
    pub fn conflicts(&self) -> Vec<(CommandId, CommandId)> {
        let entries: Vec<(&CommandId, &Chord)> = self.bindings.iter().collect();
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

    /// Serializes the keymap as TOML, one line per rebindable registry command in
    /// registry order; an unbound command serializes as an empty string. Ids are
    /// the dotted registry ids, so a keymap loaded from legacy tags migrates on
    /// the first save.
    #[must_use]
    pub fn to_toml(&self) -> String {
        use fmt::Write as _;
        let mut out = String::from(
            "# Reticle keymap: commands bound to chords like \"Ctrl+Shift+Z\".\n\
             # An empty string leaves the command unbound.\n\n[bindings]\n",
        );
        for spec in commands::registry().iter().filter(|s| s.rebindable) {
            let value = self
                .chord_for(spec.id)
                .map(ToString::to_string)
                .unwrap_or_default();
            let _ = writeln!(out, "{} = \"{value}\"", spec.id.0);
        }
        out
    }

    /// Parses a keymap from the TOML subset [`Keymap::to_toml`] writes, starting
    /// from the defaults and applying each entry over them.
    ///
    /// Keys may be dotted registry ids or the short [`LEGACY_ALIASES`] tags. The
    /// parser is deliberately tolerant so a hand-edited file degrades gracefully:
    /// unknown commands, malformed lines, unquoted values, and unparsable chords
    /// each produce a warning and leave that command's default in place. An empty
    /// string unbinds. When two entries claim the same chord the later one wins
    /// and the earlier command is unbound with a warning, so a loaded map never
    /// carries a conflict. Keys before any `[section]` header are treated as
    /// bindings; keys under a foreign section are ignored.
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
            let Some(id) = resolve_key(k) else {
                warnings.push(format!("unknown command `{k}`"));
                continue;
            };
            let Some(value) = quoted_value(v) else {
                warnings.push(format!("expected a quoted chord for `{k}`"));
                continue;
            };
            if value.is_empty() {
                map.bind(id, None);
                continue;
            }
            match Chord::parse(&value) {
                Some(c) => {
                    for loser in map.bind(id, Some(c)) {
                        warnings.push(format!(
                            "`{}` and `{}` shared a chord; `{}` is now unbound",
                            loser.0, id.0, loser.0
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

    /// The dotted ids referenced throughout these tests, as `CommandId`s.
    fn id(s: &'static str) -> CommandId {
        CommandId(s)
    }

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
    fn defaults_match_the_shortcut_regression_list() {
        // The exact defaults ia-inventory section 1 pins; the 1E regression test.
        let m = Keymap::defaults();
        let expect = [
            ("palette.open", "Ctrl+P"),
            ("edit.undo", "Ctrl+Z"),
            ("edit.redo", "Ctrl+Y"),
            ("view.zoom_fit", "F"),
            ("view.grid", "Ctrl+G"),
            ("tool.select", "V"),
            ("tool.pan", "S"),
            ("tool.measure", "M"),
            ("view.labels", "L"),
            ("view.minimap", "N"),
            ("view.split_single", "Ctrl+1"),
            ("view.split_h", "Ctrl+2"),
            ("view.split_v", "Ctrl+3"),
            // Reserved new default from ia-inventory section 4 (lane 3b, file.open_dialog).
            ("file.open_dialog", "Ctrl+O"),
        ];
        for (command, text) in expect {
            assert_eq!(
                m.chord_for(id(command)),
                Chord::parse(text).as_ref(),
                "{command} must default to {text}"
            );
        }
        assert_eq!(m.chord_for(id("view.snap")), None, "snap ships unbound");
        assert_eq!(m.len(), expect.len(), "no extra default bindings");
        assert!(m.conflicts().is_empty(), "defaults must not conflict");
    }

    #[test]
    fn command_lookup_respects_modifiers_exactly() {
        let m = Keymap::defaults();
        let undo = Chord::parse("Ctrl+Z").expect("parses");
        assert_eq!(m.command_for(&undo), Some(id("edit.undo")));
        let shifted = Chord::parse("Ctrl+Shift+Z").expect("parses");
        assert_eq!(
            m.command_for(&shifted),
            None,
            "extra modifier must not match"
        );
    }

    #[test]
    fn bind_steals_a_conflicting_chord_and_reports_it() {
        let mut m = Keymap::defaults();
        // Give redo the undo chord: undo must lose it and be reported.
        let stolen = m.bind(id("edit.redo"), Chord::parse("Ctrl+Z"));
        assert_eq!(stolen, vec![id("edit.undo")]);
        assert_eq!(m.chord_for(id("edit.undo")), None);
        assert_eq!(
            m.command_for(&Chord::parse("Ctrl+Z").expect("parses")),
            Some(id("edit.redo"))
        );
        assert!(m.conflicts().is_empty());
    }

    #[test]
    fn bind_none_unbinds() {
        let mut m = Keymap::defaults();
        let stolen = m.bind(id("edit.undo"), None);
        assert!(stolen.is_empty());
        assert_eq!(m.chord_for(id("edit.undo")), None);
        assert_eq!(m.command_for(&Chord::parse("Ctrl+Z").expect("ok")), None);
    }

    #[test]
    fn rebinding_the_same_command_is_not_a_conflict() {
        let mut m = Keymap::defaults();
        let stolen = m.bind(id("edit.undo"), Chord::parse("Ctrl+U"));
        assert!(stolen.is_empty());
        assert_eq!(
            m.chord_for(id("edit.undo")),
            Chord::parse("Ctrl+U").as_ref()
        );
    }

    #[test]
    fn toml_round_trips_including_unbound_commands() {
        let mut m = Keymap::defaults();
        m.bind(id("edit.undo"), Chord::parse("Ctrl+Shift+U"));
        m.bind(id("tool.pan"), None);
        let text = m.to_toml();
        let (parsed, warnings) = Keymap::from_toml(&text);
        assert_eq!(parsed, m);
        assert!(
            warnings.is_empty(),
            "own output must parse clean: {warnings:?}"
        );
    }

    #[test]
    fn to_toml_writes_dotted_ids_not_legacy_tags() {
        let saved = Keymap::defaults().to_toml();
        assert!(saved.contains("edit.undo = \"Ctrl+Z\""));
        for line in saved.lines().filter(|l| l.contains(" = ")) {
            let key = line.split(" = ").next().expect("key");
            assert!(key.contains('.'), "saved key `{key}` should be a dotted id");
        }
    }

    #[test]
    fn from_toml_applies_over_defaults_and_warns_on_junk() {
        let text = "# comment\n[bindings]\nedit.undo = \"Ctrl+U\"\nwombat = \"Ctrl+W\"\nedit.redo = \"NotAKey\"\ntool.pan = not quoted\n";
        let (m, warnings) = Keymap::from_toml(text);
        assert_eq!(
            m.chord_for(id("edit.undo")),
            Chord::parse("Ctrl+U").as_ref()
        );
        // Bad entries keep their defaults.
        assert_eq!(
            m.chord_for(id("edit.redo")),
            Chord::parse("Ctrl+Y").as_ref()
        );
        assert_eq!(m.chord_for(id("tool.pan")), Chord::parse("S").as_ref());
        assert_eq!(warnings.len(), 3, "unknown command, bad chord, unquoted");
    }

    #[test]
    fn from_toml_resolves_file_conflicts_with_a_warning() {
        // Both commands claim Ctrl+Q; the later entry wins.
        let text = "[bindings]\nedit.undo = \"Ctrl+Q\"\nedit.redo = \"Ctrl+Q\"\n";
        let (m, warnings) = Keymap::from_toml(text);
        assert_eq!(
            m.command_for(&Chord::parse("Ctrl+Q").expect("ok")),
            Some(id("edit.redo"))
        );
        assert_eq!(m.chord_for(id("edit.undo")), None);
        assert!(warnings.iter().any(|w| w.contains("edit.undo")));
        assert!(m.conflicts().is_empty());
    }

    #[test]
    fn from_toml_unbinds_on_empty_string() {
        let (m, warnings) = Keymap::from_toml("[bindings]\nview.zoom_fit = \"\"\n");
        assert_eq!(m.chord_for(id("view.zoom_fit")), None);
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
        let text = "[bindings]\nedit.undo = \"Ctrl+U\"\n[colors]\nedit.undo = \"Ctrl+J\"\n";
        let (m, _) = Keymap::from_toml(text);
        assert_eq!(
            m.chord_for(id("edit.undo")),
            Chord::parse("Ctrl+U").as_ref()
        );
    }

    #[test]
    fn legacy_tags_load_via_aliases_and_save_as_dotted_ids() {
        // A keymap.toml written before the registry, using the 14 short tags,
        // with one command rebound and snap left unbound as it shipped.
        let legacy = "\
# Reticle keymap (pre-registry)
[bindings]
palette = \"Ctrl+P\"
undo = \"Ctrl+Z\"
redo = \"Ctrl+Y\"
zoom_to_fit = \"F\"
toggle_grid = \"Ctrl+G\"
toggle_snap = \"\"
tool_select = \"V\"
tool_pan = \"Shift+P\"
tool_measure = \"M\"
toggle_labels = \"L\"
toggle_minimap = \"N\"
split_single = \"Ctrl+1\"
split_horizontal = \"Ctrl+2\"
split_vertical = \"Ctrl+3\"
";
        let (m, warnings) = Keymap::from_toml(legacy);
        assert!(
            warnings.is_empty(),
            "legacy tags must load clean: {warnings:?}"
        );
        // Loaded under the new dotted ids, chords preserved (including the rebind).
        assert_eq!(
            m.chord_for(id("edit.undo")),
            Chord::parse("Ctrl+Z").as_ref()
        );
        assert_eq!(
            m.chord_for(id("tool.pan")),
            Chord::parse("Shift+P").as_ref()
        );
        assert_eq!(
            m.chord_for(id("view.split_h")),
            Chord::parse("Ctrl+2").as_ref()
        );
        assert_eq!(m.chord_for(id("view.snap")), None, "snap stayed unbound");
        // Saving migrates to dotted ids; reloading the saved text is identical.
        let saved = m.to_toml();
        assert!(saved.contains("edit.undo = \"Ctrl+Z\""));
        assert!(!saved.contains("\nundo = "), "no legacy tag written back");
        let (round, w2) = Keymap::from_toml(&saved);
        assert!(w2.is_empty(), "migrated file parses clean: {w2:?}");
        assert_eq!(round, m, "legacy load then save round-trips");
    }
}
