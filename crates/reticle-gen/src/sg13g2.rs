//! IHP SG13G2's [`GenTech`]: the generator numbers for the second PDK.
//!
//! SG13G2 is IHP's open 130 nm `SiGe` `BiCMOS` process, published as the IHP-Open-PDK
//! under Apache-2.0. The generators route on the same four-conductor / three-cut role
//! stack they use for SKY130 (see [`crate::gentech`]); on SG13G2 those roles bind to
//! the bottom of the aluminium BEOL:
//!
//! | role            | SKY130 | SG13G2  | GDS   |
//! |-----------------|--------|---------|-------|
//! | conductor 0     | li1    | Metal1  | 8/0   |
//! | conductor 1     | met1   | Metal2  | 10/0  |
//! | conductor 2     | met2   | Metal3  | 30/0  |
//! | conductor 3     | met3   | Metal4  | 50/0  |
//! | cut 0 (0↔1)     | mcon   | Via1    | 19/0  |
//! | cut 1 (1↔2)     | via    | Via2    | 29/0  |
//! | cut 2 (2↔3)     | via2   | Via3    | 49/0  |
//! | substrate tap   | licon  | Cont    | 6/0   |
//!
//! # Provenance
//!
//! Every number is transcribed from the committed `tech/sg13g2-drc-subset.toml`, which
//! cites the IHP-Open-PDK `KLayout` DRC runset it came from (rule ids in brackets). In
//! DBU (1 dbu = 1 nm, per `sg13g2.lyt`):
//!
//! * Metal1 width 0.16 / spacing 0.18 (M1.a, M1.b); Metal2..4 width 0.20 / spacing
//!   0.21 (M2.a..M4.b). The open `KLayout` deck carries no metal minimum-area rule.
//! * Via1 size 0.19 (V1.a), enclosed by Metal1 by 0.01 (V1.c). Via2/Via3 size 0.19
//!   (Via2.a/Via3.a), each enclosed by the metal above by 0.005 (Via2.c/Via3.c).
//! * Cont size 0.16 (Cnt.a). The open `KLayout` deck carries no Metal1-encloses-Cont
//!   rule (Cont width equals Metal1 width, so a min-width Metal1 already covers it);
//!   the guard-ring taps keep a small conservative overlap instead (see below).
//!
//! Source: IHP-GmbH/IHP-Open-PDK, `ihp-sg13g2/libs.tech/klayout/tech/drc/rule_decks/`
//! (branch main), Apache-2.0. See `tech/sg13g2-drc-subset.toml` for the exact URLs.
//!
//! # Honest scope
//!
//! This is a *subset* mirroring the digital routing stack, exactly like the SKY130
//! subset: it carries metal/cut width and spacing and the via enclosures, and omits
//! what the generators do not draw against (the wide-metal and pattern-density
//! spacing variants, the FEOL Activ/GatPoly contact enclosures, the thick `TopMetal`
//! stack). Passing it is not tape-out clean.

use reticle_geometry::LayerId;

use crate::gentech::{Conductor, Cut, GenTech, Residue};

// --- Conductor layers (bottom-to-top of the routing stack). ---

/// Metal1 (8/0): width 160, spacing 180 (M1.a, M1.b). No metal area rule in the deck.
pub const METAL1: Conductor = Conductor {
    layer: LayerId::new(8, 0),
    min_width: 160,
    min_spacing: 180,
    min_area: None,
};

/// Metal2 (10/0): width 200, spacing 210 (M2.a, M2.b).
pub const METAL2: Conductor = Conductor {
    layer: LayerId::new(10, 0),
    min_width: 200,
    min_spacing: 210,
    min_area: None,
};

/// Metal3 (30/0): width 200, spacing 210 (M3.a, M3.b).
pub const METAL3: Conductor = Conductor {
    layer: LayerId::new(30, 0),
    min_width: 200,
    min_spacing: 210,
    min_area: None,
};

/// Metal4 (50/0): width 200, spacing 210 (M4.a, M4.b).
pub const METAL4: Conductor = Conductor {
    layer: LayerId::new(50, 0),
    min_width: 200,
    min_spacing: 210,
    min_area: None,
};

// --- Cut layers (contact/via sizes and their enclosures). ---

/// Via1 (19/0): size 190, enclosed by Metal1 by 10 (V1.a, V1.c). Bridges Metal1 and
/// Metal2. The generators grow both covering plates by this margin.
pub const VIA1: Cut = Cut {
    layer: LayerId::new(19, 0),
    size: 190,
    enclosure: Some((METAL2.layer, 10)),
};

/// Via2 (29/0): size 190, enclosed by Metal3 by 5 (Via2.a, Via2.c). Bridges Metal2
/// and Metal3.
pub const VIA2: Cut = Cut {
    layer: LayerId::new(29, 0),
    size: 190,
    enclosure: Some((METAL3.layer, 5)),
};

/// Via3 (49/0): size 190, enclosed by Metal4 by 5 (Via3.a, Via3.c). Bridges Metal3
/// and Metal4.
pub const VIA3: Cut = Cut {
    layer: LayerId::new(49, 0),
    size: 190,
    enclosure: Some((METAL4.layer, 5)),
};

/// Cont (6/0): size 160 (Cnt.a). The substrate-tap contact. The open deck carries no
/// Metal1-encloses-Cont rule, so the guard-ring taps keep the conservative overlap
/// below (`CONT_FALLBACK_ENCLOSURE`) rather than a deck-checked one.
pub const CONT: Cut = Cut {
    layer: LayerId::new(6, 0),
    size: 160,
    enclosure: Some((METAL1.layer, CONT_FALLBACK_ENCLOSURE)),
};

/// A safe cut-to-cut pitch margin: cuts are pitched at their size plus this. Chosen
/// above the largest cut spacing in the subset (Via1..3 spacing 220, Cont spacing
/// 180), so every cut run clears the deck's cut spacing.
pub const SAFE_CUT_MARGIN: i32 = 250;

/// The conservative Metal1 overlap the guard-ring taps keep around a `Cont`, which
/// the open deck carries no rule for.
const CONT_FALLBACK_ENCLOSURE: i32 = 60;

/// SG13G2's generator technology: Metal1..Metal4, the Via1/Via2/Via3 that bridge them,
/// the `Cont` substrate tap, and the conservative cut pitch.
pub const GENTECH: GenTech = GenTech::new(
    "sg13g2",
    [METAL1, METAL2, METAL3, METAL4],
    [VIA1, VIA2, VIA3],
    CONT,
    SAFE_CUT_MARGIN,
);

/// The residue [`crate::gentech::derive`] needs to reconstruct [`GENTECH`] from the
/// committed SG13G2 subset: the role assignment plus the tap overlap and cut pitch the
/// open deck lacks. Every via carries its own enclosure rule, so no via fallback is
/// needed.
pub const RESIDUE: Residue = Residue {
    name: "sg13g2",
    conductors: [METAL1.layer, METAL2.layer, METAL3.layer, METAL4.layer],
    cuts: [VIA1.layer, VIA2.layer, VIA3.layer],
    tap_cut: CONT.layer,
    cut_fallback_enclosure: [None, None, None],
    tap_fallback_enclosure: Some(CONT_FALLBACK_ENCLOSURE),
    safe_cut_margin: SAFE_CUT_MARGIN,
};
