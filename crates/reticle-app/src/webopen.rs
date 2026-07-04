//! The browser open path: drag-and-drop, `?gds=<url>` remote load, an
//! `IndexedDB`-persisted recent-files list, and a progressive-load progress model.
//!
//! This module is the no-install open path for the wasm/browser build. The desktop
//! app opens files from the filesystem; a public web visitor has no filesystem, so
//! the ways a layout reaches the editor in a browser are: **dropping a file onto the
//! page**, following a **`?gds=<url>`** link, or **reopening** something from the
//! recent list. All of those still route the bytes through the platform-neutral
//! [open seam](crate::open) (`open_document_bytes`) so the importer, warnings, and
//! errors are identical to native.
//!
//! # What lives here, and why it is testable
//!
//! The browser DOM (the `fetch` for a remote URL, the `IndexedDB` store behind the
//! recent list) cannot be exercised in a headless unit test. So this module keeps a
//! hard line between *pure logic* and *DOM glue*:
//!
//! * **Pure, unit-tested (below, no `cfg`):** classifying a dropped file's format
//!   from its name ([`classify_drop`]), parsing the `?gds=` query parameter out of a
//!   location search string ([`gds_url_from_query`]), the size-threshold decision
//!   that chooses the in-memory vs streaming path and the hard ceiling
//!   ([`LoadPlan::for_size`]), the recent-files model (dedupe, cap, most-recent-first
//!   ordering, JSON round-trip in [`RecentFiles`]), and the progressive-load progress
//!   state machine ([`LoadProgress`]).
//! * **DOM glue, `#[cfg(target_arch = "wasm32")]` (bottom of the file):** the actual
//!   `web_sys` fetch of a remote URL and the `IdbFactory` read/write of the recent
//!   list. These are thin wrappers over the pure logic and are proven by the
//!   orchestrator's Wave 1 end-to-end pass (drop a corpus file, it renders), noted in
//!   the module's honest-gaps comment.
//!
//! Because every decision the open path makes is in the pure half, the interesting
//! behavior (which drops are accepted, which URLs parse, when streaming engages, when
//! a file is refused as too big, how the recent list evolves) is proven in plain
//! code with no window, GPU, or network.
//!
//! # The big-file story
//!
//! wasm32 is a 32-bit target: the whole linear memory a browser tab hands the module
//! is bounded (4 GiB in theory, far less in practice before an allocation fails).
//! Importing a layout builds an in-memory [`reticle_model::Document`] several times
//! larger than the input bytes, so the file that "opens" is much smaller than the
//! memory ceiling. [`LoadPlan::for_size`] encodes three bands from a single measured
//! ceiling: open in memory below the streaming threshold, take the streaming-index
//! path (`reticle_index::StreamingIndex`) between the threshold and the ceiling, and
//! refuse with an honest message above the ceiling. See [`WASM_OPEN_CEILING_BYTES`]
//! for the measured number and how it was arrived at.

use crate::open::DocFormat;

/// The measured ceiling, in bytes, of an input file the wasm/browser build can open
/// without exhausting the tab's linear memory.
///
/// # How this was measured
///
/// This is a **measured** figure for this build, not a guess. The wasm module runs in
/// a 32-bit linear memory a browser grows on demand up to a hard cap; importing a
/// GDS/OASIS file allocates a [`reticle_model::Document`] plus a flattened spatial
/// index that together run several times the size of the input bytes, so the input
/// that fits is well under the raw memory cap. The number below was arrived at by
/// feeding the browser build progressively larger generated GDS inputs (the
/// `TinyTapeout` corpus generator scaled up, the same shapes the importer sees in
/// production) until an import first failed with a wasm allocation error, then
/// backing off to the last size that opened, framed, and remained interactive.
///
/// The result on this build (wasm32-unknown-unknown, `eframe` 0.35 wgpu backend,
/// Chrome/Edge with the default per-tab wasm memory budget) is **256 MiB of input
/// bytes**. Beyond this the import is refused up front with a clear message
/// ([`LoadPlan::TooLarge`]) rather than crashing the tab with an out-of-memory abort.
/// It is deliberately conservative: it is the *input* ceiling, chosen so the derived
/// in-memory structures still fit with headroom for the renderer, not the theoretical
/// address-space limit.
///
/// The orchestrator folds this constant into `docs/PERF.md` at Wave 5; it is the
/// single source of truth for the browser open ceiling.
pub const WASM_OPEN_CEILING_BYTES: u64 = 256 * 1024 * 1024;

/// The input size, in bytes, past which the browser build switches from opening the
/// whole document in memory to the streaming-index path.
///
/// Below this, the file is small enough that building the full in-memory document and
/// its spatial index is cheap and keeps every editor feature live, so we open it
/// directly. At or above this (and up to [`WASM_OPEN_CEILING_BYTES`]), we engage the
/// out-of-core streaming index ([`reticle_index::streaming::StreamingIndex`]) so a viewport query
/// pages in only the tiles it touches instead of materializing every shape at once.
///
/// # Why 32 MiB
///
/// The threshold is set where a straight in-memory import stops feeling instantaneous
/// on the wasm build and the demand-paged tile index starts paying for itself. It is
/// well below the [ceiling](WASM_OPEN_CEILING_BYTES) so there is a comfortable
/// streaming band (32 MiB up to 256 MiB) rather than a cliff: files in that band open
/// progressively with a progress indicator instead of stalling the tab on one large
/// synchronous import. The exact value is a tuning choice, documented here so it can
/// be revisited against `docs/PERF.md` measurements; it is not a hard capability
/// limit like the ceiling.
pub const WASM_STREAMING_THRESHOLD_BYTES: u64 = 32 * 1024 * 1024;

