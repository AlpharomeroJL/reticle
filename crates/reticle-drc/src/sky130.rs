//! Loader for the committed SKY130 DRC rule subset.
//!
//! [`sky130_drc_rules`] turns the cited rule table at `tech/sky130-drc-subset.toml`
//! into the engine's [`Rule`] form. The table is embedded at compile time with
//! [`include_str!`], so loading it needs no runtime path and cannot silently pick
//! up a stale copy.
//!
//! The table is a **subset** of the SKY130 periphery rules (min width,
//! spacing, key contact and via sizes, enclosures, minimum areas, and the poly
//! endcap). Passing it is *not* tape-out clean; see the coverage table in the book
//! for exactly which rule ids are and are not checked.

use reticle_geometry::LayerId;
use reticle_model::{Rule, RuleKind};
use serde::Deserialize;

/// The committed rule table, embedded so the loader needs no runtime path.
const SKY130_DRC_TOML: &str = include_str!("../../../tech/sky130-drc-subset.toml");

/// The top-level shape of `sky130-drc-subset.toml`: a list of `[[rule]]` tables.
#[derive(Debug, Deserialize)]
struct RuleFile {
    /// The `[[rule]]` entries, in file order.
    rule: Vec<RawRule>,
}

/// One `[[rule]]` table exactly as written in the TOML file.
///
/// The file also carries a human-readable `description` per rule; it has no
/// counterpart in [`Rule`] and is intentionally not deserialized.
#[derive(Debug, Deserialize)]
struct RawRule {
    /// SKY130 rule id, e.g. `"m1.1"`; becomes [`Rule::name`].
    id: String,
    /// Constraint kind: one of `width`, `spacing`, `enclosure`, `extension`, `area`.
    kind: String,
    /// Primary layer as `[gds_layer, datatype]`.
    layer: [u16; 2],
    /// Second layer for two-layer rules, as `[gds_layer, datatype]`.
    other_layer: Option<[u16; 2]>,
    /// Threshold in database units (1 dbu = 1 nm; areas are dbu squared).
    value_dbu: i64,
}

/// Loads the built-in SKY130 DRC rule subset as engine [`Rule`]s.
///
/// The rules come from the committed `tech/sky130-drc-subset.toml` (embedded at
/// compile time) and are returned in file order, ready for
/// [`DrcEngine::new`](crate::DrcEngine::new). This deck is a subset of the SKY130
/// periphery rules; passing it is not tape-out clean.
///
/// # Panics
///
/// Panics if the embedded table fails to parse, which can only mean the committed
/// file and this loader have drifted apart; the loader's own tests catch that in
/// CI before any caller can observe it.
#[must_use]
pub fn sky130_drc_rules() -> Vec<Rule> {
    parse_rules(SKY130_DRC_TOML).expect("committed tech/sky130-drc-subset.toml must parse")
}

/// Parses a rule table in the `sky130-drc-subset.toml` format.
///
/// Kept separate from [`sky130_drc_rules`] so malformed input is testable without
/// touching the committed file.
fn parse_rules(text: &str) -> Result<Vec<Rule>, String> {
    let file: RuleFile = toml::from_str(text).map_err(|e| e.to_string())?;
    file.rule
        .into_iter()
        .map(|raw| {
            let kind = parse_kind(&raw.kind)
                .ok_or_else(|| format!("rule {}: unknown kind {:?}", raw.id, raw.kind))?;
            Ok(Rule {
                name: raw.id,
                kind,
                layer: LayerId::new(raw.layer[0], raw.layer[1]),
                other_layer: raw.other_layer.map(|l| LayerId::new(l[0], l[1])),
                value: raw.value_dbu,
            })
        })
        .collect()
}

/// Maps a `kind` string from the rule table to a [`RuleKind`].
///
/// Only the five kinds the table format documents are accepted; anything else is
/// `None` so a typo in the data fails loudly instead of silently skipping a rule.
fn parse_kind(kind: &str) -> Option<RuleKind> {
    match kind {
        "width" => Some(RuleKind::Width),
        "spacing" => Some(RuleKind::Spacing),
        "enclosure" => Some(RuleKind::Enclosure),
        "extension" => Some(RuleKind::Extension),
        "area" => Some(RuleKind::Area),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn loads_the_committed_subset() {
        let rules = sky130_drc_rules();
        assert_eq!(rules.len(), 26, "the committed table has 26 rules");

        // Every rule id is unique and every threshold is positive.
        let ids: HashSet<&str> = rules.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(ids.len(), rules.len(), "rule ids must be unique");
        assert!(rules.iter().all(|r| r.value > 0), "thresholds are positive");

        // Kind distribution of the committed table.
        let count = |kind: RuleKind| rules.iter().filter(|r| r.kind == kind).count();
        assert_eq!(count(RuleKind::Width), 12);
        assert_eq!(count(RuleKind::Spacing), 8);
        assert_eq!(count(RuleKind::Enclosure), 3);
        assert_eq!(count(RuleKind::Extension), 1);
        assert_eq!(count(RuleKind::Area), 2);

        // Two-layer kinds carry `other_layer`; single-layer kinds do not.
        for r in &rules {
            match r.kind {
                RuleKind::Enclosure | RuleKind::Extension => {
                    assert!(r.other_layer.is_some(), "{} needs an other_layer", r.name);
                }
                _ => assert!(r.other_layer.is_none(), "{} is single-layer", r.name),
            }
        }
    }

    #[test]
    fn spot_checks_representative_rules() {
        let rules = sky130_drc_rules();
        let get = |name: &str| {
            rules
                .iter()
                .find(|r| r.name == name)
                .unwrap_or_else(|| panic!("rule {name} present"))
        };

        let m1_1 = get("m1.1");
        assert_eq!(m1_1.kind, RuleKind::Width);
        assert_eq!(m1_1.layer, LayerId::new(68, 20));
        assert_eq!(m1_1.other_layer, None);
        assert_eq!(m1_1.value, 140);

        let m1_4 = get("m1.4");
        assert_eq!(m1_4.kind, RuleKind::Enclosure);
        assert_eq!(m1_4.layer, LayerId::new(67, 44));
        assert_eq!(m1_4.other_layer, Some(LayerId::new(68, 20)));
        assert_eq!(m1_4.value, 30);

        let poly_8 = get("poly.8");
        assert_eq!(poly_8.kind, RuleKind::Extension);
        assert_eq!(poly_8.layer, LayerId::new(66, 20));
        assert_eq!(poly_8.other_layer, Some(LayerId::new(65, 20)));
        assert_eq!(poly_8.value, 130);

        let m1_6 = get("m1.6");
        assert_eq!(m1_6.kind, RuleKind::Area);
        assert_eq!(m1_6.value, 83_000);
    }

    #[test]
    fn unknown_kind_is_rejected() {
        let bad = "[[rule]]\nid = \"x.1\"\nkind = \"antenna\"\nlayer = [1, 0]\nvalue_dbu = 1\n";
        let err = parse_rules(bad).expect_err("unknown kind must fail");
        assert!(err.contains("x.1"), "error names the offending rule: {err}");
    }

    #[test]
    fn malformed_toml_is_rejected() {
        assert!(parse_rules("[[rule]\nid = ").is_err());
    }
}
