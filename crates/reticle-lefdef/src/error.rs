//! The crate error type and non-fatal warnings.
//!
//! LEF and DEF are untrusted text. A structural failure that stops the import (an
//! oversized input, a number that does not parse, a statement that ends before its
//! required tokens) is a [`LefDefError`]. A *recoverable* problem (an unknown
//! keyword skipped, a zero-area rectangle dropped, a component that names a macro
//! the LEF never defined) does not stop the import: the offending piece is skipped
//! and one [`LefDefWarning`] is recorded so a caller can show what was dropped.
//!
//! This mirrors the split `reticle-io` uses for GDSII import (hard [`IoError`] vs
//! recoverable `ImportWarning`), but the types are owned here rather than reused so
//! this crate takes no dependency on `reticle-io` (which is frozen-adjacent).

use core::fmt;

/// A structural LEF/DEF import failure.
///
/// Every variant carries enough to locate the problem: LEF and DEF errors name the
/// 1-based line, size and directory errors name the offending value or path. The
/// type is `#[non_exhaustive]` so later waves can add variants without breaking
/// callers.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LefDefError {
    /// A LEF statement could not be parsed.
    Lef {
        /// 1-based line number in the LEF source.
        line: usize,
        /// Human-readable reason.
        reason: String,
    },
    /// A DEF statement could not be parsed.
    Def {
        /// 1-based line number in the DEF source.
        line: usize,
        /// Human-readable reason.
        reason: String,
    },
    /// The input was larger than [`MAX_INPUT_BYTES`](crate::MAX_INPUT_BYTES) and was
    /// refused before parsing so a hostile length can never force a huge allocation.
    TooLarge {
        /// Which input was too large (`"LEF"` or `"DEF"`).
        which: &'static str,
        /// The input size in bytes.
        bytes: usize,
        /// The ceiling in bytes.
        limit: usize,
    },
    /// A required file was missing from a run directory (see
    /// [`import_run_dir`](crate::import_run_dir)).
    MissingFile(String),
    /// A filesystem error while reading a run directory.
    Io(String),
}

impl LefDefError {
    /// Builds a LEF parse error for a given 1-based line.
    pub(crate) fn lef(line: usize, reason: impl Into<String>) -> Self {
        Self::Lef {
            line,
            reason: reason.into(),
        }
    }

    /// Builds a DEF parse error for a given 1-based line.
    pub(crate) fn def(line: usize, reason: impl Into<String>) -> Self {
        Self::Def {
            line,
            reason: reason.into(),
        }
    }
}

impl fmt::Display for LefDefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lef { line, reason } => write!(f, "LEF error on line {line}: {reason}"),
            Self::Def { line, reason } => write!(f, "DEF error on line {line}: {reason}"),
            Self::TooLarge {
                which,
                bytes,
                limit,
            } => write!(
                f,
                "{which} input is {bytes} bytes, over the {limit}-byte import ceiling"
            ),
            Self::MissingFile(name) => write!(f, "run directory is missing a {name} file"),
            Self::Io(msg) => write!(f, "run directory read error: {msg}"),
        }
    }
}

impl core::error::Error for LefDefError {}

/// The category of a [`LefDefWarning`].
///
/// Kept small and stable so callers can group or count warnings without parsing
/// free text. `#[non_exhaustive]` because later waves may add recoverable checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WarningKind {
    /// A statement or block used a keyword this subset does not model; it was
    /// skipped and the rest of the file imported.
    UnsupportedFeature,
    /// A shape was dropped because its geometry was degenerate (zero area, too few
    /// points to form a wire).
    DegenerateGeometry,
    /// A reference could not be resolved (a component naming an undefined macro, a
    /// row naming an undefined site), so it was skipped.
    UnresolvedReference,
    /// A structural limit was hit (a repeat count past the guard ceiling), so some
    /// content was not materialized.
    LimitExceeded,
}

impl WarningKind {
    /// A stable, lowercase label for this kind, for logs and grouping.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::UnsupportedFeature => "unsupported-feature",
            Self::DegenerateGeometry => "degenerate-geometry",
            Self::UnresolvedReference => "unresolved-reference",
            Self::LimitExceeded => "limit-exceeded",
        }
    }
}

/// A non-fatal problem found while importing an otherwise-usable design.
///
/// Plain, owned data: a machine-friendly [`kind`](LefDefWarning::kind) for grouping
/// plus a one-line human [`summary`](LefDefWarning::summary) and a longer
/// [`detail`](LefDefWarning::detail). It borrows nothing so it can cross crate and
/// thread boundaries (lane 5B maps it onto its own notification surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LefDefWarning {
    /// The category of problem.
    pub kind: WarningKind,
    /// A short one-liner naming what happened.
    pub summary: String,
    /// A longer explanation: which item, what was expected, what was done instead.
    pub detail: String,
}

impl LefDefWarning {
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

impl fmt::Display for LefDefWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.summary, self.detail)
    }
}
