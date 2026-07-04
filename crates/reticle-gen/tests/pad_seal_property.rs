//! Cleanliness oracle for the pad-ring and seal-ring generators (lane 2B): over
//! randomized valid parameters, each must emit geometry the real SKY130 DRC subset
//! finds *zero* violations in.
//!
//! Like the framework's `tests/property.rs`, the oracle is the production checker, not
//! a reimplementation: each test sweeps random in-range parameters, generates into a
//! fresh cell, loads the committed SKY130 rule subset through
//! [`reticle_drc::sky130_drc_rules`], and runs [`RuleSet::check_cell`] over the
//! result. A single violation on any sampled input fails the property, so "DRC-clean
//! by construction" is proven against the same engine the app runs, over the whole
//! valid parameter space rather than a handful of hand-picked cases, at 400 cases
//! each.
//!
//! Two-directional `validate` coverage lives alongside: for each generator a property
//! asserts that every in-range sample validates, and that out-of-range or
//! contextually-invalid samples are rejected with a [`GenError`] naming the field.

use proptest::prelude::*;
use reticle_drc::{DrcEngine, sky130_drc_rules};
use reticle_gen::{
    GenError, GenParams, Generator, PadRing, PadRingParams, SealRing, SealRingParams, SealStack,
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

// --- Seal ring ---

/// A random *valid* seal-ring parameter set: any stack, a `ring_width` above the
/// stack's real floor, and a die large enough to leave a spacing-clean interior.
fn valid_seal_ring() -> impl Strategy<Value = SealRingParams> {
    let stack = prop_oneof![
        Just(SealStack::Li1Met1),
        Just(SealStack::UpToMet2),
        Just(SealStack::UpToMet3),
    ];
    // ring_width from comfortably above the tallest stack's cut-enclosure floor
    // (via2 200 + 2*65 = 330) up to a wide ring; die large enough for two of those
    // plus a spacing-clean interior, up to a mid-size die.
    (stack, 330..4_000i32, 20_000..400_000i32, 20_000..400_000i32).prop_map(
        |(stack, ring_width, die_width, die_height)| SealRingParams {
            stack,
            die_width,
            die_height,
            ring_width,
        },
    )
}

/// A random seal-ring parameter set that is *invalid* in exactly one way, paired with
/// the field the rejection must name.
fn invalid_seal_ring() -> impl Strategy<Value = (SealRingParams, &'static str)> {
    prop_oneof![
        // ring_width below the RING_MIN field floor (met3 width 300).
        (0..300i32).prop_map(|w| {
            (
                SealRingParams {
                    ring_width: w,
                    ..SealRingParams::default()
                },
                "ring_width",
            )
        }),
        // ring_width in [300, 330): clears the field floor but too thin to enclose the
        // full stack's via2 cut (needs 330).
        (300..330i32).prop_map(|w| {
            (
                SealRingParams {
                    stack: SealStack::UpToMet3,
                    ring_width: w,
                    ..SealRingParams::default()
                },
                "ring_width",
            )
        }),
        // die_width below its field floor (DIE_MIN = 900).
        (0..900i32).prop_map(|d| {
            (
                SealRingParams {
                    die_width: d,
                    ..SealRingParams::default()
                },
                "die_width",
            )
        }),
        // die_height in range but too small to leave a spacing-clean interior after two
        // default ring widths (2*900 + 300 = 2100).
        (900..2_100i32).prop_map(|d| {
            (
                SealRingParams {
                    die_height: d,
                    ..SealRingParams::default()
                },
                "die_height",
            )
        }),
    ]
}

// --- Pad ring ---

/// A random *valid* pad-ring parameter set: a pad size, a pitch derived to clear the
/// met3 spacing, a die large enough for the full row/column topology, and a power-pad
/// count clamped to the pads actually placed.
///
/// The generator's topology (mirrored here) runs full-height left/right columns and
/// interior-width bottom/top rows, with a fixed edge inset of the met3 spacing (300).
/// The binding die floors are therefore `die_width >= 3*pad_size + 1200` (a row pad
/// fits between the columns) and `die_height >= 2*pad_size + 900` (a column pad fits
/// and the two rows clear vertically).
fn valid_pad_ring() -> impl Strategy<Value = PadRingParams> {
    (
        1_000..80_000i32,
        0..40_000i32,
        0..300_000i32,
        0..300_000i32,
        0..64u32,
    )
        .prop_map(|(pad_size, pitch_spare, extra_w, extra_h, want_power)| {
            // Pitch must be at least pad_size + met3 spacing (300).
            let pad_pitch = pad_size + 300 + pitch_spare;
            // The die must fit the ring geometry (3 pads across, 2 down, plus margins)
            // AND clear validate's DIE_MIN range floor (10_000). For a small pad_size,
            // 3*pad_size+1_200 falls below that floor, so clamp up to it; the geometry
            // margin still holds because 10_000 exceeds what a small pad needs.
            let die_width = (3 * pad_size + 1_200 + extra_w).max(10_000);
            let die_height = (2 * pad_size + 900 + extra_h).max(10_000);
            let mut p = PadRingParams {
                die_width,
                die_height,
                pad_pitch,
                pad_size,
                power_pads: 0,
            };
            // Clamp the power-pad request to the pads the geometry actually holds.
            let total = pad_ring_total(&p);
            p.power_pads = want_power.min(total);
            p
        })
}

/// The number of pads a valid pad-ring parameter set places, recomputed here so the
/// strategy can clamp `power_pads` without reaching into crate internals: full-height
/// left/right columns plus interior-width bottom/top rows, with the met3-spacing edge
/// inset the generator uses.
fn pad_ring_total(p: &PadRingParams) -> u32 {
    const INSET: i32 = 300; // met3 spacing
    const SPACING: i32 = 300; // met3 spacing
    let fit = |span: i64| -> u32 {
        if span < i64::from(p.pad_size) {
            0
        } else {
            (1 + (span - i64::from(p.pad_size)) / i64::from(p.pad_pitch)) as u32
        }
    };
    let col_span = i64::from(p.die_height) - 2 * i64::from(INSET);
    let side = i64::from(INSET) + i64::from(p.pad_size) + i64::from(SPACING);
    let row_span = i64::from(p.die_width) - 2 * side;
    fit(col_span) * 2 + fit(row_span) * 2
}

/// A random pad-ring parameter set that is *invalid* in exactly one way, paired with
/// the field the rejection must name.
fn invalid_pad_ring() -> impl Strategy<Value = (PadRingParams, &'static str)> {
    prop_oneof![
        // pad_size below the field floor (met3 width 300).
        (0..300i32).prop_map(|s| {
            (
                PadRingParams {
                    pad_size: s,
                    ..PadRingParams::default()
                },
                "pad_size",
            )
        }),
        // pad_pitch in range but below pad_size + met3 spacing for the default pad.
        (600..60_300i32).prop_map(|pitch| {
            (
                PadRingParams {
                    pad_size: 60_000,
                    pad_pitch: pitch,
                    ..PadRingParams::default()
                },
                "pad_pitch",
            )
        }),
        // die_width too small to fit a pad between the corner keep-outs. The default
        // pad is 60_000, so keep-outs alone (2 * 60_300) exceed anything below 120_600.
        (10_000..120_000i32).prop_map(|d| {
            (
                PadRingParams {
                    die_width: d,
                    ..PadRingParams::default()
                },
                "die_width",
            )
        }),
        // more power pads than the default ring can place.
        Just((
            PadRingParams {
                power_pads: 4_000,
                ..PadRingParams::default()
            },
            "power_pads",
        )),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    /// Every valid seal ring is DRC-clean under the committed SKY130 subset.
    #[test]
    fn seal_ring_is_drc_clean(params in valid_seal_ring()) {
        prop_assert!(params.validate().is_ok(), "sampled params must be valid: {params:?}");
        let found = violations(&SealRing, &params);
        prop_assert!(
            found.is_empty(),
            "seal ring {params:?} produced {} violation(s): {}",
            found.len(),
            describe(&found)
        );
    }

    /// Every valid pad ring is DRC-clean under the committed SKY130 subset.
    #[test]
    fn pad_ring_is_drc_clean(params in valid_pad_ring()) {
        prop_assert!(params.validate().is_ok(), "sampled params must be valid: {params:?}");
        let found = violations(&PadRing, &params);
        prop_assert!(
            found.is_empty(),
            "pad ring {params:?} produced {} violation(s): {}",
            found.len(),
            describe(&found)
        );
    }

    /// `validate` accepts every in-range seal-ring sample (the positive direction).
    #[test]
    fn seal_ring_validate_accepts_valid(params in valid_seal_ring()) {
        prop_assert!(params.validate().is_ok(), "rejected a valid sample: {params:?}");
    }

    /// `validate` rejects an out-of-range or contextually-invalid seal-ring sample,
    /// naming the bad field (the negative direction).
    #[test]
    fn seal_ring_validate_rejects_invalid((params, field) in invalid_seal_ring()) {
        match params.validate() {
            Ok(()) => prop_assert!(false, "accepted an invalid sample: {params:?}"),
            Err(GenError::OutOfRange { field: f, .. } | GenError::Invalid { field: f, .. }) => {
                prop_assert_eq!(f, field, "rejection named the wrong field for {:?}", params);
            }
            Err(other) => prop_assert!(false, "unexpected error {other:?} for {params:?}"),
        }
    }

    /// `validate` accepts every in-range pad-ring sample (the positive direction).
    #[test]
    fn pad_ring_validate_accepts_valid(params in valid_pad_ring()) {
        prop_assert!(params.validate().is_ok(), "rejected a valid sample: {params:?}");
    }

    /// `validate` rejects an out-of-range or contextually-invalid pad-ring sample,
    /// naming the bad field (the negative direction).
    #[test]
    fn pad_ring_validate_rejects_invalid((params, field) in invalid_pad_ring()) {
        match params.validate() {
            Ok(()) => prop_assert!(false, "accepted an invalid sample: {params:?}"),
            Err(GenError::OutOfRange { field: f, .. } | GenError::Invalid { field: f, .. }) => {
                prop_assert_eq!(f, field, "rejection named the wrong field for {:?}", params);
            }
            Err(other) => prop_assert!(false, "unexpected error {other:?} for {params:?}"),
        }
    }
}
