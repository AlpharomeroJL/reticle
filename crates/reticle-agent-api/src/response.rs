//! The frozen agent response vocabulary.
//!
//! A command yields `Result<`[`AgentResponse`]`, `[`AgentError`](crate::AgentError)`>`.
//! Every response carries the document [`Revision`] it observed, so a caller can
//! tell whether the document changed. Structured query output is carried as a
//! `serde_json::Value` rather than a growing set of typed variants, keeping the
//! contract stable as query shapes evolve.

use serde::{Deserialize, Serialize};

use crate::ElementId;

/// A monotonic document revision: it increases by one on each applied mutation.
pub type Revision = u64;

/// A successful command result, tagged by its `result` kind.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AgentResponse {
    /// A mutation succeeded: the ids it created or affected, and the new revision.
    Ok {
        /// The document revision after the mutation.
        revision: Revision,
        /// The elements this command created or changed.
        affected: Vec<ElementId>,
    },
    /// Structured read-only output (query results, cell info, layers, netlist,
    /// violations) as JSON, plus the revision it was read at.
    Data {
        /// The document revision the data was read at.
        revision: Revision,
        /// The structured payload.
        value: serde_json::Value,
    },
    /// A binary payload (exported GDSII or OASIS, a rendered PNG).
    Blob {
        /// The document revision the blob was produced at.
        revision: Revision,
        /// The bytes.
        bytes: Vec<u8>,
    },
}