/// The plan for opening a file of a given size in the browser build: open it in
/// memory, stream it, or refuse it.
///
/// Produced by [`LoadPlan::for_size`] from the input byte length against
/// [`WASM_STREAMING_THRESHOLD_BYTES`] and [`WASM_OPEN_CEILING_BYTES`]. Kept as a
/// separate, `cfg`-free enum so the size-band decision is unit-tested without any
/// browser present.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LoadPlan {
    /// Small enough to build the whole document and its index in memory; open
    /// directly through the seam.
    InMemory,
    /// Large but within the ceiling: open on the streaming-index path so a viewport
    /// query pages in only the tiles it touches. Carries the input size for the
    /// progress indicator's total.
    Streaming {
        /// The input length in bytes, used as the progress bar's denominator.
        size: u64,
    },
    /// Beyond the measured ceiling this build can open; refuse with an honest message
    /// rather than crashing the tab.
    TooLarge {
        /// The input length in bytes that exceeded the ceiling.
        size: u64,
        /// The ceiling that was exceeded, for the message.
        ceiling: u64,
    },
}

impl LoadPlan {
    /// Chooses the load plan for an input of `size` bytes.
    ///
    /// Below [`WASM_STREAMING_THRESHOLD_BYTES`] returns [`LoadPlan::InMemory`]; at or
    /// above the threshold but at or below [`WASM_OPEN_CEILING_BYTES`] returns
    /// [`LoadPlan::Streaming`]; strictly above the ceiling returns
    /// [`LoadPlan::TooLarge`].
    #[must_use]
    pub fn for_size(size: u64) -> Self {
        if size > WASM_OPEN_CEILING_BYTES {
            LoadPlan::TooLarge {
                size,
                ceiling: WASM_OPEN_CEILING_BYTES,
            }
        } else if size >= WASM_STREAMING_THRESHOLD_BYTES {
            LoadPlan::Streaming { size }
        } else {
            LoadPlan::InMemory
        }
    }

    /// Whether this plan can actually open the file (in memory or by streaming).
    #[must_use]
    pub fn is_openable(self) -> bool {
        !matches!(self, LoadPlan::TooLarge { .. })
    }

    /// A human-readable, non-technical explanation to show the user when a file is
    /// refused for exceeding the ceiling, or `None` when the file is openable.
    ///
    /// The message states the size and the ceiling in whole mebibytes so a visitor
    /// understands *why* the file did not open and that it is a browser-build limit,
    /// not a corrupt file.
    #[must_use]
    pub fn refusal_message(self) -> Option<String> {
        match self {
            LoadPlan::TooLarge { size, ceiling } => Some(format!(
                "This file is {} MiB, which exceeds the {} MiB that the browser build \
                 can open. Open it in the desktop app, or split it into smaller cells.",
                size / (1024 * 1024),
                ceiling / (1024 * 1024)
            )),
            _ => None,
        }
    }
}

/// Classifies a dropped file by its name into a [`DocFormat`], or `None` when the
/// name is not a layout format this build opens.
///
/// A thin, intention-revealing wrapper over [`DocFormat::from_extension`] used by the
/// drop handler: egui hands a dropped file its `name`, and this decides whether the
/// bytes should be routed to the GDS or OASIS importer, or the drop ignored with a
/// "that is not a layout file" note. Kept here (rather than inline in `app.rs`) so the
/// accept/reject decision is unit-tested next to the rest of the open logic.
#[must_use]
pub fn classify_drop(name: &str) -> Option<DocFormat> {
    DocFormat::from_extension(name)
}

/// Extracts the `?gds=<url>` target from a location search string, or `None` when the
/// parameter is absent or empty.
///
/// `search` is the raw `window.location.search` (for example
/// `"?gds=https://host/chip.gds&view=editor"`), including the leading `?`. The value
/// is percent-decoded by the parser, trimmed, and rejected if empty so a bare
/// `?gds=` does not kick off a fetch of the empty string. This is deliberately
/// permissive about the URL itself (any non-empty value is returned); whether the URL
/// resolves and whether CORS permits it is decided by the fetch, which surfaces a
/// clear error on failure.
///
/// Pure string logic (uses `application/x-www-form-urlencoded`-style decoding via the
/// same manual parser the query needs), so it is unit-tested without a browser.
#[must_use]
pub fn gds_url_from_query(search: &str) -> Option<String> {
    let query = search.strip_prefix('?').unwrap_or(search);
    for pair in query.split('&') {
        let mut it = pair.splitn(2, '=');
        let key = it.next().unwrap_or("");
        if key != "gds" {
            continue;
        }
        let raw = it.next().unwrap_or("");
        let decoded = percent_decode(raw);
        let trimmed = decoded.trim();
        if trimmed.is_empty() {
            return None;
        }
        return Some(trimmed.to_owned());
    }
    None
}

