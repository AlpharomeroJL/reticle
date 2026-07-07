//! SKY130's [`GenTech`]: the generator numbers for the `SkyWater` SKY130 subset.
//!
//! Every value here is transcribed from the committed DRC subset in
//! `tech/sky130-drc-subset.toml` and the layer map in `tech/sky130.tech`, both the
//! checked-in truth. The generators read these through [`GenTech`] (see
//! [`GENTECH`]); the [cleanliness property tests](../tests/property.rs) then re-derive
//! the same rules independently through [`reticle_drc::sky130_drc_rules`] and confirm
//! zero violations, so a drift between this table and the committed deck fails a test
//! rather than shipping.
//!
//! Values are database units (1 dbu = 1 nm), matching the subset. `layer` / `datatype`
//! pairs are the GDSII addresses the SKY130 GDS uses.
//!
//! # Coverage
//!
//! This is a *subset*, mirroring the DRC subset: min width, min spacing, the
//! contact/via sizes, three enclosures, and two minimum-area rules for the digital
//! metal stack (`li1`, `met1..met3`, the `licon`/`mcon`/`via`/`via2` cuts). Rules the
//! subset does not carry, notably per-cut spacing for the contact and via layers, are
//! not encoded here either; where a generator needs a pitch it picks a conservative
//! one (see [`GenTech::safe_cut_margin`]). Passing the subset is not tape-out clean.

use reticle_geometry::LayerId;

use crate::gentech::{Conductor, Cut, GenTech, Residue};

// --- Conductor layers (from tech/sky130.tech and the width/spacing/area rules). ---

/// Local interconnect `li1` (67/20): width 170, spacing 170, area 56100 (`li.1`,
/// `li.3`, `li.6`).
pub const LI1: Conductor = Conductor {
    layer: LayerId::new(67, 20),
    min_width: 170,
    min_spacing: 170,
    min_area: Some(56_100),
};

/// Metal 1 `met1` (68/20): width 140, spacing 140, area 83000 (`m1.1`, `m1.2`,
/// `m1.6`).
pub const MET1: Conductor = Conductor {
    layer: LayerId::new(68, 20),
    min_width: 140,
    min_spacing: 140,
    min_area: Some(83_000),
};

/// Metal 2 `met2` (69/20): width 140, spacing 140 (`m2.1`, `m2.2`). The subset
/// carries no `met2` area rule.
pub const MET2: Conductor = Conductor {
    layer: LayerId::new(69, 20),
    min_width: 140,
    min_spacing: 140,
    min_area: None,
};

/// Metal 3 `met3` (70/20): width 300, spacing 300 (`m3.1`, `m3.2`). The subset
/// carries no `met3` area rule.
pub const MET3: Conductor = Conductor {
    layer: LayerId::new(70, 20),
    min_width: 300,
    min_spacing: 300,
    min_area: None,
};

// --- Cut layers (contact/via sizes and their enclosures). ---

/// Local-interconnect contact `licon1` (66/44): size 170, enclosed by `li1` by 80
/// (`licon.1`, `li.5`). This is the substrate-tap contact.
pub const LICON: Cut = Cut {
    layer: LayerId::new(66, 44),
    size: 170,
    enclosure: Some((LI1.layer, 80)),
};

/// Metal contact `mcon` (67/44): size 170, enclosed by `met1` by 30 (`ct.1`,
/// `m1.4`). Bridges `li1` and `met1`.
pub const MCON: Cut = Cut {
    layer: LayerId::new(67, 44),
    size: 170,
    enclosure: Some((MET1.layer, 30)),
};

/// Via 1 `via` (68/44): size 150, enclosed by `met2` by 55 (`via.1a`, `m2.4`).
/// Bridges `met1` and `met2`.
pub const VIA: Cut = Cut {
    layer: LayerId::new(68, 44),
    size: 150,
    enclosure: Some((MET2.layer, 55)),
};

/// Via 2 `via2` (69/44): size 200 (`via2.1a`). The subset carries no `via2` enclosure
/// rule, so the generators use a conservative margin (`SAFE_CUT_MARGIN`-scale) so the
/// covering plates fully enclose the cut. Bridges `met2` and `met3`.
pub const VIA2: Cut = Cut {
    layer: LayerId::new(69, 44),
    size: 200,
    enclosure: Some((MET3.layer, VIA2_FALLBACK_ENCLOSURE)),
};

