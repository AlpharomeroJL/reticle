//! Cleanliness oracle for the fill and test-structure generators: over randomized
//! valid parameters, every generated cell must be *zero*-violation under the real
//! SKY130 DRC subset, and the fill's honest density and keep-out promises must hold.
//!
//! Like the sibling `property.rs`, the oracle is the production checker, not a
//! reimplementation: each test sweeps random in-range parameters, generates into a
//! fresh cell, loads the committed rule subset through [`reticle_drc::sky130_drc_rules`],
//! and runs [`RuleSet::check_cell`]. A single violation on any sampled input fails the
//! property, so "DRC-clean by construction" is proven against the same engine the app
//! runs, across the whole valid parameter space.
//!
//! Beyond cleanliness, the fill properties assert the two honesty promises the
//! generator makes: no tile ever lands in (or touching) a keep-out, and the achieved
//! coverage density tracks the target within a two-sided tolerance band (it approaches,
//! it does not claim to hit it exactly) when the region is large and unobstructed.
//! Two-directional `validate` coverage for both generators lives alongside.

use proptest::prelude::*;
use reticle_drc::{DrcEngine, sky130_drc_rules};
use reticle_gen::{
    FillGen, FillLayer, FillParams, GenError, GenParams, Generator, KeepOut, StructureKind,
    StructureLayer, TestStructure, TestStructureParams,
};
use reticle_model::{Cell, Document, RuleSet, ShapeKind, Technology};

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

// --- Fill ---

/// A random *valid* fill parameter set with a handful of keep-outs inside the region:
/// any layer, a region comfortably larger than the tile, an in-range tile at or above
/// the per-layer floor, and an in-range target density.
fn valid_fill() -> impl Strategy<Value = FillParams> {
    let layer = prop_oneof![
        Just(FillLayer::Li1),
        Just(FillLayer::Met1),
        Just(FillLayer::Met2),
        Just(FillLayer::Met3),
    ];
    // Tile at or above the coarsest layer's area/width floor (met3 width 300, li1 area
    // side 237), so the sample is valid on every layer; region several tiles wide.
    (
        layer,
        400..2_000i32,    // tile
        6_000..20_000i32, // region_width
        6_000..20_000i32, // region_height
        1..900i32,        // target density permille
        0..4usize,        // keep-out count
        any::<u64>(),     // keep-out placement seed
    )
        .prop_map(
            |(layer, tile, region_width, region_height, density, n_keepouts, seed)| {
                let keepouts = keepouts_in_region(region_width, region_height, n_keepouts, seed);
                FillParams {
                    layer,
                    region_width,
                    region_height,
                    tile,
                    target_density_permille: density,
                    keepouts,
                }
            },
        )
}

/// Deterministically scatters `count` non-degenerate keep-outs wholly inside a
/// `region_w` by `region_h` region, from a seed, so a sampled fill has realistic
/// blockages to route around.
fn keepouts_in_region(region_w: i32, region_h: i32, count: usize, seed: u64) -> Vec<KeepOut> {
    let mut state = seed;
    let mut next = || {
        // A tiny xorshift so the placement is reproducible without an rng dependency.
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let width = 500 + (next() % 1_500) as i32;
        let height = 500 + (next() % 1_500) as i32;
        // Keep the whole keep-out inside [0, region_w) x [0, region_h).
        let max_x = (region_w - width).max(1);
        let max_y = (region_h - height).max(1);
        let x = (next() % max_x as u64) as i32;
        let y = (next() % max_y as u64) as i32;
        out.push(KeepOut {
            x,
            y,
            width,
            height,
        });
    }
    out
}

/// A random fill parameter set *invalid* in exactly one way, paired with the field the
/// rejection must name.
fn invalid_fill() -> impl Strategy<Value = (FillParams, &'static str)> {
    prop_oneof![
        // tile below the met3 width floor (300).
        (0..300i32).prop_map(|tile| {
            (
                FillParams {
                    layer: FillLayer::Met3,
                    tile,
                    keepouts: Vec::new(),
                    ..FillParams::default()
                },
                "tile",
            )
        }),
        // region_width below tile + min spacing on li1 (400 + 170 = 570).
        (0..570i32).prop_map(|w| {
            (
                FillParams {
                    layer: FillLayer::Li1,
                    region_width: w,
                    keepouts: Vec::new(),
                    ..FillParams::default()
                },
                "region_width",
            )
        }),
        // density out of range (above 900).
        (901..5_000i32).prop_map(|d| {
            (
                FillParams {
                    target_density_permille: d,
                    keepouts: Vec::new(),
                    ..FillParams::default()
                },
                "target_density_permille",
            )
        }),
        // a degenerate keep-out (zero width).
        Just((
            FillParams {
                keepouts: vec![KeepOut {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 100,
                }],
                ..FillParams::default()
            },
            "keepouts",
        )),
    ]
}

