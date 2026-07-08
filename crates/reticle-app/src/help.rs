//! Help-menu content that is data rather than layout: the changelog behind the
//! "What's new" dialog (catalog 26), the diagnostics block and prefilled issue link
//! behind the About dialog (catalog 99), and the honest zero-telemetry statement
//! (catalog 100).
//!
//! Everything here is **pure and egui-free** and unit-tested. The dialogs
//! themselves (rendering the changelog, the About card, the copy button) live in
//! [`crate::app`], which reads this module for the copy and the assembled strings.
//!
//! ## Zero telemetry, verified
//!
//! [`ZERO_TELEMETRY`] is a stated feature, not marketing: the codebase contains no
//! analytics or telemetry SDK and issues no background beacons. The only network
//! traffic is user-initiated: fetching a file the user opens by URL, the `IndexedDB`
//! recent-files cache, and the live-share relay socket the user starts. The About
//! dialog surfaces this claim; it is true against the tree as of this lane.

/// The project repository, linked from the About dialog and the base of the
/// prefilled issue link.
pub const REPO_URL: &str = "https://github.com/AlpharomeroJL/reticle";

/// Where the "Documentation" Help item points. The book lives in `docs/` in the
/// repository; this is the browsable rendered tree.
pub const DOCS_URL: &str = "https://github.com/AlpharomeroJL/reticle/tree/main/docs";

/// The zero-telemetry statement surfaced in the About dialog (catalog 100).
///
/// Verified true against the codebase: no analytics or telemetry is collected or
/// sent. Network access is limited to files you open, the recent-files cache in
/// your browser, and live-share sessions you start.
pub const ZERO_TELEMETRY: &str = "Reticle collects no telemetry and no analytics. \
    Nothing about your designs or your usage is sent anywhere. The only network \
    traffic is loading files you open, caching recent files in your own browser, \
    and the live-share sessions you choose to start.";

/// One released version and the highlights shipped in it, shown in the "What's new"
/// dialog. Ordered newest-first by [`changelog`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Release {
    /// The version string, e.g. `"8.1"`.
    pub version: &'static str,
    /// The release date as `YYYY-MM-DD`.
    pub date: &'static str,
    /// A one-line headline for the release.
    pub headline: &'static str,
    /// The bulleted highlights, each a short sentence.
    pub notes: &'static [&'static str],
}

/// The embedded changelog, newest release first.
///
/// This is the changelog *data* the "What's new" dialog renders (catalog 26); it is
/// compiled in so it is available identically on native and the web with no file IO.
static CHANGELOG: &[Release] = &[
    Release {
        version: "8.1",
        date: "2026-07-08",
        headline: "A rebuilt interface, onboarding, and help.",
        notes: &[
            "A menu bar and command palette reach every action, with live shortcut hints.",
            "A guided tour with editor and viewer variants, resumable and skippable.",
            "Once-only hints on the layers, DRC, and share surfaces the first time you reach them.",
            "A Settings dialog for density, reduced motion, wheel behavior, and touch mode.",
            "An About dialog with diagnostics you can copy and a prefilled issue link.",
        ],
    },
    Release {
        version: "8.0",
        date: "2026-04-01",
        headline: "Streaming archives, live share, and the agent.",
        notes: &[
            "Browse gigabyte archives streamed tile by tile with progressive residency.",
            "Share a session by link and watch collaborators' cursors live.",
            "Run a scripted agent edit session and replay any recorded run.",
        ],
    },
];

/// The full changelog, newest release first.
#[must_use]
pub fn changelog() -> &'static [Release] {
    CHANGELOG
}

/// The most recent release (the head of the changelog).
#[must_use]
pub fn latest() -> &'static Release {
    &CHANGELOG[0]
}

/// The runtime facts the About dialog reports and copies.
///
/// Borrowed strings so the app can assemble them from `env!` constants and the live
/// GPU adapter without allocating until [`Diagnostics::report`] builds the block.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Diagnostics<'a> {
    /// The application version (`CARGO_PKG_VERSION`).
    pub app_version: &'a str,
    /// The build/bundle hash, or a stand-in for an unversioned local build.
    pub bundle_hash: &'a str,
    /// The platform label, e.g. `"web (wasm32)"` or `"native"`.
    pub platform: &'a str,
    /// The GPU adapter name reported by wgpu, or a stand-in when none is known.
    pub gpu_adapter: &'a str,
    /// The GPU backend, e.g. `"Vulkan"`, `"Metal"`, `"WebGPU"`, or `"unknown"`.
    pub gpu_backend: &'a str,
}