/// A safe conductor overlap used where the subset gives no cut-to-cut spacing rule:
/// cuts are pitched at least their own size plus this margin so no two cuts touch or
/// come implausibly close, and covering plates keep a positive gap from anything
/// outside. Chosen as the `li1` min spacing (the largest interconnect spacing in the
/// subset), which comfortably clears every cut layer.
pub const SAFE_CUT_MARGIN: i32 = 170;

/// The conservative enclosure the generators keep around a `via2` cut, which the
/// subset carries no rule for.
const VIA2_FALLBACK_ENCLOSURE: i32 = 65;

/// SKY130's generator technology: the four digital interconnect conductors, the three
/// cuts that bridge them, the `licon` substrate tap, and the conservative cut pitch.
pub const GENTECH: GenTech = GenTech::new(
    "sky130",
    [LI1, MET1, MET2, MET3],
    [MCON, VIA, VIA2],
    LICON,
    SAFE_CUT_MARGIN,
);

/// The residue [`crate::gentech::derive`] needs to reconstruct [`GENTECH`] from the
/// committed SKY130 deck: the role assignment plus the two numbers the subset lacks
/// (the `via2` enclosure and the cut pitch).
pub const RESIDUE: Residue = Residue {
    name: "sky130",
    conductors: [LI1.layer, MET1.layer, MET2.layer, MET3.layer],
    cuts: [MCON.layer, VIA.layer, VIA2.layer],
    tap_cut: LICON.layer,
    cut_fallback_enclosure: [None, None, Some(VIA2_FALLBACK_ENCLOSURE)],
    tap_fallback_enclosure: None,
    safe_cut_margin: SAFE_CUT_MARGIN,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gentech::derive_gentech;
    use reticle_drc::sky130_drc_rules;
    use reticle_model::{RuleKind, Technology};

    /// Builds a [`Technology`] carrying the committed SKY130 DRC subset as its rules,
    /// so [`derive`] can reconstruct the [`GenTech`] from parsed data.
    fn sky130_technology() -> Technology {
        Technology {
            name: "sky130".to_string(),
            rules: sky130_drc_rules(),
            ..Technology::default()
        }
    }

    /// The authored [`GENTECH`] must equal the one [`derive`] reconstructs from the
    /// committed DRC subset, so a change to the deck that these generators target
    /// cannot pass unnoticed.
    #[test]
    fn gentech_matches_committed_subset() {
        let derived =
            derive_gentech(&sky130_technology(), &RESIDUE).expect("derive from committed deck");
        assert_eq!(
            derived, GENTECH,
            "authored GenTech drifted from the committed deck"
        );
    }

    /// Spot-check the individual numbers against the committed subset rule ids, so the
    /// derivation itself is anchored (not just self-consistent).
    #[test]
    fn constants_match_committed_subset() {
        let rules = sky130_drc_rules();
        let find = |name: &str| {
            rules
                .iter()
                .find(|r| r.name == name)
                .unwrap_or_else(|| panic!("subset rule {name} present"))
        };

        assert_eq!(find("li.1").value, i64::from(LI1.min_width));
        assert_eq!(find("li.3").value, i64::from(LI1.min_spacing));
        assert_eq!(find("li.6").value, LI1.min_area.unwrap());
        assert_eq!(find("m1.1").value, i64::from(MET1.min_width));
        assert_eq!(find("m1.6").value, MET1.min_area.unwrap());
        assert_eq!(find("m3.1").value, i64::from(MET3.min_width));

        assert_eq!(find("licon.1").value, i64::from(LICON.size));
        assert_eq!(find("ct.1").value, i64::from(MCON.size));
        assert_eq!(find("via.1a").value, i64::from(VIA.size));
        assert_eq!(find("via2.1a").value, i64::from(VIA2.size));

        assert_eq!(find("li.5").value, i64::from(LICON.enclosure.unwrap().1));
        assert_eq!(find("m1.4").value, i64::from(MCON.enclosure.unwrap().1));
        assert_eq!(find("m2.4").value, i64::from(VIA.enclosure.unwrap().1));

        // The subset genuinely carries no via2 enclosure rule; VIA2 uses the
        // conservative residue value instead, so cleanliness never depends on a rule
        // that does not exist.
        assert!(
            !rules
                .iter()
                .any(|r| r.kind == RuleKind::Enclosure && r.layer == VIA2.layer),
            "subset has no via2 enclosure rule"
        );
        assert_eq!(VIA2.enclosure.unwrap().1, VIA2_FALLBACK_ENCLOSURE);
    }
}
