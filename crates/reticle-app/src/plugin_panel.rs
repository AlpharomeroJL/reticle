//! The Plugin Manager Inspector section: browse the F5 plugin index on every
//! build, and (native only) install and run a selected plugin through the
//! sandboxed host, funneling its staged edits through the caller's undo-tracked
//! document (ADR 0116, ADR 0120).
//!
//! # The browser/desktop split (ADR 0120)
//!
//! Following ADR 0115/0116's native-first precedent, the plugin runtime
//! (`wasmi`, `reticle_plugin::Host`) is native-only, so the browser build can
//! never run a plugin. This module keeps that split honest rather than papering
//! over it:
//!
//! - **Everywhere** (native and wasm): [`PluginPanelState`] parses the F5 index
//!   and lets the caller browse and preview every entry (id, name, version,
//!   permissions, provenance). `reticle_plugin::{Index, IndexEntry, Manifest}`
//!   are pure serde (no wasm dependency), so this costs the browser build
//!   essentially nothing and never touches the runtime.
//! - **Native only**: [`PluginPanelState::run_selected`] loads the selected
//!   entry's installed wasm bytes through [`reticle_plugin::Host::run`],
//!   recording a [`RunSummary`] (never the native-only `RunOutcome`/`HostError`
//!   types themselves, which cannot appear in a field of a struct this crate also
//!   compiles for wasm). `crate::app::App::plugin_install`/`plugin_enable` (the
//!   `plugin.install`/`plugin.enable` command handlers) are the native callers;
//!   their wasm arms show [`BROWSER_DISCLAIMER`] instead of a live run they
//!   cannot perform. The browser never claims to run a plugin (the plugin-moat
//!   claim shape, `reticle-claims`).
//!
//! # Fixture-first (ledgered)
//!
//! The committed F5 index (`library/plugins/index.json`, the `plugin-manifest-index`
//! lane) had not merged when this lane shipped, so [`PluginPanelState::new`] embeds
//! `reticle-plugin`'s own contract fixture instead: the same
//! `crates/reticle-plugin/tests/fixtures/contracts/f5_index.json` that
//! `reticle-plugin`'s own `tests/f5_manifest.rs` pins. LEDGERED: swap
//! `F5_INDEX_FIXTURE`'s path to the real committed index once that lane merges.

use reticle_plugin::{Index, IndexEntry};

#[cfg(not(target_arch = "wasm32"))]
use reticle_model::EditableDocument;
#[cfg(not(target_arch = "wasm32"))]
use reticle_plugin::{Host, HostContext, Limits};

/// The embedded F5 plugin index (see the module doc's fixture-first note).
const F5_INDEX_FIXTURE: &str =
    include_str!("../../reticle-plugin/tests/fixtures/contracts/f5_index.json");

/// The honest browser-side disclaimer: plugins never run in the browser tab.
/// Shown by the wasm arm of the panel's render
/// (`crate::app::App::plugin_section`) and of the `plugin.install`/`plugin.enable`
/// command handlers. Pinned by a plain, native-compiled unit test below so its
/// wording is checked by `cargo test -p reticle-app` without needing a
/// wasm-target test run.
pub const BROWSER_DISCLAIMER: &str = "Plugins run in the desktop app. This browser build lists and previews the \
     index only; it never runs a plugin.";

/// A plain, cross-platform summary of a completed (or failed) native plugin run.
///
/// `reticle_plugin::RunOutcome` and `HostError` are themselves native-only types
/// (`reticle_plugin::host` is `cfg(not(wasm32))`, ADR 0116), so they cannot appear
/// in a field of [`PluginPanelState`], which this crate also compiles for wasm.
/// This is the browser-safe shape [`PluginPanelState::run_selected`] (native)
/// records after a real run.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RunSummary {
    /// The plugin ran to completion (the host did not reject or trap it).
    Ran {
        /// Edits the plugin staged, in call order.
        staged: usize,
        /// Staged edits the funnel applied.
        applied: usize,
        /// Staged edits that failed to apply (for example a missing target cell).
        errors: usize,
        /// Fuel the run consumed.
        fuel_consumed: u64,
    },
    /// Nothing was installed, or the host failed to load, gate, instantiate, or
    /// run the plugin; the message is a human-readable reason.
    Failed(String),
}

impl std::fmt::Display for RunSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ran {
                staged,
                applied,
                errors,
                fuel_consumed,
            } => write!(
                f,
                "staged {staged}, applied {applied}, errors {errors}, fuel {fuel_consumed}"
            ),
            Self::Failed(msg) => write!(f, "{msg}"),
        }
    }
}