impl Diagnostics<'_> {
    /// Assembles the multi-line diagnostics block shown in About and copied by the
    /// one-click copy button. Stable field order so it is easy to read in an issue.
    #[must_use]
    pub fn report(&self) -> String {
        format!(
            "Reticle {version} ({hash})\n\
             Platform: {platform}\n\
             GPU: {adapter} ({backend})",
            version = self.app_version,
            hash = self.bundle_hash,
            platform = self.platform,
            adapter = self.gpu_adapter,
            backend = self.gpu_backend,
        )
    }
}

/// A prefilled "new issue" URL for the repository, with `report` pasted into the
/// body so a bug report arrives with the environment already attached.
#[must_use]
pub fn issue_url(report: &str) -> String {
    let title = percent_encode("Reticle issue: ");
    let body = percent_encode(&format!(
        "Describe the problem here.\n\n---\nEnvironment:\n{report}\n"
    ));
    format!("{REPO_URL}/issues/new?title={title}&body={body}")
}

/// Percent-encodes `input` for use in a URL query value.
///
/// Conservative on purpose: it passes the RFC 3986 unreserved set through and
/// encodes everything else (including spaces as `%20`), so the result is safe in a
/// query string on every browser without depending on a URL crate.
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(hex_digit(byte >> 4));
            out.push(hex_digit(byte & 0x0f));
        }
    }
    out
}

/// The uppercase hex digit for a nibble `0..=15`.
fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'A' + (nibble - 10)) as char,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changelog_is_newest_first_and_populated() {
        let log = changelog();
        assert!(!log.is_empty());
        assert_eq!(latest().version, log[0].version);
        for release in log {
            assert!(!release.version.is_empty());
            assert!(!release.headline.is_empty());
            assert!(!release.notes.is_empty());
            // The style gate: no em dash in shipped copy.
            assert!(!release.headline.contains('\u{2014}'));
            for note in release.notes {
                assert!(!note.is_empty());
                assert!(!note.contains('\u{2014}'));
            }
        }
    }

    #[test]
    fn diagnostics_report_lists_every_field() {
        let diag = Diagnostics {
            app_version: "8.1.0",
            bundle_hash: "abc123",
            platform: "native",
            gpu_adapter: "Test Adapter",
            gpu_backend: "Vulkan",
        };
        let report = diag.report();
        assert!(report.contains("8.1.0"));
        assert!(report.contains("abc123"));
        assert!(report.contains("native"));
        assert!(report.contains("Test Adapter"));
        assert!(report.contains("Vulkan"));
    }

    #[test]
    fn issue_url_embeds_the_repo_and_encodes_the_body() {
        let url = issue_url("Reticle 8.1 (dev)\nGPU: none");
        assert!(url.starts_with(REPO_URL), "points at the repository");
        assert!(url.contains("/issues/new?"));
        // The body is percent-encoded: spaces and newlines never appear raw.
        let query = url.split_once('?').unwrap().1;
        assert!(!query.contains(' '));
        assert!(!query.contains('\n'));
        // A space encodes as %20 and a newline as %0A.
        assert!(url.contains("%20"));
        assert!(url.contains("%0A"));
    }

    #[test]
    fn percent_encode_passes_unreserved_and_escapes_the_rest() {
        assert_eq!(percent_encode("aZ0-_.~"), "aZ0-_.~");
        assert_eq!(percent_encode("a b/c"), "a%20b%2Fc");
        assert_eq!(percent_encode("\n"), "%0A");
    }

    #[test]
    fn zero_telemetry_statement_is_present_and_clean() {
        assert!(ZERO_TELEMETRY.contains("no telemetry") || ZERO_TELEMETRY.contains("no analytics"));
        assert!(!ZERO_TELEMETRY.contains('\u{2014}'));
    }
}
