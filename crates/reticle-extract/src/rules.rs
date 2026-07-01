//! Cross-layer connection rules.
//!
//! Two shapes on the *same* layer connect when their geometry touches or overlaps.
//! Shapes on *different* layers never connect on their own — an explicit
//! connector (a via or contact) must bridge them. A [`ConnectionRule`] names such
//! a bridge as an ordered triple `(bottom, via, top)`: a shape on the `via` layer
//! connects a `bottom`-layer shape and a `top`-layer shape wherever the via
//! overlaps both.
//!
//! [`ConnectionRules`] is the configurable set an [`Extractor`](crate::Extractor)
//! consults. It is intentionally data-driven so a technology's via stack can be
//! described without code changes.

use reticle_geometry::LayerId;

/// One via/contact rule: a shape on [`via`](Self::via) connects a shape on
/// [`bottom`](Self::bottom) to a shape on [`top`](Self::top) where the via
/// overlaps both.
///
/// The triple is treated symmetrically in `bottom`/`top` (a via joining metal-1 to
/// metal-2 is the same connection regardless of which is called "bottom"), but the
/// `via` layer is distinct from both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionRule {
    /// The lower conductor layer joined by the via.
    pub bottom: LayerId,
    /// The via/contact layer that bridges the two conductors.
    pub via: LayerId,
    /// The upper conductor layer joined by the via.
    pub top: LayerId,
}

impl ConnectionRule {
    /// Creates a rule joining `bottom` and `top` through `via`.
    #[must_use]
    pub fn new(bottom: LayerId, via: LayerId, top: LayerId) -> Self {
        Self { bottom, via, top }
    }

    /// Given the `via` layer matches, returns the pair of conductor layers this
    /// rule bridges as `(bottom, top)`.
    #[must_use]
    pub fn conductors(&self) -> (LayerId, LayerId) {
        (self.bottom, self.top)
    }
}

/// The configurable set of via/contact rules an extractor applies.
///
/// Empty by default (same-layer connectivity only). Add rules with
/// [`with_rule`](Self::with_rule) or [`push`](Self::push).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectionRules {
    rules: Vec<ConnectionRule>,
}

impl ConnectionRules {
    /// Creates an empty rule set (same-layer connectivity only).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a rule set from an iterator of rules.
    pub fn from_rules(rules: impl IntoIterator<Item = ConnectionRule>) -> Self {
        Self {
            rules: rules.into_iter().collect(),
        }
    }

    /// Adds a rule, returning `self` for chaining.
    #[must_use]
    pub fn with_rule(mut self, rule: ConnectionRule) -> Self {
        self.rules.push(rule);
        self
    }

    /// Adds a via rule joining `bottom` and `top` through `via`, returning `self`.
    #[must_use]
    pub fn connect(self, bottom: LayerId, via: LayerId, top: LayerId) -> Self {
        self.with_rule(ConnectionRule::new(bottom, via, top))
    }

    /// Appends a rule in place.
    pub fn push(&mut self, rule: ConnectionRule) {
        self.rules.push(rule);
    }

    /// The rules, in insertion order.
    #[must_use]
    pub fn rules(&self) -> &[ConnectionRule] {
        &self.rules
    }

    /// Returns `true` if no rules are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The number of configured rules.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Returns the conductor pair `(bottom, top)` for every rule whose `via` layer
    /// equals `via`. A via shape on that layer can bridge either conductor pair.
    pub fn conductor_pairs_for_via(
        &self,
        via: LayerId,
    ) -> impl Iterator<Item = (LayerId, LayerId)> + '_ {
        self.rules
            .iter()
            .filter(move |r| r.via == via)
            .map(ConnectionRule::conductors)
    }

    /// Returns `true` if any rule uses `layer` as its via layer.
    #[must_use]
    pub fn is_via_layer(&self, layer: LayerId) -> bool {
        self.rules.iter().any(|r| r.via == layer)
    }
}
