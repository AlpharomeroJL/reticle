//! The session history browser: enumerate past agent transcripts, load one.
//!
//! A finished agent run leaves a `*.transcript.jsonl` file next to its other
//! artifacts (see `reticle-agent`, which names them `<task-id>.transcript.jsonl`).
//! This module lists those transcripts so the user can pick a past run and load it
//! straight into the replay theater ([`crate::replay`]) with one click, through the
//! same [`crate::store`] seam the theater already loads through.
//!
//! Where the list comes from differs by platform, and that difference is the only
//! `cfg` in this module:
//!
//! * Native scans a directory of the filesystem for `*.transcript.jsonl` files
//!   ([`scan_dir`]).
//! * wasm has no filesystem, so the browser lists the one bundled demo transcript
//!   the theater already carries (`bundled_entries`, a wasm-only function).
//!
//! All the interesting logic (turning a set of file names into a sorted, labelled
//! entry list) is the platform-free [`entries_from_names`], unit-tested here over a
//! synthetic set with no filesystem touched.

/// One entry in the session history browser: a run the user can open.
///
/// The `reference` is what [`crate::store::SessionStore::load_reference`] consumes
/// (a filesystem path on native), and the `label` is the short, human-facing name
/// shown in the list (the task id, recovered from the `<id>.transcript.jsonl`
/// convention).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HistoryEntry {
    /// The short display label (the transcript's task id, without the suffix).
    pub label: String,
    /// The store reference that loads this transcript (a path on native).
    pub reference: String,
}

/// The filename suffix `reticle-agent` writes every transcript with.
pub const TRANSCRIPT_SUFFIX: &str = ".transcript.jsonl";

/// Turns a set of `reference` strings into sorted, labelled history entries.
///
/// Only references ending in [`TRANSCRIPT_SUFFIX`] are kept (anything else in the
/// directory is ignored); the label is the file's base name with that suffix and
/// any leading directory stripped, so `runs/wire-01.transcript.jsonl` lists as
/// `wire-01`. Entries are sorted by label for a stable, readable order, and exact
/// duplicate references are collapsed.
///
/// This is the platform-free core the filesystem and bundled sources both feed, so
/// the enumeration is unit-tested without touching a disk.
#[must_use]
pub fn entries_from_names<I, S>(references: I) -> Vec<HistoryEntry>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut entries: Vec<HistoryEntry> = references
        .into_iter()
        .map(Into::into)
        .filter(|r| r.ends_with(TRANSCRIPT_SUFFIX))
        .map(|reference| {
            let label = transcript_label(&reference);
            HistoryEntry { label, reference }
        })
        .collect();
    entries.sort_by(|a, b| {
        a.label
            .cmp(&b.label)
            .then_with(|| a.reference.cmp(&b.reference))
    });
    entries.dedup_by(|a, b| a.reference == b.reference);
    entries
}

/// The display label for a transcript reference: its base name with the directory
/// and the [`TRANSCRIPT_SUFFIX`] stripped.
#[must_use]
fn transcript_label(reference: &str) -> String {
    // Accept either separator so a Windows path and a POSIX path both reduce to
    // their base name.
    let base = reference.rsplit(['/', '\\']).next().unwrap_or(reference);
    base.strip_suffix(TRANSCRIPT_SUFFIX)
        .unwrap_or(base)
        .to_owned()
}

/// Scans `dir` for `*.transcript.jsonl` files and returns them as history entries.
///
/// Non-existent or unreadable directories yield an empty list rather than an error,
/// so the browser simply shows nothing to open. Entries are labelled and sorted by
/// [`entries_from_names`]. Native only; the browser uses `bundled_entries` on wasm.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn scan_dir(dir: &std::path::Path) -> Vec<HistoryEntry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let names = read.filter_map(|entry| {
        let path = entry.ok()?.path();
        let name = path.to_str()?;
        name.ends_with(TRANSCRIPT_SUFFIX).then(|| name.to_owned())
    });
    entries_from_names(names)
}

/// The default directory the history browser scans on native.
///
/// Runs are conventionally written under `runs/` in the working directory (the
/// default `--out-dir` for `reticle-agent`), so that is where the browser looks
/// first. The user can point it elsewhere from the UI.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn default_scan_dir() -> std::path::PathBuf {
    std::path::PathBuf::from("runs")
}

/// The history entries on wasm: the single bundled demo transcript the theater
/// already carries, so the browser is non-empty in the browser too.
///
/// There is no filesystem in the browser, so this is the one reference the wasm
/// [`crate::store::SessionStore`] recognizes; loading it falls back to the bundled
/// default (`load_reference` returns `Ok(None)` there), which is exactly the demo
/// the theater opens into.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn bundled_entries() -> Vec<HistoryEntry> {
    vec![HistoryEntry {
        label: "bundled demo".to_owned(),
        reference: concat!("theater-demo", ".transcript.jsonl").to_owned(),
    }]
}

/// The platform default history listing.
///
/// Native scans [`default_scan_dir`]; wasm lists the bundled demo.
#[must_use]
#[cfg(not(target_arch = "wasm32"))]
pub fn default_entries() -> Vec<HistoryEntry> {
    scan_dir(&default_scan_dir())
}

/// The platform default history listing (wasm: the bundled demo transcript).
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn default_entries() -> Vec<HistoryEntry> {
    bundled_entries()
}

/// The default directory string shown in the history browser's scan box.
#[must_use]
fn default_dir_text() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        default_scan_dir().to_string_lossy().into_owned()
    }
    #[cfg(target_arch = "wasm32")]
    {
        "bundled".to_owned()
    }
}

