//! The error type generators return from validation and generation.

use core::fmt;

/// Why a generator rejected its parameters or could not build geometry.
///
/// Validation failures ([`OutOfRange`](GenError::OutOfRange),
/// [`Invalid`](GenError::Invalid)) name the offending field so a UI form (lane 2D)
/// can point at the exact input, and are stable enough to surface to a model as a
/// tool error. The registry adds [`Deserialize`](GenError::Deserialize) and
/// [`UnknownGenerator`](GenError::UnknownGenerator) for the type-erased JSON path.
#[derive(Clone, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum GenError {
    /// A numeric field fell outside its allowed `[min, max]` range.
    OutOfRange {
        /// The parameter field name (matches the schema and the serde field).
        field: &'static str,
        /// The value that was supplied.
        value: i64,
        /// The inclusive lower bound.
        min: i64,
        /// The inclusive upper bound.
        max: i64,
    },
    /// A field was in range individually but invalid in context (for example a
    /// cross-field constraint), with a human-readable reason.
    Invalid {
        /// The parameter field name the reason is about.
        field: &'static str,
        /// Why the value is not acceptable.
        reason: &'static str,
    },
    /// The registry could not deserialize a JSON parameter blob into the
    /// generator's parameter struct, carrying the serde error text.
    Deserialize(String),
    /// The registry was asked for a generator id it does not know.
    UnknownGenerator(String),
}

impl fmt::Display for GenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange {
                field,
                value,
                min,
                max,
            } => write!(
                f,
                "parameter `{field}` = {value} is out of range [{min}, {max}]"
            ),
            Self::Invalid { field, reason } => {
                write!(f, "parameter `{field}` is invalid: {reason}")
            }
            Self::Deserialize(msg) => write!(f, "could not parse generator parameters: {msg}"),
            Self::UnknownGenerator(id) => write!(f, "unknown generator id: {id}"),
        }
    }
}

impl core::error::Error for GenError {}