/// The Plugin Manager panel's state: the browsable F5 index, the current
/// selection, and each entry's installed wasm bytes, enabled flag, and last run
/// outcome.
///
/// The `installed`/`last_run` bookkeeping is native-only IN EFFECT (only
/// `crate::app::App`'s native command handlers ever write to it), but the field
/// TYPES themselves (`Vec<u8>`, [`RunSummary`]) carry no wasmi exposure, so they
/// are declared unconditionally: the struct definition and its `Default` impl
/// stay free of field-level `cfg`, matching how [`Index`]/[`IndexEntry`] (pure
/// serde) are also unconditional.
#[derive(Debug)]
pub struct PluginPanelState {
    /// The parsed, validated F5 index.
    index: Index,
    /// The index into `index.entries` of the selected plugin.
    selected: usize,
    /// Session-only installed wasm bytes per entry, parallel to `index.entries`.
    /// Never persisted (a follow-on; see the module doc and ADR 0120): populated
    /// only by `crate::app::App::plugin_install`'s native arm.
    installed: Vec<Option<Vec<u8>>>,
    /// Whether the user has enabled each entry, parallel to `index.entries`.
    enabled: Vec<bool>,
    /// The last run outcome per entry, parallel to `index.entries`.
    last_run: Vec<Option<RunSummary>>,
}

impl Default for PluginPanelState {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginPanelState {
    /// Parses the embedded F5 index fixture (see the module doc).
    ///
    /// The fixture is a committed, already-validated contract file
    /// (`reticle-plugin`'s own `tests/f5_manifest.rs` pins it): a parse failure
    /// here is a build-time programming error, not a runtime condition to
    /// recover from, so this panics on malformed embedded JSON exactly as
    /// `crate::gallery`'s bundled-manifest constant does.
    #[must_use]
    pub fn new() -> Self {
        let index: Index = serde_json::from_str(F5_INDEX_FIXTURE).expect(
            "the embedded F5 index fixture parses (committed, pinned by reticle-plugin's own tests)",
        );
        let n = index.entries.len();
        Self {
            index,
            selected: 0,
            installed: vec![None; n],
            enabled: vec![false; n],
            last_run: vec![None; n],
        }
    }

    /// Every entry in the browsable index, in its committed (id-sorted) order.
    #[must_use]
    pub fn entries(&self) -> &[IndexEntry] {
        &self.index.entries
    }

    /// The index of the selected entry.
    #[must_use]
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Selects the entry at `index`, if in range.
    pub fn select(&mut self, index: usize) {
        if index < self.index.entries.len() {
            self.selected = index;
        }
    }

    /// The selected entry (panics if the index is empty; callers iterate
    /// `0..entries().len()` first, as `PCellPanelState::def_at`'s callers do).
    #[must_use]
    pub fn selected_entry(&self) -> &IndexEntry {
        &self.index.entries[self.selected]
    }

    /// Whether the entry at `index` has installed wasm bytes this session.
    #[must_use]
    pub fn is_installed(&self, index: usize) -> bool {
        self.installed.get(index).is_some_and(Option::is_some)
    }

    /// Whether the entry at `index` is enabled.
    #[must_use]
    pub fn is_enabled(&self, index: usize) -> bool {
        self.enabled.get(index).copied().unwrap_or(false)
    }

    /// The last run outcome for the entry at `index`, if any.
    #[must_use]
    pub fn last_run(&self, index: usize) -> Option<&RunSummary> {
        self.last_run.get(index).and_then(Option::as_ref)
    }

    /// Installs `wasm` as the selected entry's loaded plugin bytes this session
    /// (no persistence yet: a fresh session starts with nothing installed; see
    /// the module doc). Clears any previous run outcome, which described the
    /// previously loaded bytes, not these.
    pub fn install_selected(&mut self, wasm: Vec<u8>) {
        let i = self.selected;
        self.installed[i] = Some(wasm);
        self.last_run[i] = None;
    }

    /// Marks the selected entry enabled.
    pub fn enable_selected(&mut self) {
        self.enabled[self.selected] = true;
    }

    /// Marks the selected entry disabled. Keeps any installed bytes, so
    /// re-enabling does not require reinstalling.
    pub fn disable_selected(&mut self) {
        self.enabled[self.selected] = false;
    }

    /// Records `summary` as the selected entry's last run outcome, overwriting
    /// any previous one.
    pub fn record_run(&mut self, summary: RunSummary) {
        let i = self.selected;
        self.last_run[i] = Some(summary);
    }

