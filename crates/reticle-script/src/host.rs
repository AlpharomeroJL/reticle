//! Host state shared with the scripting engine.
//!
//! [`ScriptHost`] owns the [`EditableDocument`] that registered script functions
//! mutate, plus the set of DRC [`Rule`]s a script has declared and any
//! technology/top-cell metadata it has set. It is wrapped in an
//! [`Rc<RefCell<..>>`](std::rc::Rc) ([`SharedHost`]) and that handle is cloned into
//! every closure registered on the `rhai` engine, so each call borrows the host,
//! performs its edit or query, and releases the borrow before returning.
//!
//! `rhai` is built here without its `sync` feature, so registered functions are
//! only required to be `'static` (not `Send + Sync`); an `Rc<RefCell<..>>` is
//! therefore a sound and cheap way to share mutable host state single-threaded.
//!
//! # Why some state lives outside the [`EditableDocument`]
//!
//! Structural mutations (cells, shapes, instances, arrays) are expressed as
//! [`Edit`]s and applied through [`EditableDocument`] so undo history stays
//! consistent. Two document properties, the top-cell list and the technology -
//! have no [`Edit`] variant and `EditableDocument` exposes its document only
//! immutably, so a script sets them into dedicated host fields. The engine folds
//! them back into the [`Document`] it snapshots after evaluation (see
//! [`ScriptHost::snapshot`]).

use std::cell::RefCell;
use std::rc::Rc;

use reticle_model::{Document, DocumentStore, Edit, EditableDocument, Rule, Technology};

/// The mutable state a script drives: the document under construction, the DRC
/// rules declared so far, and any top-cell / technology metadata that has been set.
#[derive(Debug, Default)]
pub struct ScriptHost {
    /// The editable document. Structural edits go through it so undo history stays
    /// consistent with the rest of the model.
    doc: EditableDocument,
    /// DRC rules accumulated by `add_*_rule` / `load_technology` calls, used when a
    /// script runs a check.
    rules: Vec<Rule>,
    /// Top-cell names set by the script, folded into snapshots.
    top_cells: Vec<String>,
    /// Technology set by the script (e.g. via `load_technology`), folded into
    /// snapshots.
    technology: Option<Technology>,
}

impl ScriptHost {
    /// Creates a host over a fresh, empty document.
    pub fn new() -> Self {
        Self::default()
    }

    /// A [`Document`] reflecting every edit plus the current top-cell list and
    /// technology.
    ///
    /// The structural document from [`EditableDocument`] is cloned and the
    /// out-of-band top-cell / technology fields are applied on top, so the result
    /// is a faithful, self-contained snapshot the engine can hand out.
    pub fn snapshot(&self) -> Document {
        let mut doc = self.doc.document().clone();
        if !self.top_cells.is_empty() {
            doc.set_top_cells(self.top_cells.clone());
        }
        if let Some(tech) = &self.technology {
            doc.set_technology(tech.clone());
        }
        doc
    }

    /// Borrows the structural document (without top-cell / technology overrides).
    ///
    /// This is what query functions read; it already contains every cell, shape,
    /// instance, and array a script has added.
    pub fn document(&self) -> &Document {
        self.doc.document()
    }

    /// Applies a structural edit, returning a descriptive string on failure so the
    /// caller can surface it through `rhai`.
    pub fn apply(&mut self, edit: Edit) -> core::result::Result<(), String> {
        self.doc.apply(edit).map_err(|e| e.to_string())
    }

    /// The DRC rules declared so far.
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Appends a DRC rule.
    pub fn push_rule(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    /// Replaces the whole rule set (used when loading a technology file).
    pub fn set_rules(&mut self, rules: Vec<Rule>) {
        self.rules = rules;
    }

    /// Sets the top-cell list to fold into snapshots.
    pub fn set_top_cells(&mut self, tops: Vec<String>) {
        self.top_cells = tops;
    }

    /// Sets the technology to fold into snapshots.
    pub fn set_technology(&mut self, tech: Technology) {
        self.technology = Some(tech);
    }
}

/// A shared, interior-mutable handle to a [`ScriptHost`].
///
/// Cloning is cheap (a reference-count bump) and every registered script function
/// captures its own clone.
pub type SharedHost = Rc<RefCell<ScriptHost>>;

/// Creates a fresh shared host.
pub fn shared_host() -> SharedHost {
    Rc::new(RefCell::new(ScriptHost::new()))
}