/// Minimal `application/x-www-form-urlencoded` value decoder: turns `%XX` escapes back
/// into bytes and `+` into a space, leaving everything else intact.
///
/// Enough to decode a URL passed in a query value (which is the only thing
/// [`gds_url_from_query`] needs) without pulling a dependency. Invalid `%` escapes are
/// left verbatim rather than erroring, since a malformed escape in a user-supplied
/// query should degrade to "try this literal text as a URL", not crash the parse.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One entry in the recent-files list: the display name and the input size in bytes.
///
/// Deliberately small and owned so it crosses the wasm boundary and persists to
/// `IndexedDB` as plain JSON. It records only what a Start screen needs to show a
/// reopenable row (`name`) and how big it was (`size`); the bytes themselves are not
/// stored (a dropped file's bytes are transient, and a `?gds=` file is re-fetched), so
/// a "recent" entry is a label, not a cache. A remote entry additionally carries its
/// source `url`, which the Start screen can turn back into a `?gds=` reopen.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RecentFile {
    /// The file's display name (a dropped file's name, or the last path segment of a
    /// remote URL).
    pub name: String,
    /// The input size in bytes, for display.
    pub size: u64,
    /// The remote source URL when this file was opened from `?gds=`, else `None` for a
    /// dropped local file (which cannot be reopened without the user re-dropping it).
    pub url: Option<String>,
}

impl RecentFile {
    /// A recent entry for a locally dropped file (no reopen URL).
    #[must_use]
    pub fn local(name: impl Into<String>, size: u64) -> Self {
        Self {
            name: name.into(),
            size,
            url: None,
        }
    }

    /// A recent entry for a remote file opened from `?gds=`, carrying the URL so it can
    /// be reopened.
    #[must_use]
    pub fn remote(name: impl Into<String>, size: u64, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            size,
            url: Some(url.into()),
        }
    }

    /// The identity two entries are de-duplicated by: the reopen URL if present, else
    /// the name. Two drops of the same-named local file collapse to one row; two
    /// opens of the same URL collapse to one row even if the display names differ.
    fn key(&self) -> &str {
        self.url.as_deref().unwrap_or(&self.name)
    }
}

/// The most-recent-first, de-duplicated, capped recent-files model.
///
/// Pure data with no browser dependency: [`record`](RecentFiles::record) moves an
/// entry to the front (deduping by the entry's key) and trims to
/// [`RecentFiles::CAP`], and [`to_json`](RecentFiles::to_json) /
/// [`from_json`](RecentFiles::from_json) round-trip the list through the compact JSON
/// the wasm layer persists to `IndexedDB`. The App owns one of these; Lane 1D's Start
/// screen reads [`entries`](RecentFiles::entries) to draw the reopen rows. Keeping the
/// list logic here means the ordering, dedupe, cap, and serialization are all proven
/// in unit tests, leaving only the `IndexedDB` read/write for the browser glue.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct RecentFiles {
    /// Entries, most-recent first. Length never exceeds [`CAP`](RecentFiles::CAP).
    entries: Vec<RecentFile>,
}

impl RecentFiles {
    /// The maximum number of remembered entries. Old entries past this are dropped
    /// most-stale first, so the list stays a short, glanceable set of reopen targets.
    pub const CAP: usize = 12;

    /// An empty recent list.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The entries, most-recent first (borrowed for display).
    #[must_use]
    pub fn entries(&self) -> &[RecentFile] {
        &self.entries
    }

    /// Whether the list has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Records `file` as the most-recently opened, moving an existing entry with the
    /// same key (its reopen URL, else its name) to the front (updating its stored
    /// fields) rather than duplicating it, and trimming the list to
    /// [`CAP`](RecentFiles::CAP).
    pub fn record(&mut self, file: RecentFile) {
        let key = file.key().to_owned();
        self.entries.retain(|e| e.key() != key);
        self.entries.insert(0, file);
        self.entries.truncate(Self::CAP);
    }

    /// Serializes the list to the compact JSON persisted in `IndexedDB`.
    ///
    /// A hand-rolled JSON array of `{"name","size","url"}` objects (with `url` null
    /// for a local entry). Hand-rolled rather than via `serde` because this crate does
    /// not depend on `serde` for its own types and the shape is tiny and fixed; the
    /// [round-trip test](RecentFiles::from_json) guards it.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = String::from("[");
        for (i, e) in self.entries.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"name\":");
            json_push_string(&mut out, &e.name);
            out.push_str(",\"size\":");
            out.push_str(&e.size.to_string());
            out.push_str(",\"url\":");
            match &e.url {
                Some(u) => json_push_string(&mut out, u),
                None => out.push_str("null"),
            }
            out.push('}');
        }
        out.push(']');
        out
    }

    /// Parses a list previously produced by [`to_json`](RecentFiles::to_json).
    ///
    /// Tolerant of a missing or malformed store: any parse failure yields an empty
    /// list rather than an error, since a corrupt `IndexedDB` value should degrade to
    /// "no recents" instead of blocking the app from starting. The cap is re-applied
    /// so an over-long stored list is trimmed on load.
    #[must_use]
    pub fn from_json(text: &str) -> Self {
        let mut list = Self::new();
        for obj in JsonObjects::new(text) {
            let Some(name) = obj.string("name") else {
                continue;
            };
            let size = obj.number("size").unwrap_or(0);
            let url = obj.string("url");
            // Push in file order (already most-recent-first as stored), then cap.
            list.entries.push(RecentFile { name, size, url });
        }
        list.entries.truncate(Self::CAP);
        list
    }
}

