//! Cleanliness oracle: every generator, over randomized valid parameters, must emit
//! geometry the real SKY130 DRC subset finds *zero* violations in.
//!
//! The oracle is deliberately the production checker, not a reimplementation: each
//! test sweeps random in-range parameters, generates into a fresh cell, loads the
//! committed SKY130 rule subset through [`reticle_drc::sky130_drc_rules`], and runs
//! [`RuleSet::check_cell`] over the result. A single violation on any sampled input
//! fails the property, so "DRC-clean by construction" is proven against the same
//! engine the app runs, over the whole valid parameter space rather than a handful
//! of hand-picked cases.
//!
//! Two-directional `validate` coverage lives alongside: for each generator a
//! property asserts that every in-range sample validates, and that out-of-range or
//! contextually-invalid samples are rejected with a [`GenError`] naming the field.

use proptest::prelude::*;
use reticle_drc::{DrcEngine, sky130_drc_rules};
use reticle_gen::{
    CutKind, GenError, GenParams, Generator, GuardRing, GuardRingParams, Registry, RingLayer,
    ViaFarm, ViaFarmParams,
};
use reticle_model::{Cell, Document, RuleSet, Technology};

/// Builds a document holding just the generated cell and returns its DRC violations
/// under the committed SKY130 subset.
fn violations<G: Generator>(generator: &G, params: &G::Params) -> Vec<reticle_model::Violation> {
    let mut cell = Cell::new("top");
    generator
        .generate(params, &Technology::default(), &mut cell)
        .expect("valid params must generate");
    let mut doc = Document::new();
    doc.insert_cell(cell);
    let engine = DrcEngine::new(sky130_drc_rules());
    engine.check_cell(&doc, "top")
}

/// A concise message listing the first few violations for a failed cleanliness
/// assertion, so a counterexample is actionable.
fn describe(violations: &[reticle_model::Violation]) -> String {
    violations
        .iter()
        .take(4)
        .map(|v| format!("{}: {}", v.rule, v.message))
        .collect::<Vec<_>>()
        .join(" | ")
}

// --- Guard ring ---

