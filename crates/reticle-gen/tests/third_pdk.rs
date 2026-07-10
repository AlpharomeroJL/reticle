//! Third-PDK cleanliness oracle: every generator, over randomized valid parameters,
//! must emit geometry the real DRC engine finds *zero* violations in for
//! `GlobalFoundries` GF180MCU, mirroring the cross-PDK proof in `second_pdk.rs` for
//! SKY130 and IHP SG13G2.
//!
//! This is the proof that the `GenTech` refactor stays data-driven for a third,
//! structurally different process: the same generator code, handed the GF180MCU
//! [`Technology`] by name, draws against that process's own layers, widths,
//! spacings, and enclosures and stays clean. Like `second_pdk.rs`, the oracle is
//! the production checker ([`RuleSet::check_cell`]) run over the committed rule
//! deck, not a reimplementation.
//!
//! GF180MCU's committed subset (see `crates/reticle-gen/src/gf180.rs`) carries only
//! two real interconnect levels (Metal1, Metal2) bridged by one cut (Via1), unlike
//! SKY130's and SG13G2's four-level stacks, so [`GenTech::gf180`]'s top two
//! conductor slots repeat Metal2 and its two padded cut slots repeat Contact. This
//! file's sampling ranges account for that: every dimension clears GF180MCU's own
//! (sometimes larger per-role) floors, not just the SKY130-referenced bounds each
//! generator's `validate()` checks against, and the via-farm sampler avoids the one
//! single-cut-array edge case Via1's zero-margin sourced enclosure does not cover
//! (documented at the sampler).

use proptest::prelude::*;
use reticle_drc::DrcEngine;
use reticle_gen::{
    CutKind, FillGen, FillLayer, FillParams, GenParams, GenTech, Generator, GuardRing,
    GuardRingParams, PadRing, PadRingParams, Registry, RingLayer, SealRing, SealRingParams,
    SealStack, StructureKind, StructureLayer, TestStructure, TestStructureParams, ViaFarm,
    ViaFarmParams, derive_gentech,
};
use reticle_io::parse_technology;
use reticle_model::{Cell, Document, Rule, RuleKind, RuleSet, Technology, Violation};

/// The committed GF180MCU technology file (layers, stack, and the inline DRC
/// subset).
const GF180_TECH: &str = include_str!("../../../tech/gf180.tech");
/// The committed GF180MCU DRC subset, the cited source of record for the rules.
const GF180_DRC_SUBSET: &str = include_str!("../../../tech/gf180-drc-subset.toml");

/// The GF180MCU process under test: a [`Technology`] (whose name selects the
/// generators' `GenTech`) and the [`DrcEngine`] loaded with that process's rules.
struct Pdk {
    tech: Technology,
    engine: DrcEngine,
}

/// The GF180MCU process, parsed from the committed technology file.
fn gf180_pdk() -> Pdk {
    let tech = parse_technology(GF180_TECH).expect("committed tech/gf180.tech must parse");
    let engine = DrcEngine::new(tech.rules.clone());
    Pdk { tech, engine }
}

/// Generates `params` into a fresh cell under gf180 and returns the DRC violations
/// under the committed gf180 deck.
fn violations<G: Generator>(generator: &G, params: &G::Params, pdk: &Pdk) -> Vec<Violation> {
    let mut cell = Cell::new("top");
    generator
        .generate(params, &pdk.tech, &mut cell)
        .expect("valid params must generate");
    let mut doc = Document::new();
    doc.insert_cell(cell);
    pdk.engine.check_cell(&doc, "top")
}

/// A concise message listing the first few violations for a failed assertion.
fn describe(violations: &[Violation]) -> String {
    violations
        .iter()
        .take(4)
        .map(|v| format!("{}: {}", v.rule, v.message))
        .collect::<Vec<_>>()
        .join(" | ")
}

// --- Valid-parameter samplers (well above gf180's floors: Metal1 230, Metal2 280,
// Contact 220/spacing 250, Via1 260/spacing 260). Every range below has a minimum
// that already clears the largest of those (280), so no per-role branching is
// needed the way `GenTech::gf180`'s repeated roles might otherwise require. ---

fn valid_guard_ring() -> impl Strategy<Value = GuardRingParams> {
    let layer = prop_oneof![
        Just(RingLayer::Li1),
        Just(RingLayer::Met1),
        Just(RingLayer::Met2),
        Just(RingLayer::Met3),
    ];
    (
        layer,
        400..8_000i32,
        400..8_000i32,
        400..3_000i32,
        any::<bool>(),
    )
        .prop_map(
            |(layer, region_width, region_height, ring_width, want_taps)| {
                // Taps only where valid: on the base interconnect with a wide-enough ring.
                let taps = want_taps && layer == RingLayer::Li1 && ring_width >= 400;
                GuardRingParams {
                    layer,
                    region_width,
                    region_height,
                    ring_width,
                    taps,
                }
            },
        )
}

