//! Property test for the compute-shader DRC heatmap ([`DrcHeatmap`]) against the CPU
//! [`DrcEngine`] oracle.
//!
//! Over randomized rectangle layouts, the set of instances the GPU flags must equal the
//! set of instances the CPU design-rule engine reports as violating, for a width rule
//! and a spacing rule evaluated together. The CPU engine is authoritative: this test
//! never re-implements its geometry, it only *attributes* each [`Violation`] back to the
//! instance(s) that produced it, using the violation's reported `location`:
//!
//! * a **width** violation's location is exactly the offending instance's bounding box,
//!   so it maps to a single instance;
//! * a **spacing** violation's location is the union of the offending pair's boxes, so
//!   it maps to the two instances whose union equals it.
//!
//! Both maps are made unambiguous by rejecting (via [`prop_assume!`]) the rare generated
//! layout where two instances share a bounding box or two distinct pairs share a union
//! rectangle. This keeps the attribution a pure lookup with no geometry of its own.
//!
//! # Coordinate constraint
//!
//! Every generated coordinate, width, and height is a positive integer well below
//! `2^24`, the range over which `f32` represents consecutive integers exactly, so the
//! `f32` compute path is bit-exact against the integer CPU engine (see the module docs
//! on [`reticle_render::DrcHeatmap`]). Placements are the identity transform: the
//! instance geometry lives entirely in `min_xy`/`max_xy`, so each instance's GPU world
//! box equals the CPU shape's bounding box directly. The shader's orientation,
//! magnification, and translation paths are exercised by the `drc_heatmap` unit tests
//! rather than this property test.
//!
//! Skips (and passes) without a usable GPU adapter, mirroring the other GPU tests and
//! ADR 0027, so it is safe in CI. The GPU context and pipeline are built once and shared
//! across every generated case.

use std::collections::{BTreeSet, HashMap};

use proptest::prelude::*;
use reticle_drc::DrcEngine;
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, Document, DrawShape, Rule, RuleKind, RuleSet, ShapeKind};
use reticle_render::{DrcHeatmap, DrcRules, RectInstanceT, WgpuContext};

/// The single layer every generated rectangle lives on.
const LAYER: LayerId = LayerId::new(0, 0);
/// The cell the oracle checks.
const CELL: &str = "top";

/// Extent of the world the min corners are scattered across, in DBU. Small enough
/// (relative to the rule and size ranges) that close pairs, touching pairs, overlaps,
/// and far-apart pairs are all well sampled.
const SPAN: i32 = 120;
/// Largest generated instance side, in DBU.
const MAX_SIZE: i32 = 20;
/// Largest rule threshold (width and spacing), in DBU.
const RULE_MAX: i64 = 25;
/// Largest instance count per layout.
const MAX_N: usize = 20;

/// Builds the width+spacing oracle and returns the set of instance indices it flags,
/// or `None` if the layout is ambiguous to attribute (duplicate bbox or union), in
/// which case the caller rejects the case.
///
/// `rects[i]` is instance `i`'s world box; index alignment with the GPU instance list
/// is what makes the two flag sets directly comparable.
fn oracle_flags(rects: &[Rect], min_width: i64, min_spacing: i64) -> Option<BTreeSet<u32>> {
    // A width violation reports the offending box; map each box back to its instance.
    let mut bbox_index: HashMap<Rect, u32> = HashMap::with_capacity(rects.len());
    for (i, r) in rects.iter().enumerate() {
        if bbox_index.insert(*r, i as u32).is_some() {
            return None; // two instances share a box: width attribution is ambiguous
        }
    }
    // A spacing violation reports the union of the offending pair; map each union back
    // to its pair. Distinct unions across all pairs make this a bijection.
    let mut union_pair: HashMap<Rect, (u32, u32)> = HashMap::new();
    for i in 0..rects.len() {
        for j in (i + 1)..rects.len() {
            let u = rects[i].union(&rects[j]);
            if union_pair.insert(u, (i as u32, j as u32)).is_some() {
                return None; // two pairs share a union: spacing attribution is ambiguous
            }
        }
    }

    let mut cell = Cell::new(CELL);
    for r in rects {
        cell.shapes.push(DrawShape::new(LAYER, ShapeKind::Rect(*r)));
    }
    let mut doc = Document::new();
    doc.insert_cell(cell);

    let engine = DrcEngine::new(vec![
        Rule {
            name: "width".into(),
            kind: RuleKind::Width,
            layer: LAYER,
            other_layer: None,
            value: min_width,
        },
        Rule {
            name: "spacing".into(),
            kind: RuleKind::Spacing,
            layer: LAYER,
            other_layer: None,
            value: min_spacing,
        },
    ]);

    let mut flagged = BTreeSet::new();
    for v in engine.check_cell(&doc, CELL) {
        match v.kind {
            RuleKind::Width => {
                let i = *bbox_index
                    .get(&v.location)
                    .expect("a width violation's location is an instance box");
                flagged.insert(i);
            }
            RuleKind::Spacing => {
                let (a, b) = *union_pair
                    .get(&v.location)
                    .expect("a spacing violation's location is a pair union");
                flagged.insert(a);
                flagged.insert(b);
            }
            other => unreachable!("unexpected rule kind {other:?} from a width+spacing engine"),
        }
    }
    Some(flagged)
}

