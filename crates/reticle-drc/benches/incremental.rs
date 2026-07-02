//! Criterion benchmarks for design-rule checking latency.
//!
//! The headline number is the incremental edit-to-recheck path: after a local
//! edit an interactive layout editor re-runs DRC only over the touched region via
//! [`DrcEngine::check_region`], and that must return well inside the 100ms budget
//! that keeps checking feeling instantaneous while typing geometry. For contrast
//! the full-cell pass ([`RuleSet::check_cell`]) over the same layout is measured
//! too, so the speedup of the incremental path over a from-scratch check is
//! visible. Numbers are produced by Criterion at run time
//! (`cargo bench -p reticle-drc --bench incremental`); nothing here is hard-coded.

use criterion::{Criterion, criterion_group, criterion_main};
use reticle_drc::DrcEngine;
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, Document, DrawShape, Rule, RuleKind, RuleSet, ShapeKind};

const METAL1: LayerId = LayerId::new(1, 0);
const METAL2: LayerId = LayerId::new(2, 0);

/// Side length of the square DBU window the layout is scattered across. Chosen so
/// that a ~100k-rectangle fill leaves shapes close enough for spacing candidates
/// to be found without the whole layout collapsing into one overlapping blob.
const EXTENT: i32 = 400_000;

/// A tiny deterministic xorshift PRNG so benches are reproducible without pulling
/// in a `rand` dependency (same pattern as the reticle-index benches).
struct XorShift(u64);

impl XorShift {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A coordinate in roughly `[0, EXTENT)` DBU.
    fn coord(&mut self) -> i32 {
        (self.next_u64() % EXTENT as u64) as i32
    }
}

/// Builds a flat cell named `top` filled with `count` small rectangles spread over
/// two layers, the kind of geometry a real metal-fill region looks like to DRC.
///
/// Roughly half the shapes land on each of `METAL1` and `METAL2`; each is a small
/// axis-aligned box a few hundred DBU on a side, so with the rule thresholds below
/// most shapes are clean but a scattering of narrow or tight ones exercise every
/// checker.
fn build_cell(count: usize) -> Document {
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    let mut cell = Cell::new("top");
    cell.shapes.reserve(count);
    for i in 0..count {
        let x = rng.coord();
        let y = rng.coord();
        // Widths/heights in [20, 320): mostly above the width minimum, some below.
        let w = (rng.next_u64() % 300 + 20) as i32;
        let h = (rng.next_u64() % 300 + 20) as i32;
        let layer = if i % 2 == 0 { METAL1 } else { METAL2 };
        cell.shapes.push(DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(x, y), Point::new(x + w, y + h))),
        ));
    }
    let mut doc = Document::new();
    doc.insert_cell(cell);
    doc
}

/// The rule set under benchmark: a minimum width, a same-layer spacing, and a
/// minimum area on `METAL1`, matching the checker mix a technology file drives.
fn rules() -> Vec<Rule> {
    vec![
        Rule {
            name: "m1.width".into(),
            kind: RuleKind::Width,
            layer: METAL1,
            other_layer: None,
            value: 30,
        },
        Rule {
            name: "m1.space".into(),
            kind: RuleKind::Spacing,
            layer: METAL1,
            other_layer: None,
            value: 40,
        },
        Rule {
            name: "m1.area".into(),
            kind: RuleKind::Area,
            layer: METAL1,
            other_layer: None,
            value: 2_500,
        },
    ]
}

/// Benchmarks the full pass and the incremental region re-check at one cell size.
///
/// `label` distinguishes the sizes in the report (for example `100k` / `1m`).
fn bench_size(c: &mut Criterion, label: &str, count: usize) {
    let doc = build_cell(count);
    let engine = DrcEngine::new(rules());

    // (a) Full-cell pass: what an editor pays if it re-checks everything on each edit.
    c.bench_function(&format!("drc_full_cell_{label}"), |b| {
        b.iter(|| std::hint::black_box(engine.check_cell(std::hint::black_box(&doc), "top")));
    });

    // (b) Incremental re-check over a small edited rectangle near the middle of the
    // layout: the region an interactive edit dirties. This is the headline latency.
    let edit = Rect::new(
        Point::new(EXTENT / 2, EXTENT / 2),
        Point::new(EXTENT / 2 + 500, EXTENT / 2 + 500),
    );
    c.bench_function(&format!("drc_incremental_region_{label}"), |b| {
        b.iter(|| {
            std::hint::black_box(engine.check_region(
                std::hint::black_box(&doc),
                "top",
                std::hint::black_box(edit),
            ))
        });
    });
}

fn bench_drc(c: &mut Criterion) {
    // ~100k rectangles: a large but routine flat editing region.
    bench_size(c, "100k", 100_000);
    // ~1M rectangles: a stress size, to show the incremental path stays flat while
    // the full pass grows with the cell.
    bench_size(c, "1m", 1_000_000);
}

criterion_group!(benches, bench_drc);
criterion_main!(benches);
