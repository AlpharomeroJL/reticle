//! The right Inspector's group model and remembered panel state (lane 2B).
//!
//! ADR-2B: the right `Panel::right` is a managed panel (ADR 0096), not a docked
//! one. Its sections are grouped into four modes chosen by a segmented control
//! (Figma's Inspector discipline: one panel, grouped modes, sections that appear
//! only when they have meaning). Exactly one [`PanelGroup`] is visible at a time;
//! within it every section is a token-styled [`crate::theme::components::Collapsible`]
//! with a remembered-open flag.
//!
//! This module owns the pure, testable parts: the group enum, the static section
//! registry ([`SECTIONS`], ordered per `docs/design/ia-inventory.md` section 2),
//! and [`InspectorState`] (the selected group, the persisted width, the
//! icon-rail collapse flag, and the per-section open flags). The rendering lives
//! in [`crate::app`], which reads this state; the persistence round-trips through
//! [`crate::session`].

use std::collections::BTreeMap;

use crate::theme::icons;

/// The default width, in points, of the expanded Inspector panel.
pub const DEFAULT_WIDTH: f32 = 264.0;
/// The minimum width the Inspector may be dragged to.
pub const MIN_WIDTH: f32 = 200.0;
/// The maximum width the Inspector may be dragged to.
pub const MAX_WIDTH: f32 = 520.0;
/// The width, in points, of the collapsed icon rail.
pub const RAIL_WIDTH: f32 = 40.0;

/// One of the four Inspector modes the segmented control chooses between.
///
/// The order is the segmented-control order and the tab order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelGroup {
    /// Selection properties, search and outline, and undo history.
    Inspect,
    /// DRC, layout diff, and comments: the review surfaces.
    Review,
    /// The agent and the parametric generator.
    Automate,
    /// Operations, productivity, snapping, export, and the technology editor.
    Settings,
}

impl PanelGroup {
    /// The four groups in segmented-control order.
    pub const ALL: [PanelGroup; 4] = [
        PanelGroup::Inspect,
        PanelGroup::Review,
        PanelGroup::Automate,
        PanelGroup::Settings,
    ];

    /// The label shown on the segmented control and the icon-rail tooltip.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            PanelGroup::Inspect => "Inspect",
            PanelGroup::Review => "Review",
            PanelGroup::Automate => "Automate",
            PanelGroup::Settings => "Settings",
        }
    }

    /// The Lucide glyph that keys this group in the collapsed icon rail.
    #[must_use]
    pub fn icon(self) -> char {
        match self {
            PanelGroup::Inspect => icons::INSPECT,
            PanelGroup::Review => icons::SHIELD_CHECK,
            PanelGroup::Automate => icons::BOT,
            // Sliders, not the gear: this group holds Operations, Productivity,
            // Snap, Export, and the tech editor (adjustments), and the gear glyph
            // is reserved for the per-panel options menu beside these tabs.
            PanelGroup::Settings => icons::SLIDERS_HORIZONTAL,
        }
    }

    /// The index of this group in [`PanelGroup::ALL`] (the persisted segment id).
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            PanelGroup::Inspect => 0,
            PanelGroup::Review => 1,
            PanelGroup::Automate => 2,
            PanelGroup::Settings => 3,
        }
    }

    /// The group at `index`, clamped into range (an out-of-range persisted value
    /// resolves to [`PanelGroup::Inspect`] rather than failing).
    #[must_use]
    pub fn from_index(index: usize) -> PanelGroup {
        PanelGroup::ALL
            .get(index)
            .copied()
            .unwrap_or(PanelGroup::Inspect)
    }
}

/// A single Inspector section: its stable key (used for the open flag and
/// persistence), its collapsible title, the group it lives under, and whether it
/// starts open.
///
/// Low-frequency sections start collapsed; the one high-frequency section per
/// group starts open so a group is useful the instant it is selected.
#[derive(Clone, Copy, Debug)]
pub struct SectionSpec {
    /// The stable key (persisted; never shown to the user).
    pub key: &'static str,
    /// The collapsible header title.
    pub title: &'static str,
    /// The group this section belongs to.
    pub group: PanelGroup,
    /// Whether the section starts open on a fresh install.
    pub default_open: bool,
}