/// An identity-placement instance carrying its whole geometry in `min_xy`/`max_xy`.
fn instance(r: &Rect) -> RectInstanceT {
    RectInstanceT {
        min_xy: [r.min.x as f32, r.min.y as f32],
        max_xy: [r.max.x as f32, r.max.y as f32],
        color: [1.0, 1.0, 1.0, 1.0],
        orientation_code: 0,
        magnification: 1.0,
        translate: [0, 0],
    }
}

/// The set of instance indices the GPU flagged (flag != 0).
fn gpu_flag_set(flags: &[u32]) -> BTreeSet<u32> {
    flags
        .iter()
        .enumerate()
        .filter(|&(_, &f)| f != 0)
        .map(|(i, _)| i as u32)
        .collect()
}

#[test]
fn gpu_flags_match_cpu_oracle() {
    let Some(ctx) = WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    let heatmap = DrcHeatmap::new(&ctx);

    // A layout is a list of (min_x, min_y, width, height) tuples plus the two rule
    // thresholds. Sizes are strictly positive so every box is non-degenerate.
    let strategy = (
        proptest::collection::vec(
            (0i32..SPAN, 0i32..SPAN, 1i32..=MAX_SIZE, 1i32..=MAX_SIZE),
            1..=MAX_N,
        ),
        1i64..=RULE_MAX,
        1i64..=RULE_MAX,
    );

    // GPU work per case is real (four dispatches + two readbacks), so cap the case
    // count to keep the test brisk while still covering a broad range of layouts.
    let mut runner = proptest::test_runner::TestRunner::new(proptest::test_runner::Config {
        cases: 48,
        ..proptest::test_runner::Config::default()
    });

    runner
        .run(&strategy, |(boxes, min_width, min_spacing)| {
            let rects: Vec<Rect> = boxes
                .iter()
                .map(|&(x, y, w, h)| Rect::new(Point::new(x, y), Point::new(x + w, y + h)))
                .collect();

            let Some(expected) = oracle_flags(&rects, min_width, min_spacing) else {
                return Err(TestCaseError::reject(
                    "ambiguous layout (duplicate box or union)",
                ));
            };

            let instances: Vec<RectInstanceT> = rects.iter().map(instance).collect();
            let rules = DrcRules {
                min_width: min_width as u32,
                min_spacing: min_spacing as u32,
            };
            let out = heatmap.run(&ctx, &instances, rules);
            let got = gpu_flag_set(&heatmap.read_flags(&ctx, &out));

            prop_assert_eq!(
                &got,
                &expected,
                "GPU flag set must equal the CPU oracle's violating-instance set"
            );

            // Every flagged instance adds exactly one to its bin, so the heatmap total
            // is the flagged count.
            let heat_total: u32 = heatmap.read_heatmap(&ctx, &out).iter().sum();
            prop_assert_eq!(
                heat_total as usize,
                got.len(),
                "heatmap total must equal the flagged-instance count"
            );
            Ok(())
        })
        .expect("GPU DRC flags match the CPU oracle for all generated layouts");
}