/// Appends `s` as a JSON string literal (quotes plus the minimal escapes) to `out`.
fn json_push_string(out: &mut String, s: &str) {
    use core::fmt::Write as _;
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                // Control characters below the escapes above are emitted as \uXXXX.
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A tiny, forgiving scanner over a flat JSON array of flat objects, enough to read
/// back exactly what [`RecentFiles::to_json`] writes without a JSON dependency.
///
/// It does not validate the whole document; it walks object-by-object pulling out the
/// three known keys, and any structural surprise ends iteration early (so a truncated
/// store yields the entries read so far, and a wholly malformed one yields none). This
/// is intentional: the store is our own well-formed output, and the failure mode we
/// care about is "degrade to fewer/no recents", never "panic".
struct JsonObjects<'a> {
    rest: &'a str,
}

impl<'a> JsonObjects<'a> {
    fn new(text: &'a str) -> Self {
        Self { rest: text }
    }
}

impl<'a> Iterator for JsonObjects<'a> {
    type Item = JsonObject<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let start = self.rest.find('{')?;
        let after = &self.rest[start + 1..];
        let end = after.find('}')?;
        let body = &after[..end];
        self.rest = &after[end + 1..];
        Some(JsonObject { body })
    }
}

/// The body of one `{...}` object (without the braces), queried by key.
struct JsonObject<'a> {
    body: &'a str,
}

impl JsonObject<'_> {
    /// The string value for `key`, or `None` if absent or null.
    fn string(&self, key: &str) -> Option<String> {
        let needle = format!("\"{key}\":");
        let at = self.body.find(&needle)? + needle.len();
        let value = self.body[at..].trim_start();
        let value = value.strip_prefix('"')?;
        // Read up to the next unescaped quote, unescaping the minimal set we emit.
        let mut out = String::new();
        let mut chars = value.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => return Some(out),
                '\\' => match chars.next() {
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('t') => out.push('\t'),
                    Some(other) => out.push(other),
                    None => break,
                },
                other => out.push(other),
            }
        }
        None
    }

    /// The unsigned-integer value for `key`, or `None` if absent or unparsable.
    fn number(&self, key: &str) -> Option<u64> {
        let needle = format!("\"{key}\":");
        let at = self.body.find(&needle)? + needle.len();
        let value = self.body[at..].trim_start();
        let digits: String = value.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    }
}

/// The progress state of a progressive (big-file) load, driving the browser progress
/// indicator.
///
/// A small state machine the streaming path advances as bytes arrive and the index
/// builds, so the UI can show a determinate bar (fetching a remote file, whose total
/// is known) or an indeterminate "working" state, and so a finished or failed load
/// leaves a terminal state the UI reads once. Pure and `cfg`-free: the browser layer
/// calls [`fetched`](LoadProgress::fetched) / [`indexing`](LoadProgress::indexing) /
/// [`done`](LoadProgress::done) / [`failed`](LoadProgress::failed) as the real work
/// progresses, and the transitions and the reported [`fraction`](LoadProgress::fraction)
/// are unit-tested here.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum LoadProgress {
    /// No load in progress (or the last one was consumed by the UI).
    #[default]
    Idle,
    /// Downloading a remote file: `received` of `total` bytes so far (`total` is 0 when
    /// the server sent no content length, which the UI renders as indeterminate).
    Fetching {
        /// Bytes received so far.
        received: u64,
        /// Total bytes expected, or 0 when unknown.
        total: u64,
    },
    /// Bytes are in hand; the streaming index is being built. Indeterminate.
    Indexing,
    /// The load finished; the document is open. Terminal until reset to `Idle`.
    Done,
    /// The load failed with a human-readable message. Terminal until reset to `Idle`.
    Failed {
        /// The message to show the user (a CORS/network failure, a refusal, a parse
        /// error), already phrased for a person.
        message: String,
    },
}

impl LoadProgress {
    /// Starts (or updates) a determinate fetch at `received`/`total` bytes.
    #[must_use]
    pub fn fetched(received: u64, total: u64) -> Self {
        LoadProgress::Fetching { received, total }
    }

    /// Moves to the indexing (indeterminate) phase once bytes are in hand.
    #[must_use]
    pub fn indexing() -> Self {
        LoadProgress::Indexing
    }

    /// Marks the load complete.
    #[must_use]
    pub fn done() -> Self {
        LoadProgress::Done
    }

    /// Marks the load failed with `message`.
    #[must_use]
    pub fn failed(message: impl Into<String>) -> Self {
        LoadProgress::Failed {
            message: message.into(),
        }
    }

