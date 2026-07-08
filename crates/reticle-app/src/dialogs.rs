//! Pure models behind the open-related dialogs: Open-from-URL validation and the
//! CORS explainer (catalog item 2), the corrupt/unsupported-file explanation
//! (item 8), and the staged open progress (item 6).
//!
//! Like [`crate::notify`] and [`crate::webopen`], this module is deliberately
//! window-free: it is the *decision and wording* layer (is this URL openable, what
//! does a CORS failure mean in plain language, which formats do we accept, how far
//! along is a staged open) with no `egui`, no GPU, and no network. The dialog glue
//! in [`crate::app`] renders these through `theme::components` and drives the async
//! fetch; every branch a user can hit (a bad URL, a blocked fetch, a corrupt file)
//! is unit-tested here in plain code.
//!
//! The wording routes through [`Diagnostic`] so a failure
//! carries the same cause / next-step / copyable-block shape the unified toast
//! system uses everywhere else (item 72).

use crate::notify::Diagnostic;
use crate::open::{DocFormat, OpenError};

/// The layout formats this build opens, phrased for the "supported formats" line an
/// unsupported-file explainer shows (item 8). Data, not chrome, so it is shared by
/// the drop affordance, the Open-from-URL hint, and the corrupt-file dialog.
pub const SUPPORTED_FORMATS: &[&str] = &["GDSII (.gds, .gdsii, .gds2)", "OASIS (.oas, .oasis)"];

/// Why a user-entered Open-from-URL string was rejected before any fetch (item 2).
///
/// Surfaced inline under the field so the user fixes the URL without a round trip to
/// the network. Ordered from most to least fundamental so [`validate_open_url`] can
/// report the first thing wrong.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UrlError {
    /// The field is empty (or whitespace only).
    Empty,
    /// The scheme is missing or is not `http`/`https` (a `file:` or bare host).
    NotHttp,
    /// There is a scheme but no host after it.
    NoHost,
    /// The URL does not name a `.gds`/`.oas` file, so we cannot pick an importer.
    NotLayout,
}

impl UrlError {
    /// A one-line, human-readable reason to show under the field.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            UrlError::Empty => "Enter a URL to a .gds or .oas file.",
            UrlError::NotHttp => "The URL must start with http:// or https://.",
            UrlError::NoHost => "The URL is missing a host, e.g. https://example.com/chip.gds.",
            UrlError::NotLayout => {
                "The URL must point at a .gds or .oas file so Reticle knows how to read it."
            }
        }
    }
}

/// Validates a user-entered Open-from-URL string before any fetch (item 2).
///
/// Requires an `http`/`https` scheme, a non-empty host, and a recognized layout
/// extension on the file name (query and fragment are ignored for the extension
/// check, matching the `?gds=` path). Returns the trimmed URL ready to fetch, or the
/// first [`UrlError`] that applies so the dialog can explain it inline.
///
/// # Errors
///
/// Returns [`UrlError`] describing why the string is not an openable layout URL.
pub fn validate_open_url(input: &str) -> Result<String, UrlError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(UrlError::Empty);
    }
    let rest = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"));
    let Some(rest) = rest else {
        return Err(UrlError::NotHttp);
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() {
        return Err(UrlError::NoHost);
    }
    if DocFormat::from_extension(&crate::webopen::url_file_name(trimmed)).is_none() {
        return Err(UrlError::NotLayout);
    }
    Ok(trimmed.to_owned())
}

/// A plain-language explanation of a cross-origin (CORS) fetch failure for `url`,
/// as a [`Diagnostic`] (item 2).
///
/// A browser blocks a cross-origin `fetch` unless the server opts in with CORS
/// headers, and the failure it reports to the page is deliberately opaque (no status,
/// no body) for security. That opacity is exactly what confuses a user ("the file is
/// right there"), so this spells out the cause and the concrete next steps rather
/// than surfacing a bare "`TypeError`: Failed to fetch".
#[must_use]
pub fn cors_diagnostic(url: &str) -> Diagnostic {
    Diagnostic::new(
        "The browser blocked the download because the server did not allow a \
         cross-origin request (CORS). Browsers hide the real error for security, so \
         the file may exist and still fail to load this way.",
        "Host the file where cross-origin reads are allowed (an \
         Access-Control-Allow-Origin header), open it from the same site as Reticle, \
         or download it and drag the file onto the window instead.",
        format!("fetch {url}\nblocked by: cross-origin resource sharing (CORS) policy"),
    )
}

/// A [`Diagnostic`] for a network or HTTP failure fetching `url` that is not
/// specifically a CORS block (a 404, a DNS failure, an offline network). `detail` is
/// the transport's own message.
#[must_use]
pub fn fetch_failure_diagnostic(url: &str, detail: &str) -> Diagnostic {
    Diagnostic::new(
        "The file could not be downloaded.",
        "Check that the link is correct and reachable, then try again. If the file is \
         on your computer, drag it onto the window instead.",
        format!("fetch {url}\n{detail}"),
    )
}

