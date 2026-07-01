//! Design-rule checking for Reticle.
//!
//! Wave 2 implements a declarative rule engine driven by the technology file
//! (width, spacing, enclosure, extension, notch, area, density, angle), with
//! incremental re-check on edit (< 100 ms for local changes) and an error model
//! that zooms to each violation.
//!
//! The Wave 0 contract is [`DrcEngine`], a [`RuleSet`] implementation that holds
//! rules and (for now) reports no violations.

use reticle_model::{Document, Rule, RuleSet, Violation};

/// The declarative DRC engine (Wave 2). Holds a rule set and checks cells.
#[derive(Debug, Default, Clone)]
pub struct DrcEngine {
    rules: Vec<Rule>,
}

impl DrcEngine {
    /// Creates a DRC engine from a set of rules.
    #[must_use]
    pub fn new(rules: Vec<Rule>) -> Self {
        Self { rules }
    }
}

impl RuleSet for DrcEngine {
    fn rules(&self) -> &[Rule] {
        &self.rules
    }

    fn check_cell(&self, _doc: &Document, _cell: &str) -> Vec<Violation> {
        // Wave 2: evaluate each rule against the cell's geometry via the index.
        Vec::new()
    }
}
