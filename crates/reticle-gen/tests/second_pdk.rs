//! Cross-PDK cleanliness oracle: every generator, over randomized valid parameters,
//! must emit geometry the real DRC engine finds *zero* violations in for **both**
//! shipped processes — SKY130 and IHP SG13G2.
//!
//! This is the proof that the `GenTech` refactor is data-driven, not SKY130-shaped: the
//! same generator code, handed a different [`Technology`] by name, draws against that
//! process's own layers, widths, spacings, and enclosures and stays clean. Like the
//! sibling `property.rs`, the oracle is the production checker
//! ([`RuleSet::check_cell`]) run over the committed rule deck, not a reimplementation.
//!
//! The two decks come from the committed tech files: SKY130 through
//! [`reticle_drc::sky130_drc_rules`], and SG13G2 by parsing `tech/ihp-sg13g2.tech`
//! (whose inline rules are the cited subset in `tech/sg13g2-drc-subset.toml`). Two
//! provenance tests anchor the SG13G2 data: its `GenTech` is reconstructed from the
//! parsed technology, and the `.tech` rules are checked against the `.toml` subset.

use proptest::prelude::*;
use reticle_drc::{DrcEngine, sky130_drc_rules};
use reticle_gen::{
    CutKind, FillGen, FillLayer, FillParams, GenParams, GenTech, Generator, GuardRing,
    GuardRingParams, PadRing, PadRingParams, Registry, RingLayer, SealRing, SealRingParams,
    SealStack, StructureKind, StructureLayer, TestStructure, TestStructureParams, ViaFarm,
    ViaFarmParams, derive_gentech,
};
use reticle_io::parse_technology;
use reticle_model::{Cell, Document, RuleSet, Technology, Violation};

/// The committed SG13G2 technology file (layers, stack, and the inline DRC subset).
const SG13G2_TECH: &str = include_str!("../../../tech/ihp-sg13g2.tech");
/// The committed SG13G2 DRC subset, the cited source of record for the rules.
const SG13G2_DRC_SUBSET: &str = include_str!("../../../tech/sg13g2-drc-subset.toml");

/// One process under test: a [`Technology`] (whose name selects the generators'
/// `GenTech`) and the [`DrcEngine`] loaded with that process's rules.
struct Pdk {
    tech: Technology,
    engine: DrcEngine,
}

/// The SKY130 process: name selects the SKY130 `GenTech`; the engine runs the committed
/// SKY130 subset.
fn sky130_pdk() -> Pdk {
    let tech = Technology {
        name: "sky130".to_string(),
        rules: sky130_drc_rules(),
        ..Technology::default()
    };
    let engine = DrcEngine::new(sky130_drc_rules());
    Pdk { tech, engine }
}

/// The IHP SG13G2 process, parsed from the committed technology file.
fn sg13g2_pdk() -> Pdk {
    let tech = parse_technology(SG13G2_TECH).expect("committed tech/ihp-sg13g2.tech must parse");
    let engine = DrcEngine::new(tech.rules.clone());
    Pdk { tech, engine }
}

/// Both shipped processes.
fn pdks() -> Vec<Pdk> {
    vec![sky130_pdk(), sg13g2_pdk()]
}

/// Generates `params` into a fresh cell under `pdk`'s technology and returns the DRC
/// violations under that process's deck.
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

// --- Valid-parameter samplers (well above both processes' floors). ---

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

fn valid_via_farm() -> impl Strategy<Value = ViaFarmParams> {
    let cut = prop_oneof![Just(CutKind::Mcon), Just(CutKind::Via), Just(CutKind::Via2)];
    (cut, 1..20u32, 1..20u32).prop_map(|(cut, rows, cols)| ViaFarmParams { cut, rows, cols })
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
            // Length clears every per-layer, per-kind floor at once (area floor and the
            // serpentine's 2*width + spacing geometric floor).
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

/// Asserts a generator's geometry is DRC-clean on every process for one sample.
macro_rules! assert_clean_on_all_pdks {
    ($gen:expr, $params:expr) => {{
        for pdk in pdks() {
            let found = violations(&$gen, &$params, &pdk);
            prop_assert!(
                found.is_empty(),
                "{} on {}: {:?} produced {} violation(s): {}",
                stringify!($gen),
                pdk.tech.name,
                $params,
                found.len(),
                describe(&found)
            );
        }
    }};
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn guard_ring_clean_on_both_pdks(params in valid_guard_ring()) {
        prop_assume!(params.validate().is_ok());
        assert_clean_on_all_pdks!(GuardRing, params);
    }

    #[test]
    fn via_farm_clean_on_both_pdks(params in valid_via_farm()) {
        prop_assume!(params.validate().is_ok());
        assert_clean_on_all_pdks!(ViaFarm, params);
    }

    #[test]
    fn fill_clean_on_both_pdks(params in valid_fill()) {
        prop_assume!(params.validate().is_ok());
        assert_clean_on_all_pdks!(FillGen, params);
    }

    #[test]
    fn test_structure_clean_on_both_pdks(params in valid_test_structure()) {
        prop_assume!(params.validate().is_ok());
        assert_clean_on_all_pdks!(TestStructure, params);
    }

    #[test]
    fn pad_ring_clean_on_both_pdks(params in valid_pad_ring()) {
        prop_assume!(params.validate().is_ok());
        assert_clean_on_all_pdks!(PadRing, params);
    }

    #[test]
    fn seal_ring_clean_on_both_pdks(params in valid_seal_ring()) {
        prop_assume!(params.validate().is_ok());
        assert_clean_on_all_pdks!(SealRing, params);
    }
}

/// Driving every registered generator from its default JSON parameters is clean on
/// both processes, exercising the registry's type-erased path end to end.
#[test]
fn registry_defaults_clean_on_both_pdks() {
    let reg = Registry::with_builtins();
    for pdk in pdks() {
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
                "generator {id} on {} produced violations: {}",
                pdk.tech.name,
                describe(&found)
            );
        }
    }
}

/// The SG13G2 [`GenTech`] the generators ship with must be reconstructible from the
/// parsed committed technology file plus the residue, so the authored constants cannot
/// drift from the committed deck or the stack ordering.
#[test]
fn sg13g2_gentech_derives_from_parsed_tech() {
    let tech = parse_technology(SG13G2_TECH).expect("parse ihp-sg13g2.tech");
    let derived =
        derive_gentech(&tech, &GenTech::SG13G2_RESIDUE).expect("derive sg13g2 GenTech from tech");
    assert_eq!(
        derived,
        GenTech::sg13g2(),
        "authored SG13G2 GenTech drifted from the committed technology file"
    );
}

/// The inline DRC rules in `ihp-sg13g2.tech` must match the cited `sg13g2-drc-subset.toml`
/// exactly (as sets of kind/layer/other/value), so the two committed representations of
/// the subset cannot silently diverge.
#[test]
fn sg13g2_tech_rules_match_drc_subset() {
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

    let tech = parse_technology(SG13G2_TECH).expect("parse ihp-sg13g2.tech");
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

    let subset: RuleFile = toml::from_str(SG13G2_DRC_SUBSET).expect("parse sg13g2 drc subset");
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
        "ihp-sg13g2.tech rules and sg13g2-drc-subset.toml diverge"
    );
}