/// A [`Diagnostic`] for a dropped or named file whose extension is not a layout
/// format this build opens (item 8).
///
/// Names what was refused, lists the supported formats, and suggests the convert path
/// for a foreign format, so an unsupported drop is a clear next step rather than a
/// silently ignored gesture.
#[must_use]
pub fn unsupported_file_diagnostic(name: &str) -> Diagnostic {
    Diagnostic::new(
        format!("\"{name}\" is not a layout format Reticle can open."),
        format!(
            "Open one of: {}. If you have a different format, convert it to GDSII or \
             OASIS first, then open the result.",
            SUPPORTED_FORMATS.join(", ")
        ),
        format!("rejected file: {name}\nreason: unrecognized extension"),
    )
}

/// A [`Diagnostic`] for a hard [`OpenError`] from the import seam: a corrupt file, the
/// wrong format for the extension, or an empty document (item 8).
///
/// The cause is the seam's own message (already phrased for a person); the next step
/// points at the supported formats and the convert path, so a corrupt or foreign file
/// gets the same "here is what I accept, here is how to get there" guidance as an
/// unsupported extension.
#[must_use]
pub fn open_error_diagnostic(err: &OpenError) -> Diagnostic {
    let details = match err {
        OpenError::Import { format, reason } => {
            format!("format tried: {}\nreason: {reason}", format.label())
        }
        OpenError::Empty { format } => {
            format!(
                "format: {}\nreason: the file parsed but contains no cells",
                format.label()
            )
        }
    };
    Diagnostic::new(
        err.to_string(),
        format!(
            "Reticle opens {}. If the file is another format or was exported by an \
             unusual tool, convert it to GDSII or OASIS and open the result.",
            SUPPORTED_FORMATS.join(" and ")
        ),
        details,
    )
}

/// The staged progress of opening a document (item 6): parse the bytes, tessellate
/// the geometry, upload it to the GPU, then done.
///
/// The synchronous import seam ([`crate::open::open_document_bytes`]) does parse and
/// build in one call, and the first paint uploads to the GPU, so these stages are the
/// honest phases a user waits through; the model lets the app show which phase is
/// running and a determinate bar, and lets a long streaming load be canceled between
/// phases (a small in-memory open runs straight through to [`Done`](OpenStage::Done)).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpenStage {
    /// Reading and importing the file bytes into a document.
    Parse,
    /// Tessellating the geometry into renderable meshes.
    Tessellate,
    /// Uploading the meshes to the GPU for the first frame.
    Upload,
    /// The document is open and framed.
    Done,
}

impl OpenStage {
    /// The first stage of an open.
    #[must_use]
    pub fn first() -> Self {
        OpenStage::Parse
    }

    /// A short label for the current phase, for the progress toast.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            OpenStage::Parse => "Reading the file",
            OpenStage::Tessellate => "Preparing the geometry",
            OpenStage::Upload => "Uploading to the GPU",
            OpenStage::Done => "Done",
        }
    }

    /// The determinate progress fraction (0.0..=1.0) at the start of this stage, so a
    /// bar advances as the open moves through the phases.
    #[must_use]
    pub fn fraction(self) -> f32 {
        match self {
            OpenStage::Parse => 0.1,
            OpenStage::Tessellate => 0.5,
            OpenStage::Upload => 0.85,
            OpenStage::Done => 1.0,
        }
    }

    /// The next stage, or [`Done`](OpenStage::Done) once complete (which stays put).
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            OpenStage::Parse => OpenStage::Tessellate,
            OpenStage::Tessellate => OpenStage::Upload,
            OpenStage::Upload | OpenStage::Done => OpenStage::Done,
        }
    }

    /// Whether the open has finished.
    #[must_use]
    pub fn is_done(self) -> bool {
        matches!(self, OpenStage::Done)
    }
}

/// The open/closed and text state of the lane's dialogs, folded into one field on
/// the App so the many boolean flags do not sprawl across the struct.
///
/// Pure UI state (no `egui`): which dialog is showing, the Open-from-URL text, and
/// whether the Share dialog is offering the view-only or the editable link. The
/// dialog glue in [`crate::app`] reads and mutates these each frame.
// Independent one-bit UI facts (which dialog is open, which link is selected);
// folding them into enums would only add indirection to the glue layer.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Default)]
pub struct DialogState {
    /// Whether the Open-from-URL dialog is showing (item 2).
    pub open_url_shown: bool,
    /// The URL the user is typing into the Open-from-URL field.
    pub open_url_text: String,
    /// Whether the Convert-to-archive dialog is showing (`file.convert_gds`).
    pub convert_shown: bool,
    /// Whether the Share dialog is showing (item 85).
    pub share_shown: bool,
    /// In the Share dialog, whether the view-only link is selected (`true`) or the
    /// full editable link (`false`).
    pub share_view_only: bool,
}