    /// Runs the selected entry's installed wasm bytes through a fresh
    /// [`Host`] against `doc`, recording a [`RunSummary`] (success or failure,
    /// always) as the selected entry's last run outcome.
    ///
    /// Returns the raw [`reticle_plugin::RunOutcome`] on success, so the caller
    /// can fold its staged edits into its OWN undo-tracked document (see the
    /// note on `doc` below); returns `None` on any failure, including nothing
    /// installed for the selected entry. Either way `last_run` already carries
    /// the honest reason.
    ///
    /// `doc` is a throwaway scratch document the caller clones from its real,
    /// undo-tracked one. [`Host::run`] funnels staged edits through
    /// `EditableDocument::apply` directly as it goes, bypassing any
    /// undo-grouping a caller's own wrapper keeps (`crate::history::History`,
    /// for `crate::app::App`); handing it that wrapper's private document
    /// directly would apply edits it never grouped, desyncing the wrapper's
    /// undo/redo bookkeeping from the document's real undo stack. So the caller
    /// runs against a scratch document here, then re-applies
    /// `RunOutcome::staged` for real through its own wrapper (for example
    /// `History::apply_group`) as one undoable step, exactly as a boolean op's
    /// multi-edit batch lands; `doc` itself is then discarded.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn run_selected(
        &mut self,
        doc: &mut EditableDocument,
    ) -> Option<reticle_plugin::RunOutcome> {
        let i = self.selected;
        let Some(wasm) = self.installed[i].clone() else {
            self.last_run[i] = Some(RunSummary::Failed(
                "no plugin installed for the selected entry".to_owned(),
            ));
            return None;
        };
        let manifest = self.index.entries[i].manifest.clone();
        let host = Host::new();
        match host.run(
            &wasm,
            &manifest,
            doc,
            &HostContext::default(),
            &Limits::default(),
        ) {
            Ok(outcome) => {
                self.last_run[i] = Some(RunSummary::Ran {
                    staged: outcome.staged.len(),
                    applied: outcome.applied,
                    errors: outcome.apply_errors.len(),
                    fuel_consumed: outcome.fuel_consumed,
                });
                Some(outcome)
            }
            Err(e) => {
                self.last_run[i] = Some(RunSummary::Failed(e.to_string()));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The embedded F5 fixture parses and lists at least one browsable plugin,
    /// selected by default: the "everywhere" half of the browser/desktop split.
    #[test]
    fn embedded_fixture_lists_at_least_one_plugin() {
        let state = PluginPanelState::new();
        assert!(
            !state.entries().is_empty(),
            "the F5 fixture seeds at least one entry"
        );
        assert_eq!(state.selected(), 0);
        assert!(!state.is_installed(0));
        assert!(!state.is_enabled(0));
        assert!(state.last_run(0).is_none());
    }

    /// The browser disclaimer is honest about the desktop-only boundary: pinned
    /// here (a plain, native-compiled test) so `cargo test -p reticle-app` checks
    /// the wording the wasm build shows without needing a wasm-target test run
    /// (the plugin-moat claim shape, ADR 0120 / `reticle-claims`).
    #[test]
    fn browser_disclaimer_is_honest_about_the_desktop_only_boundary() {
        assert!(BROWSER_DISCLAIMER.contains("desktop app"));
        assert!(BROWSER_DISCLAIMER.contains("never runs a plugin"));
    }

    /// Selecting an index out of range is a no-op, matching
    /// `PCellPanelState::select`'s bounds-checked behavior.
    #[test]
    fn select_ignores_out_of_range_index() {
        let mut state = PluginPanelState::new();
        let before = state.selected();
        state.select(9999);
        assert_eq!(state.selected(), before);
    }

    /// Disabling keeps the installed bytes (re-enabling does not require
    /// reinstalling), but does clear the enabled flag.
    #[test]
    fn install_then_disable_keeps_bytes_but_clears_enabled() {
        let mut state = PluginPanelState::new();
        state.install_selected(vec![0u8; 4]);
        assert!(state.is_installed(0));
        state.enable_selected();
        assert!(state.is_enabled(0));
        state.disable_selected();
        assert!(!state.is_enabled(0));
        assert!(state.is_installed(0), "disable keeps the installed bytes");
    }

    /// A recorded run outcome is retrievable, and a fresh install invalidates a
    /// stale one (it described the previously loaded bytes, not the new ones).
    #[test]
    fn record_run_is_retrievable_and_reinstall_clears_it() {
        let mut state = PluginPanelState::new();
        assert!(state.last_run(0).is_none());
        state.record_run(RunSummary::Ran {
            staged: 1,
            applied: 1,
            errors: 0,
            fuel_consumed: 10,
        });
        assert!(state.last_run(0).is_some());
        state.install_selected(vec![1u8; 2]);
        assert!(
            state.last_run(0).is_none(),
            "a fresh install invalidates the stale outcome"
        );
    }

    /// `Display` for a failed run is just the reason (no redundant wrapper text);
    /// for a completed run it names every count so the status bar reads clearly.
    #[test]
    fn run_summary_displays_readably() {
        assert_eq!(RunSummary::Failed("boom".to_owned()).to_string(), "boom");
        let ran = RunSummary::Ran {
            staged: 2,
            applied: 1,
            errors: 1,
            fuel_consumed: 42,
        };
        let text = ran.to_string();
        assert!(text.contains("staged 2"));
        assert!(text.contains("applied 1"));
        assert!(text.contains("errors 1"));
        assert!(text.contains("fuel 42"));
    }
}