/// Via-farm parameters. Rows/cols start at **2**, not 1: gf180's Via1 enclosure
/// (`V1.3a`) is a genuine zero-margin sourced rule (see `src/gf180.rs`), so a
/// single-cut (1x1) array's covering plate would be exactly Via1's own 260 size,
/// which clears Metal1's 230 floor but falls short of Metal2's wider 280 floor.
/// Sampling at least a 2x2 array keeps every plate comfortably above both floors
/// (the pitch already exceeds 260, so a 2-count axis's plate is far past 280)
/// without inflating the sourced enclosure number to paper over one corner case.
fn valid_via_farm() -> impl Strategy<Value = ViaFarmParams> {
    let cut = prop_oneof![Just(CutKind::Mcon), Just(CutKind::Via), Just(CutKind::Via2)];
    (cut, 2..20u32, 2..20u32).prop_map(|(cut, rows, cols)| ViaFarmParams { cut, rows, cols })
}

fn valid_fill() -> impl Strategy<Value = FillParams> {
    let layer = prop_oneof![
        Just(FillLayer::Li1),
        Just(FillLayer::Met1),
        Just(FillLayer::Met2),
        Just(FillLayer::Met3),
    ];
    (
        layer,
        400..1_500i32,
        6_000..16_000i32,
        6_000..16_000i32,
        1..900i32,
    )
        .prop_map(
            |(layer, tile, region_width, region_height, density)| FillParams {
                layer,
                region_width,
                region_height,
                tile,
                target_density_permille: density,
                keepouts: Vec::new(),
            },
        )
}

fn valid_test_structure() -> impl Strategy<Value = TestStructureParams> {
    let kind = prop_oneof![
        Just(StructureKind::VanDerPauw),
        Just(StructureKind::ContactChain),
        Just(StructureKind::Comb),
        Just(StructureKind::Serpentine),
    ];
    let layer = prop_oneof![
        Just(StructureLayer::Li1),
        Just(StructureLayer::Met1),
        Just(StructureLayer::Met2),
        Just(StructureLayer::Met3),
    ];
    (kind, layer, 300..1_500i32, 400..4_000i32, 2..32u32).prop_map(
        |(kind, layer, feature_width, slack, count)| {
            // Length clears every per-layer, per-kind floor at once (the serpentine's
            // 2*width + spacing geometric floor is the largest one gf180 has, since
            // this subset carries no metal area rule): 400 already exceeds gf180's
            // widest spacing (280), so `2*width + 400 + slack` always clears it.
            let feature_length = 2 * feature_width + 400 + slack;
            TestStructureParams {
                kind,
                layer,
                feature_width,
                feature_length,
                count,
            }
        },
    )
}

fn valid_pad_ring() -> impl Strategy<Value = PadRingParams> {
    (
        350_000..450_000i32,
        350_000..450_000i32,
        30_000..70_000i32,
        30_000..90_000i32,
        0..6u32,
    )
        .prop_map(
            |(die_width, die_height, pad_size, extra_pitch, power_pads)| PadRingParams {
                die_width,
                die_height,
                pad_pitch: pad_size + extra_pitch,
                pad_size,
                power_pads,
            },
        )
}

fn valid_seal_ring() -> impl Strategy<Value = SealRingParams> {
    let stack = prop_oneof![
        Just(SealStack::Li1Met1),
        Just(SealStack::UpToMet2),
        Just(SealStack::UpToMet3),
    ];
    (stack, 40_000..200_000i32, 40_000..200_000i32, 900..3_000i32).prop_map(
        |(stack, die_width, die_height, ring_width)| SealRingParams {
            stack,
            die_width,
            die_height,
            ring_width,
        },
    )
}