impl DialogState {
    /// Whether any modal-style dialog this lane owns is currently showing (so the app
    /// can, for example, suppress a shortcut that would collide).
    #[must_use]
    pub fn any_open(&self) -> bool {
        self.open_url_shown || self.convert_shown || self.share_shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_state_reports_when_any_dialog_is_open() {
        let mut d = DialogState::default();
        assert!(!d.any_open());
        d.share_shown = true;
        assert!(d.any_open());
    }

    #[test]
    fn validate_open_url_accepts_a_well_formed_layout_url() {
        assert_eq!(
            validate_open_url("  https://host/dir/chip.gds  "),
            Ok("https://host/dir/chip.gds".to_owned())
        );
        // http is allowed, and a query/fragment does not defeat the extension check.
        assert_eq!(
            validate_open_url("http://h/c.oas?v=2#f"),
            Ok("http://h/c.oas?v=2#f".to_owned())
        );
    }

    #[test]
    fn validate_open_url_rejects_with_the_first_applicable_reason() {
        assert_eq!(validate_open_url("   "), Err(UrlError::Empty));
        assert_eq!(validate_open_url("ftp://h/c.gds"), Err(UrlError::NotHttp));
        assert_eq!(validate_open_url("host/chip.gds"), Err(UrlError::NotHttp));
        assert_eq!(
            validate_open_url("https:///chip.gds"),
            Err(UrlError::NoHost)
        );
        assert_eq!(
            validate_open_url("https://host/notes.txt"),
            Err(UrlError::NotLayout)
        );
        // Every reason has a non-empty, em-dash-free message.
        for e in [
            UrlError::Empty,
            UrlError::NotHttp,
            UrlError::NoHost,
            UrlError::NotLayout,
        ] {
            assert!(!e.message().is_empty());
            assert!(!e.message().contains('\u{2014}'));
        }
    }

    #[test]
    fn cors_diagnostic_explains_the_block_and_offers_a_next_step() {
        let d = cors_diagnostic("https://host/chip.gds");
        assert!(d.cause.to_lowercase().contains("cors"));
        assert!(
            d.next_step.to_lowercase().contains("drag"),
            "offers the drag fallback"
        );
        assert!(d.details.contains("https://host/chip.gds"));
        assert!(!d.clipboard_text().contains('\u{2014}'));
    }

    #[test]
    fn unsupported_file_diagnostic_lists_formats_and_suggests_convert() {
        let d = unsupported_file_diagnostic("notes.txt");
        assert!(d.cause.contains("notes.txt"));
        assert!(d.next_step.contains("GDSII"));
        assert!(d.next_step.to_lowercase().contains("convert"));
    }

    #[test]
    fn open_error_diagnostic_carries_cause_and_convert_suggestion() {
        let err = OpenError::Import {
            format: DocFormat::Gds,
            reason: "unexpected end of stream".to_owned(),
        };
        let d = open_error_diagnostic(&err);
        assert!(d.cause.contains("unexpected end of stream"));
        assert!(d.next_step.to_lowercase().contains("convert"));
        assert!(d.details.contains("GDSII"));

        let empty = OpenError::Empty {
            format: DocFormat::Oasis,
        };
        let d2 = open_error_diagnostic(&empty);
        assert!(d2.cause.to_lowercase().contains("no cells"));
    }

    #[test]
    fn open_stage_advances_through_the_phases_with_rising_fraction() {
        let mut s = OpenStage::first();
        assert_eq!(s, OpenStage::Parse);
        let mut last = -1.0_f32;
        let mut seen = Vec::new();
        loop {
            assert!(s.fraction() > last, "fraction strictly rises: {s:?}");
            last = s.fraction();
            seen.push(s);
            if s.is_done() {
                break;
            }
            s = s.next();
        }
        assert_eq!(
            seen,
            vec![
                OpenStage::Parse,
                OpenStage::Tessellate,
                OpenStage::Upload,
                OpenStage::Done
            ]
        );
        assert!((OpenStage::Done.fraction() - 1.0).abs() < f32::EPSILON);
        // `next` on Done is a fixed point (no wraparound).
        assert_eq!(OpenStage::Done.next(), OpenStage::Done);
    }

    #[test]
    fn open_stage_labels_are_present_and_clean() {
        for s in [
            OpenStage::Parse,
            OpenStage::Tessellate,
            OpenStage::Upload,
            OpenStage::Done,
        ] {
            assert!(!s.label().is_empty());
            assert!(!s.label().contains('\u{2014}'));
        }
    }
}
