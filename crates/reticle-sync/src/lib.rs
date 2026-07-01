//! Real-time collaboration for Reticle.
//!
//! Wave 3 wraps a `yrs` document over the hierarchical model (ADR 0007), encodes
//! and decodes updates, and manages presence (cursor/selection/viewport) and
//! threaded comments over a WebSocket. Offline edits reconcile on reconnect.
//!
//! The Wave 0 contract is [`SyncDocument`], the local mirror that Wave 3 backs
//! with a CRDT.

use reticle_model::Document;

/// A collaboratively-edited document (Wave 3: backed by a `yrs` CRDT).
#[derive(Debug, Default)]
pub struct SyncDocument {
    doc: Document,
    actor: String,
}

impl SyncDocument {
    /// Creates a sync document for the given actor id.
    #[must_use]
    pub fn new(actor: impl Into<String>) -> Self {
        Self {
            doc: Document::new(),
            actor: actor.into(),
        }
    }

    /// The underlying model document.
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// This peer's actor id.
    #[must_use]
    pub fn actor(&self) -> &str {
        &self.actor
    }
}