/// Asserts a generator's geometry is DRC-clean on gf180 for one sample.
macro_rules! assert_clean_on_gf180 {
    ($gen:expr, $params:expr, $pdk:expr) => {{
        let found = violations(&$gen, &$params, &$pdk);
        prop_assert!(
            found.is_empty(),
            "{} on gf180: {:?} produced {} violation(s): {}",
            stringify!($gen),
            $params,
            found.len(),
            describe(&found)
        );
    }};
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn guard_ring_clean_on_gf180(params in valid_guard_ring()) {
        prop_assume!(params.validate().is_ok());
        assert_clean_on_gf180!(GuardRing, params, gf180_pdk());
    }

    #[test]
    fn via_farm_clean_on_gf180(params in valid_via_farm()) {
        prop_assume!(params.validate().is_ok());
        assert_clean_on_gf180!(ViaFarm, params, gf180_pdk());
    }

    #[test]
    fn fill_clean_on_gf180(params in valid_fill()) {
        prop_assume!(params.validate().is_ok());
        assert_clean_on_gf180!(FillGen, params, gf180_pdk());
    }

    #[test]
    fn test_structure_clean_on_gf180(params in valid_test_structure()) {
        prop_assume!(params.validate().is_ok());
        assert_clean_on_gf180!(TestStructure, params, gf180_pdk());
    }

    #[test]
    fn pad_ring_clean_on_gf180(params in valid_pad_ring()) {
        prop_assume!(params.validate().is_ok());
        assert_clean_on_gf180!(PadRing, params, gf180_pdk());
    }

    #[test]
    fn seal_ring_clean_on_gf180(params in valid_seal_ring()) {
        prop_assume!(params.validate().is_ok());
        assert_clean_on_gf180!(SealRing, params, gf180_pdk());
    }
}

/// Driving every registered generator from its default JSON parameters is clean on
/// gf180, exercising the registry's type-erased path end to end.
#[test]
fn registry_defaults_clean_on_gf180() {
    let reg = Registry::with_builtins();
    let pdk = gf180_pdk();
    for id in reg.ids() {
        let params = reg.default_params(id).expect("registered generator");
        let mut cell = Cell::new("top");
        reg.generate(id, &params, &pdk.tech, &mut cell)
            .expect("default params generate");
        let mut doc = Document::new();
        doc.insert_cell(cell);
        let found = pdk.engine.check_cell(&doc, "top");
        assert!(
            found.is_empty(),
            "generator {id} on gf180 produced violations: {}",
            describe(&found)
        );
    }
}

/// The real, non-padded numbers in [`GenTech::gf180`] must trace to the parsed
/// committed technology file: both conductors, every cut, and the tap, each
/// spot-checked against the specific `tech/gf180.tech` rule it comes from. This is
/// the same anchor `sg13g2.rs`'s and `sky130.rs`'s unit tests give their own
/// numbers, applied here directly against the parsed rules (`tech/gf180.tech`'s
/// inline rules carry no rule-id names, only auto-generated `kind_layer_datatype`
/// ones, so matching is by kind/layer/other-layer rather than by cited name; the
/// companion `gf180_tech_rules_match_drc_subset` test below ties those parsed
/// rules back to the cited `.toml` rule ids).
#[test]
fn gf180_gentech_constants_match_parsed_tech() {
    let tech = parse_technology(GF180_TECH).expect("parse gf180.tech");
    let rules = &tech.rules;
    let gt = GenTech::gf180();

    let find = |kind: RuleKind, layer: reticle_geometry::LayerId| -> &Rule {
        rules
            .iter()
            .find(|r| r.kind == kind && r.layer == layer)
            .unwrap_or_else(|| panic!("no {kind:?} rule for layer {layer:?} in gf180.tech"))
    };

    let metal1 = reticle_geometry::LayerId::new(34, 0);
    let metal2 = reticle_geometry::LayerId::new(36, 0);
    let via1 = reticle_geometry::LayerId::new(35, 0);
    let contact = reticle_geometry::LayerId::new(33, 0);

    // conductor(0): Metal1.
    assert_eq!(gt.conductor(0).layer, metal1);
    assert_eq!(
        gt.conductor(0).min_width,
        i32::try_from(find(RuleKind::Width, metal1).value).unwrap()
    );
    assert_eq!(
        gt.conductor(0).min_spacing,
        i32::try_from(find(RuleKind::Spacing, metal1).value).unwrap()
    );

    // conductor(1): Metal2. Repeated (by construction, not by a second deck entry)
    // at conductor(2) and conductor(3); see src/gf180.rs's module docs.
    assert_eq!(gt.conductor(1).layer, metal2);
    assert_eq!(
        gt.conductor(1).min_width,
        i32::try_from(find(RuleKind::Width, metal2).value).unwrap()
    );
    assert_eq!(
        gt.conductor(1).min_spacing,
        i32::try_from(find(RuleKind::Spacing, metal2).value).unwrap()
    );
    assert_eq!(
        gt.conductor(2),
        gt.conductor(1),
        "conductor(2) repeats Metal2"
    );
    assert_eq!(
        gt.conductor(3),
        gt.conductor(1),
        "conductor(3) repeats Metal2"
    );

    // cut(0): Via1, enclosed by Metal1 at the deck's own sourced margin (zero).
    assert_eq!(gt.cut(0).layer, via1);
    assert_eq!(
        gt.cut(0).size,
        i32::try_from(find(RuleKind::Width, via1).value).unwrap()
    );
    let via1_enc = rules
        .iter()
        .find(|r| r.kind == RuleKind::Enclosure && r.layer == via1 && r.other_layer == Some(metal1))
        .expect("V1.3a enclosure rule");
    assert_eq!(
        gt.cut(0).enclosure,
        Some((metal1, i32::try_from(via1_enc.value).unwrap()))
    );

    // The deck carries no enclosure rule for Contact at all, so cut(1)/cut(2)/the
    // tap use a fallback GenTech::gf180 documents, not a deck-derived number.
    assert!(
        !rules
            .iter()
            .any(|r| r.kind == RuleKind::Enclosure && r.layer == contact),
        "subset has no Contact enclosure rule"
    );
    assert_eq!(gt.cut(1).layer, contact);
    assert_eq!(
        gt.cut(1).size,
        i32::try_from(find(RuleKind::Width, contact).value).unwrap()
    );
    assert_eq!(gt.cut(1), gt.cut(2), "cut(2) repeats the Contact stand-in");
    assert_eq!(gt.tap_cut().layer, contact);
    assert_eq!(gt.tap_cut().size, gt.cut(1).size);
}