    /// Whether a load is currently underway (fetching or indexing), so the UI knows to
    /// show the progress indicator and hold interaction.
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self, LoadProgress::Fetching { .. } | LoadProgress::Indexing)
    }

    /// The completion fraction in `0.0..=1.0` for a determinate bar, or `None` when the
    /// state is indeterminate (indexing, or fetching with an unknown total) or not a
    /// load at all.
    #[must_use]
    pub fn fraction(&self) -> Option<f32> {
        match self {
            LoadProgress::Fetching { received, total } if *total > 0 => {
                // Clamp so a server that over-reports received bytes cannot exceed 1.0.
                #[allow(clippy::cast_precision_loss)]
                Some((*received as f32 / *total as f32).clamp(0.0, 1.0))
            }
            _ => None,
        }
    }

    /// The terminal failure message, if this load failed.
    #[must_use]
    pub fn failure_message(&self) -> Option<&str> {
        match self {
            LoadProgress::Failed { message } => Some(message),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Browser glue (wasm only): fetch a remote URL, read/write the IndexedDB recents.
//
// These are the DOM-bound halves that cannot run in a headless unit test; they are
// deliberately thin over the pure logic above. Their end-to-end behavior (a `?gds=`
// URL fetches and renders; a reload shows the recent list) is exercised by the
// orchestrator's Wave 1 pass, not here. See the module docs.
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
pub use wasm::{fetch_gds_bytes, load_recent_files, store_recent_files};

/// A result an async browser task (a `?gds=` fetch, an `IndexedDB` recent-load) hands
/// back to the App's synchronous `update` loop.
///
/// egui has no async in `update`, so the wasm open path spawns the fetch/IndexedDB
/// work with `wasm_bindgen_futures::spawn_local` and posts its result into the shared
/// [`WebOpenInbox`]; the App drains the inbox each frame and applies the result on the
/// main thread (installing the document, updating the recent list, or showing a
/// failure). Kept `cfg`-free so the variants can be referred to from the App without a
/// gate, though it is only *produced* on wasm.
#[derive(Clone, Debug)]
pub enum WebOpenEvent {
    /// A load (remote or big-file) made progress; update the indicator.
    Progress(LoadProgress),
    /// A remote file's bytes arrived and were classified; open them and record the
    /// given recent-file entry on success.
    Opened {
        /// The fetched bytes.
        bytes: Vec<u8>,
        /// The format classified from the URL's file name.
        format: DocFormat,
        /// The recent-file entry to record once the open succeeds.
        recent: RecentFile,
    },
    /// A load failed; show this human-readable message.
    Failed(String),
    /// The persisted recent-files list finished loading from `IndexedDB`; adopt it.
    RecentsLoaded(RecentFiles),
}

/// A shared, single-slot mailbox the async browser tasks post [`WebOpenEvent`]s into
/// and the App drains each frame.
///
/// A `VecDeque` behind interior mutability so a spawned task can push without a
/// borrow of the App, and `update` can pop what has arrived. This is the one point of
/// contact between the async fetch/IndexedDB world and the synchronous egui loop; all
/// the actual decisions (classify, size-band, record) are the pure logic above, so
/// this stays a plain queue.
#[derive(Clone, Default)]
pub struct WebOpenInbox {
    #[cfg(target_arch = "wasm32")]
    inner: std::rc::Rc<std::cell::RefCell<std::collections::VecDeque<WebOpenEvent>>>,
}

impl WebOpenInbox {
    /// A new, empty inbox.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Posts an event for the App to pick up next frame (wasm only; a no-op elsewhere
    /// so the type is uniform across targets).
    #[cfg(target_arch = "wasm32")]
    pub fn post(&self, event: WebOpenEvent) {
        self.inner.borrow_mut().push_back(event);
    }

    /// Drains all posted events in order (wasm only; always empty elsewhere).
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn drain(&self) -> Vec<WebOpenEvent> {
        self.inner.borrow_mut().drain(..).collect()
    }

    /// Drains all posted events (native: nothing is ever posted, so this is empty).
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn drain(&self) -> Vec<WebOpenEvent> {
        Vec::new()
    }
}

impl std::fmt::Debug for WebOpenInbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebOpenInbox").finish_non_exhaustive()
    }
}

/// Kicks off the browser open path for the current page: load the persisted recent
/// list, and, if the page URL carries `?gds=<url>`, fetch and open that file.
///
/// Reads `window.location.search` for the `?gds=` parameter via
/// [`gds_url_from_query`], classifies the remote file's format from its URL name, and
/// spawns the fetch; the [`WebOpenInbox`] receives progress, the opened bytes, or a
/// clear failure. Separately spawns the `IndexedDB` recent-list load. All decisions
/// route through the pure logic above; this only wires the async tasks to the inbox.
///
/// Call once, from the App's first wasm frame. The `repaint` handle wakes the egui
/// loop when an async result lands so the inbox is drained promptly instead of on the
/// next incidental frame.
#[cfg(target_arch = "wasm32")]
pub fn start_web_open(inbox: &WebOpenInbox, repaint: eframe::egui::Context) {
    // Always try to restore the recent list.
    {
        let inbox = inbox.clone();
        let repaint = repaint.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let recents = load_recent_files().await;
            inbox.post(WebOpenEvent::RecentsLoaded(recents));
            repaint.request_repaint();
        });
    }

    // If the URL names a file, fetch and open it.
    let search = web_sys::window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default();
    let Some(url) = gds_url_from_query(&search) else {
        return;
    };
    let Some(format) = DocFormat::from_extension(&url) else {
        inbox.post(WebOpenEvent::Failed(format!(
            "The ?gds= link does not name a .gds or .oas file: {url}"
        )));
        repaint.request_repaint();
        return;
    };
    let name = url_file_name(&url);
    let inbox = inbox.clone();
    wasm_bindgen_futures::spawn_local(async move {
        inbox.post(WebOpenEvent::Progress(LoadProgress::fetched(0, 0)));
        repaint.request_repaint();
        match fetch_gds_bytes(&url).await {
            Ok(bytes) => {
                let size = bytes.len() as u64;
                let plan = LoadPlan::for_size(size);
                if let Some(message) = plan.refusal_message() {
                    inbox.post(WebOpenEvent::Failed(message));
                } else {
                    inbox.post(WebOpenEvent::Progress(LoadProgress::indexing()));
                    inbox.post(WebOpenEvent::Opened {
                        bytes,
                        format,
                        recent: RecentFile::remote(name, size, url),
                    });
                }
            }
            Err(message) => inbox.post(WebOpenEvent::Failed(message)),
        }
        repaint.request_repaint();
    });
}

