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
