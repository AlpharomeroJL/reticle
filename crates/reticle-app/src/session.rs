//! Session save/restore and autosave of the view/UI state.
//!
//! The document itself is the built-in demo (regenerated on launch), so a session
//! only needs to persist the *view*: camera center and zoom, the active tool, grid
//! settings, and which layers are hidden. That state is serialized to a tiny,
//! dependency-free `key=value` text format (the crate deliberately pulls in no
//! serde), so it round-trips without extra dependencies and stays wasm-clean.
//!
//! Reading and writing the file is native-only (there is no filesystem on
//! `wasm32-unknown-unknown`); the serialization itself is portable and unit-tested.

use crate::camera::ViewCamera;
use crate::grid::GridSettings;
use crate::tool::Tool;
use reticle_geometry::{LayerId, Point};

/// A serializable snapshot of the app's view and UI state.
#[derive(Clone, PartialEq, Debug)]
pub struct SessionState {
    /// Camera center x, in DBU.
    pub center_x: i32,
    /// Camera center y, in DBU.
    pub center_y: i32,
    /// Zoom, in pixels per DBU.
    pub pixels_per_dbu: f64,
    /// The active tool.
    pub tool: Tool,
    /// Whether the grid is drawn.
    pub grid_visible: bool,
    /// Whether snapping is on.
    pub snap_enabled: bool,
    /// Base grid step, in DBU.
    pub grid_step: i32,
    /// The `(layer, datatype)` pairs of layers the user has hidden.
    pub hidden_layers: Vec<(u16, u16)>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            center_x: 0,
            center_y: 0,
            pixels_per_dbu: 1.0,
            tool: Tool::Select,
            grid_visible: true,
            snap_enabled: true,
            grid_step: 100,
            hidden_layers: Vec::new(),
        }
    }
}

impl SessionState {
    /// Builds a snapshot from the live camera, tool, grid, and hidden-layer set.
    #[must_use]
    pub fn capture(
        camera: &ViewCamera,
        tool: Tool,
        grid: GridSettings,
        hidden: &[LayerId],
    ) -> Self {
        let center = camera.center();
        Self {
            center_x: center.x,
            center_y: center.y,
            pixels_per_dbu: camera.pixels_per_dbu(),
            tool,
            grid_visible: grid.visible,
            snap_enabled: grid.snap_enabled,
            grid_step: grid.base_step_dbu,
            hidden_layers: hidden.iter().map(|l| (l.layer, l.datatype)).collect(),
        }
    }

    /// The camera described by this snapshot.
    #[must_use]
    pub fn camera(&self) -> ViewCamera {
        ViewCamera::new(
            Point::new(self.center_x, self.center_y),
            self.pixels_per_dbu,
        )
    }

    /// The grid settings described by this snapshot.
    #[must_use]
    pub fn grid(&self) -> GridSettings {
        GridSettings {
            base_step_dbu: self.grid_step,
            snap_enabled: self.snap_enabled,
            visible: self.grid_visible,
        }
    }

    /// The hidden layers described by this snapshot.
    #[must_use]
    pub fn hidden_layers(&self) -> Vec<LayerId> {
        self.hidden_layers
            .iter()
            .map(|&(l, d)| LayerId::new(l, d))
            .collect()
    }

    /// Serializes the snapshot to the `key=value` text format.
    #[must_use]
    pub fn to_text(&self) -> String {
        let hidden: Vec<String> = self
            .hidden_layers
            .iter()
            .map(|(l, d)| format!("{l}/{d}"))
            .collect();
        format!(
            "center_x={}\ncenter_y={}\nppd={}\ntool={}\ngrid_visible={}\nsnap={}\ngrid_step={}\nhidden={}\n",
            self.center_x,
            self.center_y,
            self.pixels_per_dbu,
            tool_tag(self.tool),
            self.grid_visible,
            self.snap_enabled,
            self.grid_step,
            hidden.join(",")
        )
    }

    /// Parses a snapshot from the `key=value` text format.
    ///
    /// Missing or malformed fields fall back to their [`Default`] values, so a
    /// partial or slightly corrupt file still restores what it can rather than
    /// failing outright. Returns `None` only if the input is empty.
    #[must_use]
    pub fn from_text(text: &str) -> Option<Self> {
        if text.trim().is_empty() {
            return None;
        }
        let mut s = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            match key {
                "center_x" => {
                    if let Ok(v) = value.parse() {
                        s.center_x = v;
                    }
                }
                "center_y" => {
                    if let Ok(v) = value.parse() {
                        s.center_y = v;
                    }
                }
                "ppd" => {
                    if let Ok(v) = value.parse() {
                        s.pixels_per_dbu = v;
                    }
                }
                "tool" => s.tool = tool_from_tag(value),
                "grid_visible" => s.grid_visible = value == "true",
                "snap" => s.snap_enabled = value == "true",
                "grid_step" => {
                    if let Ok(v) = value.parse() {
                        s.grid_step = v;
                    }
                }
                "hidden" => s.hidden_layers = parse_hidden(value),
                _ => {}
            }
        }
        Some(s)
    }
}

