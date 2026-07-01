//! The crate error type and its conversions.
//!
//! Script evaluation can fail for three broad reasons, each captured by a
//! [`ScriptError`] variant: the `rhai` engine rejected or trapped the script
//! ([`ScriptError::Eval`]), a model edit was invalid ([`ScriptError::Model`]), or a
//! plugin directory could not be read ([`ScriptError::Io`]). Errors carry a
//! human-readable message so a failing plugin points at what went wrong.

use core::fmt;
use std::path::PathBuf;

/// An error raised while evaluating a script or loading a plugin directory.
///
/// The scripting layer maps the underlying `rhai` [`EvalAltResult`](rhai::EvalAltResult)
/// and the model's [`ModelError`](reticle_model::ModelError) into this single type
/// so callers handle one error surface. [`ScriptError::Eval`] preserves the script
/// position and message produced by `rhai`; [`ScriptError::Model`] wraps the
/// document error that a failed edit produced from inside a script.
#[derive(Debug)]
#[non_exhaustive]
pub enum ScriptError {
    /// The `rhai` engine failed to compile or evaluate the script. The string is
    /// `rhai`'s own diagnostic (including source position when available).
    Eval(String),
    /// A document edit performed by the script was rejected by the model.
    Model(reticle_model::ModelError),
    /// A plugin directory (or a file within it) could not be read. Carries the
    /// offending path and the underlying I/O error message.
    Io {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying I/O error, rendered to a string.
        source: String,
    },
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Eval(msg) => write!(f, "script evaluation error: {msg}"),
            Self::Model(err) => write!(f, "model error during script: {err}"),
            Self::Io { path, source } => {
                write!(f, "plugin i/o error at {}: {source}", path.display())
            }
        }
    }
}

impl core::error::Error for ScriptError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Model(err) => Some(err),
            Self::Eval(_) | Self::Io { .. } => None,
        }
    }
}

impl From<reticle_model::ModelError> for ScriptError {
    fn from(err: reticle_model::ModelError) -> Self {
        Self::Model(err)
    }
}

impl From<Box<rhai::EvalAltResult>> for ScriptError {
    fn from(err: Box<rhai::EvalAltResult>) -> Self {
        Self::Eval(err.to_string())
    }
}

/// The crate result type.
pub type Result<T> = core::result::Result<T, ScriptError>;
