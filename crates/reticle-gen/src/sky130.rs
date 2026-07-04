//! The SKY130 subset numbers the generators are written against.
//!
//! Every value here is transcribed from the committed DRC subset in
//! `tech/sky130-drc-subset.toml` and the layer map in `tech/sky130.tech`, both of
//! which are the checked-in truth. The generators bake these numbers in so they are
//! DRC-clean *by construction*; the cleanliness property tests then re-derive the
//! same rules independently through [`reticle_drc::sky130_drc_rules`] and confirm
//! zero violations, so a drift between this table and the committed deck fails the
//! test rather than shipping.
//!
//! Values are database units (1 dbu = 1 nm), matching the subset. `layer` /
//! `datatype` pairs are the GDSII addresses the SKY130 GDS uses.
//!
//! # Coverage
//!
//! This is a *subset*, mirroring the DRC subset: min width, min spacing, the
//! contact/via sizes, three enclosures, and two minimum-area rules for the digital
//! metal stack (`li1`, `met1..met3`, poly/diff, and the `licon`/`mcon`/`via`/`via2`
//! cuts). Rules the subset does not carry, notably per-cut spacing for the contact
//! and via layers, are not encoded here either; where a generator needs a pitch it
//! picks a conservative one (see the generator docs). Passing the subset is not
//! tape-out clean.

use reticle_geometry::LayerId;

/// A conductor layer the generators can draw on, with the width, spacing, and
/// (optional) minimum-area numbers the SKY130 subset attaches to it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Conductor {
    /// The GDSII layer address.
    pub layer: LayerId,
    /// Minimum drawn width in DBU.
    pub min_width: i32,
    /// Minimum same-layer spacing in DBU.
    pub min_spacing: i32,
    /// Minimum shape area in DBU², or `None` if the subset carries no area rule for
    /// this layer.
    pub min_area: Option<i64>,
}

/// A cut layer (a contact or via) the generators can array, with its exact drawn
/// size and, when the subset carries one, the enclosure a covering conductor owes
/// it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cut {
    /// The GDSII layer address of the cut.
    pub layer: LayerId,
    /// The exact drawn cut size in DBU (the subset encodes it as a min width, and a
    /// square cut at that size is the standard drawn geometry).
    pub size: i32,
    /// The conductor that must enclose the cut, and by how much (DBU), or `None`
    /// when the subset carries no enclosure rule for this cut.
    pub enclosure: Option<(LayerId, i32)>,
}

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
/// (`licon.1`, `li.5`).
pub const LICON: Cut = Cut {
    layer: LayerId::new(66, 44),
    size: 170,
    enclosure: Some((LI1.layer, 80)),
};

/// Metal contact `mcon` (67/44): size 170, enclosed by `met1` by 30 (`ct.1`,
/// `m1.4`).
pub const MCON: Cut = Cut {
    layer: LayerId::new(67, 44),
    size: 170,
    enclosure: Some((MET1.layer, 30)),
};

/// Via 1 `via` (68/44): size 150, enclosed by `met2` by 55 (`via.1a`, `m2.4`).
pub const VIA: Cut = Cut {
    layer: LayerId::new(68, 44),
    size: 150,
    enclosure: Some((MET2.layer, 55)),
};

/// Via 2 `via2` (69/44): size 200 (`via2.1a`). The subset carries no `via2`
/// enclosure rule, so [`Cut::enclosure`] is `None`; the via-farm generator still
/// applies a conservative enclosure so the plates fully cover the cuts.
pub const VIA2: Cut = Cut {
    layer: LayerId::new(69, 44),
    size: 200,
    enclosure: None,
};

/// A safe conductor overlap used where the subset gives no cut-to-cut spacing rule:
/// cuts are pitched at least their own size plus this margin so no two cuts touch
/// or come implausibly close, and covering plates keep a positive gap from anything
/// outside. Chosen as the `li1` min spacing (the largest interconnect spacing in
/// the subset), which comfortably clears every cut layer.
pub const SAFE_CUT_MARGIN: i32 = 170;

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_drc::sky130_drc_rules;
    use reticle_model::RuleKind;

    /// Every number baked into this module must match the committed DRC subset, so a
    /// change to the deck that these generators target cannot pass unnoticed.
    #[test]
    fn constants_match_committed_subset() {
        let rules = sky130_drc_rules();
        let find = |name: &str| {
            rules
                .iter()
                .find(|r| r.name == name)
                .unwrap_or_else(|| panic!("subset rule {name} present"))
        };

        // Conductors: width, spacing, area.
        assert_eq!(find("li.1").value, i64::from(LI1.min_width));
        assert_eq!(find("li.3").value, i64::from(LI1.min_spacing));
        assert_eq!(find("li.6").value, LI1.min_area.unwrap());
        assert_eq!(find("m1.1").value, i64::from(MET1.min_width));
        assert_eq!(find("m1.2").value, i64::from(MET1.min_spacing));
        assert_eq!(find("m1.6").value, MET1.min_area.unwrap());
        assert_eq!(find("m2.1").value, i64::from(MET2.min_width));
        assert_eq!(find("m2.2").value, i64::from(MET2.min_spacing));
        assert_eq!(find("m3.1").value, i64::from(MET3.min_width));
        assert_eq!(find("m3.2").value, i64::from(MET3.min_spacing));

        // Cuts: exact size (encoded as a width rule).
        assert_eq!(find("licon.1").value, i64::from(LICON.size));
        assert_eq!(find("ct.1").value, i64::from(MCON.size));
        assert_eq!(find("via.1a").value, i64::from(VIA.size));
        assert_eq!(find("via2.1a").value, i64::from(VIA2.size));

        // Enclosures the subset carries.
        let li5 = find("li.5");
        assert_eq!(li5.kind, RuleKind::Enclosure);
        assert_eq!(li5.value, i64::from(LICON.enclosure.unwrap().1));
        let m1_4 = find("m1.4");
        assert_eq!(m1_4.kind, RuleKind::Enclosure);
        assert_eq!(m1_4.value, i64::from(MCON.enclosure.unwrap().1));
        let m2_4 = find("m2.4");
        assert_eq!(m2_4.kind, RuleKind::Enclosure);
        assert_eq!(m2_4.value, i64::from(VIA.enclosure.unwrap().1));

        // The subset genuinely carries no via2 enclosure, matching `VIA2.enclosure`.
        assert!(VIA2.enclosure.is_none());
        assert!(
            !rules
                .iter()
                .any(|r| r.kind == RuleKind::Enclosure && r.layer == VIA2.layer),
            "subset has no via2 enclosure rule"
        );
    }
}