/// The Inspector sections in render order, grouped per `ia-inventory.md` section 2.
pub const SECTIONS: &[SectionSpec] = &[
    // Inspect
    SectionSpec {
        key: "properties",
        title: "Properties",
        group: PanelGroup::Inspect,
        default_open: true,
    },
    SectionSpec {
        key: "search",
        title: "Search and outline",
        group: PanelGroup::Inspect,
        default_open: false,
    },
    SectionSpec {
        key: "history",
        title: "History",
        group: PanelGroup::Inspect,
        default_open: false,
    },
    // Review
    SectionSpec {
        key: "drc",
        title: "DRC",
        group: PanelGroup::Review,
        default_open: true,
    },
    SectionSpec {
        key: "diff",
        title: "Layout diff",
        group: PanelGroup::Review,
        default_open: false,
    },
    SectionSpec {
        key: "comments",
        title: "Comments",
        group: PanelGroup::Review,
        default_open: false,
    },
    // --- lane trace-ui: net-trace Inspector section (F3 consumer; ADR 0103) ---
    SectionSpec {
        key: "trace",
        title: "Trace",
        group: PanelGroup::Review,
        default_open: false,
    },
    // --- end lane trace-ui ---
    // Automate
    SectionSpec {
        key: "agent",
        title: "Agent",
        group: PanelGroup::Automate,
        default_open: true,
    },
    SectionSpec {
        key: "generate",
        title: "Generate",
        group: PanelGroup::Automate,
        default_open: false,
    },
    SectionSpec {
        key: "pcell",
        title: "PCell",
        group: PanelGroup::Automate,
        default_open: false,
    },
    // --- lane waveform-ui: waveform viewer Inspector section (ADR 0110) ---
    SectionSpec {
        key: "waveform",
        title: "Waveform",
        group: PanelGroup::Automate,
        default_open: false,
    },
    // --- end lane waveform-ui ---
    // --- lane plugin-ui: plugin manager Inspector section (ADR 0116/0120) ---
    SectionSpec {
        key: "plugins",
        title: "Plugins",
        group: PanelGroup::Automate,
        default_open: false,
    },
    // --- end lane plugin-ui ---
    // Settings
    SectionSpec {
        key: "operations",
        title: "Operations",
        group: PanelGroup::Settings,
        default_open: false,
    },
    SectionSpec {
        key: "productivity",
        title: "Productivity",
        group: PanelGroup::Settings,
        default_open: false,
    },
    SectionSpec {
        key: "snap",
        title: "Snap and guides",
        group: PanelGroup::Settings,
        default_open: false,
    },
    SectionSpec {
        key: "export",
        title: "Export",
        group: PanelGroup::Settings,
        default_open: false,
    },
    SectionSpec {
        key: "tech",
        title: "Technology editor",
        group: PanelGroup::Settings,
        default_open: false,
    },
    // --- lane underlay: image underlay Inspector section (ADR 0118) ---
    SectionSpec {
        key: "underlay",
        title: "Underlay",
        group: PanelGroup::Settings,
        default_open: false,
    },
    // --- end lane underlay ---
];

/// The remembered state of the right Inspector panel.
///
/// Persisted per device through [`crate::session`] (catalog 61): the selected
/// group, the dragged width, the icon-rail collapse flag (catalog 59), and every
/// section's open flag.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InspectorState {
    /// The group the segmented control has selected.
    pub group: PanelGroup,
    /// The panel width, in points (persisted; seeds the panel's default width and
    /// tracks user drags).
    pub width: OrderedWidth,
    /// Whether the panel is collapsed to the icon rail (catalog 59).
    pub collapsed: bool,
    /// Per-section open flags, keyed by [`SectionSpec::key`].
    open: BTreeMap<&'static str, bool>,
}

/// A width value that keeps [`InspectorState`] `Eq` (points, rounded to whole
/// pixels for comparison so the derived `Eq`/`Hash` are well defined even though
/// the stored value is a float).
//
// egui reports fractional widths; persistence stores the raw value. Comparing two
// states for equality (the session round-trip test) needs a total order, so the
// float is wrapped and compared on its bit pattern after rounding.
#[derive(Clone, Copy, Debug)]
pub struct OrderedWidth(pub f32);

impl OrderedWidth {
    /// The width in points, clamped to the resizable range.
    #[must_use]
    pub fn clamped(self) -> f32 {
        self.0.clamp(MIN_WIDTH, MAX_WIDTH)
    }
}

impl PartialEq for OrderedWidth {
    fn eq(&self, other: &Self) -> bool {
        (self.0.round() - other.0.round()).abs() < f32::EPSILON
    }
}

impl Eq for OrderedWidth {}

impl Default for InspectorState {
    fn default() -> Self {
        let open = SECTIONS.iter().map(|s| (s.key, s.default_open)).collect();
        Self {
            group: PanelGroup::Inspect,
            width: OrderedWidth(DEFAULT_WIDTH),
            collapsed: false,
            open,
        }
    }
}