/// The stable text tag for a tool.
fn tool_tag(tool: Tool) -> &'static str {
    match tool {
        Tool::Select => "select",
        Tool::Pan => "pan",
        Tool::Measure => "measure",
        Tool::CutLine => "cutline",
    }
}

/// Parses a tool tag, defaulting to [`Tool::Select`] for anything unrecognized.
fn tool_from_tag(tag: &str) -> Tool {
    match tag {
        "pan" => Tool::Pan,
        "measure" => Tool::Measure,
        "cutline" => Tool::CutLine,
        _ => Tool::Select,
    }
}

/// Parses a comma-separated list of `layer/datatype` pairs, skipping malformed ones.
fn parse_hidden(value: &str) -> Vec<(u16, u16)> {
    value
        .split(',')
        .filter(|s| !s.is_empty())
        .filter_map(|pair| {
            let (l, d) = pair.split_once('/')?;
            Some((l.trim().parse().ok()?, d.trim().parse().ok()?))
        })
        .collect()
}

/// The path of the session file under the user's config directory, native only.
///
/// Returns `None` if no config directory can be determined. The file lives at
/// `<config>/reticle/session.txt`.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn session_path() -> Option<std::path::PathBuf> {
    config_dir().map(|d| d.join("reticle").join("session.txt"))
}

/// Best-effort location of a per-user config directory without extra dependencies.
///
/// Uses `APPDATA` on Windows and `XDG_CONFIG_HOME`/`HOME/.config` elsewhere.
/// Shared with [`crate::keymap`] so the keymap file lives next to the session
/// file.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn config_dir() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
        Some(PathBuf::from(x))
    } else {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
    }
}

/// Saves `state` to the session file, creating parent directories as needed.
///
/// # Errors
///
/// Returns any IO error from creating the directory or writing the file.
#[cfg(not(target_arch = "wasm32"))]
pub fn save(state: &SessionState) -> std::io::Result<()> {
    let Some(path) = session_path() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no config directory",
        ));
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, state.to_text())
}

/// Loads the session file, or `None` if it is absent or unreadable.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn load() -> Option<SessionState> {
    let path = session_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    SessionState::from_text(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SessionState {
        SessionState {
            center_x: 1234,
            center_y: -5678,
            pixels_per_dbu: 0.125,
            tool: Tool::Measure,
            grid_visible: false,
            snap_enabled: true,
            grid_step: 250,
            hidden_layers: vec![(4, 0), (5, 0)],
        }
    }

    #[test]
    fn text_round_trips() {
        let s = sample();
        let parsed = SessionState::from_text(&s.to_text()).expect("parses");
        assert_eq!(parsed, s);
    }

    #[test]
    fn capture_and_restore_camera() {
        let cam = ViewCamera::new(Point::new(700, 900), 4.0);
        let grid = GridSettings::default();
        let s = SessionState::capture(&cam, Tool::Pan, grid, &[LayerId::new(3, 0)]);
        let restored = s.camera();
        assert_eq!(restored.center(), Point::new(700, 900));
        assert!((restored.pixels_per_dbu() - 4.0).abs() < 1e-9);
        assert_eq!(s.tool, Tool::Pan);
        assert_eq!(s.hidden_layers(), vec![LayerId::new(3, 0)]);
    }

    #[test]
    fn empty_text_is_none() {
        assert!(SessionState::from_text("").is_none());
        assert!(SessionState::from_text("   \n  ").is_none());
    }

    #[test]
    fn partial_text_uses_defaults() {
        let s = SessionState::from_text("center_x=42\ntool=pan\n").expect("parses");
        assert_eq!(s.center_x, 42);
        assert_eq!(s.tool, Tool::Pan);
        // Untouched fields keep defaults.
        assert_eq!(s.center_y, 0);
        assert!(s.grid_visible);
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let s = SessionState::from_text("garbage line\ncenter_x=7\n=oops\nppd=notanumber\n")
            .expect("parses");
        assert_eq!(s.center_x, 7);
        // ppd failed to parse -> default.
        assert!((s.pixels_per_dbu - 1.0).abs() < 1e-9);
    }

    #[test]
    fn unknown_tool_tag_defaults_to_select() {
        let s = SessionState::from_text("tool=wombat\n").expect("parses");
        assert_eq!(s.tool, Tool::Select);
    }

    #[test]
    fn hidden_layers_parse_and_skip_bad_pairs() {
        let s = SessionState::from_text("hidden=1/0,2/0,bad,3/,/4\n").expect("parses");
        assert_eq!(s.hidden_layers, vec![(1, 0), (2, 0)]);
    }
}