/// A random *valid* fill parameter set with **no** keep-outs and a region that is a
/// large multiple of the tile, so edge-clipping loss is small and the achieved density
/// should track the target within tolerance.
fn valid_fill_no_keepout() -> impl Strategy<Value = FillParams> {
    let layer = prop_oneof![
        Just(FillLayer::Li1),
        Just(FillLayer::Met1),
        Just(FillLayer::Met2),
        Just(FillLayer::Met3),
    ];
    (layer, 400..800i32, 100..700i32).prop_map(|(layer, tile, density)| {
        // A region ~60 tiles across, so the whole-tile count divides the target closely
        // and the finite-region rounding error stays within the tolerance band.
        let region = tile * 60;
        FillParams {
            layer,
            region_width: region,
            region_height: region,
            tile,
            target_density_permille: density,
            keepouts: Vec::new(),
        }
    })
}

// --- Test structures ---

/// A random *valid* test-structure parameter set: any kind, any single-layer choice, an
/// in-range width at or above the coarsest floor, a length long enough for the area
/// rule on every layer, and an in-range count.
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
    // width >= 300 (met3 floor); count in range. Length is derived to clear every
    // per-layer, per-kind floor at once: at least the area floor and, for the
    // serpentine, `2*width + min_spacing` (spacing at most 300 on met3). Adding a
    // random slack on top of `2*width + 300` keeps every sampled structure valid.
    (kind, layer, 300..1_500i32, 400..4_000i32, 2..40u32).prop_map(
        |(kind, layer, feature_width, slack, count)| {
            let feature_length = 2 * feature_width + 300 + slack;
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

/// A random test-structure parameter set *invalid* in exactly one way, paired with the
/// field the rejection must name.
fn invalid_test_structure() -> impl Strategy<Value = (TestStructureParams, &'static str)> {
    prop_oneof![
        // feature_width below the met3 width floor (300).
        (0..300i32).prop_map(|w| {
            (
                TestStructureParams {
                    layer: StructureLayer::Met3,
                    feature_width: w,
                    ..TestStructureParams::default()
                },
                "feature_width",
            )
        }),
        // feature_length below the area-derived floor on met1 at width 300 (277); use
        // 0..277 which is short for met1's 83000 area.
        (0..277i32).prop_map(|len| {
            (
                TestStructureParams {
                    kind: StructureKind::Serpentine,
                    layer: StructureLayer::Met1,
                    feature_width: 300,
                    feature_length: len,
                    ..TestStructureParams::default()
                },
                "feature_length",
            )
        }),
        // count out of range (above 128).
        prop_oneof![Just(0u32), Just(1u32), 129..1_000u32].prop_map(|count| {
            (
                TestStructureParams {
                    kind: StructureKind::Comb,
                    count,
                    ..TestStructureParams::default()
                },
                "count",
            )
        }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    /// Every valid fill (with keep-outs) is DRC-clean under the committed SKY130
    /// subset, and no tile lands in or touching a keep-out.
    #[test]
    fn fill_is_drc_clean(params in valid_fill()) {
        prop_assert!(params.validate().is_ok(), "sampled params must be valid: {params:?}");

        // Generate once and inspect the tiles for both cleanliness and keep-out respect.
        let mut cell = Cell::new("top");
        FillGen
            .generate(&params, &Technology::default(), &mut cell)
            .expect("valid params generate");

        // No tile overlaps or touches any keep-out.
        for shape in &cell.shapes {
            let ShapeKind::Rect(tile) = shape.kind else {
                prop_assert!(false, "fill emits only rects");
                unreachable!()
            };
            for k in &params.keepouts {
                let block = reticle_geometry::Rect::new(
                    reticle_geometry::Point::new(k.x, k.y),
                    reticle_geometry::Point::new(k.x + k.width, k.y + k.height),
                );
                let touches = tile.min.x <= block.max.x
                    && block.min.x <= tile.max.x
                    && tile.min.y <= block.max.y
                    && block.min.y <= tile.max.y;
                prop_assert!(!touches, "tile {tile:?} intrudes on keep-out {block:?}");
            }
        }

        // DRC-clean under the real engine.
        let mut doc = Document::new();
        doc.insert_cell(cell);
        let engine = DrcEngine::new(sky130_drc_rules());
        let found = engine.check_cell(&doc, "top");
        prop_assert!(
            found.is_empty(),
            "fill {params:?} produced {} violation(s): {}",
            found.len(),
            describe(&found)
        );
    }

    /// With a large, unobstructed region the achieved fill density tracks the target
    /// within a tolerance band on both sides (it approaches, never claims to be exact),
    /// capped by the min-spacing ceiling.
    #[test]
    fn fill_density_is_honest(params in valid_fill_no_keepout()) {
        let target = i64::from(params.target_density_permille);
        let achieved = FillGen::achieved_density_permille(&params);

        // The min-spacing pitch caps how dense fill can get; the reachable target is the
        // requested value clamped to that ceiling.
        let cond_spacing = match params.layer {
            FillLayer::Li1 => 170,
            FillLayer::Met1 | FillLayer::Met2 => 140,
            FillLayer::Met3 => 300,
        };
        let min_pitch = params.tile + cond_spacing;
        // Ceiling density in permille: tile^2 / min_pitch^2 * 1000.
        let ceiling = (i64::from(params.tile) * i64::from(params.tile) * 1000)
            / (i64::from(min_pitch) * i64::from(min_pitch));
        let reachable = target.min(ceiling);

        // Two-sided tolerance for whole-DBU pitch rounding over a finite region and the
        // few dropped edge tiles. Over a 60-tile region the count-rounding error is well
        // within ~12%; the floor covers very low targets.
        let tolerance = (reachable * 12 / 100).max(25);
        prop_assert!(
            (achieved - reachable).abs() <= tolerance,
            "achieved {achieved} outside tolerance {tolerance} of reachable {reachable} for {params:?}"
        );
    }

    /// `validate` accepts every in-range fill sample (the positive direction).
    #[test]
    fn fill_validate_accepts_valid(params in valid_fill()) {
        prop_assert!(params.validate().is_ok(), "rejected a valid sample: {params:?}");
    }

    /// `validate` rejects a fill sample that is invalid in one way, naming the bad
    /// field (the negative direction).
    #[test]
    fn fill_validate_rejects_invalid((params, field) in invalid_fill()) {
        match params.validate() {
            Ok(()) => prop_assert!(false, "accepted an invalid sample: {params:?}"),
            Err(GenError::OutOfRange { field: f, .. } | GenError::Invalid { field: f, .. }) => {
                prop_assert_eq!(f, field, "rejection named the wrong field for {:?}", params);
            }
            Err(other) => prop_assert!(false, "unexpected error {other:?} for {params:?}"),
        }
    }

    /// Every valid test structure (all four kinds) is DRC-clean under the committed
    /// SKY130 subset.
    #[test]
    fn test_structure_is_drc_clean(params in valid_test_structure()) {
        prop_assert!(params.validate().is_ok(), "sampled params must be valid: {params:?}");
        let found = violations(&TestStructure, &params);
        prop_assert!(
            found.is_empty(),
            "test structure {params:?} produced {} violation(s): {}",
            found.len(),
            describe(&found)
        );
    }

    /// `validate` accepts every in-range test-structure sample (the positive
    /// direction).
    #[test]
    fn test_structure_validate_accepts_valid(params in valid_test_structure()) {
        prop_assert!(params.validate().is_ok(), "rejected a valid sample: {params:?}");
    }

    /// `validate` rejects a test-structure sample that is invalid in one way, naming
    /// the bad field (the negative direction).
    #[test]
    fn test_structure_validate_rejects_invalid((params, field) in invalid_test_structure()) {
        match params.validate() {
            Ok(()) => prop_assert!(false, "accepted an invalid sample: {params:?}"),
            Err(GenError::OutOfRange { field: f, .. } | GenError::Invalid { field: f, .. }) => {
                prop_assert_eq!(f, field, "rejection named the wrong field for {:?}", params);
            }
            Err(other) => prop_assert!(false, "unexpected error {other:?} for {params:?}"),
        }
    }
}
