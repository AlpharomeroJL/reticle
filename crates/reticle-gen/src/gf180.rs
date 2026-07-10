//! `GlobalFoundries` GF180MCU's [`GenTech`]: the generator numbers for the third PDK.
//!
//! GF180MCU is `GlobalFoundries`' open 180 nm mixed-signal/RF process, published as
//! the `gf180mcu_fd_pr` PDK under Apache-2.0. Every number here is transcribed from
//! the committed `tech/gf180-drc-subset.toml` and the layer map in `tech/gf180.tech`
//! (both cite the upstream `KLayout` DRC rule decks and the Design Rule Manual; see
//! those files' headers for the exact source lines and rule ids). This module does
//! not re-derive or re-cite provenance beyond pointing at those files; the numbers
//! are transcribed from the committed, already-merged deck.
//!
//! # A two-metal subset, padded to the four-level shape
//!
//! Unlike the SKY130 and SG13G2 subsets (four interconnect conductors each), the
//! committed GF180MCU subset carries width *and* spacing rules for only **two**
//! interconnect conductors: `Metal1` and `Metal2`, bridged by one cut, `Via1`.
//! `tech/gf180.tech` stops at `Metal2`/`Via1` by design (see its header): that is
//! the base subset the generators draw against, not an oversight. [`GenTech`] is
//! shaped for a four-level stack (mirroring every process it currently ships), so
//! the top two conductor slots ([`GenTech::conductor`]`(2)` and `(3)`) repeat
//! `Metal2` rather than name a third or fourth metal this subset does not have.
//!
//! This is safe for every generator here: each draws at most one bare conductor
//! shape per role, and no generator relies on two *different* physical layers
//! occupying slots 2 and 3 simultaneously (see `tests/third_pdk.rs`'s oracle, which
//! exercises every role, including the repeated ones, against the real DRC engine).
//! A repeated `Metal2` behaves exactly like the real top level, just reachable
//! under two role names ("met2" and "met3"). See [`RESIDUE`] for what this means
//! for [`derive_gentech`](crate::gentech::derive_gentech).
//!
//! Symmetrically, the subset gives only one bridging cut (`Via1`, between `Metal1`
//! and `Metal2`); the two padded cut slots ([`GenTech::cut`]`(1)` and `(2)`, which
//! would bridge the padded conductor levels) reuse `Contact` rather than `Via1`.
//! `Via1`'s one enclosure rule (`V1.3a`) requires a *real* `Metal1` shape under
//! every `Via1` shape; a generator that draws a cut role in isolation (a via
//! farm's plate, a pad-ring staple) would then draw a bare `Via1` with no `Metal1`
//! anywhere nearby, which genuinely violates `V1.3a`. `Contact` carries no
//! enclosure rule in this subset at all (see below), so a bare `Contact` shape can
//! never violate one; reusing it for the padded cut slots keeps every generator
//! clean-by-construction without inventing a rule the deck does not have.
//!
//! # Provenance
//!
//! In DBU (1 dbu = 1 nm, `dbu_per_micron 1000` per `tech/gf180.tech`):
//!
//! * Metal1 width/spacing 230 (`M1.1`, `M1.2a`); Metal2 width/spacing 280 (`M2.1`,
//!   `M2.2a`). Neither carries a minimum-area rule in this subset (`M1.3`/`M2.3`
//!   are out of scope, see `tech/gf180-drc-subset.toml`'s header).
//! * Via1 size 260 (`V1.1`), enclosed by Metal1 with **zero** margin (`V1.3a`,
//!   `via1.not(metal1)`: exact containment, not a placeholder -- the source has no
//!   numeric margin beyond exact containment). [`VIA1`] keeps that number exactly
//!   as sourced rather than inflating it; `tests/third_pdk.rs` documents the one
//!   place this matters (a single-cut via farm's plate would then be exactly
//!   Via1's own 260 size, which clears Metal1's 230 floor but not Metal2's wider
//!   280 floor, so that oracle samples at least a 2x2 array instead of inflating
//!   the sourced margin).
//! * Contact size 220 (`CO.1`), spacing 250 (`CO.2a`). The subset carries no
//!   Metal1-encloses-Contact rule (the upstream `CO.5`-`CO.8` enclosure rules are
//!   out of this subset's scope), so [`CONTACT`]'s enclosure is a conservative
//!   fallback, not a sourced number (mirroring SG13G2's `Cont`, which has the same
//!   gap).
//!
//! Source: the open GF180MCU PDK (`gf180mcu_fd_pr`, branch main), Apache-2.0. See
//! `tech/gf180.tech` and `tech/gf180-drc-subset.toml` for the exact upstream URLs
//! and rule ids.
//!
//! # Honest scope
//!
//! This is a *subset* of a subset: the digital two-metal routing stack
//! (`Metal1`/`Metal2`/`Via1`) plus the `Contact` substrate tap, omitting the
//! well/implant layers (`Nwell`/`Pplus`/`Nplus`, present in `tech/gf180.tech`'s
//! layer table but carrying no DRC rules the generators draw against), the
//! Poly2/COMP FEOL rules, every wide-metal and array-conditional variant, and
//! Via1's Metal2-side enclosure (`V1.4a`, sourced but out of the committed
//! subset's scope). Passing it is not tape-out clean, exactly like the SKY130 and
//! SG13G2 subsets it mirrors.

