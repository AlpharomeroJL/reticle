//! Declarative DRC rules and the [`RuleSet`] trait implemented by `reticle-drc`.

use crate::Document;
use reticle_geometry::{LayerId, Rect};

/// The kind of geometric constraint a [`Rule`] expresses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum RuleKind {
    /// Minimum feature width on a layer.
    Width,
    /// Minimum spacing between shapes (same layer, or `layer` vs `other_layer`).
    Spacing,
    /// Minimum enclosure of one layer by another.
    Enclosure,
    /// Minimum extension of one layer past another.
    Extension,
    /// Minimum notch (concave) width.
    Notch,
    /// Minimum shape area.
    Area,
    /// Maximum/minimum layer density over a window.
    Density,
    /// Allowed edge angles.
    Angle,
}

/// A single declarative design rule, driven by the technology file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rule {
    /// Rule name (shown in the DRC error browser).
    pub name: String,
    /// The kind of constraint.
    pub kind: RuleKind,
    /// The primary layer the rule applies to.
    pub layer: LayerId,
    /// The second layer, for two-layer rules (spacing/enclosure/extension).
    pub other_layer: Option<LayerId>,
    /// Threshold: DBU for length rules, DBU² for area, milli-degrees for angle.
    pub value: i64,
}

/// A concrete DRC violation, ready to be shown and zoomed to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Violation {
    /// Name of the rule that was violated.
    pub rule: String,
    /// Bounding box of the offending geometry, for zoom-to navigation.
    pub location: Rect,
    /// Human-readable description of the violation.
    pub message: String,
}

/// A set of design rules that can check a document. Implemented by `reticle-drc`.
pub trait RuleSet {
    /// The rules in this set.
    fn rules(&self) -> &[Rule];

    /// Checks a single cell of `doc` and returns any violations found.
    fn check_cell(&self, doc: &Document, cell: &str) -> Vec<Violation>;
}