/// Fixed layouts the random search may under-sample: empty input, a lone under-width
/// instance, a too-close pair (spacing), and a comfortably-separated pair (clean).
#[test]
fn gpu_flags_edge_cases() {
    let Some(ctx) = WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    let heatmap = DrcHeatmap::new(&ctx);

    let run = |rects: &[Rect], min_width: i64, min_spacing: i64| {
        let instances: Vec<RectInstanceT> = rects.iter().map(instance).collect();
        let out = heatmap.run(
            &ctx,
            &instances,
            DrcRules {
                min_width: min_width as u32,
                min_spacing: min_spacing as u32,
            },
        );
        gpu_flag_set(&heatmap.read_flags(&ctx, &out))
    };
    let rect = |x0, y0, x1, y1| Rect::new(Point::new(x0, y0), Point::new(x1, y1));

    // Empty input: nothing flagged, and no dispatch.
    assert!(run(&[], 10, 10).is_empty());

    // A single 4-wide instance under a width rule of 10 is flagged; under 3 it is not.
    let thin = [rect(0, 0, 4, 100)];
    assert_eq!(run(&thin, 10, 1), BTreeSet::from([0]));
    assert!(run(&thin, 3, 1).is_empty());

    // Two wide boxes 5 apart: a spacing rule of 10 flags both; a rule of 5 does not
    // (the gap equals the rule, and the check is strictly-less-than).
    let pair = [rect(0, 0, 20, 20), rect(25, 0, 45, 20)];
    assert_eq!(run(&pair, 1, 10), BTreeSet::from([0, 1]));
    assert!(run(&pair, 1, 5).is_empty());

    // The same two boxes moved far apart: no spacing violation at any modest rule.
    let far = [rect(0, 0, 20, 20), rect(1000, 0, 1020, 20)];
    assert!(run(&far, 1, 25).is_empty());
}

/// A deterministic jittered grid of `n` small instances, dense enough that the spacing
/// and width rules both bite, spread so the 16x16 bins stay balanced. No RNG so the
/// benchmark reproduces run to run.
fn deterministic_layout(n: usize) -> Vec<RectInstanceT> {
    let cols = (n as f64).sqrt().ceil() as i32;
    // Pitch exceeds the spacing rule so most neighbours are clean; per-instance cost is
    // the fixed 3x3 neighbour scan (about n/256 instances per bin), not the hit rate.
    let pitch = 20;
    (0..n)
        .map(|idx| {
            let col = idx as i32 % cols;
            let row = idx as i32 / cols;
            let jitter_x = (idx.wrapping_mul(2_654_435_761) % 4) as i32;
            let jitter_y = (idx.wrapping_mul(40_503) % 4) as i32;
            let x = col * pitch + jitter_x;
            let y = row * pitch + jitter_y;
            let box_w = 3 + (idx % 5) as i32;
            let box_h = 3 + (idx % 3) as i32;
            instance(&Rect::new(
                Point::new(x, y),
                Point::new(x + box_w, y + box_h),
            ))
        })
        .collect()
}

/// A rough native throughput measurement for the DRC heatmap, ignored by default.
///
/// Run with:
/// `cargo test -p reticle-render --test drc_heatmap_gpu -- --ignored --nocapture`
///
/// Each timed iteration is one full heatmap recompute (bin count/scan/scatter + check)
/// followed by the flag readback that drives the GPU work to completion, so the figure
/// includes a blocking CPU readback a live overlay redraw (which keeps the heatmap
/// GPU-resident) would skip. Counts are post-cull *visible* instances: the grid is
/// capped at 256 bins, so this targets thousands-to-tens-of-thousands, not whole 1M
/// designs.
#[test]
#[ignore = "benchmark; run explicitly with --ignored --nocapture"]
fn bench_drc_heatmap() {
    let Some(ctx) = WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping benchmark");
        return;
    };
    let heatmap = DrcHeatmap::new(&ctx);
    let rules = DrcRules {
        min_width: 4,
        min_spacing: 8,
    };

    for &n in &[10_000usize, 50_000] {
        let instances = deterministic_layout(n);

        for _ in 0..3 {
            let out = heatmap.run(&ctx, &instances, rules);
            std::hint::black_box(heatmap.read_flags(&ctx, &out));
        }

        let iters = 30;
        let mut times = Vec::with_capacity(iters);
        for _ in 0..iters {
            let start = std::time::Instant::now();
            let out = heatmap.run(&ctx, &instances, rules);
            let flags = heatmap.read_flags(&ctx, &out);
            times.push(start.elapsed());
            std::hint::black_box(flags);
        }
        times.sort_unstable();
        let median = times[iters / 2].as_secs_f64() * 1e3;
        let flagged = {
            let out = heatmap.run(&ctx, &instances, rules);
            heatmap
                .read_flags(&ctx, &out)
                .iter()
                .filter(|&&f| f != 0)
                .count()
        };
        eprintln!(
            "DRC heatmap: {n} instances -> median {median:.3} ms/recompute ({flagged} flagged, incl. readback)"
        );
    }
}