use reticle_geometry::LayerId;

use crate::gentech::{Conductor, Cut, GenTech, Residue};

// --- Conductor layers (the two real interconnect levels the subset carries). ---

/// Metal1 (34/0): width 230, spacing 230 (`M1.1`, `M1.2a`). No metal area rule in
/// this subset. The base interconnect: `conductor(0)`.
pub const METAL1: Conductor = Conductor {
    layer: LayerId::new(34, 0),
    min_width: 230,
    min_spacing: 230,
    min_area: None,
};

/// Metal2 (36/0): width 280, spacing 280 (`M2.1`, `M2.2a`). No metal area rule in
/// this subset. The top (and only other) real interconnect level: `conductor(1)`,
/// repeated at `conductor(2)` and `conductor(3)` (see the module docs).
pub const METAL2: Conductor = Conductor {
    layer: LayerId::new(36, 0),
    min_width: 280,
    min_spacing: 280,
    min_area: None,
};

// --- Cut layers. ---

/// Via1 (35/0): size 260, enclosed by Metal1 with **zero** margin (`V1.1`,
/// `V1.3a`). Bridges Metal1 and Metal2: `cut(0)`. The zero margin is the sourced
/// value (exact containment), kept as-is rather than inflated; see the module
/// docs for the one place this matters.
pub const VIA1: Cut = Cut {
    layer: LayerId::new(35, 0),
    size: 260,
    enclosure: Some((METAL1.layer, 0)),
};

/// Contact (33/0): size 220, spacing 250 (`CO.1`, `CO.2a`). The substrate-tap
/// contact (mirroring SG13G2's `Cont`), and also stands in for the padded cut
/// slots `cut(1)`/`cut(2)` (see the module docs): the subset carries no
/// Metal1-encloses-Contact rule, so [`CONTACT_FALLBACK_ENCLOSURE`] is a
/// conservative choice, not a sourced number.
pub const CONTACT: Cut = Cut {
    layer: LayerId::new(33, 0),
    size: 220,
    enclosure: Some((METAL1.layer, CONTACT_FALLBACK_ENCLOSURE)),
};

/// The conservative enclosure [`CONTACT`] carries, which the subset gives no deck
/// rule for. Sized so that even a single-cut covering plate (220 + 2*30 = 280)
/// clears Metal2's own 280 width floor, the widest floor a padded cut(1)/cut(2)
/// plate needs to clear, without inflating a number the deck does supply (compare
/// [`VIA1`]'s enclosure, which stays at its sourced zero).
const CONTACT_FALLBACK_ENCLOSURE: i32 = 30;

/// A safe cut-to-cut pitch margin: cuts are pitched at their size plus this.
/// Chosen above the largest cut spacing in the subset (Via1 260, Contact 250), so
/// every cut run clears the deck's cut spacing.
pub const SAFE_CUT_MARGIN: i32 = 270;

/// GF180MCU's generator technology: Metal1/Metal2 (Metal2 repeated to fill the
/// four-level shape, see the module docs), the Via1 that bridges the two real
/// levels (Contact repeated for the two padded cut slots), the Contact substrate
/// tap, and the conservative cut pitch.
pub const GENTECH: GenTech = GenTech::new(
    "gf180",
    [METAL1, METAL2, METAL2, METAL2],
    [VIA1, CONTACT, CONTACT],
    CONTACT,
    SAFE_CUT_MARGIN,
);

/// The residue documenting [`GENTECH`]'s role assignment, including the repeats
/// the module docs explain.
///
/// Passing this to [`derive_gentech`](crate::gentech::derive_gentech) against the
/// parsed `tech/gf180.tech` does **not** succeed end to end the way
/// [`GenTech::SG13G2_RESIDUE`] does: the shared stack-order guard
/// (`verify_stack_order`) correctly rejects a residue that lists the same
/// physical layer (`Metal2`) twice, because gf180's committed `stack` block gives
/// it one z-position, and two conductor slots resolving to the same z-entry can
/// never be "strictly above the previous" one. That is the guard working as
/// designed: it exists to catch an ambiguous stack, and a genuinely two-level
/// process padded into a four-level shape is exactly that ambiguity, made
/// explicit rather than silently accepted.
///
/// `tests/third_pdk.rs`'s provenance test proves instead what can honestly be
/// proven: every individual number here (both conductors, every cut, the tap)
/// traces to the parsed deck's rules (or is a documented, labeled fallback), and
/// the full four-slot residue is demonstrably rejected by the ordering guard for
/// the reason above, rather than asserting the one-line success `derive_gentech`
/// cannot honestly give for a padded stack.
pub const RESIDUE: Residue = Residue {
    name: "gf180",
    conductors: [METAL1.layer, METAL2.layer, METAL2.layer, METAL2.layer],
    cuts: [VIA1.layer, CONTACT.layer, CONTACT.layer],
    tap_cut: CONTACT.layer,
    cut_fallback_enclosure: [
        None, // Via1 carries a real deck enclosure rule (V1.3a); no fallback needed.
        Some(CONTACT_FALLBACK_ENCLOSURE),
        Some(CONTACT_FALLBACK_ENCLOSURE),
    ],
    tap_fallback_enclosure: Some(CONTACT_FALLBACK_ENCLOSURE),
    safe_cut_margin: SAFE_CUT_MARGIN,
};