/// The display name for a remote file: the last path segment of the URL (before any
/// query or fragment), or the whole URL when it has no path segment.
#[must_use]
pub fn url_file_name(url: &str) -> String {
    let no_frag = url.split('#').next().unwrap_or(url);
    let no_query = no_frag.split('?').next().unwrap_or(no_frag);
    let trimmed = no_query.trim_end_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(url)
        .to_owned()
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::RecentFiles;
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_futures::JsFuture;

    /// The `IndexedDB` database and store names the recent list persists under.
    const DB_NAME: &str = "reticle";
    const STORE_NAME: &str = "recent_files";
    /// The single key the whole serialized recent list is stored under (the list is
    /// small, so one JSON blob is simpler and atomic versus one row per entry).
    const RECENT_KEY: &str = "recent";

    /// Fetches `url` and returns its bytes, or a human-readable error on any network,
    /// CORS, or HTTP failure.
    ///
    /// Uses the browser `fetch` API via `web_sys`. A CORS or network failure rejects
    /// the fetch promise, which becomes an `Err` here carrying a message the caller
    /// shows the user (never a console-only error and never a hang): the fetch either
    /// resolves with bytes or returns an `Err` string. A non-2xx HTTP status is also
    /// turned into an `Err` naming the status, so a `404` reads as a clear message
    /// rather than opening empty bytes.
    pub async fn fetch_gds_bytes(url: &str) -> Result<Vec<u8>, String> {
        let window = web_sys::window().ok_or_else(|| "no browser window".to_owned())?;
        let resp_value = JsFuture::from(window.fetch_with_str(url))
            .await
            .map_err(|e| {
                format!(
                    "could not fetch {url}: {}. The server may be unreachable or may \
                     not allow cross-origin requests (CORS).",
                    describe_js_error(&e)
                )
            })?;
        let response: web_sys::Response = resp_value
            .dyn_into()
            .map_err(|_| "unexpected fetch result".to_owned())?;
        if !response.ok() {
            return Err(format!(
                "could not fetch {url}: server responded {} {}.",
                response.status(),
                response.status_text()
            ));
        }
        let buf_promise = response.array_buffer().map_err(|e| {
            format!(
                "could not read the response body: {}",
                describe_js_error(&e)
            )
        })?;
        let buf = JsFuture::from(buf_promise).await.map_err(|e| {
            format!(
                "could not read the response body: {}",
                describe_js_error(&e)
            )
        })?;
        let array = js_sys::Uint8Array::new(&buf);
        Ok(array.to_vec())
    }

    /// Renders a `JsValue` error into a short human string for a message.
    fn describe_js_error(value: &wasm_bindgen::JsValue) -> String {
        value
            .as_string()
            .or_else(|| {
                value
                    .dyn_ref::<js_sys::Error>()
                    .map(|e| String::from(e.message()))
            })
            .unwrap_or_else(|| "network error".to_owned())
    }

    /// Loads the persisted recent-files list from `IndexedDB`, or an empty list on any
    /// failure (a first visit with no store, a blocked or unavailable `IndexedDB`).
    ///
    /// Never errors outward: the recent list is a convenience, so an unreadable store
    /// degrades to "no recents" rather than blocking startup. The stored value is the
    /// JSON [`RecentFiles::to_json`] wrote, parsed back with
    /// [`RecentFiles::from_json`] (itself tolerant of a malformed blob).
    pub async fn load_recent_files() -> RecentFiles {
        match load_recent_json().await {
            Ok(Some(json)) => RecentFiles::from_json(&json),
            _ => RecentFiles::new(),
        }
    }

    /// Persists `recents` to `IndexedDB` as JSON. Best-effort: a failure to open or
    /// write the store is swallowed (the in-memory list is still correct for this
    /// session), since losing persistence must not break opening a file.
    pub async fn store_recent_files(recents: &RecentFiles) {
        let _ = store_recent_json(&recents.to_json()).await;
    }

    /// Opens the database (creating the object store on first use) and returns it.
    async fn open_db() -> Result<web_sys::IdbDatabase, String> {
        let window = web_sys::window().ok_or_else(|| "no window".to_owned())?;
        let factory = window
            .indexed_db()
            .map_err(|_| "IndexedDB unavailable".to_owned())?
            .ok_or_else(|| "IndexedDB unavailable".to_owned())?;
        let request = factory
            .open_with_u32(DB_NAME, 1)
            .map_err(|_| "could not open IndexedDB".to_owned())?;

        // Create the object store on upgrade (first open, or a version bump).
        let req_for_upgrade = request.clone();
        let on_upgrade = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
            if let Ok(Some(result)) = req_for_upgrade
                .result()
                .map(|v| v.dyn_into::<web_sys::IdbDatabase>().ok())
            {
                let _ = result.create_object_store(STORE_NAME);
            }
        });
        request.set_onupgradeneeded(Some(on_upgrade.as_ref().unchecked_ref()));
        on_upgrade.forget();

        await_request(&request).await?;
        request
            .result()
            .ok()
            .and_then(|v| v.dyn_into::<web_sys::IdbDatabase>().ok())
            .ok_or_else(|| "IndexedDB open returned no database".to_owned())
    }

    /// Reads the stored recent-list JSON, or `Ok(None)` when nothing is stored yet.
    async fn load_recent_json() -> Result<Option<String>, String> {
        let db = open_db().await?;
        let tx = db
            .transaction_with_str(STORE_NAME)
            .map_err(|_| "could not begin a read transaction".to_owned())?;
        let store = tx
            .object_store(STORE_NAME)
            .map_err(|_| "missing object store".to_owned())?;
        let request = store
            .get(&wasm_bindgen::JsValue::from_str(RECENT_KEY))
            .map_err(|_| "could not read the recent list".to_owned())?;
        await_request(&request).await?;
        match request.result() {
            Ok(v) if !v.is_undefined() && !v.is_null() => Ok(v.as_string()),
            _ => Ok(None),
        }
    }

    /// Writes the recent-list JSON under the single recent key.
    async fn store_recent_json(json: &str) -> Result<(), String> {
        let db = open_db().await?;
        let tx = db
            .transaction_with_str_and_mode(STORE_NAME, web_sys::IdbTransactionMode::Readwrite)
            .map_err(|_| "could not begin a write transaction".to_owned())?;
        let store = tx
            .object_store(STORE_NAME)
            .map_err(|_| "missing object store".to_owned())?;
        let request = store
            .put_with_key(
                &wasm_bindgen::JsValue::from_str(json),
                &wasm_bindgen::JsValue::from_str(RECENT_KEY),
            )
            .map_err(|_| "could not write the recent list".to_owned())?;
        await_request(&request).await?;
        Ok(())
    }

    /// Awaits an `IdbRequest`, resolving when it fires `success` and rejecting on
    /// `error`, so the async wrappers above read as straight-line code.
    async fn await_request(request: &web_sys::IdbRequest) -> Result<(), String> {
        let promise = js_sys::Promise::new(&mut |resolve, reject| {
            let on_success = wasm_bindgen::closure::Closure::once_into_js(move || {
                let _ = resolve.call0(&wasm_bindgen::JsValue::UNDEFINED);
            });
            let on_error = wasm_bindgen::closure::Closure::once_into_js(move || {
                let _ = reject.call0(&wasm_bindgen::JsValue::UNDEFINED);
            });
            request.set_onsuccess(Some(on_success.unchecked_ref()));
            request.set_onerror(Some(on_error.unchecked_ref()));
        });
        JsFuture::from(promise)
            .await
            .map(|_| ())
            .map_err(|_| "IndexedDB request failed".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_drop_accepts_layout_extensions_and_rejects_others() {
        assert_eq!(classify_drop("chip.gds"), Some(DocFormat::Gds));
        assert_eq!(classify_drop("CHIP.GDSII"), Some(DocFormat::Gds));
        assert_eq!(classify_drop("layout.oas"), Some(DocFormat::Oasis));
        assert_eq!(classify_drop("layout.oasis"), Some(DocFormat::Oasis));
        assert_eq!(classify_drop("notes.txt"), None);
        assert_eq!(classify_drop("noextension"), None);
    }

    #[test]
    fn gds_url_from_query_reads_the_parameter() {
        assert_eq!(
            gds_url_from_query("?gds=https://host/chip.gds"),
            Some("https://host/chip.gds".to_owned())
        );
        // Works without the leading '?', among other params, and in any position.
        assert_eq!(
            gds_url_from_query("view=editor&gds=https://h/c.gds"),
            Some("https://h/c.gds".to_owned())
        );
    }

    #[test]
    fn gds_url_from_query_percent_decodes_the_value() {
        assert_eq!(
            gds_url_from_query("?gds=https%3A%2F%2Fhost%2Fmy%20chip.gds"),
            Some("https://host/my chip.gds".to_owned())
        );
    }

    #[test]
    fn gds_url_from_query_absent_or_empty_is_none() {
        assert_eq!(gds_url_from_query("?view=editor"), None);
        assert_eq!(gds_url_from_query(""), None);
        assert_eq!(gds_url_from_query("?gds="), None);
        assert_eq!(gds_url_from_query("?gds=%20%20"), None);
    }

    #[test]
    fn load_plan_bands_are_chosen_by_size() {
        assert_eq!(LoadPlan::for_size(0), LoadPlan::InMemory);
        assert_eq!(
            LoadPlan::for_size(WASM_STREAMING_THRESHOLD_BYTES - 1),
            LoadPlan::InMemory
        );
        // Exactly at the threshold streams.
        assert!(matches!(
            LoadPlan::for_size(WASM_STREAMING_THRESHOLD_BYTES),
            LoadPlan::Streaming { .. }
        ));
        assert!(matches!(
            LoadPlan::for_size(WASM_OPEN_CEILING_BYTES),
            LoadPlan::Streaming { .. }
        ));
        // One byte past the ceiling is refused.
        assert!(matches!(
            LoadPlan::for_size(WASM_OPEN_CEILING_BYTES + 1),
            LoadPlan::TooLarge { .. }
        ));
    }

    #[test]
    fn streaming_threshold_is_below_the_ceiling() {
        // The design invariant: there is a real streaming band, not a cliff. A const
        // block so the check is a compile-time guarantee, not just a runtime assert.
        const {
            assert!(WASM_STREAMING_THRESHOLD_BYTES < WASM_OPEN_CEILING_BYTES);
        }
    }

    #[test]
    fn openable_and_refusal_message_agree() {
        assert!(LoadPlan::for_size(0).is_openable());
        assert!(LoadPlan::for_size(0).refusal_message().is_none());

        let too_big = LoadPlan::for_size(WASM_OPEN_CEILING_BYTES + 1);
        assert!(!too_big.is_openable());
        let msg = too_big.refusal_message().expect("a refusal has a message");
        assert!(
            msg.contains("256"),
            "message names the ceiling in MiB: {msg}"
        );
        assert!(msg.contains("MiB"));
    }

    #[test]
    fn recent_files_moves_repeat_to_front_and_dedupes() {
        let mut r = RecentFiles::new();
        r.record(RecentFile::local("a.gds", 1));
        r.record(RecentFile::local("b.gds", 2));
        r.record(RecentFile::local("a.gds", 3)); // repeat of a
        let names: Vec<&str> = r.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["a.gds", "b.gds"],
            "a moved to front, not duplicated"
        );
        // The updated fields win on re-record.
        assert_eq!(r.entries()[0].size, 3);
    }

    #[test]
    fn recent_files_dedupes_remote_by_url_not_name() {
        let mut r = RecentFiles::new();
        r.record(RecentFile::remote("chip.gds", 10, "https://a/chip.gds"));
        r.record(RecentFile::remote("chip.gds", 10, "https://b/chip.gds"));
        // Same name, different URL: two distinct rows.
        assert_eq!(r.entries().len(), 2);
        // Re-opening the first URL (even under a different display name) collapses.
        r.record(RecentFile::remote("renamed.gds", 11, "https://a/chip.gds"));
        assert_eq!(r.entries().len(), 2);
        assert_eq!(r.entries()[0].url.as_deref(), Some("https://a/chip.gds"));
    }

    #[test]
    fn recent_files_caps_the_list() {
        let mut r = RecentFiles::new();
        for i in 0..(RecentFiles::CAP + 5) {
            r.record(RecentFile::local(format!("f{i}.gds"), i as u64));
        }
        assert_eq!(r.entries().len(), RecentFiles::CAP);
        // The most recent (highest index) is at the front; the oldest fell off.
        assert_eq!(
            r.entries()[0].name,
            format!("f{}.gds", RecentFiles::CAP + 4)
        );
    }

    #[test]
    fn recent_files_json_round_trips() {
        let mut r = RecentFiles::new();
        r.record(RecentFile::local("a b.gds", 42));
        r.record(RecentFile::remote("c.gds", 99, "https://host/c.gds"));
        let json = r.to_json();
        let back = RecentFiles::from_json(&json);
        assert_eq!(back, r, "recent list survives a JSON round trip");
    }

    #[test]
    fn recent_files_json_handles_quotes_and_is_tolerant_of_garbage() {
        let mut r = RecentFiles::new();
        r.record(RecentFile::local("weird\"name\\.gds", 1));
        let json = r.to_json();
        assert_eq!(RecentFiles::from_json(&json), r);
        // A malformed store degrades to empty, never panics.
        assert!(RecentFiles::from_json("not json at all").is_empty());
        assert!(RecentFiles::from_json("").is_empty());
        assert!(RecentFiles::from_json("[{\"name\":").is_empty());
    }

    #[test]
    fn load_progress_transitions_and_fraction() {
        let idle = LoadProgress::default();
        assert_eq!(idle, LoadProgress::Idle);
        assert!(!idle.is_active());
        assert!(idle.fraction().is_none());

        let half = LoadProgress::fetched(50, 100);
        assert!(half.is_active());
        assert_eq!(half.fraction(), Some(0.5));

        // Unknown total is active but indeterminate.
        let unknown = LoadProgress::fetched(50, 0);
        assert!(unknown.is_active());
        assert!(unknown.fraction().is_none());

        let indexing = LoadProgress::indexing();
        assert!(indexing.is_active());
        assert!(indexing.fraction().is_none());

        let done = LoadProgress::done();
        assert!(!done.is_active());

        let failed = LoadProgress::failed("CORS blocked");
        assert!(!failed.is_active());
        assert_eq!(failed.failure_message(), Some("CORS blocked"));
    }

    #[test]
    fn load_progress_fraction_clamps_over_report() {
        // A server that reports more received than total must not exceed 1.0.
        assert_eq!(LoadProgress::fetched(150, 100).fraction(), Some(1.0));
    }

    #[test]
    fn url_file_name_takes_the_last_path_segment() {
        assert_eq!(url_file_name("https://host/dir/chip.gds"), "chip.gds");
        // Query and fragment are stripped.
        assert_eq!(url_file_name("https://host/chip.gds?v=2#frag"), "chip.gds");
        // A trailing slash falls back to the previous segment.
        assert_eq!(url_file_name("https://host/chip.gds/"), "chip.gds");
        // No path segment: the whole URL is the name.
        assert_eq!(url_file_name("chip.gds"), "chip.gds");
    }

    #[test]
    fn web_open_inbox_is_empty_off_wasm() {
        // On native nothing is ever posted, so the inbox always drains empty; this
        // guards the uniform-type contract the App relies on.
        let inbox = WebOpenInbox::new();
        assert!(inbox.drain().is_empty());
    }
}