/// The session history browser's UI state: the listed entries, the directory the
/// native scan reads, and its last error line.
///
/// Bundled into one value so the [`App`](crate::app::App) carries a single field
/// for the whole browser. The scan itself is on-demand ([`refresh`](Self::refresh)),
/// never per frame, so listing past runs never costs a directory read during
/// drawing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HistoryBrowser {
    /// The transcripts currently listed, sorted by label.
    entries: Vec<HistoryEntry>,
    /// The directory the native scan reads (ignored on wasm).
    pub dir: String,
    /// The last scan/load error, shown under the list (empty when none).
    pub error: String,
}

impl Default for HistoryBrowser {
    fn default() -> Self {
        Self::new()
    }
}

impl HistoryBrowser {
    /// An empty browser seeded with the platform default scan directory. The list
    /// starts empty; [`refresh`](Self::refresh) fills it on demand.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            dir: default_dir_text(),
            error: String::new(),
        }
    }

    /// The listed entries.
    #[must_use]
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    /// Whether the list is currently empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Replaces the listed entries (used by the UI after a scan, and by tests).
    pub fn set_entries(&mut self, entries: Vec<HistoryEntry>) {
        self.entries = entries;
    }

    /// Re-scans for transcripts and updates the list.
    ///
    /// Native scans the directory named in [`dir`](Self::dir); wasm lists the
    /// bundled demo. The error line is set when native finds nothing (a hint that
    /// the path may be wrong) and cleared otherwise.
    pub fn refresh(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let entries = scan_dir(std::path::Path::new(self.dir.trim()));
            if entries.is_empty() {
                self.error = format!("No *.transcript.jsonl under \"{}\"", self.dir.trim());
            } else {
                self.error.clear();
            }
            self.entries = entries;
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.entries = bundled_entries();
            self.error.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerates_and_sorts_a_synthetic_set() {
        // A synthetic directory listing: three transcripts plus noise.
        let names = vec![
            "runs/wire-02.transcript.jsonl",
            "runs/wire-01.transcript.jsonl",
            "runs/notes.txt",
            "runs/via-stack.transcript.jsonl",
            "runs/render.png",
        ];
        let entries = entries_from_names(names);
        // Only the three transcripts survive, sorted by label.
        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries.iter().map(|e| e.label.as_str()).collect::<Vec<_>>(),
            ["via-stack", "wire-01", "wire-02"]
        );
        // The reference is the full path the store loads through.
        assert_eq!(entries[1].reference, "runs/wire-01.transcript.jsonl");
    }

    #[test]
    fn label_strips_directory_and_suffix_for_both_separators() {
        assert_eq!(transcript_label("a/b/c/run-7.transcript.jsonl"), "run-7");
        assert_eq!(transcript_label(r"C:\runs\job42.transcript.jsonl"), "job42");
        assert_eq!(transcript_label("bare.transcript.jsonl"), "bare");
        // A reference without the suffix keeps its base name (it is filtered out
        // of listings, but the label helper is still total).
        assert_eq!(transcript_label("runs/loose.json"), "loose.json");
    }

    #[test]
    fn non_transcripts_are_filtered_out() {
        let entries = entries_from_names(["a.txt", "b.gds", "c.png", "d.json"]);
        assert!(entries.is_empty());
    }

    #[test]
    fn duplicate_references_collapse() {
        let entries = entries_from_names([
            "runs/dup.transcript.jsonl",
            "runs/dup.transcript.jsonl",
            "runs/other.transcript.jsonl",
        ]);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].label, "dup");
        assert_eq!(entries[1].label, "other");
    }

    #[test]
    fn empty_set_is_empty() {
        assert!(entries_from_names(Vec::<String>::new()).is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn scan_of_a_missing_directory_is_empty() {
        let missing = std::path::Path::new("this-directory-does-not-exist-9f3a");
        assert!(scan_dir(missing).is_empty());
    }

    #[test]
    fn browser_starts_empty_and_takes_entries() {
        let mut browser = HistoryBrowser::new();
        assert!(browser.is_empty());
        assert!(!browser.dir.is_empty(), "seeded with a default scan dir");
        browser.set_entries(entries_from_names(["runs/x.transcript.jsonl"]));
        assert_eq!(browser.entries().len(), 1);
        assert!(!browser.is_empty());
        assert_eq!(browser.entries()[0].label, "x");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn browser_refresh_reports_empty_directory() {
        let mut browser = HistoryBrowser::new();
        browser.dir = "no-such-history-dir-4b2c".to_owned();
        browser.refresh();
        assert!(browser.is_empty());
        assert!(browser.error.contains("No *.transcript.jsonl"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn scan_finds_written_transcripts() {
        // Write two transcript files and a decoy into a temp dir, then scan it.
        let dir = std::env::temp_dir().join(format!("reticle-history-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("alpha.transcript.jsonl"), "{}").expect("write alpha");
        std::fs::write(dir.join("beta.transcript.jsonl"), "{}").expect("write beta");
        std::fs::write(dir.join("ignore.txt"), "x").expect("write decoy");

        let entries = scan_dir(&dir);
        assert_eq!(entries.len(), 2, "two transcripts, decoy ignored");
        assert_eq!(entries[0].label, "alpha");
        assert_eq!(entries[1].label, "beta");
        assert!(entries[0].reference.ends_with("alpha.transcript.jsonl"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
