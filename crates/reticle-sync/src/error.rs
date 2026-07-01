//! Error type for the collaboration layer.

use core::fmt;

/// Errors produced while mapping the model onto the CRDT, exchanging updates, or
/// decoding presence and comment messages.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SyncError {
    /// A binary CRDT update could not be decoded from the `yrs` v1 wire format.
    DecodeUpdate(String),
    /// A binary state vector could not be decoded from the `yrs` v1 wire format.
    DecodeStateVector(String),
    /// Applying a decoded update to the local document failed.
    ApplyUpdate(String),
    /// The CRDT held a value whose shape did not match what materialization
    /// expected (for example a shape record with too few coordinates).
    Malformed(&'static str),
    /// A required field was missing while decoding a proto message.
    MissingField(&'static str),
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DecodeUpdate(e) => write!(f, "failed to decode CRDT update: {e}"),
            Self::DecodeStateVector(e) => write!(f, "failed to decode state vector: {e}"),
            Self::ApplyUpdate(e) => write!(f, "failed to apply CRDT update: {e}"),
            Self::Malformed(what) => write!(f, "malformed CRDT value: {what}"),
            Self::MissingField(name) => write!(f, "missing required field: {name}"),
        }
    }
}

impl core::error::Error for SyncError {}

/// Result type for fallible collaboration operations.
pub type Result<T> = core::result::Result<T, SyncError>;