/// [`derive_gentech`] on the parsed gf180 technology cannot succeed end to end
/// against the full four-slot [`GenTech::GF180_RESIDUE`] the way the SG13G2
/// equivalent does in `second_pdk.rs`: gf180's subset has only two real
/// interconnect levels, so the residue lists the same physical layer (Metal2)
/// twice, and the shared `verify_stack_order` guard correctly rejects a residue
/// that lists one physical layer at two different stack slots (it exists to catch
/// exactly that ambiguity). This test proves that rejection is the actual,
/// specific, documented reason -- not a silent or unrelated failure -- so a
/// future change to the guard or the deck that turned this into a different kind
/// of failure would be caught here.
#[test]
fn gf180_full_residue_rejected_by_stack_order_guard() {
    let tech = parse_technology(GF180_TECH).expect("parse gf180.tech");
    let err = derive_gentech(&tech, &GenTech::GF180_RESIDUE)
        .expect_err("a residue repeating Metal2 across two stack slots must be rejected");
    assert!(
        err.contains("not above the previous"),
        "expected the stack-order guard's message, got: {err}"
    );
}

/// The inline DRC rules in `gf180.tech` must match the cited `gf180-drc-subset.toml`
/// exactly (as sets of kind/layer/other/value), so the two committed
/// representations of the subset cannot silently diverge. Mirrors
/// `second_pdk.rs`'s `sg13g2_tech_rules_match_drc_subset`.
#[test]
fn gf180_tech_rules_match_drc_subset() {
    use std::collections::BTreeSet;

    #[derive(serde::Deserialize)]
    struct RuleFile {
        rule: Vec<RawRule>,
    }
    #[derive(serde::Deserialize)]
    struct RawRule {
        kind: String,
        layer: [u16; 2],
        other_layer: Option<[u16; 2]>,
        value_dbu: i64,
    }

    // Canonical tuple: (kind, layer, datatype, other?, value).
    type Key = (String, u16, u16, Option<(u16, u16)>, i64);

    let tech = parse_technology(GF180_TECH).expect("parse gf180.tech");
    let from_tech: BTreeSet<Key> = tech
        .rules
        .iter()
        .map(|r| {
            (
                format!("{:?}", r.kind).to_lowercase(),
                r.layer.layer,
                r.layer.datatype,
                r.other_layer.map(|l| (l.layer, l.datatype)),
                r.value,
            )
        })
        .collect();

    let subset: RuleFile = toml::from_str(GF180_DRC_SUBSET).expect("parse gf180 drc subset");
    let from_toml: BTreeSet<Key> = subset
        .rule
        .iter()
        .map(|r| {
            (
                r.kind.to_lowercase(),
                r.layer[0],
                r.layer[1],
                r.other_layer.map(|l| (l[0], l[1])),
                r.value_dbu,
            )
        })
        .collect();

    assert_eq!(
        from_tech, from_toml,
        "gf180.tech rules and gf180-drc-subset.toml diverge"
    );
}
