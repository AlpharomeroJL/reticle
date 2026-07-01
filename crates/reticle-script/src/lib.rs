//! Embedded scripting for Reticle.
//!
//! Wave 3 exposes the model to an embedded `rhai` engine — create, query,
//! transform, run DRC, route, and export — plus a plugin folder and worked example
//! scripts.
//!
//! The Wave 0 contract is [`ScriptEngine`].

use reticle_model::Document;

/// An embedded script engine bound to a document (Wave 3: `rhai`).
#[derive(Debug, Default)]
pub struct ScriptEngine {
    document: Document,
}

impl ScriptEngine {
    /// Creates a script engine over a fresh document.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Evaluates a script against the bound document.
    ///
    /// # Errors
    ///
    /// Returns a [`reticle_model::ModelError`] if evaluation fails. Wave 3 wires
    /// the `rhai` engine; the signature is the frozen contract.
    pub fn eval(&mut self, source: &str) -> reticle_model::Result<()> {
        let _ = source;
        todo!("Wave 3: evaluate via rhai")
    }

    /// The bound document.
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.document
    }
}
