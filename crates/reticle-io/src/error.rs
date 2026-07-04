//! The crate-local error type and its bridge to [`reticle_model::ModelError`].
//!
//! The public [`Importer`](reticle_model::Importer)/[`Exporter`](reticle_model::Exporter)
//! traits and [`parse_technology`](crate::parse_technology) return
//! [`reticle_model::Result`], whose error is the frozen [`reticle_model::ModelError`].
//! That enum is `#[non_exhaustive]` and its free-form variant,
//! [`ModelError::Unsupported`](reticle_model::ModelError::Unsupported), only
//! carries a `&'static str`, so it cannot hold the dynamic detail a parser wants
//! to report.
//!
//! [`IoError`] is this crate's richer, owned error type. It captures the specific
//! failure (which GDS record, which technology-file line) for diagnostics, and
//! lowers into a stable [`ModelError`](reticle_model::ModelError) category via
//! [`From`]. Callers that want the full message can downcast or format an
//! [`IoError`] before it is lowered; callers on the frozen trait boundary still
//! get a well-formed `ModelError`.

use core::fmt;

/// An import/export or technology-parse error with full diagnostic detail.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IoError {
    /// The GDSII byte stream was malformed. Carries a static category note.
    Malformed(&'static str),
    /// The underlying `gds21` parser or writer reported an error (owned text,
    /// since `gds21`'s error type is not `Clone`).
    Gds(String),
    /// A technology-file line could not be parsed.
    Technology {
        /// 1-based line number in the source.
        line: usize,
        /// Human-readable reason.
        reason: String,
    },
    /// A format or record that this crate does not yet support (honest coverage
    /// gap). Carries a static description of the gap.
    Unsupported(&'static str),
}

impl IoError {
    /// Wraps a `gds21` error, capturing its `Display` text.
    pub(crate) fn gds(e: &gds21::GdsError) -> Self {
        Self::Gds(e.to_string())
    }

    /// Builds a technology-parse error for a given 1-based line.
    pub(crate) fn tech(line: usize, reason: impl Into<String>) -> Self {
        Self::Technology {
            line,
            reason: reason.into(),
        }
    }
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(why) => write!(f, "malformed input: {why}"),
            Self::Gds(msg) => write!(f, "gds21 error: {msg}"),
            Self::Technology { line, reason } => {
                write!(f, "technology file error on line {line}: {reason}")
            }
            Self::Unsupported(what) => write!(f, "unsupported: {what}"),
        }
    }
}

impl core::error::Error for IoError {}

/// A non-fatal problem found while importing an otherwise-usable document.
///
/// Import surfaces come in two flavours. A hard failure (no HEADER record, a
/// record length that does not decode, a parser panic) is an [`IoError`] and the
/// import returns `Err`. A *recoverable* problem (a boundary with too few
/// vertices to be a polygon, a degenerate zero-area rectangle, an element count
/// so large the import was capped) does not stop the import: the offending piece
/// is skipped or clamped and the rest of the document is still returned, with one
/// [`ImportWarning`] recorded per problem so a caller can show the user what was
/// dropped rather than silently losing it.
///
/// A warning is deliberately plain data: a machine-friendly [`kind`](ImportWarning::kind)
/// for grouping, plus a one-line human [`summary`](ImportWarning::summary) and a
/// longer [`detail`](ImportWarning::detail). It carries no borrowed data so it can
/// cross crate and thread boundaries freely (the app maps it straight onto its own
/// `OpenWarning`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportWarning {
    /// The category of problem, for grouping and filtering.
    pub kind: WarningKind,
    /// A short, human-readable one-liner naming what happened.
    pub summary: String,
    /// A longer explanation: which cell/element, what was expected, what was done
    /// instead (skipped, clamped, defaulted).
    pub detail: String,
}

impl ImportWarning {
    /// Builds a warning from its category, summary, and detail.
    pub(crate) fn new(
        kind: WarningKind,
        summary: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            summary: summary.into(),
            detail: detail.into(),
        }
    }
}

impl fmt::Display for ImportWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.summary, self.detail)
    }
}

/// The category of an [`ImportWarning`].
///
/// Kept small and stable so callers can group or count warnings without parsing
/// free text. `#[non_exhaustive]` because new recoverable checks may be added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WarningKind {
    /// A shape or element was skipped because its geometry was degenerate
    /// (too few vertices, zero area, or otherwise not representable).
    DegenerateGeometry,
    /// A structural limit was hit and the import was capped (for example an
    /// element or vertex count beyond the guard ceiling), so some content past
    /// the cap was not imported.
    LimitExceeded,
    /// A value was out of the range the model can hold and was clamped or
    /// defaulted (for example a magnification that did not fit).
    ValueClamped,
    /// A well-formed record carried a feature this importer does not model, so it
    /// was ignored while the rest of the element imported.
    UnsupportedFeature,
}

impl WarningKind {
    /// A stable, lowercase label for this kind, for logs and grouping.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::DegenerateGeometry => "degenerate-geometry",
            Self::LimitExceeded => "limit-exceeded",
            Self::ValueClamped => "value-clamped",
            Self::UnsupportedFeature => "unsupported-feature",
        }
    }
}

impl From<IoError> for reticle_model::ModelError {
    /// Lowers a rich [`IoError`] onto the frozen [`reticle_model::ModelError`]
    /// surface. Dynamic detail collapses to a stable static category so the
    /// mapping is total without touching the `#[non_exhaustive]` model enum.
    fn from(e: IoError) -> Self {
        match e {
            IoError::Malformed(_) | IoError::Gds(_) => {
                reticle_model::ModelError::Unsupported("malformed or unreadable input")
            }
            IoError::Technology { .. } => {
                reticle_model::ModelError::Unsupported("invalid technology file")
            }
            IoError::Unsupported(_) => {
                reticle_model::ModelError::Unsupported("unsupported format feature")
            }
        }
    }
}