/// A random *valid* guard-ring parameter set: layer, in-range dimensions, and taps
/// only where they are allowed (li1 with a thick-enough ring).
fn valid_guard_ring() -> impl Strategy<Value = GuardRingParams> {
    let layer = prop_oneof![
        Just(RingLayer::Li1),
        Just(RingLayer::Met1),
        Just(RingLayer::Met2),
        Just(RingLayer::Met3),
    ];
    // Cover from just above the tightest rule up to a mid-size ring.
    (
        layer,
        300..8_000i32,
        300..8_000i32,
        300..3_000i32,
        any::<bool>(),
    )
        .prop_map(
            |(layer, region_width, region_height, ring_width, want_taps)| {
                // Taps are valid only on li1 with a ring wide enough for the enclosure;
                // force those preconditions so the sample stays in the valid space.
                let taps = want_taps && layer == RingLayer::Li1 && ring_width >= 330;
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

/// A random guard-ring parameter set that is *invalid* in exactly one way, paired
/// with the field the rejection must name.
fn invalid_guard_ring() -> impl Strategy<Value = (GuardRingParams, &'static str)> {
    prop_oneof![
        // region_width strictly below the chosen layer's min spacing. Met3's spacing
        // (300) is the largest in the subset, so `0..300` is below it for certain.
        (0..300i32).prop_map(|w| {
            (
                GuardRingParams {
                    layer: RingLayer::Met3,
                    region_width: w,
                    taps: false,
                    ..GuardRingParams::default()
                },
                "region_width",
            )
        }),
        // ring_width below the chosen layer's min width (met1 = 140).
        (0..140i32).prop_map(|w| {
            (
                GuardRingParams {
                    layer: RingLayer::Met1,
                    ring_width: w,
                    taps: false,
                    ..GuardRingParams::default()
                },
                "ring_width",
            )
        }),
        // taps requested on a non-li1 layer.
        Just((
            GuardRingParams {
                layer: RingLayer::Met2,
                taps: true,
                ..GuardRingParams::default()
            },
            "taps",
        )),
    ]
}

// --- Via farm ---

/// A random *valid* via-farm parameter set: any cut kind and in-range array shape.
fn valid_via_farm() -> impl Strategy<Value = ViaFarmParams> {
    let cut = prop_oneof![Just(CutKind::Mcon), Just(CutKind::Via), Just(CutKind::Via2)];
    (cut, 1..24u32, 1..24u32).prop_map(|(cut, rows, cols)| ViaFarmParams { cut, rows, cols })
}

/// A random via-farm parameter set that is *invalid* in one dimension, paired with
/// the field the rejection must name.
fn invalid_via_farm() -> impl Strategy<Value = (ViaFarmParams, &'static str)> {
    prop_oneof![
        prop_oneof![Just(0u32), 257..1_000u32].prop_map(|rows| {
            (
                ViaFarmParams {
                    rows,
                    ..ViaFarmParams::default()
                },
                "rows",
            )
        }),
        prop_oneof![Just(0u32), 257..1_000u32].prop_map(|cols| {
            (
                ViaFarmParams {
                    cols,
                    ..ViaFarmParams::default()
                },
                "cols",
            )
        }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    /// Every valid guard ring is DRC-clean under the committed SKY130 subset.
    #[test]
    fn guard_ring_is_drc_clean(params in valid_guard_ring()) {
        prop_assert!(params.validate().is_ok(), "sampled params must be valid: {params:?}");
        let found = violations(&GuardRing, &params);
        prop_assert!(
            found.is_empty(),
            "guard ring {params:?} produced {} violation(s): {}",
            found.len(),
            describe(&found)
        );
    }

    /// Every valid via farm is DRC-clean under the committed SKY130 subset.
    #[test]
    fn via_farm_is_drc_clean(params in valid_via_farm()) {
        prop_assert!(params.validate().is_ok(), "sampled params must be valid: {params:?}");
        let found = violations(&ViaFarm, &params);
        prop_assert!(
            found.is_empty(),
            "via farm {params:?} produced {} violation(s): {}",
            found.len(),
            describe(&found)
        );
    }

    /// `validate` accepts every in-range guard-ring sample (the positive direction).
    #[test]
    fn guard_ring_validate_accepts_valid(params in valid_guard_ring()) {
        prop_assert!(params.validate().is_ok(), "rejected a valid sample: {params:?}");
    }

    /// `validate` rejects an out-of-range guard-ring sample, naming the bad field
    /// (the negative direction).
    #[test]
    fn guard_ring_validate_rejects_invalid((params, field) in invalid_guard_ring()) {
        match params.validate() {
            Ok(()) => prop_assert!(false, "accepted an invalid sample: {params:?}"),
            Err(GenError::OutOfRange { field: f, .. } | GenError::Invalid { field: f, .. }) => {
                prop_assert_eq!(f, field, "rejection named the wrong field for {:?}", params);
            }
            Err(other) => prop_assert!(false, "unexpected error {other:?} for {params:?}"),
        }
    }

    /// `validate` accepts every in-range via-farm sample (the positive direction).
    #[test]
    fn via_farm_validate_accepts_valid(params in valid_via_farm()) {
        prop_assert!(params.validate().is_ok(), "rejected a valid sample: {params:?}");
    }

    /// `validate` rejects an out-of-range via-farm sample, naming the bad field (the
    /// negative direction).
    #[test]
    fn via_farm_validate_rejects_invalid((params, field) in invalid_via_farm()) {
        match params.validate() {
            Ok(()) => prop_assert!(false, "accepted an invalid sample: {params:?}"),
            Err(GenError::OutOfRange { field: f, .. } | GenError::Invalid { field: f, .. }) => {
                prop_assert_eq!(f, field, "rejection named the wrong field for {:?}", params);
            }
            Err(other) => prop_assert!(false, "unexpected error {other:?} for {params:?}"),
        }
    }

    /// The registry's type-erased path is clean too: driving each built-in generator
    /// from its default JSON parameters produces the same DRC-clean geometry, and the
    /// reported `shapes_added` matches what landed in the cell.
    #[test]
    fn registry_generate_matches_typed(_seed in 0..8u8) {
        let reg = Registry::with_builtins();
        for id in reg.ids() {
            let params = reg.default_params(id).expect("registered generator");
            let mut cell = Cell::new("top");
            let out = reg
                .generate(id, &params, &Technology::default(), &mut cell)
                .expect("default params generate");
            prop_assert_eq!(out.shapes_added, cell.shapes.len());

            let mut doc = Document::new();
            doc.insert_cell(cell);
            let engine = DrcEngine::new(sky130_drc_rules());
            let found = engine.check_cell(&doc, "top");
            prop_assert!(
                found.is_empty(),
                "registry generator {} produced violations: {}",
                id,
                describe(&found)
            );
        }
    }
}