impl InspectorState {
    /// Whether the section keyed by `key` is currently open.
    #[must_use]
    pub fn is_open(&self, key: &str) -> bool {
        self.open.get(key).copied().unwrap_or(false)
    }

    /// Sets the open flag for the section keyed by `key` (ignoring unknown keys).
    pub fn set_open(&mut self, key: &'static str, open: bool) {
        if let Some(flag) = self.open.get_mut(key) {
            *flag = open;
        }
    }

    /// Opens a section and selects its group, so an event that produces content
    /// (a DRC run, a new comment thread) surfaces the section that shows it
    /// (catalog 67). A no-op for an unknown key.
    pub fn reveal(&mut self, key: &str) {
        if let Some(spec) = SECTIONS.iter().find(|s| s.key == key) {
            self.group = spec.group;
            self.collapsed = false;
            self.open.insert(spec.key, true);
        }
    }

    /// The sections belonging to `group`, in render order.
    pub fn sections_in(group: PanelGroup) -> impl Iterator<Item = &'static SectionSpec> {
        SECTIONS.iter().filter(move |s| s.group == group)
    }

    /// The open sections as a comma-separated key list (the persisted form). Empty
    /// only when every section is closed.
    #[must_use]
    pub fn open_tags(&self) -> String {
        self.open
            .iter()
            .filter(|(_, open)| **open)
            .map(|(key, _)| *key)
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Restores open flags from a persisted [`open_tags`](Self::open_tags) string.
    ///
    /// An empty string is treated as "no record" and leaves the defaults intact
    /// (so a session that predates this key keeps the default open set). A
    /// non-empty string is authoritative: every listed section is opened and every
    /// other closed.
    pub fn apply_open_tags(&mut self, tags: &str) {
        if tags.trim().is_empty() {
            return;
        }
        for flag in self.open.values_mut() {
            *flag = false;
        }
        for key in tags.split(',').map(str::trim).filter(|k| !k.is_empty()) {
            // `key` is a borrowed slice; match it to the static registry key so the
            // map keeps its `'static` keys.
            if let Some(spec) = SECTIONS.iter().find(|s| s.key == key) {
                self.open.insert(spec.key, true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_has_a_known_group_and_unique_key() {
        let mut seen = std::collections::HashSet::new();
        for spec in SECTIONS {
            assert!(seen.insert(spec.key), "duplicate section key {}", spec.key);
            assert!(PanelGroup::ALL.contains(&spec.group));
        }
    }

    #[test]
    fn no_group_starts_with_more_than_one_open_section() {
        // The high-frequency groups lead with exactly one open section; Settings is
        // all low-frequency and starts fully collapsed. Never more than one, so a
        // selected group never opens as a wall of expanded panels.
        for group in PanelGroup::ALL {
            let open = InspectorState::sections_in(group)
                .filter(|s| s.default_open)
                .count();
            assert!(
                open <= 1,
                "{} should start with at most one open section",
                group.label()
            );
        }
        // Inspect, Review, and Automate each lead with one open section.
        let total_open = SECTIONS.iter().filter(|s| s.default_open).count();
        assert_eq!(
            total_open, 3,
            "three groups lead open; Settings starts collapsed"
        );
    }

    #[test]
    fn group_index_round_trips_and_clamps() {
        for group in PanelGroup::ALL {
            assert_eq!(PanelGroup::from_index(group.index()), group);
        }
        assert_eq!(PanelGroup::from_index(99), PanelGroup::Inspect);
    }

    #[test]
    fn open_tags_round_trip_is_authoritative() {
        let mut state = InspectorState::default();
        state.set_open("properties", false);
        state.set_open("history", true);
        let tags = state.open_tags();

        let mut restored = InspectorState::default();
        restored.apply_open_tags(&tags);
        assert!(!restored.is_open("properties"), "explicit close survives");
        assert!(restored.is_open("history"), "explicit open survives");
        assert_eq!(restored.open, state.open);
    }

    #[test]
    fn empty_open_tags_keeps_defaults() {
        let mut state = InspectorState::default();
        state.apply_open_tags("");
        assert!(
            state.is_open("properties"),
            "default-open section stays open"
        );
        assert!(
            !state.is_open("history"),
            "default-closed section stays closed"
        );
    }

    #[test]
    fn reveal_selects_group_and_opens_section() {
        let mut state = InspectorState {
            group: PanelGroup::Inspect,
            collapsed: true,
            ..InspectorState::default()
        };
        state.reveal("comments");
        assert_eq!(state.group, PanelGroup::Review);
        assert!(!state.collapsed);
        assert!(state.is_open("comments"));
    }
}
